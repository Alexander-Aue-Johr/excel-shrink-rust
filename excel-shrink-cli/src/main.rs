use clap::{Parser, ValueEnum};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::time::Instant;
use tracing::info;
use tracing_appender::rolling;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

mod pipeline;
mod regression;

#[derive(Parser, Debug)]
#[command(name = "excel-shrink", about = "Clean and shrink XLSX files.")]
pub struct Cli {
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
    #[arg(long, default_value_t = 134_217_728)]
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

    /// Disable keeping empty styleless cells that preserve default-font row layout.
    /// Without these anchors, deleting styleless empty cells can cause rows to shift.
    #[arg(long)]
    disable_default_font_representatives: bool,

    /// Disable keeping empty font-only style cells that conservatively preserve row height/alignment.
    #[arg(long)]
    disable_empty_font_style_representatives: bool,

    /// Disable keeping empty cells whose font style overrides an inherited column font.
    #[arg(long)]
    disable_column_font_style_representatives: bool,

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

    /// Keep cells that only contain whitespace, even when completely unreferenced.
    #[arg(long)]
    keep_unreferenced_whitespace_cells: bool,

    /// Remove cells with content in hidden rows/cols when they are completely unreferenced.
    #[arg(long)]
    remove_unreferenced_hidden_content_cells: bool,

    /// Treat applyProtection-only cell styles as removable when the worksheet has no active sheetProtection.
    /// By default, applyProtection styles are kept conservatively even on unprotected sheets.
    #[arg(long)]
    remove_apply_protection_without_sheet_protection: bool,

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
        let regression_tail: Vec<OsString> = raw_args.collect();
        let console_log_level =
            regression_console_log_level(&regression_tail).unwrap_or(LogLevel::Info);
        let regression_args = std::iter::once(program).chain(regression_tail);
        let _guard = init_logging(console_log_level, None)?;
        return regression::run_from_args(regression_args);
    }

    let cli = Cli::parse();
    if let Some(working_directory) = &cli.working_directory {
        std::env::set_current_dir(working_directory)?;
    }

    let _guard = init_logging(cli.console_log_level, cli.file_log_level)?;

    let start = Instant::now();
    info!("Starting excel-shrink pipeline...");

    let files = gather_files(&cli.path, cli.recursive)?;
    if files.is_empty() {
        info!("No XLSX files found.");
        return Ok(());
    }
    info!(files_total = files.len(), "Found XLSX files");

    pipeline::process_files(files, &cli)?;

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

fn init_logging(
    console_log_level: LogLevel,
    file_log_level: Option<LogLevel>,
) -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
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

    tracing_subscriber::registry()
        .with(file_layer)
        .with(console_layer)
        .try_init()?;

    Ok(guard)
}

fn gather_files(path: &PathBuf, recursive: bool) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    if path.is_file() && is_real_xlsx_file(path) {
        files.push(path.clone());
    } else if path.is_dir() {
        if recursive {
            for entry in walkdir::WalkDir::new(path)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if is_real_xlsx_file(entry.path()) {
                    files.push(entry.path().to_path_buf());
                }
            }
        } else {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let entry_path = entry.path();

                if is_real_xlsx_file(&entry_path) {
                    files.push(entry_path);
                }
            }
        }
    }

    Ok(files)
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
    fn cli_parses_disable_column_font_style_representatives() {
        let cli = Cli::parse_from([
            "excel-shrink",
            "input.xlsx",
            "out",
            "--disable-column-font-style-representatives",
        ]);

        assert!(cli.disable_column_font_style_representatives);
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
    fn cli_parses_disable_empty_font_style_representatives() {
        let cli = Cli::parse_from([
            "excel-shrink",
            "input.xlsx",
            "out",
            "--disable-empty-font-style-representatives",
        ]);

        assert!(cli.disable_empty_font_style_representatives);
    }
}

#[cfg(test)]
#[path = "corpus_regression_tests.rs"]
mod corpus_regression_tests;
