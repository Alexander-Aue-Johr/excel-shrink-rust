use clap::Parser;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;
use std::path::PathBuf;
use std::time::Instant;

mod pipeline;

#[derive(Parser, Debug)]
#[command(name = "excel-shrink", about = "Clean and shrink XLSX files.")]
struct Cli {
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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let start = Instant::now();
    info!("Starting excel-shrink pipeline...");

    let files = gather_files(&cli.path, cli.recursive)?;
    if files.is_empty() {
        info!("No XLSX files found.");
        return Ok(());
    }

    pipeline::process_files(
        files,
        cli.output,
        cli.chunk_size,
        cli.disable_multiprocessing,
        cli.force_overwrite,
    )?;

    info!("Script completed in {:.2?}", start.elapsed());
    Ok(())
}

fn gather_files(path: &PathBuf, recursive: bool) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if path.is_file() && path.extension().map_or(false, |ext| ext == "xlsx") {
        files.push(path.clone());
    } else if path.is_dir() {
        if recursive {
            for entry in walkdir::WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                if entry.path().extension().map_or(false, |e| e == "xlsx") {
                    files.push(entry.path().to_path_buf());
                }
            }
        } else {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                if entry.path().extension().map_or(false, |e| e == "xlsx") {
                    files.push(entry.path());
                }
            }
        }
    }
    Ok(files)
}