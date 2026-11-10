//! Preserve complete workbook order without opening Excel or scanning worksheet cells.
use anyhow::Result;
use serde_json::{json, Value};
use std::{fs::File, io::Read, path::Path};
use zip::ZipArchive;

pub(crate) fn read(path: &Path) -> Result<Value> {
    let before = std::fs::metadata(path)?;
    let mut archive = ZipArchive::new(File::open(path)?)?;
    let mut bytes = Vec::new();
    archive
        .by_name("xl/workbook.xml")?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 8 * 1024 * 1024,
        "Workbook inventory exceeds 8 MiB"
    );
    let sheets = crate::regression::workbook_sheet_inventory(&bytes)?;
    let sha256 = deltasieve_engine::reuse::file_sha256(path)?;
    let after = std::fs::metadata(path)?;
    anyhow::ensure!(
        before.len() == after.len() && before.modified()? == after.modified()?,
        "Workbook changed during inventory"
    );
    Ok(json!({"complete":true,"original_sha256":sha256,
        "sheets":sheets}))
}

pub(super) fn recover(config: &Value, name: &str) -> Option<Value> {
    let source = Path::new(config["source_a"].as_str()?);
    if !source.is_dir() {
        return None;
    }
    let root = source.canonicalize().ok()?;
    let path = super::evidence::confined_file(&root, name).ok()?;
    read(&path).ok()
}
