//! A run advances through explicit phases; durable evidence is published per workbook.

use super::{capture, pairing::{self, WorkbookPair}, storage::{self, CacheLock, StageManifest, WorkDirectory}, Output, RunConfig};
use crate::{validation_process, validation_report, validation_reuse::{file_sha256, PairFingerprint}, validation_sources::{self, MaterializedSource}};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, VecDeque}, fs, io::Write, path::{Path, PathBuf}, sync::{Arc, Mutex}};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunState {
    #[default]
    Idle,
    Preparing,
    Processing,
    Pairing,
    Validating,
    Publishing,
    Completed,
    Failed,
}

impl RunState {
    fn permits(self, next: Self) -> bool {
        use RunState::*;
        matches!((self, next), (Idle, Preparing) | (Preparing, Processing | Pairing)
            | (Processing, Processing | Pairing) | (Pairing, Validating)
            | (Validating, Publishing) | (Publishing, Completed)
            | (Preparing | Processing | Pairing | Validating | Publishing, Failed))
    }
}

#[derive(Clone, Default, Serialize)]
pub(crate) struct LiveProgress {
    pub state: RunState,
    pub report_dir: Option<PathBuf>,
    pub current: Option<String>,
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub reused: usize,
    pub error: Option<String>,
    pub recent: VecDeque<Value>,
    pub different_pairs: usize,
    pub checked_equal_pairs: usize,
    pub processing: Option<super::ProcessingProgress>,
    #[serde(skip)]
    pub workbooks: BTreeMap<String, Value>,
    #[serde(skip)]
    pub comparisons: VecDeque<Value>,
    #[serde(skip)]
    pub telemetry_path: Option<PathBuf>,
}

type Progress = Arc<Mutex<LiveProgress>>;

struct Run {
    config: RunConfig,
    root: PathBuf,
    work: WorkDirectory,
    progress: Progress,
    state: RunState,
    header: Value,
    previous: Option<PreviousRun>,
    processing_failures: usize,
    cleanup_unconfirmed: bool,
}

struct PreviousRun {
    root: PathBuf,
    pairs: BTreeMap<String, Value>,
}

/// The run's expensive work is never owned by a browser request or polling connection.
pub(crate) fn execute(mut config: RunConfig, live: Option<Progress>) -> Result<Value> {
    config.resolve_paths(&std::env::current_dir()?)?;
    config.validate()?;
    let mut run = Run::prepare(config, live.unwrap_or_default())?;
    let result = run.perform();
    run.finish(result)
}

impl Run {
    fn prepare(config: RunConfig, progress: Progress) -> Result<Self> {
        let root = storage::destination(&config.report_dir)?;
        let signature = capture::signature(&config.capture, &config.environment_key)?;
        let previous = PreviousRun::load(config.reuse_from.as_deref(), &signature)?;
        fs::create_dir_all(root.parent().context("Report directory has no parent")?)?;
        fs::create_dir(&root)?;
        for directory in ["records", "identities", "pairs", "logs"] { fs::create_dir(root.join(directory))?; }
        let work = WorkDirectory::create(&root)?;
        let header = json!({
            "schema":1, "tool":"SheetScope", "version":env!("CARGO_PKG_VERSION"),
            "run_id":uuid::Uuid::new_v4().to_string(), "status":"preparing",
            "started_at":chrono::Utc::now().to_rfc3339(), "finished_at":null,
            "index_complete":false, "config":config, "validation_signature":signature,
            "scope":"Selected worksheet regions and complete charts under the recorded targeted policy; not whole-workbook visual equivalence",
            "baseline_after_step":config.baseline_after_step,
            "pairs":[], "unmatched_candidates":[], "steps":[],
        });
        let mut run = Self { config, root, work, progress, state:RunState::Idle, header,
            previous, processing_failures:0, cleanup_unconfirmed:false };
        run.update_progress(|progress| { *progress = LiveProgress {
            report_dir:Some(run.root.clone()), ..LiveProgress::default()
        }; })?;
        run.transition(RunState::Preparing)?;
        Ok(run)
    }

    /// High-level phases own source I/O, external execution, scientific work and publication.
    fn perform(&mut self) -> Result<()> {
        let originals = self.materialize_originals()?;
        let (baseline, candidates) = self.obtain_candidates(originals)?;
        self.publish_processing_output(&candidates)?;
        let pairs = self.select_pairs(&baseline, &candidates)?;
        self.validate_pairs(pairs)?;
        self.transition(RunState::Publishing)?;
        Ok(())
    }

    fn materialize_originals(&self) -> Result<MaterializedSource> {
        let source = validation_sources::materialize(&self.config.source_a, &self.work.path.join("source-a"),
            &self.config.cache_dir, self.config.refresh_downloads)?;
        self.update_progress(|progress| {
            progress.total = source.files.len();
            progress.workbooks = source.files.keys().map(|name| {
                let id = pair_id(name);
                (id.clone(), json!({"id":id,"name":name,"status":"pending","report":null}))
            }).collect();
        })?;
        Ok(source)
    }

    fn obtain_candidates(&mut self, originals: MaterializedSource) -> Result<(MaterializedSource, MaterializedSource)> {
        if let Some(source) = &self.config.source_b {
            let candidate = validation_sources::materialize(source, &self.work.path.join("source-b"),
                &self.config.cache_dir, self.config.refresh_downloads)?;
            return Ok((originals, candidate));
        }
        self.transition(RunState::Processing)?;
        let mut baseline = clone_source(&originals);
        let mut current = originals;
        for index in 0..self.config.steps.len() {
            current = self.process_stage(index, &current)?;
            if self.config.baseline_after_step == Some(index + 1) { baseline = clone_source(&current); }
        }
        Ok((baseline, current))
    }

    fn process_stage(&mut self, index: usize, input: &MaterializedSource) -> Result<MaterializedSource> {
        let stage = self.config.steps[index].clone();
        self.update_progress(|progress| progress.current = Some(format!("Step {}: {}", index + 1, stage.name)))?;
        let logs = self.root.join("logs").join(format!("stage-{}", index + 1));
        let direct = if index + 1 == self.config.steps.len() {
            match &self.config.output { Output::Directory { path } => Some(path.clone()), _ => None }
        } else { None };
        let cache = self.config.stage_cache_dir.clone().filter(|_| direct.is_none());
        let key = if cache.is_some() { Some(self.processing_key(&stage, input)?) } else { None };
        let cache_target = cache.as_ref().zip(key.as_ref()).map(|(root, key)| root.join(key));
        if let Some(target) = &cache_target {
            if let Some(source) = self.cached_stage(index, &stage.name, target, key.as_deref().unwrap())? {
                return Ok(source);
            }
        }
        let _lock = if let Some(target) = &cache_target {
            fs::create_dir_all(target.parent().unwrap())?;
            Some(CacheLock::acquire(target.with_extension("lock"))?)
        } else { None };
        // A concurrent run may have published between the first read and our lock.
        if let Some(target) = &cache_target {
            if let Some(source) = self.cached_stage(index, &stage.name, target, key.as_deref().unwrap())? {
                return Ok(source);
            }
        }
        let stage_root = if let Some(cache) = &cache {
            let staging = cache.join(format!(".pending-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&staging)?;
            self.work.track(staging.clone());
            staging
        } else { self.work.path.join(format!("stage-{}", index + 1)) };
        let output = direct.unwrap_or_else(|| stage_root.join("files"));
        let invocation_count = if stage.input == validation_process::InputKind::File { input.files.len() } else { 1 };
        self.update_progress(|progress| {
            progress.processing = Some(super::ProcessingProgress::new(stage.name.clone(), index + 1, invocation_count));
        })?;
        let live = self.progress.clone();
        let mut observe = |outcome: &validation_process::ProcessOutcome, pid: Option<u32>| -> Result<()> {
            let mut progress = live.lock().map_err(|_| anyhow::anyhow!("Progress lock poisoned"))?;
            if let Some(processing) = &mut progress.processing { processing.observe(outcome, pid); }
            for name in &outcome.inputs {
                if let Some(record) = progress.workbooks.get_mut(&pair_id(name)) {
                    record["processing_status"] = serde_json::to_value(outcome.status)?;
                }
            }
            Ok(())
        };
        let result = validation_process::run_stage(&stage, input, &output, &logs,
            self.config.max_processes, self.config.continue_on_error, Some(&mut observe));
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                if error.is::<validation_process::UnconfirmedTermination>() {
                    self.work.preserve();
                    self.cleanup_unconfirmed = true;
                }
                self.record_step(json!({"number":index+1,"name":stage.name,"status":"failed",
                    "error":format!("{error:#}"),"logs":format!("logs/stage-{}",index+1)}))?;
                return Err(error);
            }
        };
        self.processing_failures += result.failed;
        self.record_step(json!({"number":index+1,"name":stage.name,
            "status":if result.failed == 0 {"completed"} else {"partial"},
            "failed_processes":result.failed,"files":result.source.files.len(),
            "logs":format!("logs/stage-{}",index+1)}))?;
        if let Some(target) = cache_target.filter(|_| result.failed == 0) {
            let manifest = StageManifest::create(key.unwrap(), &result.source.root, &result.source.files)?;
            storage::write_json(&stage_root.join("manifest.json"), &manifest)?;
            fs::rename(&stage_root, &target).context("Publish immutable processing cache")?;
            let root = target.join("files");
            let files = manifest.files.keys().map(|name| (name.clone(), root.join(name))).collect();
            return Ok(MaterializedSource { root, files, archive:None });
        }
        Ok(result.source)
    }

    fn cached_stage(&mut self, index: usize, name: &str, target: &Path, key: &str) -> Result<Option<MaterializedSource>> {
        if !target.join("manifest.json").is_file() { return Ok(None) }
        let manifest: StageManifest = serde_json::from_slice(&fs::read(target.join("manifest.json"))?)?;
        let root = fs::canonicalize(target.join("files"))?;
        let files = manifest.verify(key, &root)?;
        self.record_step(json!({"number":index+1,"name":name,"status":"reused",
            "cache":target,"cache_key":key,"files":files.len()}))?;
        Ok(Some(MaterializedSource { root, files, archive:None }))
    }

    fn processing_key(&self, stage: &validation_process::Stage, input: &MaterializedSource) -> Result<String> {
        let inputs = input.files.iter().map(|(name, path)| Ok((name.clone(), file_sha256(path)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let dependencies = stage.dependencies.iter().map(|path| Ok((path.to_string_lossy().into_owned(), file_sha256(path)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let archive_sha256 = if stage.input == validation_process::InputKind::Archive {
            input.archive.as_deref().map(file_sha256).transpose()?
        } else { None };
        let directory_files = if stage.input == validation_process::InputKind::Directory {
            hash_directory(&input.root)?
        } else { BTreeMap::new() };
        storage::hash_json(&json!({"schema":2,"stage":stage,"inputs":inputs,
            "archive_sha256":archive_sha256,"directory_files":directory_files,
            "program_sha256":file_sha256(&stage.program)?,"dependency_files":dependencies,
            "dependency_environment_key":self.config.processing_cache_key}))
    }

    fn record_step(&mut self, record: Value) -> Result<()> {
        self.header["steps"].as_array_mut().unwrap().push(record);
        storage::write_json(&self.root.join("run.json"), &self.header)
    }

    fn select_pairs(&mut self, baseline: &MaterializedSource, candidate: &MaterializedSource) -> Result<Vec<WorkbookPair>> {
        self.transition(RunState::Pairing)?;
        let paired = pairing::pair_files(&baseline.files, &candidate.files);
        ensure!(!paired.pairs.is_empty(), "Source A contains no supported workbook files");
        self.header["unmatched_candidates"] = json!(paired.unmatched_candidates);
        self.update_progress(|progress| {
            progress.total = paired.pairs.len();
            progress.workbooks = paired.pairs.iter().map(|pair| {
                let id = pair_id(&pair.name);
                (id.clone(), json!({"id":id,"name":pair.name,"status":"pending","report":null}))
            }).collect();
        })?;
        self.transition(RunState::Validating)?;
        Ok(paired.pairs)
    }

    fn validate_pairs(&mut self, pairs: Vec<WorkbookPair>) -> Result<()> {
        for pair in pairs {
            let pair_id = pair_id(&pair.name);
            let id = pair_id.as_str();
            let mut record = json!({"id":id,"name":pair.name,"candidate_name":pair.candidate_name,
                "status":"running","report":null,"identity":null,"reuse":null,"errors":[]});
            storage::write_json(&self.root.join("records").join(format!("{id}.json")), &record)?;
            self.update_progress(|progress| {
                progress.current = Some(pair.name.clone());
                progress.telemetry_path = Some(self.work.path.join("captures").join(id).join("telemetry.json"));
                progress.workbooks.insert(id.to_owned(), record.clone());
            })?;
            let outcome = self.validate_pair(&pair, id, &mut record);
            if let Err(error) = &outcome {
                record["status"] = json!(if pair.candidate.is_none() {"missing"} else {"failed"});
                record["errors"].as_array_mut().unwrap().push(json!(format!("{error:#}")));
            }
            self.publish_record(id, record)?;
            if let Err(error) = outcome {
                if self.cleanup_unconfirmed || !self.config.continue_on_error { return Err(error); }
            }
        }
        Ok(())
    }

    fn validate_pair(&mut self, pair: &WorkbookPair, id: &str, record: &mut Value) -> Result<()> {
        ensure!(pair.problem.is_none(), "{}: {}", pair.name, pair.problem.as_deref().unwrap_or_default());
        let candidate = pair.candidate.as_ref().context("Missing comparison partner")?;
        ensure!(is_xlsx(&pair.original) && is_xlsx(candidate),
            "Targeted comparison requires XLSX on both sides. To evaluate a post-conversion chain, explicitly set baseline_after_step; conversion itself is then outside visual coverage");
        let identity = PairFingerprint::read(&pair.original, candidate, &self.config.reuse_parts)?;
        let identity_relative = format!("identities/{id}.json");
        storage::write_json(&self.root.join(&identity_relative), &identity)?;
        record["identity"] = json!(identity_relative);
        record["identity_sha256"] = json!(file_sha256(&self.root.join(&identity_relative))?);
        let destination = self.root.join("pairs").join(id);
        if let Some(previous) = &self.previous {
            if let Some(source) = previous.matching(&pair.name, &identity)? {
                let bundle = validation_report::package_comparison(&source, &destination)?;
                record["status"] = json!("reused");
                record["reuse"] = json!({"source_report":previous.root,"source_pair":pair.name,
                    "selection":self.config.reuse_parts,
                    "meaning":"Previous observation reused under user-selected OOXML components; no new visual comparison"});
                self.publish_pair_view(id, &pair.name, bundle.different_regions, record)?;
                return Ok(());
            }
        }
        let raw = self.work.path.join("captures").join(id);
        let logs = self.root.join("logs/validation").join(id);
        let outcome = capture::compare(&pair.original, candidate, &raw, &logs, &pair.name, &self.config.capture);
        if let Ok(telemetry) = storage::read_json(&raw.join("telemetry.json")) {
            self.update_progress(|progress| {
                progress.comparisons.push_front(telemetry);
                progress.comparisons.truncate(20);
            })?;
        }
        if outcome.as_ref().err().is_some_and(|error| error.is::<crate::capture_watchdog::CleanupUnconfirmed>())
            || !cleanup_confirmed(&raw).unwrap_or(false) {
            self.work.preserve();
            self.cleanup_unconfirmed = true;
            anyhow::bail!("Excel cleanup is unconfirmed; no further captures will start. Working evidence retained at {}", raw.display());
        }
        let publication = (|| -> Result<()> {
            if let Err(error) = &outcome {
                if raw.exists() && !raw.join("failure.json").exists() {
                    storage::write_json(&raw.join("failure.json"), &json!({"status":"failed","error":format!("{error:#}")}))?;
                }
            }
            if raw.join("report.json").is_file() || raw.join("partial-report.json").is_file() {
                let bundle = validation_report::package_comparison(&raw, &destination)?;
                self.publish_pair_view(id, &pair.name, bundle.different_regions, record)?;
            }
            if raw.exists() { fs::remove_dir_all(&raw)?; }
            Ok(())
        })();
        if let Err(error) = publication {
            self.work.preserve();
            return Err(error).context("Preserved working evidence because portable publication failed");
        }
        outcome?;
        record["status"] = json!("completed_with_limitations");
        Ok(())
    }

    fn publish_pair_view(&self, id: &str, name: &str, differences: usize, record: &mut Value) -> Result<()> {
        let report = format!("pairs/{id}/report.json");
        record["report"] = json!(&report);
        record["different_regions"] = json!(differences);
        validation_report::write_pair_page(&self.root, &report, name)?;
        Ok(())
    }

    fn publish_record(&mut self, id: &str, record: Value) -> Result<()> {
        storage::write_json(&self.root.join("records").join(format!("{id}.json")), &record)?;
        self.update_progress(|progress| {
            progress.completed += 1;
            progress.failed += usize::from(matches!(record["status"].as_str(), Some("failed" | "missing")));
            progress.reused += usize::from(record["status"] == "reused");
            progress.different_pairs += usize::from(record["different_regions"].as_u64().unwrap_or(0) > 0);
            progress.checked_equal_pairs += usize::from(record["status"] == "completed_with_limitations"
                && record["different_regions"].as_u64() == Some(0));
            progress.workbooks.insert(id.to_owned(), record.clone());
            progress.telemetry_path = None;
            progress.recent.push_front(record.clone());
            progress.recent.truncate(20);
        })?;
        Ok(())
    }

    fn publish_processing_output(&mut self, candidate: &MaterializedSource) -> Result<()> {
        if let Output::Archive { path } = &self.config.output {
            if let Err(error) = validation_sources::archive_files(&candidate.files, path) {
                self.work.preserve();
                return Err(error).context("Preserved processing results because archive publication failed");
            }
        }
        Ok(())
    }

    /// Publish a terminal index only after evidence viewing assets have been generated.
    fn finish(&mut self, mut outcome: Result<()>) -> Result<Value> {
        if let Err(error) = self.work.clean() {
            outcome = Err(error.context("Could not remove owned temporary work"));
        }
        let progress = self.progress.lock().map_err(|_| anyhow::anyhow!("Progress lock poisoned"))?.clone();
        self.header["status"] = json!(if outcome.is_err() {"failed"}
            else if progress.failed > 0 || self.processing_failures > 0 {"completed_with_errors"}
            else {"completed_with_limitations"});
        self.header["finished_at"] = json!(chrono::Utc::now().to_rfc3339());
        // Receipts, including interrupted running records, are authoritative.
        // A failed progress write must not erase an already durable pair.
        match read_pair_records(&self.root) {
            Ok(records) => {
                self.header["pairs"] = json!(records);
                self.header["index_complete"] = json!(true);
            }
            Err(error) => {
                outcome = Err(error.context("Could not assemble the durable pair index"));
                self.header["index_complete"] = json!(false);
                self.header["status"] = json!("failed");
            }
        }
        self.header["summary"] = json!({"total":progress.total,"processed_pairs":progress.completed,
            "failed_pairs":progress.failed,"reused_pairs":progress.reused,
            "processing_failures":self.processing_failures});
        if let Err(error) = &outcome { self.header["error"] = json!(format!("{error:#}")); }
        if let Err(error) = validation_report::write_site(&self.root, &self.header) {
            outcome = Err(error.context("Could not publish the report viewer"));
            self.header["status"] = json!("failed");
        }
        if let Err(error) = &outcome {
            self.header["error"] = json!(format!("{error:#}"));
            self.update_progress(|progress| progress.error = Some(format!("{error:#}")))?;
        }
        self.transition(if outcome.is_err() { RunState::Failed } else { RunState::Completed })?;
        eprintln!("SheetScope report: {}", self.root.join("index.html").display());
        outcome?;
        ensure!(progress.failed == 0 && self.processing_failures == 0,
            "Run completed with errors; successful pairs and failure logs are retained in {}", self.root.display());
        Ok(self.header.clone())
    }

    fn transition(&mut self, next: RunState) -> Result<()> {
        ensure!(self.state.permits(next), "Invalid run transition {:?} -> {:?}", self.state, next);
        self.state = next;
        self.header["phase"] = serde_json::to_value(next)?;
        if !matches!(next, RunState::Completed | RunState::Failed) {
            self.header["status"] = serde_json::to_value(next)?;
        }
        storage::write_json(&self.root.join("run.json"), &self.header)?;
        let mut journal = fs::OpenOptions::new().create(true).append(true).open(self.root.join("events.jsonl"))?;
        serde_json::to_writer(&mut journal, &json!({"state":next,"at":chrono::Utc::now().to_rfc3339()}))?;
        journal.write_all(b"\n")?;
        journal.sync_data()?;
        self.update_progress(|progress| progress.state = next)
    }

    fn update_progress(&self, update: impl FnOnce(&mut LiveProgress)) -> Result<()> {
        let mut progress = self.progress.lock().map_err(|_| anyhow::anyhow!("Progress lock poisoned"))?;
        update(&mut progress);
        storage::write_json(&self.root.join("progress.json"), &*progress)
    }
}

fn pair_id(name: &str) -> String {
    format!("{:x}", Sha256::digest(name.as_bytes()))[..24].to_owned()
}

impl PreviousRun {
    fn load(path: Option<&Path>, signature: &Value) -> Result<Option<Self>> {
        let Some(path) = path else { return Ok(None) };
        let root = super::report_root(path)?;
        let run = read_run(&root)?;
        ensure!(run["schema"] == 1, "Unsupported previous report schema");
        ensure!(run["validation_signature"] == *signature,
            "Previous validation used a different renderer, policy, options or environment_key");
        let mut pairs = BTreeMap::new();
        for record in run["pairs"].as_array().context("Previous report has no pairs")? {
            if !matches!(record["status"].as_str(), Some("completed_with_limitations" | "reused")) { continue; }
            let name = record["name"].as_str().context("Previous pair name missing")?;
            ensure!(pairs.insert(name.to_owned(), record.clone()).is_none(),
                "Previous report contains duplicate completed pair names: {name}");
        }
        Ok(Some(Self { root, pairs }))
    }

    fn matching(&self, name: &str, current: &PairFingerprint) -> Result<Option<PathBuf>> {
        let Some(record) = self.pairs.get(name) else { return Ok(None) };
        let Some(identity) = record["identity"].as_str() else { return Ok(None) };
        let path = storage::confined_file(&self.root, identity)?;
        ensure!(record["identity_sha256"].as_str() == Some(file_sha256(&path)?.as_str()),
            "Previous OOXML identity receipt changed or has no checksum");
        let previous: PairFingerprint = serde_json::from_slice(&fs::read(path)?)?;
        if !current.matches_previous(&previous) { return Ok(None) }
        let report = record["report"].as_str().context("Previous completed pair has no evidence")?;
        let report = storage::confined_file(&self.root, report)?;
        Ok(Some(verify_completed_observation(&report)?))
    }
}

fn verify_completed_observation(report: &Path) -> Result<PathBuf> {
    ensure!(report.file_name() == Some(std::ffi::OsStr::new("report.json")),
        "Previous pair must reference its scientific report.json");
    let directory = report.parent().context("Previous report has no directory")?;
    validation_report::verify_comparison(directory)?;
    let evidence = storage::read_json(report)?;
    ensure!(evidence["status"] == "completed_with_limitations"
        && evidence["portable_report"]["failure"].is_null(),
        "A partial or failed observation cannot be reused as completed evidence");
    Ok(directory.to_path_buf())
}

/// Reconstruct an interrupted index once, from complete per-pair records. Live
/// polling uses compact progress snapshots instead of repeatedly running this scan.
pub(crate) fn read_run(root: &Path) -> Result<Value> {
    let mut run = storage::read_json(&root.join("run.json"))?;
    ensure!(run["schema"] == 1 && run["tool"] == "SheetScope", "Not a supported SheetScope report");
    if run["index_complete"] == true { return Ok(run) }
    run["pairs"] = json!(read_pair_records(root)?);
    run["interrupted_index"] = json!(true);
    Ok(run)
}

fn read_pair_records(root: &Path) -> Result<Vec<Value>> {
    let mut records = Vec::new();
    for entry in fs::read_dir(root.join("records"))? {
        let entry = entry?;
        if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|ext| ext == "json") {
            records.push(storage::read_json(&entry.path())?);
        }
    }
    records.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(records)
}

fn clone_source(source: &MaterializedSource) -> MaterializedSource {
    MaterializedSource { root:source.root.clone(), files:source.files.clone(), archive:source.archive.clone() }
}

/// A batch program can inspect sidecars in its directory. Include them in the
/// processing identity even when they are not workbooks selected for comparison.
fn hash_directory(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_dir() { continue; }
        validation_sources::require_regular_file(entry.path())?;
        files.insert(entry.path().strip_prefix(root)?.to_string_lossy().replace('\\', "/"), file_sha256(entry.path())?);
    }
    Ok(files)
}

fn is_xlsx(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"))
}

fn cleanup_confirmed(raw: &Path) -> Result<bool> {
    let path = raw.join("telemetry.json");
    if !path.exists() { return Ok(true) }
    let telemetry = storage::read_json(&path)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let mut confirmed = true;
        for lane in telemetry["lanes"].as_array().context("Cleanup telemetry has no process lanes")? {
            if let Some(pid) = lane["pid"].as_i64() {
                let Some(start) = lane["process_started_at"].as_i64() else { return Ok(false) };
                if crate::native_process::process_alive(pid, start) != Some(false) { confirmed = false; }
            }
        }
        if confirmed { return Ok(true) }
        if std::time::Instant::now() >= deadline { return Ok(false) }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_does_not_allow_completed_to_be_reported_before_publication() {
        assert!(RunState::Preparing.permits(RunState::Pairing));
        assert!(RunState::Processing.permits(RunState::Failed));
        assert!(!RunState::Validating.permits(RunState::Completed));
        assert!(!RunState::Completed.permits(RunState::Validating));
    }

    #[test]
    fn interrupted_run_recovers_successful_pairs_without_inventing_completion() -> Result<()> {
        let root = std::env::temp_dir().join(format!("sheet-scope-journal-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("records"))?;
        storage::write_json(&root.join("run.json"), &json!({"schema":1,"tool":"SheetScope",
            "status":"validating","index_complete":false,"pairs":[]}))?;
        storage::write_json(&root.join("records/first.json"), &json!({"id":"first","name":"a.xlsx","status":"completed_with_limitations"}))?;
        storage::write_json(&root.join("records/second.json"), &json!({"id":"second","name":"b.xlsx","status":"running"}))?;
        let run = read_run(&root)?;
        assert_eq!(run["pairs"].as_array().unwrap().len(), 2);
        assert_eq!(run["status"], "validating");
        assert_eq!(run["pairs"][1]["status"], "running");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn intact_partial_evidence_is_not_eligible_for_completed_result_reuse() -> Result<()> {
        let root = std::env::temp_dir().join(format!("sheet-scope-partial-{}", uuid::Uuid::new_v4()));
        let raw = root.join("raw");
        let bundle = root.join("bundle");
        fs::create_dir_all(&raw)?;
        storage::write_json(&raw.join("partial-report.json"),
            &json!({"status":"failed","comparisons":[]}))?;
        storage::write_json(&raw.join("failure.json"), &json!({"error":"capture interrupted"}))?;
        validation_report::package_comparison(&raw, &bundle)?;
        validation_report::verify_comparison(&bundle)?;
        assert!(verify_completed_observation(&bundle.join("report.json")).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
