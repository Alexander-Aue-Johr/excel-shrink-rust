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
        #[command(flatten)]
        job: performance::Args,
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
    if !command.job.input.is_empty() {
        command.job.input =
            corpus_runner::sources::expand_paths(&command.job.input, &["xlsx"], command.recursive)?;
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
    performance::run_with_context(command.job, context)
}

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("Check Performance requires Windows and installed Excel")
}
