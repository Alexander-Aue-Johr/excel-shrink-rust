use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    Processed,
    Preserved,
    Skipped,
    Failed,
}

#[derive(Serialize)]
pub(crate) struct FileOutcome {
    pub input: PathBuf,
    pub output: PathBuf,
    pub status: Status,
    pub elapsed_ms: u128,
    pub input_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Default, Serialize)]
struct Summary {
    total: usize,
    processed: usize,
    preserved: usize,
    skipped: usize,
    failed: usize,
}

#[derive(Serialize)]
struct BatchReport<'a> {
    schema_version: u32,
    elapsed_ms: u128,
    summary: Summary,
    files: &'a [FileOutcome],
}

pub(crate) fn write(
    path: &Path,
    elapsed_ms: u128,
    files: &mut [FileOutcome],
) -> anyhow::Result<()> {
    files.sort_by(|a, b| a.input.cmp(&b.input));
    let mut summary = Summary {
        total: files.len(),
        ..Summary::default()
    };
    for file in files.iter() {
        match file.status {
            Status::Processed => summary.processed += 1,
            Status::Preserved => summary.preserved += 1,
            Status::Skipped => summary.skipped += 1,
            Status::Failed => summary.failed += 1,
        }
    }
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let report = BatchReport {
        schema_version: 1,
        elapsed_ms,
        summary,
        files,
    };
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}
