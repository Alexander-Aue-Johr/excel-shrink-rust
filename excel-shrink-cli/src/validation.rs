//! SheetScope: configure producers, compare their workbooks, retain independent evidence.
//! The original comparison implementation remains the only scientific execution path.

mod capture;
mod pairing;
mod run;
mod storage;

use crate::validation_process::Stage;
use crate::validation_reuse::PartSelection;
use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{ffi::OsStr, fs, path::{Path, PathBuf}};

pub(crate) use run::{execute, LiveProgress, RunState};
pub(crate) use run::read_run;

#[derive(Debug, Parser)]
#[command(name = "sheet-scope", about = "Targeted visual validation of spreadsheet transformations.")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Execute a complete JSON configuration without opening the configuration UI.
    Run {
        #[arg(long)]
        config: PathBuf,
    },
    /// Compare existing folders, ZIP files or archive URLs without running a producer.
    Compare {
        #[arg(long)]
        a: String,
        #[arg(long)]
        b: String,
        #[arg(long)]
        report: PathBuf,
        #[arg(long)]
        continue_on_error: bool,
        #[arg(long)]
        reuse_from: Option<PathBuf>,
        /// Comma-separated OOXML categories; omitted means all package members.
        #[arg(long, value_delimiter = ',')]
        reuse_parts: Vec<String>,
        /// User-recorded renderer/font/environment identity for deliberate result reuse.
        #[arg(long, default_value = "")]
        environment_key: String,
    },
    /// Configure a run interactively; no source or processor arguments are required.
    Observatory {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long)]
        no_open: bool,
    },
    /// Open an existing independent report without Excel or the original workbooks.
    Open { report: PathBuf },
    /// Copy a completed report into a new, publishable static website directory.
    Export { report: PathBuf, output: PathBuf },
    /// Print an editable configuration; arguments are an array, never a shell string.
    Template,
    #[command(hide = true)]
    ComparePair(crate::regression::targeted_comparison::Args),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Output {
    Temporary,
    Directory { path: PathBuf },
    Archive { path: PathBuf },
}

impl Default for Output {
    fn default() -> Self { Self::Temporary }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CaptureOptions {
    pub operation_timeout_seconds: u64,
    pub idle_timeout_seconds: u64,
    pub serial_captures: bool,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self { operation_timeout_seconds: 120, idle_timeout_seconds: 120, serial_captures: false }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RunConfig {
    pub source_a: String,
    pub source_b: Option<String>,
    pub steps: Vec<Stage>,
    /// One-based step number whose output becomes A (e.g. an explicitly accepted XLS conversion).
    pub baseline_after_step: Option<usize>,
    pub report_dir: PathBuf,
    pub output: Output,
    pub cache_dir: PathBuf,
    pub refresh_downloads: bool,
    pub max_processes: usize,
    pub continue_on_error: bool,
    pub stage_cache_dir: Option<PathBuf>,
    /// Change this when a processor's dependencies or external environment change.
    pub processing_cache_key: String,
    pub reuse_from: Option<PathBuf>,
    pub reuse_parts: PartSelection,
    /// An explicit declaration when reusing observations made with the same renderer setup.
    pub environment_key: String,
    pub capture: CaptureOptions,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            source_a: String::new(), source_b: None, steps: Vec::new(), baseline_after_step: None,
            report_dir: PathBuf::new(), output: Output::Temporary,
            cache_dir: PathBuf::from(".tmp/sheet-scope/downloads"),
            refresh_downloads: false, max_processes: 1, continue_on_error: false,
            stage_cache_dir: None, processing_cache_key: String::new(),
            reuse_from: None, reuse_parts: PartSelection::default(),
            environment_key: String::new(), capture: CaptureOptions::default(),
        }
    }
}

impl RunConfig {
    /// Resolve a file's paths against its own directory, not a later browser working directory.
    pub fn from_file(path: &Path) -> Result<Self> {
        let path = fs::canonicalize(path)?;
        let mut config: Self = serde_json::from_slice(&fs::read(&path)?)?;
        config.resolve_paths(path.parent().context("Configuration has no parent")?)?;
        Ok(config)
    }

    pub fn resolve_paths(&mut self, base: &Path) -> Result<()> {
        self.source_a = resolve_source(&self.source_a, base);
        self.source_b = self.source_b.as_ref().map(|source| resolve_source(source, base));
        for path in [&mut self.report_dir, &mut self.cache_dir] {
            if !path.as_os_str().is_empty() && path.is_relative() { *path = base.join(&*path); }
        }
        for path in [&mut self.reuse_from, &mut self.stage_cache_dir].into_iter().flatten() {
            if path.is_relative() { *path = base.join(&*path); }
        }
        match &mut self.output {
            Output::Directory { path } | Output::Archive { path } if path.is_relative() => {
                *path = base.join(&*path);
            }
            _ => {}
        }
        for stage in &mut self.steps {
            if stage.program.is_relative() { stage.program = base.join(&stage.program); }
            if let Some(directory) = &mut stage.working_directory {
                if directory.is_relative() { *directory = base.join(&*directory); }
            }
            for dependency in &mut stage.dependencies {
                if dependency.is_relative() { *dependency = base.join(&*dependency); }
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(!self.source_a.trim().is_empty(), "Select source A");
        ensure!(!self.report_dir.as_os_str().is_empty(), "Select an independent report directory");
        ensure!(!self.report_dir.exists(), "Report directory already exists; choose a new run directory");
        ensure!((1..=64).contains(&self.max_processes), "Process parallelism must be between 1 and 64");
        ensure!((1..=86400).contains(&self.capture.operation_timeout_seconds)
            && (1..=86400).contains(&self.capture.idle_timeout_seconds),
            "Capture timeouts must be between 1 and 86400 seconds");
        self.reuse_parts.validate()?;
        match &self.source_b {
            Some(source) => {
                ensure!(!source.trim().is_empty(), "Source B is empty");
                ensure!(self.steps.is_empty(), "Choose source B or processing steps, not both");
                ensure!(matches!(self.output, Output::Temporary),
                    "An output destination only applies when processing new workbooks");
            }
            None => ensure!(!self.steps.is_empty(), "Select source B or add a processing step"),
        }
        if self.stage_cache_dir.is_some() {
            ensure!(!self.processing_cache_key.trim().is_empty(),
                "Persistent processing requires a processing_cache_key for external dependencies");
        }
        if let Some(step) = self.baseline_after_step {
            ensure!(self.source_b.is_none() && step > 0 && step < self.steps.len(),
                "baseline_after_step must select an intermediate processing step (one-based)");
        }
        if self.reuse_from.is_some() {
            ensure!(!self.environment_key.trim().is_empty(),
                "Record an environment_key before reusing visual observations");
        }
        if let Output::Directory { path } | Output::Archive { path } = &self.output {
            ensure!(!path.as_os_str().is_empty() && !path.exists(),
                "The processing destination must be a new path: {}", path.display());
        }
        storage::validate_locations(self)
    }
}

fn resolve_source(value: &str, base: &Path) -> String {
    if value.is_empty() || value.starts_with("https://") || value.starts_with("http://")
        || Path::new(value).is_absolute() {
        value.to_owned()
    } else {
        base.join(value).to_string_lossy().into_owned()
    }
}

pub(crate) fn run_cli() -> Result<()> {
    // Resident Office.js hosts reuse the current binary. This internal transport
    // remains available independently of the user-facing command grammar.
    if std::env::args_os().nth(1).as_deref() == Some(OsStr::new("capture-host")) {
        return crate::capture_host::run(std::env::args_os().skip(2).collect());
    }
    match Cli::parse().command {
        None => crate::validation_room::serve(RunConfig::default(), 0, true),
        Some(Command::Observatory { config, port, no_open }) => {
            let config = config.as_deref().map(RunConfig::from_file).transpose()?.unwrap_or_default();
            crate::validation_room::serve(config, port, !no_open)
        }
        Some(Command::Run { config }) => {
            execute(RunConfig::from_file(&config)?, None)?;
            Ok(())
        }
        Some(Command::Compare { a, b, report, continue_on_error, reuse_from, reuse_parts, environment_key }) => {
            let mut config = RunConfig { source_a: a, source_b: Some(b), report_dir: report,
                continue_on_error, reuse_from, environment_key, ..RunConfig::default() };
            if !reuse_parts.is_empty() { config.reuse_parts = selected_parts(&reuse_parts)?; }
            config.resolve_paths(&std::env::current_dir()?)?;
            execute(config, None)?;
            Ok(())
        }
        Some(Command::Open { report }) => {
            let root = report_root(&report)?;
            let run = read_run(&root)?;
            let index = crate::validation_report::write_site(&root, &run)?;
            open_browser(index.as_os_str())
        }
        Some(Command::Export { report, output }) => {
            let root = report_root(&report)?;
            crate::validation_report::export_site(&root, &output)?;
            println!("Static report: {}", output.display());
            Ok(())
        }
        Some(Command::Template) => {
            println!("{}", serde_json::to_string_pretty(&RunConfig::default())?);
            Ok(())
        }
        Some(Command::ComparePair(args)) => crate::regression::targeted_comparison::run(args),
    }
}

fn selected_parts(names: &[String]) -> Result<PartSelection> {
    let mut parts = PartSelection {
        worksheets: false, workbook: false, styles: false, shared_strings: false,
        relationships: false, drawings: false, charts: false, other: false, extra_parts: Vec::new(),
    };
    for name in names {
        match name.as_str() {
            "worksheets" => parts.worksheets = true,
            "workbook" => parts.workbook = true,
            "styles" => parts.styles = true,
            "shared-strings" | "shared_strings" => parts.shared_strings = true,
            "relationships" => parts.relationships = true,
            "drawings" => parts.drawings = true,
            "charts" => parts.charts = true,
            "other" => parts.other = true,
            _ => anyhow::bail!("Unknown OOXML category: {name}"),
        }
    }
    parts.validate()?;
    Ok(parts)
}

pub(crate) fn report_root(path: &Path) -> Result<PathBuf> {
    let path = fs::canonicalize(path)?;
    let root = if path.is_file() {
        ensure!(path.file_name() == Some(OsStr::new("run.json")), "Select run.json or its directory");
        path.parent().context("Report has no parent")?.to_path_buf()
    } else { path };
    ensure!(root.join("run.json").is_file(), "SheetScope run.json is missing");
    Ok(root)
}

/// Hand the exact path/URL to the OS without evaluating it as shell code.
pub(crate) fn open_browser(target: &OsStr) -> Result<()> {
    #[cfg(windows)]
    let target = {
        let text = target.to_string_lossy();
        if text.starts_with("http://") || text.starts_with("https://") {
            text.into_owned()
        } else {
            crate::regression::power_shell_path(Path::new(target))?
        }
    };
    #[cfg(windows)]
    let result = std::process::Command::new("rundll32.exe")
        .arg("url.dll,FileProtocolHandler").arg(&target).spawn();
    #[cfg(not(windows))]
    let result = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" })
        .arg(target).spawn();
    match result {
        Ok(_) => Ok(()),
        Err(error) => {
            eprintln!("Open {} in your browser ({error})", Path::new(&target).display());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cli_opens_configuration_and_direct_mode_needs_no_processor() {
        assert!(Cli::try_parse_from(["sheet-scope"]).unwrap().command.is_none());
        assert!(Cli::try_parse_from(["sheet-scope", "compare", "--a", "A", "--b", "B", "--report", "report"]).is_ok());
        assert!(Cli::try_parse_from(["sheet-scope", "open", "report/run.json"]).is_ok());
    }

    #[test]
    fn reuse_selection_is_explicit_and_rejects_typographical_errors() {
        let selected = selected_parts(&["worksheets".into(), "shared-strings".into()]).unwrap();
        assert!(selected.worksheets && selected.shared_strings);
        assert!(!selected.styles && !selected.other);
        assert!(selected_parts(&["workbok".into()]).is_err());
    }
}
