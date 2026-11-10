//! Standalone Check Performance preparation using the same backend as DeltaSieve.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use clap::Parser;
    use corpus_runner::{
        memory::MemoryReserve,
        telemetry::{JsonLines, Operation},
    };
    use excel_automation::{context::RunContext, pool::performance};
    use std::{path::PathBuf, sync::Arc};

    #[derive(Parser)]
    #[command(
        name = "excel-check-performance",
        about = "Prepare confirmed Excel Check Performance copies"
    )]
    struct Command {
        #[arg(long, num_args = 1.., conflicts_with = "source_manifest")]
        input: Vec<PathBuf>,
        #[arg(long)]
        source_manifest: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 2)]
        workers: usize,
        #[arg(long)]
        reuse_from: Option<PathBuf>,
        /// Open a local configuration and progress web server.
        #[arg(long)]
        web: bool,
        /// Start the web interface and open it in the default browser.
        #[arg(long)]
        open_browser: bool,
        /// Loopback port. Zero chooses an available port.
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Recursively discover XLSX files when an input directory is selected.
        #[arg(long)]
        recursive: bool,
        /// Optional ordered event log for dashboards and replay.
        #[arg(long)]
        events: Option<PathBuf>,
        /// Physical RAM and system-wide commit reserve. Omit for a machine-sized default.
        #[arg(long)]
        reserve_mib: Option<u64>,
    }

    let mut command = Command::parse();
    if command.web
        || command.open_browser
        || (command.input.is_empty()
            && command.source_manifest.is_none()
            && command.output.is_none())
    {
        return preparation_web::serve_with_browser(
            Operation::CheckPerformance,
            preparation_web::Defaults {
                inputs: command.input,
                output: command.output,
                workers: command.workers,
                recursive: command.recursive,
                reserve_mib: command.reserve_mib,
                source_manifest: command.source_manifest,
                reuse_from: command.reuse_from,
                max_active_workbooks: None,
                parallel_range_chunk_size: None,
            },
            command.port,
            command.open_browser,
        );
    }
    if !command.input.is_empty() {
        command.input =
            corpus_runner::sources::expand_paths(&command.input, &["xlsx"], command.recursive)?;
    }
    let mut context = match command.events {
        Some(path) => RunContext::observed(
            Operation::CheckPerformance,
            Arc::new(JsonLines::create(&path)?),
        ),
        None => RunContext::quiet(Operation::CheckPerformance),
    };
    context.memory_reserve = command
        .reserve_mib
        .map(MemoryReserve::from_mib)
        .transpose()?;
    performance::run_with_context(
        performance::Args {
            input: command.input,
            source_manifest: command.source_manifest,
            output: command
                .output
                .ok_or_else(|| anyhow::anyhow!("Choose --output or --web"))?,
            workers: command.workers,
            reuse_from: command.reuse_from,
        },
        context,
    )
}

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("Check Performance requires Windows and installed Excel")
}
