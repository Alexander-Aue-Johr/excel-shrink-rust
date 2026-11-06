//! External workbook processors for SheetScope.
//!
//! A stage invokes a declared executable with an argument vector. Processing
//! outcomes and logs are evidence; only accepted outputs become the next stage.
mod invocation;
#[cfg(test)]
mod tests;

use crate::validation_sources::{self as sources, MaterializedSource, PortableNames};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Invocation granularity is independent of the processor's output contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InputKind {
    File,
    Directory,
    Archive,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutputKind {
    File,
    #[default]
    Directory,
    Archive,
}

/// Paths are passed as individual arguments, never joined into a shell command.
/// Use `environment` only for explicit overrides; inherited variables are not
/// captured as reproducibility evidence or assumed by persistent result reuse.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Stage {
    pub name: String,
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub input: InputKind,
    #[serde(default)]
    pub output: OutputKind,
    #[serde(default)]
    pub output_extension: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub working_directory: Option<PathBuf>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Scripts/configuration read by the executable; the run coordinator hashes
    /// these explicitly when reusing persistent processing results.
    #[serde(default)]
    pub dependencies: Vec<PathBuf>,
}

fn default_timeout() -> u64 {
    3600
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProcessStatus {
    Pending,
    Running,
    Succeeded,
    Incomplete,
    Failed,
    TimedOut,
    Skipped,
}

/// Log paths are relative to the stage's log directory. A running receipt left
/// after a crash is an interrupted attempt, never an implicit successful result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProcessOutcome {
    pub invocation: usize,
    pub stage: String,
    pub inputs: Vec<String>,
    pub expected_outputs: Vec<String>,
    pub produced_outputs: Vec<String>,
    pub accepted_outputs: Vec<String>,
    pub missing_outputs: Vec<String>,
    pub status: ProcessStatus,
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub working_directory: Option<PathBuf>,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub elapsed_ms: u64,
    pub exit_code: Option<i32>,
    pub exit_description: Option<String>,
    pub error: Option<String>,
}

/// A caller must retain owned working data and stop scheduling when this error
/// is present anywhere in the anyhow chain. Presence/absence of an output file
/// cannot establish that an external process has exited.
#[derive(Debug)]
pub(crate) struct UnconfirmedTermination {
    detail: String,
}

impl std::fmt::Display for UnconfirmedTermination {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Owned processor termination could not be confirmed: {}", self.detail)
    }
}

impl std::error::Error for UnconfirmedTermination {}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StageResult {
    pub source: MaterializedSource,
    pub outcomes: Vec<ProcessOutcome>,
    /// Failed/incomplete invocations, not a claim about every workbook in a batch.
    pub failed: usize,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum StageState {
    Preparing,
    Running,
    Completed,
    Incomplete,
    Failed,
}

#[derive(Serialize)]
struct StageRecord<'a> {
    schema: u32,
    state: StageState,
    stage: &'a Stage,
    input_count: usize,
    max_processes: usize,
    continue_on_error: bool,
    outcomes: &'a [ProcessOutcome],
    error: Option<String>,
}

struct PlannedInvocation {
    id: usize,
    inputs: Vec<String>,
    expected: BTreeMap<String, PathBuf>,
    input_path: PathBuf,
    output_path: PathBuf,
    output_directory: PathBuf,
    output_file: Option<PathBuf>,
    archive_directory: PathBuf,
}

/// Run one stage with bounded subprocess concurrency. Batch inputs start exactly
/// one process. A fail-fast run stops dispatching after the first observed
/// failure, finishes already-owned processes, persists every outcome, and errs.
///
/// `output_root` contains only workbook results; `logs_root` belongs to the report.
/// Both are caller-selected. Expected output paths must be new, preventing stale
/// results from being mistaken for outputs of this attempt.
pub(crate) fn run_stage(
    stage: &Stage,
    inputs: &MaterializedSource,
    output_root: &Path,
    logs_root: &Path,
    max_processes: usize,
    continue_on_error: bool,
) -> Result<StageResult> {
    validate_stage(stage, inputs, max_processes)?;
    let output_root = prepare_output_directory(stage, inputs, output_root)?;
    fs::create_dir_all(logs_root)?;
    let logs_root = fs::canonicalize(logs_root)?;
    ensure!(!logs_root.join("stage.json").exists(), "Stage log directory already contains an attempt");
    ensure!(!logs_root.starts_with(&output_root), "Stage logs must be separate from processing outputs");
    let mut record = StageRecord {
        schema: 1,
        state: StageState::Preparing,
        stage,
        input_count: inputs.files.len(),
        max_processes: if stage.input == InputKind::File { max_processes } else { 1 },
        continue_on_error,
        outcomes: &[],
        error: None,
    };
    persist_stage(&logs_root, &record)?;

    let result = execute_stage(stage, inputs, &output_root, &logs_root, &mut record);
    match result {
        Ok(result) => {
            record.state = if result.failed == 0 { StageState::Completed } else { StageState::Incomplete };
            record.outcomes = &result.outcomes;
            persist_stage(&logs_root, &record)?;
            ensure!(continue_on_error || result.failed == 0,
                "Stage '{}' failed; {} invocation(s) were unsuccessful. See {}",
                stage.name, result.failed, logs_root.join("stage.json").display());
            Ok(result)
        }
        Err(error) => {
            record.state = StageState::Failed;
            record.error = Some(format!("{error:#}"));
            if let Err(publication_error) = persist_stage(&logs_root, &record) {
                return Err(error.context(format!("Failure record also could not be persisted: {publication_error:#}")));
            }
            Err(error)
        }
    }
}

fn execute_stage(
    stage: &Stage,
    inputs: &MaterializedSource,
    output_root: &Path,
    logs_root: &Path,
    record: &mut StageRecord<'_>,
) -> Result<StageResult> {
    let scratch = ScratchDirectory::create(output_root.parent().context("Output has no parent")?)?;
    let plan = plan_invocations(stage, inputs, output_root, &scratch.path)?;
    record.state = StageState::Running;
    persist_stage(logs_root, record)?;
    let outcomes = match invocation::execute(
        stage, plan, output_root, logs_root, record.max_processes, record.continue_on_error,
    ) {
        Ok(outcomes) => outcomes,
        Err(error) => {
            if error.is::<UnconfirmedTermination>() {
                let retained = scratch.path.clone();
                scratch.preserve();
                return Err(error.context(format!("Retained processing scratch at {}", retained.display())));
            }
            return Err(error);
        }
    };
    scratch.remove()?;
    let result = accepted_results(output_root, outcomes);
    Ok(result)
}

fn accepted_results(output_root: &Path, outcomes: Vec<ProcessOutcome>) -> StageResult {
    let mut files = BTreeMap::new();
    let mut failed = 0;
    for outcome in &outcomes {
        if !matches!(outcome.status, ProcessStatus::Succeeded | ProcessStatus::Skipped) {
            failed += 1;
        }
        for name in &outcome.accepted_outputs {
            files.insert(name.clone(), output_root.join(name));
        }
    }
    StageResult {
        source: MaterializedSource { root: output_root.to_owned(), files, archive: None },
        outcomes,
        failed,
    }
}

fn validate_stage(stage: &Stage, inputs: &MaterializedSource, max_processes: usize) -> Result<()> {
    ensure!(!stage.name.trim().is_empty() && !stage.name.chars().any(char::is_control),
        "A processing stage needs a printable name");
    let program = fs::canonicalize(&stage.program)
        .context("Processor must name an existing executable file")?;
    sources::require_regular_file(&program)?;
    for dependency in &stage.dependencies {
        let dependency = fs::canonicalize(dependency)
            .with_context(|| format!("Resolve processor dependency {}", dependency.display()))?;
        sources::require_regular_file(&dependency)?;
    }
    ensure!(stage.timeout_seconds > 0, "Stage timeout must be greater than zero");
    ensure!(max_processes > 0, "At least one processing slot is required");
    ensure!(!inputs.files.is_empty(), "Stage '{}' has no input workbooks", stage.name);
    ensure!(stage.input == InputKind::File || stage.output != OutputKind::File,
        "A directory/archive invocation cannot produce one workbook file");
    if let Some(directory) = &stage.working_directory {
        ensure!(directory.is_dir(), "Processor working directory does not exist: {}", directory.display());
    }
    for (name, value) in &stage.environment {
        ensure!(!name.is_empty() && !name.contains(['=', '\0']) && !value.contains('\0'),
            "Invalid environment variable override: {name}");
    }
    invocation::validate_templates(stage)?;
    let mut source_names = PortableNames::default();
    let mut output_names = PortableNames::default();
    for (name, path) in &inputs.files {
        source_names.insert(name, false)?;
        sources::require_regular_file(path)?;
        output_names.insert(&output_name(name, stage.output_extension.as_deref())?, false)?;
    }
    Ok(())
}

fn prepare_output_directory(stage: &Stage, inputs: &MaterializedSource, requested: &Path) -> Result<PathBuf> {
    fs::create_dir_all(requested)?;
    let output = fs::canonicalize(requested)?;
    let source_root = fs::canonicalize(&inputs.root)?;
    ensure!(output != source_root, "Processing output must be separate from its inputs");
    if stage.input == InputKind::Directory {
        ensure!(!output.starts_with(&source_root),
            "Directory processor output must not be inside its input directory");
    }
    if stage.input != InputKind::File {
        ensure!(fs::read_dir(&output)?.next().is_none(),
            "Batch processor output directory must be empty: {}", output.display());
    }
    for name in inputs.files.keys() {
        let target = output.join(output_name(name, stage.output_extension.as_deref())?);
        ensure!(!target.try_exists()?, "Expected output already exists: {}", target.display());
    }
    Ok(output)
}

fn output_name(input: &str, extension: Option<&str>) -> Result<String> {
    let input = sources::relative_name(input)?;
    let Some(extension) = extension else { return Ok(input) };
    let extension = extension.strip_prefix('.').unwrap_or(extension);
    ensure!(!extension.is_empty() && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "Output extension must contain only letters and digits");
    let mut name = PathBuf::from(&input);
    name.set_extension(extension);
    ensure!(sources::is_workbook(&name), "Unsupported output workbook extension: {extension}");
    sources::relative_name(&name.to_string_lossy().replace('\\', "/"))
}

fn plan_invocations(
    stage: &Stage,
    source: &MaterializedSource,
    output_root: &Path,
    scratch: &Path,
) -> Result<Vec<PlannedInvocation>> {
    if stage.input == InputKind::File {
        return source.files.iter().enumerate().map(|(index, (name, input))| {
            let output_name = output_name(name, stage.output_extension.as_deref())?;
            let output = output_root.join(&output_name);
            let directory = output.parent().context("Output workbook has no directory")?.to_owned();
            let id = index + 1;
            let output_path = match stage.output {
                OutputKind::File => output.clone(),
                OutputKind::Directory => directory.clone(),
                OutputKind::Archive => scratch.join(format!("result-{id}.zip")),
            };
            Ok(PlannedInvocation {
                id,
                inputs: vec![name.clone()],
                expected: BTreeMap::from([(output_name, output.clone())]),
                input_path: fs::canonicalize(input)?,
                output_path,
                output_directory: directory,
                output_file: Some(output),
                archive_directory: scratch.join(format!("unpacked-{id}")),
            })
        }).collect();
    }
    let input_path = prepare_batch_input(stage.input, source, scratch)?;
    let output_path = if stage.output == OutputKind::Archive {
        scratch.join("result.zip")
    } else {
        output_root.to_owned()
    };
    let expected = source.files.keys().map(|name| {
        let name = output_name(name, stage.output_extension.as_deref())?;
        let path = output_root.join(&name);
        Ok((name, path))
    }).collect::<Result<BTreeMap<_, _>>>()?;
    Ok(vec![PlannedInvocation {
        id: 1,
        inputs: source.files.keys().cloned().collect(),
        expected,
        input_path,
        output_path,
        output_directory: output_root.to_owned(),
        output_file: None,
        archive_directory: scratch.join("unpacked"),
    }])
}

fn prepare_batch_input(kind: InputKind, source: &MaterializedSource, scratch: &Path) -> Result<PathBuf> {
    if kind == InputKind::Archive {
        if let Some(archive) = &source.archive {
            sources::require_regular_file(archive)?;
            return Ok(fs::canonicalize(archive)?);
        }
        let archive = scratch.join("input.zip");
        sources::archive_files(&source.files, &archive)?;
        return Ok(archive);
    }
    // Never hand failed, stale or unrelated workbook outputs to the next stage.
    // A complete directory input remains in place, avoiding a full corpus copy.
    if sources::discover(&source.root)?.files == source.files {
        return Ok(fs::canonicalize(&source.root)?);
    }
    let directory = scratch.join("input");
    fs::create_dir(&directory)?;
    for (name, path) in &source.files {
        let target = directory.join(name);
        fs::create_dir_all(target.parent().context("Input workbook has no directory")?)?;
        fs::copy(path, &target).with_context(|| format!("Prepare accepted input {name}"))?;
    }
    Ok(directory)
}

fn persist_stage(directory: &Path, record: &StageRecord<'_>) -> Result<()> {
    crate::regression::write_atomic_artifact(&directory.join("stage.json"), &serde_json::to_vec_pretty(record)?)
}

struct ScratchDirectory {
    path: PathBuf,
    cleanup_on_drop: bool,
}

impl ScratchDirectory {
    fn create(parent: &Path) -> Result<Self> {
        let path = parent.join(format!(".sheet-scope-stage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path)?;
        Ok(Self { path, cleanup_on_drop: true })
    }

    fn preserve(mut self) {
        tracing::error!("Retaining processing scratch after unconfirmed process cleanup: {}", self.path.display());
        self.cleanup_on_drop = false;
    }

    fn remove(mut self) -> Result<()> {
        fs::remove_dir_all(&self.path)
            .with_context(|| format!("Remove processing scratch directory {}", self.path.display()))?;
        self.cleanup_on_drop = false;
        Ok(())
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            if let Err(error) = fs::remove_dir_all(&self.path) {
                tracing::warn!("Could not remove {}: {error}", self.path.display());
            }
        }
    }
}
