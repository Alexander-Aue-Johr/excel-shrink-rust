//! Standalone conversion through owned, reusable native Excel sessions.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use clap::Parser;
    use corpus_runner::{
        memory::MemoryReserve,
        telemetry::{JsonLines, Operation},
    };
    use excel_automation::{
        context::RunContext,
        conversion::{self, Options},
    };
    use std::{path::PathBuf, sync::Arc};

    #[derive(Parser)]
    #[command(
        name = "xlsx-convert",
        about = "Save XLS and XLSX inputs as separate XLSX copies using Excel"
    )]
    struct Command {
        #[arg(long, num_args = 1..)]
        input: Vec<PathBuf>,
        /// New output directory retaining source copies, converted files and receipts.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Open a local configuration and progress web server.
        #[arg(long)]
        web: bool,
        /// Loopback port. Zero chooses an available port.
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long, default_value_t = 2)]
        workers: usize,
        #[arg(long)]
        recursive: bool,
        /// Optional ordered event log for dashboards and replay.
        #[arg(long)]
        events: Option<PathBuf>,
        /// Physical RAM and system-wide commit reserve. Omit for a machine-sized default.
        #[arg(long)]
        reserve_mib: Option<u64>,
    }

    let command = Command::parse();
    if command.web || (command.input.is_empty() && command.output.is_none()) {
        return preparation_web::serve(
            Operation::XlsxConvert,
            preparation_web::Defaults {
                inputs: command.input,
                output: command.output,
                workers: command.workers,
                recursive: command.recursive,
                reserve_mib: command.reserve_mib,
                source_manifest: None,
                reuse_from: None,
            },
            command.port,
        );
    }
    let mut context = match command.events {
        Some(path) => {
            RunContext::observed(Operation::XlsxConvert, Arc::new(JsonLines::create(&path)?))
        }
        None => RunContext::quiet(Operation::XlsxConvert),
    };
    context.memory_reserve = command
        .reserve_mib
        .map(MemoryReserve::from_mib)
        .transpose()?;
    conversion::run(
        Options {
            inputs: command.input,
            output: command
                .output
                .ok_or_else(|| anyhow::anyhow!("Choose --output or --web"))?,
            workers: command.workers,
            recursive: command.recursive,
        },
        context,
    )
}

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("Native XLSX conversion requires Windows and installed Excel")
}
