//! Prepare attributable optimization artifacts with a continuously refilled Excel pool.
use super::*;
use corpus_runner::sources::{self as validation_preparation, Input};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
mod reuse;
use crate::context::RunContext;
use corpus_runner::{package_sizes, telemetry::Operation};

#[derive(Clone, Copy)]
enum Mode {
    CheckPerformance,
    Conversion,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::CheckPerformance => "check-performance",
            Self::Conversion => "xlsx-convert",
        }
    }
}

struct ProgressCounters {
    operation: &'static str,
    prepared: AtomicUsize,
    failed: AtomicUsize,
    reused: AtomicUsize,
}

impl Default for ProgressCounters {
    fn default() -> Self {
        Self {
            operation: "check-performance",
            prepared: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            reused: AtomicUsize::new(0),
        }
    }
}

/// Publish compact counters without rescanning per-workbook receipts for web status queries.
fn publish_progress(
    output: &Path,
    counters: &ProgressCounters,
    started: usize,
    total: usize,
    workers: usize,
    elapsed: Duration,
    status: &str,
) -> Result<()> {
    let prepared = counters.prepared.load(Ordering::Acquire);
    let failed = counters.failed.load(Ordering::Acquire);
    let completed = prepared + failed;
    let started = started.min(total).max(completed);
    write_json(
        &output.join("progress.json"),
        &json!({
            "version":1,"operation":counters.operation,"status":status,
            "total":total,"started":started,"prepared":prepared,"failed":failed,
            "reused":counters.reused.load(Ordering::Acquire),
            "in_flight":started.saturating_sub(completed),"queued":total.saturating_sub(started),
            "unresolved":total.saturating_sub(completed),"requested_workers":workers,
            "elapsed_seconds":elapsed.as_secs_f64(),"updated_at":chrono::Utc::now().to_rfc3339()
        }),
    )
}

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Prepared XLSX inputs. Legacy conversion remains a separate operation.
    #[arg(long, required_unless_present = "source_manifest", conflicts_with = "source_manifest", num_args = 1..)]
    pub input: Vec<PathBuf>,
    /// Stable source occurrences and expected baseline hashes for corpus reuse.
    #[arg(long)]
    pub source_manifest: Option<PathBuf>,
    /// New directory retaining per-input results, failures and worker ownership.
    #[arg(long)]
    pub output: PathBuf,
    /// Independent Excel workers; may exceed the logical processor count.
    #[arg(long, default_value_t = 2)]
    pub workers: usize,
    /// Reuse verified successful inputs from a completed or positively departed preparation.
    #[arg(long)]
    pub reuse_from: Option<PathBuf>,
}

/// Run one continuously refilled queue, retaining every input's terminal receipt.
pub fn run_manifest(
    source_manifest: PathBuf,
    output: PathBuf,
    workers: usize,
    reuse_from: Option<PathBuf>,
) -> Result<()> {
    run(Args {
        input: Vec::new(),
        source_manifest: Some(source_manifest),
        output,
        workers,
        reuse_from,
    })
}

/// Run one continuously refilled queue, retaining every input's terminal receipt.
pub fn run(args: Args) -> Result<()> {
    run_with_context(args, RunContext::quiet(Operation::CheckPerformance))
}

pub fn run_with_context(args: Args, context: RunContext) -> Result<()> {
    anyhow::ensure!(
        args.workers > 0,
        "Check Performance workers must be positive"
    );
    let inputs = match &args.source_manifest {
        Some(path) => validation_preparation::read(path)?,
        None => validation_preparation::explicit(&args.input)?,
    };
    run_inputs(args, inputs, context, Mode::CheckPerformance)
}

pub(crate) fn run_conversion(
    inputs: Vec<Input>,
    output: PathBuf,
    workers: usize,
    context: RunContext,
) -> Result<()> {
    let args = Args {
        input: Vec::new(),
        source_manifest: None,
        output,
        workers,
        reuse_from: None,
    };
    run_inputs(args, inputs, context, Mode::Conversion)
}

fn run_inputs(args: Args, inputs: Vec<Input>, context: RunContext, mode: Mode) -> Result<()> {
    anyhow::ensure!(args.workers > 0, "Excel workers must be positive");
    anyhow::ensure!(
        !args.output.exists(),
        "Choose a new preparation output directory"
    );
    let selection_hash = validation_preparation::selection_hash(&inputs)?;
    let previous = args
        .reuse_from
        .as_deref()
        .map(reuse::Index::open)
        .transpose()?;
    fs::create_dir(&args.output)?;
    let output = fs::canonicalize(&args.output)?;
    write_json(
        &output.join("control.json"),
        &json!({"version":1,"workers":args.workers}),
    )?;
    let producer = corpus_runner::artifacts::hash_file(&std::env::current_exe()?)?;
    write_json(
        &output.join("owner.json"),
        &json!({"pid":std::process::id(),"started_ms":crate::native_process::current_started_unix_ms()}),
    )?;
    let mut pool = match mode {
        Mode::CheckPerformance => Pool::new_performance(output.join("workers"))?,
        Mode::Conversion => Pool::new(output.join("workers"))?,
    };
    if let Some(reserve) = context.memory_reserve {
        pool.set_memory_reserve(reserve);
    }
    context.observe(
        "run.started",
        None,
        None,
        json!({
            "total": inputs.len(), "work_unit": "workbook", "sheet_jobs": false,
            "size_reduction": matches!(mode, Mode::CheckPerformance), "worksheet_sizes": true
        }),
    );
    let pool_root = pool.root.clone();
    let next = AtomicUsize::new(0);
    let counters = ProgressCounters {
        operation: mode.name(),
        ..Default::default()
    };
    let started_at = Instant::now();
    let recipe = generation_recipe(&pool_root)?;
    let mut retained = Vec::new();
    let mut pending = Vec::new();
    let mut reuse_lookups = std::collections::BTreeMap::new();
    let mut last_restore_progress = Instant::now();
    publish_progress(
        &output,
        &counters,
        0,
        inputs.len(),
        args.workers,
        started_at.elapsed(),
        "restoring",
    )?;
    for (index, input) in inputs.iter().enumerate() {
        let directory = output.join(format!("input-{index:06}"));
        let lookup = previous
            .as_ref()
            .map(|previous| previous.lookup(input, &recipe));
        match lookup {
            Some(Ok(Some(entry))) => {
                entry.restore(&directory)?;
                context.observe(
                    "workbook.reused",
                    Some(&input.source_id),
                    None,
                    json!({"name": input.name}),
                );
                retained.push(json!({"index":index,"source_id":input.source_id,"name":input.name,
                    "input":input.baseline,"directory":directory,"status":"prepared","error":null,"reused":true}));
                counters.prepared.fetch_add(1, Ordering::Release);
                counters.reused.fetch_add(1, Ordering::Release);
            }
            other => {
                let detail = match other {
                    Some(Err(error)) => {
                        format!("incompatible or invalid prior evidence: {error:#}")
                    }
                    _ => "no compatible completed input".into(),
                };
                if previous.is_some() {
                    reuse_lookups.insert(index, detail);
                }
                pending.push(index);
            }
        }
        if last_restore_progress.elapsed() >= Duration::from_secs(1) {
            publish_progress(
                &output,
                &counters,
                retained.len(),
                inputs.len(),
                args.workers,
                started_at.elapsed(),
                "restoring",
            )?;
            last_restore_progress = Instant::now();
        }
    }
    let reused = retained.len();
    let outcomes = Mutex::new(retained);
    let count = args.workers.min(pending.len());
    let target = AtomicUsize::new(count);
    let cancelled = AtomicBool::new(false);
    let requested_target = AtomicUsize::new(args.workers);
    let peak_dispatch_slots = AtomicUsize::new(count);
    publish_progress(
        &output,
        &counters,
        reused,
        inputs.len(),
        args.workers,
        started_at.elapsed(),
        "running",
    )?;
    let pool_result = std::thread::scope(|scope| -> Result<()> {
        let mut handles = Vec::new();
        let mut last_control = Instant::now();
        loop {
            if context.should_stop() {
                cancelled.store(true, Ordering::Release);
                let error = anyhow::anyhow!("Preparation stopped by user");
                pool.abort(&error)?;
                return Err(error);
            }
            let desired = target.load(Ordering::Acquire);
            for slot in handles.len()..desired {
                let target = &target;
                let cancelled = &cancelled;
                let context = &context;
                let next = &next;
                let pending = &pending;
                let inputs = &inputs;
                let output = &output;
                let pool_root = &pool_root;
                let producer = &producer;
                let outcomes = &outcomes;
                let counters = &counters;
                let reuse_lookups = &reuse_lookups;
                handles.push(scope.spawn(move || loop {
                    if context.should_stop() || cancelled.load(Ordering::Acquire) || next.load(Ordering::Acquire) >= pending.len() {
                        break;
                    }
                    if slot >= target.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    let position = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&index) = pending.get(position) else {
                        break;
                    };
                    let input = &inputs[index];
                    let directory = output.join(format!("input-{index:06}"));
                    context.observe("job.started", Some(&input.source_id), Some(slot), json!({"name": input.name}));
                    let metadata = match mode {
                        Mode::CheckPerformance => package_sizes::inspect(&input.baseline)
                            .map(|sizes| json!({"package": sizes})),
                        Mode::Conversion => fs::metadata(&input.baseline)
                            .map(|file| json!({"file_bytes": file.len()}))
                            .map_err(Into::into),
                    };
                    context.observe(
                        "workbook.metadata",
                        Some(&input.source_id),
                        Some(slot),
                        match metadata {
                            Ok(metadata) => metadata,
                            Err(error) => json!({"error": format!("{error:#}")}),
                        },
                    );
                    let result = match mode {
                        Mode::CheckPerformance => prepare(&pool_root, input, &directory, &producer),
                        Mode::Conversion => prepare_conversion(&pool_root, input, &directory, &producer),
                    };
                    if result.is_err() && pool_root.join("error.json").exists() {
                        cancelled.store(true, Ordering::Release);
                    }
                    let receipt = read_json(&directory.join("prepared.json"));
                    context.observe(
                        if result.is_ok() { "job.completed" } else { "job.failed" },
                        Some(&input.source_id), Some(slot), json!({
                            "name": input.name,
                            "archive_sizes": receipt.as_ref().and_then(|value| value.get("archive_sizes")),
                            "error": result.as_ref().err().map(|error| format!("{error:#}"))
                        }),
                    );
                    let succeeded = result.is_ok();
                    let outcome = json!({
                        "index": index,
                        "source_id": input.source_id,
                        "name": input.name,
                        "input": input.baseline,
                        "directory": directory,
                        "status": if result.is_ok() { "prepared" } else { "failed" },
                        "error": result.err().map(|error| format!("{error:#}")),
                        "reused":false,"reuse_lookup":reuse_lookups.get(&index)
                    });
                    if outcome["status"] == "failed" {
                        if let Err(error) = write_json(&directory.join("failed.json"), &outcome) {
                            eprintln!("Cannot retain preparation failure: {error:#}");
                        }
                    }
                    outcomes
                        .lock()
                        .expect("preparation outcomes lock poisoned")
                        .push(outcome);
                    if succeeded {
                        counters.prepared.fetch_add(1, Ordering::Release);
                    } else {
                        counters.failed.fetch_add(1, Ordering::Release);
                    }
                }));
            }
            peak_dispatch_slots.store(handles.len(), Ordering::Release);
            if handles.iter().all(|handle| handle.is_finished()) {
                break;
            }
            if let Err(error) = pool.reconcile(desired, 512) {
                cancelled.store(true, Ordering::Release);
                pool.abort(&error)?;
                return Err(error);
            }
            if last_control.elapsed() >= Duration::from_secs(1) {
                let requested_workers = match read_worker_target(&output.join("control.json")) {
                    Ok(workers) => {
                        requested_target.store(workers, Ordering::Release);
                        target.store(workers.min(pending.len()), Ordering::Release);
                        workers
                    }
                    Err(error) => {
                        cancelled.store(true, Ordering::Release);
                        pool.abort(&error)?;
                        return Err(error);
                    }
                };
                if let Err(error) = publish_progress(
                    &output,
                    &counters,
                    reused + next.load(Ordering::Acquire).min(pending.len()),
                    inputs.len(),
                    requested_workers,
                    started_at.elapsed(),
                    "running",
                ) {
                    cancelled.store(true, Ordering::Release);
                    pool.abort(&error)?;
                    return Err(error);
                }
                last_control = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        for handle in handles {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("Check Performance worker thread panicked"))?;
        }
        Ok(())
    });
    drop(pool);
    let mut outcomes = outcomes
        .into_inner()
        .map_err(|_| anyhow::anyhow!("Preparation outcomes poisoned"))?;
    outcomes.sort_by_key(|row| row["index"].as_u64());
    let failed = outcomes
        .iter()
        .filter(|row| row["status"] == "failed")
        .count();
    write_json(
        &output.join("preparation.json"),
        &json!({
            "version": 1,
            "operation": mode.name(),
            "telemetry_error": context.observation_error(),
            "workers": peak_dispatch_slots.load(Ordering::Acquire),
            "initial_dispatch_slots": count,
            "requested_workers": args.workers,
            "final_requested_workers": requested_target.load(Ordering::Acquire),
            "producer_sha256": producer,
            "selection_sha256": selection_hash,
            "identity_scope": if args.source_manifest.is_some() { "source-manifest" } else { "explicit-file-list" },
            "manager_error": pool_result.as_ref().err().map(|error| format!("{error:#}")),
            "visual_validation": "pending",
            "total": inputs.len(),
            "failed": failed,
            "reused":reused,
            "reuse_from":args.reuse_from,
            "inputs": outcomes
        }),
    )?;
    publish_progress(
        &output,
        &counters,
        reused + next.load(Ordering::Acquire).min(pending.len()),
        inputs.len(),
        requested_target.load(Ordering::Acquire),
        started_at.elapsed(),
        if context.should_stop() {
            "stopped"
        } else if pool_result.is_ok() && failed == 0 {
            "completed"
        } else {
            "failed"
        },
    )?;
    context.observe(
        if context.should_stop() {
            "run.stopped"
        } else if pool_result.is_ok() && failed == 0 {
            "run.completed"
        } else {
            "run.failed"
        },
        None,
        None,
        json!({
            "prepared": counters.prepared.load(Ordering::Acquire), "failed": failed,
            "interrupted": pool_result.is_err(),
            "unresolved": inputs.len().saturating_sub(outcomes.len()),
        }),
    );
    if matches!(mode, Mode::Conversion) {
        publish_converted_sources(&output, &inputs)?;
    }
    pool_result?;
    context.ensure_observed()?;
    anyhow::ensure!(
        failed == 0,
        "{failed} {} preparations failed; inspect preparation.json",
        mode.name()
    );
    Ok(())
}

/// Read the independently adjustable native pool target from its atomic control file.
fn read_worker_target(path: &Path) -> Result<usize> {
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    anyhow::ensure!(
        value["version"] == 1,
        "Unsupported preparation control version"
    );
    let workers = value["workers"]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok());
    let workers = workers.context("Preparation worker target must be a positive integer")?;
    anyhow::ensure!(workers > 0, "Preparation worker target must be positive");
    Ok(workers)
}

fn prepare(root: &Path, source: &Input, directory: &Path, producer_sha256: &str) -> Result<()> {
    let input = &source.baseline;
    fs::create_dir(directory)?;
    let runtime = directory.join("runtime");
    fs::create_dir(&runtime)?;
    let source_hash = corpus_runner::artifacts::hash_file(input)?;
    if let Some(expected) = &source.expected_sha256 {
        anyhow::ensure!(
            expected == &source_hash,
            "Prepared baseline does not match its expected source identity"
        );
    }
    let baseline = directory.join("baseline.xlsx");
    fs::copy(input, &baseline)?;
    anyhow::ensure!(
        source_hash == corpus_runner::artifacts::hash_file(&baseline)?,
        "Baseline changed while freezing the preparation input"
    );
    zip::ZipArchive::new(File::open(&baseline)?)?
        .by_name("xl/workbook.xml")
        .context("Expected an XLSX workbook")?;
    let (session, lease) = client::lease_session(root)?;
    let request = Request {
        id: uuid::Uuid::new_v4().to_string(),
        input: PathBuf::from(crate::shell::power_shell_path(&baseline)?),
        output: PathBuf::from(crate::shell::power_shell_path(directory)?),
        directory: PathBuf::from(crate::shell::power_shell_path(&runtime)?),
        performance: true,
    };
    let response = client::run_request(root, &session, &request)?
        .context("Check Performance worker exited before completing this input")?;
    drop(lease);
    anyhow::ensure!(
        source_hash == corpus_runner::artifacts::hash_file(input)?,
        "Input changed during preparation"
    );
    anyhow::ensure!(
        matches!(
            response["native_result"]["status"].as_str(),
            Some("optimization_confirmed" | "no_optimization_needed")
        ),
        "No confirmed native optimization outcome"
    );
    let mut artifacts = std::collections::BTreeMap::new();
    for name in validation_preparation::PREPARATION_CONTROLS {
        let path = directory.join(name);
        artifacts.insert(
            name,
            json!({
                "bytes": fs::metadata(&path)?.len(),
                "sha256": corpus_runner::artifacts::hash_file(&path)?
            }),
        );
    }
    let recipe = generation_recipe(root)?;
    write_json(
        &directory.join("prepared.json"),
        &json!({
            "version": 1,
            "operation": "check-performance",
            "source": input,
            "producer_sha256": producer_sha256,
            "source_id": source.source_id,
            "name": source.name,
            "raw_source": source.raw_source,
            "source_sha256": source_hash,
            "baseline": {
                "file": "baseline.xlsx",
                "sha256": source_hash,
                "bytes": fs::metadata(&baseline)?.len()
            },
            "artifacts": artifacts,
            "archive_sizes": {
                "baseline": package_sizes::inspect(&baseline)?,
                "original_saved": package_sizes::inspect(&directory.join("original-saved.xlsx"))?,
                "before_optimization": package_sizes::inspect(&directory.join("before-optimization.xlsx"))?,
                "after_optimization": package_sizes::inspect(&directory.join("after-optimization.xlsx"))?,
            },
            "native_result": response["native_result"],
            "measurements": response["metrics"],
            "environment": response["environment"],
            "generation_recipe": recipe,
            "visual_validation": "pending"
        }),
    )
}

/// Freeze source bytes before Excel opens them; never overwrite the user's input.
fn prepare_conversion(root: &Path, input: &Input, directory: &Path, producer: &str) -> Result<()> {
    fs::create_dir(directory)?;
    let source_hash = corpus_runner::artifacts::hash_file(&input.baseline)?;
    let extension = input
        .baseline
        .extension()
        .and_then(|value| value.to_str())
        .context("Input format missing")?;
    let frozen = directory.join(format!("source.{extension}"));
    fs::copy(&input.baseline, &frozen)?;
    anyhow::ensure!(
        corpus_runner::artifacts::hash_file(&frozen)? == source_hash,
        "Input changed while freezing conversion source"
    );
    let output = directory.join("converted.xlsx");
    let metrics = client::convert(root, &frozen, &output, &directory.join("runtime"))?;
    anyhow::ensure!(
        corpus_runner::artifacts::hash_file(&input.baseline)? == source_hash,
        "Input changed during conversion"
    );
    let output_hash = corpus_runner::artifacts::hash_file(&output)?;
    write_json(
        &directory.join("prepared.json"),
        &json!({
            "version": 1, "operation": "xlsx-convert", "source_id": input.source_id,
            "name": input.name, "source_sha256": source_hash, "producer_sha256": producer,
            "raw_source": {"sha256": source_hash, "format": extension, "member": null},
            "baseline": {"file": "converted.xlsx", "sha256": output_hash, "bytes": fs::metadata(&output)?.len()},
            "measurements": metrics,
            "archive_sizes": {"source_file_bytes": fs::metadata(&frozen)?.len(), "converted": package_sizes::inspect(&output)?},
            "visual_validation": "pending"
        }),
    )
}

fn publish_converted_sources(output: &Path, inputs: &[Input]) -> Result<()> {
    let mut retained = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        let directory = output.join(format!("input-{index:06}"));
        if let Some(receipt) = read_json(&directory.join("prepared.json")) {
            retained.push(json!({
                "source_id": input.source_id, "name": input.name,
                "baseline": format!("input-{index:06}/converted.xlsx"),
                "baseline_sha256": receipt["baseline"]["sha256"],
                "raw_source": receipt["raw_source"]
            }));
        }
    }
    write_json(
        &output.join("sources.json"),
        &json!({"version": 1, "inputs": retained}),
    )
}

fn generation_recipe(root: &Path) -> Result<Value> {
    let mut recipe = std::collections::BTreeMap::new();
    for name in [
        "worker.ps1",
        "WorkerPerformance.psm1",
        "CheckPerformance.psm1",
        "NativeWindow.cs",
        "ExcelProcessId.cs",
    ] {
        recipe.insert(name, corpus_runner::artifacts::hash_file(&root.join(name))?);
    }
    Ok(serde_json::to_value(recipe)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_stop_keeps_the_queue_unstarted() -> Result<()> {
        let (root, _, manifest, _) = reuse::fixture()?;
        let output = root.join("user-stopped");
        let context = RunContext::quiet(Operation::CheckPerformance);
        context.stop_handle().store(true, Ordering::Release);
        let error = run_with_context(
            Args {
                input: Vec::new(),
                source_manifest: Some(manifest),
                output: output.clone(),
                workers: 2,
                reuse_from: None,
            },
            context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("stopped by user"));
        let progress = read_json(&output.join("progress.json")).unwrap();
        assert_eq!(progress["started"], 0);
        assert_eq!(progress["queued"], 1);
        assert_eq!(progress["status"], "stopped");
        assert_eq!(
            read_json(&output.join("workers/manifest.json")).unwrap()["workers"],
            json!([])
        );
        Ok(())
    }

    #[test]
    fn memory_stop_preserves_unstarted_jobs_without_launching_excel() -> Result<()> {
        let (root, _, manifest, _) = reuse::fixture()?;
        let selected = validation_preparation::read(&manifest)?;
        let output = root.join("memory-stopped");
        let mut context = RunContext::quiet(Operation::CheckPerformance);
        context.memory_reserve = Some(corpus_runner::memory::MemoryReserve { bytes: u64::MAX });
        let error = run_with_context(
            Args {
                input: vec![selected[0].baseline.clone(); 32],
                source_manifest: None,
                output: output.clone(),
                workers: 2,
                reuse_from: None,
            },
            context,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("memory reserve"));
        let progress = read_json(&output.join("progress.json")).unwrap();
        assert!(progress["queued"].as_u64().unwrap() >= 30);
        assert!(progress["failed"].as_u64().unwrap() <= 2);
        let workers = read_json(&output.join("workers/manifest.json")).unwrap();
        assert_eq!(workers["workers"], json!([]));
        Ok(())
    }

    #[test]
    fn complete_continuation_needs_no_excel_worker_and_preserves_producer() -> Result<()> {
        let (root, previous, manifest, _) = reuse::fixture()?;
        let output = root.join("continued");
        run(Args {
            input: Vec::new(),
            source_manifest: Some(manifest),
            output: output.clone(),
            workers: 32,
            reuse_from: Some(previous),
        })?;
        let batch = read_json(&output.join("preparation.json")).unwrap();
        assert_eq!(batch["reused"], 1);
        assert_eq!(batch["workers"], 0);
        assert_eq!(batch["inputs"][0]["status"], "prepared");
        let receipt = read_json(&output.join("input-000000/prepared.json")).unwrap();
        assert_eq!(receipt["producer_sha256"], "a".repeat(64));
        assert_eq!(
            read_json(&output.join("workers/manifest.json")).unwrap()["workers"],
            json!([])
        );
        let progress = read_json(&output.join("progress.json")).unwrap();
        assert_eq!(progress["reused"], 1);
        assert_eq!(progress["status"], "completed");
        Ok(())
    }

    #[test]
    fn compact_progress_counts_queued_in_flight_and_terminal_outcomes() -> Result<()> {
        let output =
            std::env::temp_dir().join(format!("performance-progress-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&output)?;
        let counters = ProgressCounters::default();
        counters.prepared.store(2, Ordering::Release);
        counters.failed.store(1, Ordering::Release);
        publish_progress(
            &output,
            &counters,
            5,
            10,
            32,
            Duration::from_secs(12),
            "running",
        )?;
        let progress = read_json(&output.join("progress.json")).unwrap();
        assert_eq!(progress["prepared"], 2);
        assert_eq!(progress["failed"], 1);
        assert_eq!(progress["in_flight"], 2);
        assert_eq!(progress["queued"], 5);
        assert_eq!(progress["unresolved"], 7);
        assert_eq!(progress["requested_workers"], 32);
        assert_eq!(progress["elapsed_seconds"], 12.0);
        counters.prepared.store(9, Ordering::Release);
        publish_progress(
            &output,
            &counters,
            42,
            10,
            32,
            Duration::from_secs(20),
            "failed",
        )?;
        let progress = read_json(&output.join("progress.json")).unwrap();
        assert_eq!(progress["started"], 10);
        assert_eq!(progress["in_flight"], 0);
        assert_eq!(progress["queued"], 0);
        assert_eq!(progress["unresolved"], 0);
        assert_eq!(progress["status"], "failed");
        Ok(())
    }

    #[test]
    fn preparation_target_requires_versioned_positive_integer() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("preparation-target-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let path = root.join("control.json");
        for workers in [1, 4, 48] {
            write_json(&path, &json!({"version":1,"workers":workers}))?;
            assert_eq!(read_worker_target(&path)?, workers);
        }
        for value in [
            json!({"version":2,"workers":4}),
            json!({"version":1,"workers":0}),
            json!({"version":1,"workers":-1}),
            json!({"version":1,"workers":1.5}),
            json!({"version":1,"workers":"4"}),
        ] {
            write_json(&path, &value)?;
            assert!(read_worker_target(&path).is_err());
        }
        Ok(())
    }

    #[test]
    fn rejects_zero_workers_before_creating_output() {
        let output = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let error = run(Args {
            input: vec![],
            source_manifest: None,
            output: output.clone(),
            workers: 0,
            reuse_from: None,
        })
        .unwrap_err();
        assert!(error.to_string().contains("positive"));
        assert!(!output.exists());
    }

    #[test]
    fn old_conversion_requests_keep_the_conversion_operation() {
        let request: Request = serde_json::from_value(json!({
            "id":"old", "input":"input.xls", "output":"output.xlsx", "directory":"runtime"
        }))
        .unwrap();
        assert!(!request.performance);
    }

    #[test]
    fn expected_baseline_mismatch_fails_before_leasing_excel() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("preparation-identity-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let baseline = root.join("source.xlsx");
        fs::write(&baseline, b"changed source")?;
        let pool = root.join("pool");
        fs::create_dir(&pool)?;
        write_json(
            &pool.join("error.json"),
            &json!({"error":"Excel leasing must not be reached"}),
        )?;
        let input = Input {
            source_id: "stable-source".into(),
            name: "source.xlsx".into(),
            baseline,
            expected_sha256: Some("a".repeat(64)),
            raw_source: None,
        };
        let error = prepare(&pool, &input, &root.join("result"), &"a".repeat(64)).unwrap_err();
        assert!(error.to_string().contains("expected source identity"));
        assert!(!root.join("result/prepared.json").exists());
        Ok(())
    }

    #[test]
    #[ignore = "requires desktop Excel with Check Performance and PERFORMANCE_PREPARATION_FIXTURE"]
    fn native_performance_pool_refills_fresh_excel_and_retains_controls() -> Result<()> {
        let input = PathBuf::from(std::env::var("PERFORMANCE_PREPARATION_FIXTURE")?);
        let output =
            std::env::temp_dir().join(format!("performance-pool-{}", uuid::Uuid::new_v4()));
        run(Args {
            input: vec![input.clone(), input.clone(), input],
            source_manifest: None,
            output: output.clone(),
            workers: 1,
            reuse_from: None,
        })?;
        let progress =
            read_json(&output.join("progress.json")).context("Progress receipt missing")?;
        assert_eq!(progress["status"], "completed");
        assert_eq!(progress["prepared"], 3);
        assert_eq!(progress["unresolved"], 0);
        let first = read_json(&output.join("input-000000/prepared.json"))
            .context("First receipt missing")?;
        let second = read_json(&output.join("input-000001/prepared.json"))
            .context("Second receipt missing")?;
        let third = read_json(&output.join("input-000002/prepared.json"))
            .context("Third receipt missing")?;
        let batch = read_json(&output.join("preparation.json")).context("Batch receipt missing")?;
        assert_eq!(first["producer_sha256"], batch["producer_sha256"]);
        assert_eq!(second["producer_sha256"], batch["producer_sha256"]);
        assert_ne!(
            first["measurements"]["excel_pid"],
            second["measurements"]["excel_pid"]
        );
        assert_ne!(
            first["measurements"]["excel_pid"],
            third["measurements"]["excel_pid"]
        );
        assert_ne!(
            second["measurements"]["excel_pid"],
            third["measurements"]["excel_pid"]
        );
        assert_eq!(first["source_sha256"], second["source_sha256"]);
        for receipt in [&first, &second, &third] {
            assert_eq!(receipt["measurements"]["worker_jobs_completed"], 1);
            assert_eq!(
                receipt["measurements"]["excel_reuse_policy"],
                "fresh-excel-per-input"
            );
            assert_eq!(
                receipt["native_result"]["status"], first["native_result"]["status"],
                "Repeating the same original must not reuse the previous optimization outcome"
            );
            assert_eq!(receipt["artifacts"].as_object().unwrap().len(), 3);
            assert!(matches!(
                receipt["native_result"]["status"].as_str(),
                Some("optimization_confirmed" | "no_optimization_needed")
            ));
        }
        for directory in ["input-000000", "input-000001", "input-000002"] {
            let receipt = read_json(&output.join(directory).join("prepared.json")).unwrap();
            assert_eq!(
                corpus_runner::artifacts::hash_file(&output.join(directory).join("baseline.xlsx"))?,
                receipt["source_sha256"]
            );
        }
        assert_eq!(
            read_json(&output.join("workers/status.json")).unwrap()["stopped"],
            true
        );
        for worker in fs::read_dir(output.join("workers"))? {
            let identity = worker?.path().join("identity.json");
            if let Some(identity) = read_json(&identity) {
                assert_eq!(
                    crate::native_process::process_alive(
                        identity["pid"].as_i64().unwrap(),
                        identity["started_ms"].as_i64().unwrap()
                    ),
                    Some(false)
                );
            }
        }
        Ok(())
    }
}
