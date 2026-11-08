//! SheetShrink owns its application entry point and shares the existing execution engine.
fn main() -> anyhow::Result<()> {
    excel_shrink_cli::run_sheetshrink()
}
