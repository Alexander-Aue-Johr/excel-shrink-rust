use clap::{Parser, ValueEnum};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::time::Instant;
use tracing::info;
use tracing_appender::rolling;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

mod pipeline;
mod profiling;
mod regression;
mod visual_profile;

pub(crate) const DEFAULT_PARALLEL_RANGE_CHUNK_SIZE: usize = 16 * 1024 * 1024;

#[derive(Parser, Debug)]
#[command(name = "excel-shrink", about = "Clean and shrink XLSX files.")]
pub struct Cli {
    /// Write a Perfetto-compatible trace to a new JSON file. Use {timestamp} for unique recordings.
    #[arg(long)]
    trace_file: Option<PathBuf>,

    /// Path to xlsx file or directory containing xlsx files
    path: PathBuf,

    /// Path to output directory or file
    output: PathBuf,

    /// Recursively process directories
    #[arg(long)]
    recursive: bool,

    /// Run in single-threaded mode (no multiprocessing)
    #[arg(long)]
    disable_multiprocessing: bool,

    /// Read buffer size for streaming XML preprocessing.
    /// This is not the number or size of Rayon work items.
    #[arg(long, default_value_t = 16_777_216)]
    chunk_size: usize,

    /// Maximum uncompressed XML bytes per parallel range chunk.
    /// If a concrete range is at or below this size, it is processed sequentially.
    /// Set to 0 to disable range chunking completely.
    #[arg(long, default_value_t = DEFAULT_PARALLEL_RANGE_CHUNK_SIZE)]
    parallel_range_chunk_size: usize,

    /// Force overwrite existing files
    #[arg(long)]
    force_overwrite: bool,

    /// Worksheet XML files to process. Omit to clean all sheets.
    /// Accepts values like sheet1.xml, worksheets/sheet1.xml, or xl/worksheets/sheet1.xml.
    #[arg(long, value_delimiter = ',', num_args = 1..)]
    sheets: Vec<String>,

    /// Only clean the workbook.xml file and not the sheet.xml files
    #[arg(long)]
    only_clean_workbook: bool,

    /// Disable splitting of sheet data range with cells and range with visible rows
    #[arg(long)]
    disable_sheetdata_splitting: bool,

    /// Run slower but attribution-safe analysis mode.
    #[arg(long)]
    analysis_mode: bool,

    /// Write per-sheet cleanup statistics to this CSV file.
    /// This also uses attribution-safe analysis behavior.
    #[arg(long)]
    analysis_csv: Option<PathBuf>,

    /// Resolve relative input, output, log, and analysis CSV paths from this directory.
    #[arg(long)]
    working_directory: Option<PathBuf>,

    /// Disable trailing sheetData range pruning in analysis-sensitive runs.
    #[arg(long)]
    disable_tail_pruning: bool,

    /// Disable trailing layout-only row cleanup.
    #[arg(long)]
    disable_trailing_layout_only_rows: bool,

    /// Disable extraneous column range merging.
    #[arg(long)]
    disable_extraneous_column_merge: bool,

    /// Disable merging borderless same-fill column tails.
    #[arg(long)]
    disable_column_tail_fill_merge: bool,

    /// Remove textless styleless/default-font representative cells.
    /// Default without this flag: these textless representatives are kept to preserve visual row layout.
    /// Aggressive cleanup opt-in: removing them can slightly change visual representation.
    #[arg(long)]
    remove_textless_default_font_representatives: bool,

    /// Remove textless cells with explicit style that can act as visual representatives.
    /// Default without this flag: these textless style representatives are kept.
    /// Aggressive cleanup opt-in: removing them can slightly change visual representation.
    #[arg(long)]
    remove_textless_style_representatives: bool,

    /// Remove textless cells whose visual relevance comes from column style/font overrides.
    /// Default without this flag: these textless column-style representatives are kept.
    /// Aggressive cleanup opt-in: removing them can slightly change visual representation.
    #[arg(long)]
    remove_textless_column_style_representatives: bool,

    /// Enable quick-win preprocessing that removes definitely safe empty self-closing cells.
    #[arg(long)]
    preprocess_empty_cells: bool,

    /// Number of XLSX assembler/writer threads.
    /// Defaults to the number of logical CPU cores.
    #[arg(long)]
    writer_workers: Option<usize>,

    /// Maximum number of workbooks that may be active at once.
    /// A workbook stays active until its output XLSX has been fully written.
    /// Defaults to the number of logical CPU cores.
    #[arg(long)]
    max_active_workbooks: Option<usize>,

    /// Deflate compression level for cleaned XML entries.
    /// 1 is much faster than the ZIP default and usually still compresses XLSX XML very well.
    #[arg(long, default_value_t = 2)]
    compression_level: i32,

    /// Store cleaned XML entries without Deflate compression.
    /// Useful for RAM-disk throughput benchmarks and for showing pure cleaning/package speed.
    #[arg(long)]
    store_output: bool,

    /// Remove unreferenced whitespace-only cells.
    /// Default without this flag: unreferenced whitespace-only cells are kept conservatively.
    /// Aggressive cleanup opt-in: removing them can slightly change visual representation.
    #[arg(long)]
    remove_unreferenced_whitespace_cells: bool,

    /// Keep unreferenced hidden-content cells.
    /// Default without this flag: unreferenced hidden-content cells are removed because this normally does not change visible representation.
    /// Conservative opt-in: use this to retain hidden content even when no reference is found.
    #[arg(long)]
    keep_unreferenced_hidden_content_cells: bool,

    /// Policy for cells that are only relevant because of applyProtection/protection style
    /// while the worksheet has no active sheetProtection.
    ///
    /// remove (default): remove inactive protection-only cells, including inside the visible data range.
    /// keep-in-data-range: keep inactive protection-only cells only inside the actual visible data range.
    /// keep-all: keep all inactive protection-only cells, including outside the actual visible data range.
    #[arg(long, value_enum, default_value_t = InactiveProtectionCellPolicyArg::Remove)]
    inactive_protection_cell_policy: InactiveProtectionCellPolicyArg,

    /// Console log level.
    #[arg(long, value_enum, default_value_t = LogLevel::Info)]
    console_log_level: LogLevel,

    /// File log level. Defaults to RUST_LOG when set, otherwise debug.
    #[arg(long, value_enum)]
    file_log_level: Option<LogLevel>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum InactiveProtectionCellPolicyArg {
    /// Remove inactive protection-only cells, including inside the visible data range.
    #[value(
        help = "Remove inactive protection-only cells, including inside the visible data range"
    )]
    Remove,
    /// Keep inactive protection-only cells only inside the actual visible data range.
    #[value(
        help = "Keep inactive protection-only cells only inside the actual visible data range"
    )]
    KeepInDataRange,
    /// Keep all inactive protection-only cells, including outside the actual visible data range.
    #[value(
        help = "Keep all inactive protection-only cells, including outside the actual visible data range"
    )]
    KeepAll,
}

impl LogLevel {
    fn as_filter(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    pub(crate) fn includes(self, message_level: Self) -> bool {
        self.verbosity_rank() >= message_level.verbosity_rank()
    }

    fn verbosity_rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warn => 1,
            Self::Info => 2,
            Self::Debug => 3,
            Self::Trace => 4,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let mut raw_args = std::env::args_os();
    let program = raw_args.next().unwrap_or_else(|| "excel-shrink".into());
    if raw_args.next().is_some_and(|arg| arg == "regression") {
        return run_regression_command(program, raw_args.collect());
    }

    run_cleanup_command(Cli::parse())
}

fn run_regression_command(program: OsString, args: Vec<OsString>) -> anyhow::Result<()> {
    let console_log_level = regression_console_log_level(&args).unwrap_or(LogLevel::Info);
    let regression_args = std::iter::once(program).chain(args);
    let parsed = regression::RegressionCli::parse_from(regression_args);
    let guard = init_logging(console_log_level, None, parsed.trace_file.as_deref())?;
    let result = regression::run(parsed);
    guard.finish()?;
    result
}

fn run_cleanup_command(cli: Cli) -> anyhow::Result<()> {
    set_working_directory(&cli)?;
    let guard = init_logging(
        cli.console_log_level,
        cli.file_log_level,
        cli.trace_file.as_deref(),
    )?;
    let result = run_cleanup_pipeline(&cli);
    guard.finish()?;
    result
}

fn set_working_directory(cli: &Cli) -> anyhow::Result<()> {
    if let Some(working_directory) = &cli.working_directory {
        std::env::set_current_dir(working_directory)?;
    }
    Ok(())
}

fn run_cleanup_pipeline(cli: &Cli) -> anyhow::Result<()> {
    let start = Instant::now();
    info!("Starting excel-shrink pipeline...");

    let files = gather_files(&cli.path, cli.recursive)?;
    if files.is_empty() {
        info!("No XLSX files found.");
        return Ok(());
    }
    info!(files_total = files.len(), "Found XLSX files");

    pipeline::process_files(files, cli)?;

    info!("Script completed in {:.2?}", start.elapsed());
    Ok(())
}

fn regression_console_log_level(args: &[OsString]) -> Option<LogLevel> {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_os_str();
        if arg == OsStr::new("--") {
            break;
        }
        if arg == OsStr::new("--cli-arg") {
            index += 2;
            continue;
        }
        if let Some(arg) = arg.to_str() {
            if let Some(value) = arg.strip_prefix("--console-log-level=") {
                return parse_log_level(value);
            }
            if arg == "--console-log-level" {
                return args
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .and_then(parse_log_level);
            }
        }
        index += 1;
    }
    None
}

fn parse_log_level(value: &str) -> Option<LogLevel> {
    match value.trim().to_ascii_lowercase().as_str() {
        "error" => Some(LogLevel::Error),
        "warn" => Some(LogLevel::Warn),
        "info" => Some(LogLevel::Info),
        "debug" => Some(LogLevel::Debug),
        "trace" => Some(LogLevel::Trace),
        _ => None,
    }
}

struct LoggingGuard {
    trace: Option<profiling::TraceGuard>,
    _logs: tracing_appender::non_blocking::WorkerGuard,
}

impl LoggingGuard {
    fn finish(mut self) -> std::io::Result<()> {
        if let Some(trace) = self.trace.take() {
            trace.finish()?;
        }
        Ok(())
    }
}

fn init_logging(
    console_log_level: LogLevel,
    file_log_level: Option<LogLevel>,
    trace_file: Option<&std::path::Path>,
) -> anyhow::Result<LoggingGuard> {
    std::fs::create_dir_all("logs")?;

    let file_appender = rolling::never("logs", "excel-shrink-run.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let file_filter = match file_log_level {
        Some(level) => EnvFilter::new(level.as_filter()),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
    };
    let console_filter = EnvFilter::new(console_log_level.as_filter());

    let file_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_filter(file_filter);

    let console_layer = fmt::layer()
        .compact()
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_filter(console_filter);

    let (trace_layer, trace_guard) = match trace_file {
        Some(path) => {
            let (layer, guard) = profiling::create(path)?;
            (Some(layer), Some(guard))
        }
        None => (None, None),
    };
    let profile_filter =
        tracing_subscriber::filter::filter_fn(|metadata| metadata.target() == profiling::TARGET);
    let log_filter =
        tracing_subscriber::filter::filter_fn(|metadata| metadata.target() != profiling::TARGET);
    tracing_subscriber::registry()
        .with(trace_layer.map(|layer| layer.with_filter(profile_filter)))
        .with(file_layer.with_filter(log_filter.clone()))
        .with(console_layer.with_filter(log_filter))
        .try_init()?;

    Ok(LoggingGuard {
        trace: trace_guard,
        _logs: guard,
    })
}

fn gather_files(path: &PathBuf, recursive: bool) -> std::io::Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(is_real_xlsx_file(path)
            .then(|| path.clone())
            .into_iter()
            .collect());
    }

    if !path.is_dir() {
        return Ok(Vec::new());
    }

    if recursive {
        Ok(gather_files_recursively(path))
    } else {
        gather_files_in_directory(path)
    }
}

fn gather_files_recursively(path: &std::path::Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| is_real_xlsx_file(entry.path()))
        .map(|entry| entry.path().to_path_buf())
        .collect()
}

fn gather_files_in_directory(path: &std::path::Path) -> std::io::Result<Vec<PathBuf>> {
    std::fs::read_dir(path)?
        .filter_map(|entry| match entry {
            Ok(entry) if is_real_xlsx_file(&entry.path()) => Some(Ok(entry.path())),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn is_real_xlsx_file(path: &std::path::Path) -> bool {
    let is_xlsx = path
        .extension()
        .map_or(false, |ext| ext.eq_ignore_ascii_case("xlsx"));

    let is_excel_lock_file = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("~$"));

    is_xlsx && !is_excel_lock_file
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn os_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn regression_console_log_level_reads_subcommand_flag() {
        assert_eq!(
            regression_console_log_level(&os_args(&[
                "smoke",
                "--excel-screenshot-check",
                "--console-log-level",
                "error",
            ])),
            Some(LogLevel::Error)
        );
    }

    #[test]
    fn regression_console_log_level_ignores_cli_arg_values() {
        assert_eq!(
            regression_console_log_level(&os_args(&[
                "smoke",
                "--cli-arg",
                "--console-log-level",
                "--cli-arg",
                "error",
            ])),
            None
        );
    }

    #[test]
    fn cli_parses_remove_textless_column_style_representatives() {
        let cli = Cli::parse_from([
            "excel-shrink",
            "input.xlsx",
            "out",
            "--remove-textless-column-style-representatives",
        ]);

        assert!(cli.remove_textless_column_style_representatives);
    }

    #[test]
    fn cli_parses_remove_textless_default_font_representatives() {
        let cli = Cli::parse_from([
            "excel-shrink",
            "input.xlsx",
            "out",
            "--remove-textless-default-font-representatives",
        ]);

        assert!(cli.remove_textless_default_font_representatives);
    }

    #[test]
    fn cli_parses_disable_column_tail_fill_merge() {
        let cli = Cli::parse_from([
            "excel-shrink",
            "input.xlsx",
            "out",
            "--disable-column-tail-fill-merge",
        ]);

        assert!(cli.disable_column_tail_fill_merge);
    }

    #[test]
    fn cli_parses_remove_textless_style_representatives() {
        let cli = Cli::parse_from([
            "excel-shrink",
            "input.xlsx",
            "out",
            "--remove-textless-style-representatives",
        ]);

        assert!(cli.remove_textless_style_representatives);
    }

    #[test]
    fn cli_parses_advanced_cleanup_flags_with_new_polarity() {
        let cli = Cli::parse_from([
            "excel-shrink",
            "input.xlsx",
            "out",
            "--remove-unreferenced-whitespace-cells",
            "--keep-unreferenced-hidden-content-cells",
        ]);

        assert!(cli.remove_unreferenced_whitespace_cells);
        assert!(cli.keep_unreferenced_hidden_content_cells);
    }

    #[test]
    fn cli_parses_inactive_protection_policy_values() {
        let keep_in_data_range = Cli::parse_from([
            "excel-shrink",
            "input.xlsx",
            "out",
            "--inactive-protection-cell-policy",
            "keep-in-data-range",
        ]);
        let keep_all = Cli::parse_from([
            "excel-shrink",
            "input.xlsx",
            "out",
            "--inactive-protection-cell-policy",
            "keep-all",
        ]);

        assert_eq!(
            keep_in_data_range.inactive_protection_cell_policy,
            InactiveProtectionCellPolicyArg::KeepInDataRange
        );
        assert_eq!(
            keep_all.inactive_protection_cell_policy,
            InactiveProtectionCellPolicyArg::KeepAll
        );
    }

    #[test]
    fn cli_defaults_inactive_protection_policy_to_remove() {
        let cli = Cli::parse_from(["excel-shrink", "input.xlsx", "out"]);

        assert_eq!(
            cli.inactive_protection_cell_policy,
            InactiveProtectionCellPolicyArg::Remove
        );
    }

    #[test]
    fn cli_rejects_removed_cleanup_flag_names() {
        for old_flag in [
            "--disable-default-font-representatives",
            "--disable-empty-style-representatives",
            "--disable-column-font-style-representatives",
            "--remove-unreferenced-hidden-content-cells",
            "--remove-inactive-protection-cells",
        ] {
            let result = Cli::try_parse_from(["excel-shrink", "input.xlsx", "out", old_flag]);

            assert!(result.is_err(), "{old_flag} should not be accepted");
        }
    }

    #[test]
    fn cli_help_documents_cleanup_flag_semantics() {
        let mut help = Vec::new();
        Cli::command().write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();

        assert!(help.contains("Aggressive cleanup opt-in"));
        assert!(help
            .contains("Default without this flag: unreferenced hidden-content cells are removed"));
        assert!(help.contains("remove (default): remove inactive protection-only cells"));
        assert!(
            help.contains("keep-in-data-range: keep inactive protection-only cells only inside")
        );
        assert!(help.contains("keep-all: keep all inactive protection-only cells"));
    }
}

#[cfg(test)]
#[path = "corpus_regression_tests.rs"]
mod corpus_regression_tests;
