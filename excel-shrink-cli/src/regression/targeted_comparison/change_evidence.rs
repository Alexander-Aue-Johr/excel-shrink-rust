//! Complete XML-change audit outside compact progress and comparison reports.
use super::xml_changes::XmlChange;
use anyhow::Result;
use flate2::{write::GzEncoder, Compression};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

/// Write every change once, with shared contexts and compact named columns.
/// A completed gzip is published before the report can reference it.
pub(super) fn write(directory: &Path, index: usize, changes: &[XmlChange]) -> Result<PathBuf> {
    let directory = directory.join("xml-changes");
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{index:02}.jsonl.gz"));
    let pending = path.with_extension("gz.pending");
    let mut contexts = BTreeMap::new();
    for change in changes {
        for context in [&change.context_before, &change.context_after] {
            let next = contexts.len();
            contexts.entry(context.as_str()).or_insert(next);
        }
    }
    let mut ordered = vec![""; contexts.len()];
    for (&context, &index) in &contexts {
        ordered[index] = context;
    }
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    let mut output = GzEncoder::new(BufWriter::new(file), Compression::fast());
    serde_json::to_writer(
        &mut output,
        &serde_json::json!({"schema":1,"contexts":ordered,
        "columns":["sheet","location","before","after","context_before","context_after","also_in_original_open_save"]}),
    )?;
    output.write_all(b"\n")?;
    for change in changes {
        serde_json::to_writer(
            &mut output,
            &(
                &change.sheet,
                &change.location,
                &change.before,
                &change.after,
                contexts[change.context_before.as_str()],
                contexts[change.context_after.as_str()],
                change.also_in_original_open_save,
            ),
        )?;
        output.write_all(b"\n")?;
    }
    let mut writer = output.finish()?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    drop(writer);
    fs::rename(&pending, &path)?;
    Ok(path)
}
