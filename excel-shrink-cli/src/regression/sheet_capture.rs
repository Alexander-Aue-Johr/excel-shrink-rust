//! Explicit inspection captures are separate from corpus validation results.
use super::*;
use anyhow::{ensure, Context, Result};

#[derive(Parser, Debug)]
pub(super) struct Args {
    #[arg(long)]
    original: PathBuf,
    #[arg(long)]
    variant: PathBuf,
    /// One-based workbook sheet index.
    #[arg(long)]
    sheet: usize,
    #[arg(long)]
    output: PathBuf,
}

/// Capture one complete sheet on demand without changing the corpus verdict.
pub(super) fn run(args: Args) -> Result<()> {
    ensure!(args.sheet > 0, "Sheet indexes start at one");
    ensure!(!args.output.exists(), "Inspection output already exists");
    fs::create_dir_all(&args.output)?;
    let outcome = capture(&args);
    let result = match &outcome {
        Ok(()) => serde_json::json!({"status":"complete","sheet":args.sheet}),
        Err(error) => {
            serde_json::json!({"status":"failed","sheet":args.sheet,"error":format!("{error:#}")})
        }
    };
    artifact_cache::atomic_write(
        &args.output.join("inspection-result.json"),
        &serde_json::to_vec(&result)?,
    )?;
    outcome
}

/// Always finish the owned Excel session, including when capture fails.
fn capture(args: &Args) -> Result<()> {
    let mut session = office_capture::ResidentCapture::start_sheet_inspection(
        &args.original,
        &args.variant,
        args.sheet,
        &args.output,
    )
    .context("Open manual sheet inspection")?;
    let captured = session.capture_inspection_images();
    let finished = session.finish();
    captured?;
    finished?;
    Ok(())
}
