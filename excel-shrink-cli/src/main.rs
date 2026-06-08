use clap::Parser;
use std::path::PathBuf;
use std::time::Instant;
use tracing::info;
use tracing_appender::rolling;
use tracing_subscriber::{fmt, EnvFilter};

mod pipeline;

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

    /// Set the chunk size for processing XML data
    #[arg(long, default_value_t = 16_777_216)]
    chunk_size: usize,

    /// Force overwrite existing files
    #[arg(long)]
    force_overwrite: bool,

    /// List of sheet names to process (e.g. sheet1.xml sheet2.xml)
    #[arg(long, value_delimiter = ' ')]
    sheets: Option<Vec<String>>,

    /// Only clean the workbook.xml file and not the sheet.xml files
    #[arg(long)]
    only_clean_workbook: bool,

    /// Disable splitting of sheet data range with cells and range with visible rows
    #[arg(long)]
    disable_sheetdata_splitting: bool,

    /// Enable quick-win preprocessing that collapses consecutive hidden empty rows.
    #[arg(long)]
    preprocess_hidden_rows: bool,

    /// Enable quick-win preprocessing that removes definitely safe empty self-closing cells.
    #[arg(long)]
    preprocess_empty_cells: bool,

    /// Number of XLSX assembler/writer threads.
    /// Defaults to the number of logical CPU cores.
    #[arg(long)]
    writer_workers: Option<usize>,

    /// Deflate compression level for cleaned XML entries.
    /// 1 is much faster than the ZIP default and usually still compresses XLSX XML very well.
    #[arg(long, default_value_t = 1)]
    compression_level: i32,

    /// Store cleaned XML entries without Deflate compression.
    /// Useful for RAM-disk throughput benchmarks and for showing pure cleaning/package speed.
    #[arg(long)]
    store_output: bool,

    /// Optional: Remove cells that only contain whitespace, IF they are completely unreferenced.
    #[arg(long)]
    remove_unreferenced_whitespace_cells: bool,

    /// Optional: Remove cells with content in hidden rows/cols, IF they are completely unreferenced.
    #[arg(long)]
    remove_unreferenced_hidden_content_cells: bool,
}

fn main() -> anyhow::Result<()> {
    let _guard = init_logging()?;

    let cli = Cli::parse();

    let start = Instant::now();
    info!("Starting excel-shrink pipeline...");

    let files = gather_files(&cli.path, cli.recursive)?;
    if files.is_empty() {
        info!("No XLSX files found.");
        return Ok(());
    }

    pipeline::process_files(files, &cli)?;

    info!("Script completed in {:.2?}", start.elapsed());
    Ok(())
}

fn init_logging() -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
    std::fs::create_dir_all("logs")?;

    let file_appender = rolling::never("logs", "excel-shrink-run.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let subscriber = fmt()
        .with_env_filter(env_filter)
        .with_writer(non_blocking)
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

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
