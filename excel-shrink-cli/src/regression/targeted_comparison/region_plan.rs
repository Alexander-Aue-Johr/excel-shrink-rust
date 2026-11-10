//! One atomic plan index, outside the frequently republished telemetry snapshot.
use super::Candidate;
use anyhow::Result;
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    io::{BufWriter, Write},
    path::Path,
};

pub(super) fn publish_inputs(
    directory: &Path,
    sheets: &[super::content_bounds::SelectionInput<'_>],
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(&json!({
        "version": 1,
        "stage": "before_grid_expansion",
        "sheets": sheets,
    }))?;
    crate::regression::artifact_cache::atomic_write(
        &directory.join("capture-selection-inputs.json"),
        &bytes,
    )
}

pub(super) fn publish(directory: &Path, regions: &BTreeSet<Candidate>, stage: &str) -> Result<()> {
    let pending = crate::regression::artifact_cache::unique_path(directory, "region-plan")?;
    let result = (|| -> Result<()> {
        let mut output = BufWriter::new(
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&pending)?,
        );
        serde_json::to_writer(
            &mut output,
            &json!({"version":1,"stage":stage,"total":regions.len()}),
        )?;
        output.write_all(b"\n")?;
        for region in regions {
            serde_json::to_writer(
                &mut output,
                &json!({"sheet":region.sheet,"range":region.cell}),
            )?;
            output.write_all(b"\n")?;
        }
        output.flush()?;
        output.get_ref().sync_all()?;
        drop(output);
        crate::regression::artifact_cache::atomic_replace(
            &pending,
            &directory.join("capture-regions.jsonl"),
        )
    })();
    if result.is_err() {
        let _ = fs::remove_file(&pending);
    }
    result
}
