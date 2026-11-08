//! Immutable transfer packages with indexed metadata and byte-deduplicated workbooks.
use crate::regression::artifact_cache;
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const POLICY: &str = "deltasieve-preparation-package-v1";
const APPLICATION_ID: u32 = 0x44535050;
const CONTROLS: [&str; 3] = [
    "original-saved.xlsx",
    "before-optimization.xlsx",
    "after-optimization.xlsx",
];

#[cfg(test)]
mod tests;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Header {
    version: u32,
    policy: String,
    database_sha256: String,
    pub entries: usize,
    prepared: usize,
    failed: usize,
    blobs: usize,
    identity_scope: String,
    exported_at: String,
    producer_error: Value,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Blob {
    sha256: String,
    bytes: u64,
}

/// Reserve a new destination and publish its manifest only after all data is durable.
/// An interrupted destination remains identifiable and cannot replace an existing package.
pub(crate) fn export(preparation: &Path, destination: &Path) -> Result<Header> {
    let source = fs::canonicalize(preparation)?;
    let batch: Value =
        serde_json::from_reader(fs::File::open(confined_file(&source, "preparation.json")?)?)?;
    ensure!(
        batch["version"] == 1 && batch["operation"] == "check-performance",
        "Unsupported preparation receipt"
    );
    let rows = batch["inputs"]
        .as_array()
        .context("Preparation has no input outcomes")?;
    ensure!(
        batch["total"].as_u64() == Some(rows.len() as u64),
        "Preparation outcome enumeration is incomplete"
    );
    let destination = std::path::absolute(destination)?;
    let parent = fs::canonicalize(destination.parent().context("Package has no parent")?)?;
    let target = parent.join(
        destination
            .file_name()
            .context("Choose a package directory")?,
    );
    ensure!(
        !target.starts_with(&source),
        "Package destination must be outside the preparation source"
    );
    fs::create_dir(&target).context("Choose a new package destination")?;
    let pending = target.join("incomplete.json");
    artifact_cache::atomic_write(&pending, br#"{"state":"building"}"#)?;
    let result = (|| -> Result<Header> {
        let header = build(&source, &target, &batch, rows)?;
        fs::remove_file(&pending)?;
        artifact_cache::atomic_write(
            &target.join("manifest.json"),
            &serde_json::to_vec_pretty(&header)?,
        )?;
        Ok(header)
    })();
    if let Err(error) = &result {
        if let Err(publication_error) = artifact_cache::atomic_write(
            &pending,
            &serde_json::to_vec_pretty(
                &json!({"state":"incomplete", "error":format!("{error:#}")}),
            )?,
        ) {
            eprintln!("Cannot retain package failure receipt: {publication_error:#}");
        }
    }
    result.with_context(|| format!("Export preparation package {}", target.display()))
}

fn build(source: &Path, target: &Path, batch: &Value, rows: &[Value]) -> Result<Header> {
    fs::create_dir(target.join("blobs"))?;
    let database = target.join("metadata.sqlite");
    let mut connection = Connection::open(&database)?;
    connection.pragma_update(None, "application_id", APPLICATION_ID)?;
    connection.pragma_update(None, "user_version", 1)?;
    connection.execute_batch(
        "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
        CREATE TABLE provenance (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
        CREATE TABLE entries (source_id TEXT NOT NULL, generation_id TEXT NOT NULL,
            status TEXT NOT NULL, provenance_id TEXT, payload TEXT NOT NULL,
            PRIMARY KEY(source_id,generation_id));
        CREATE TABLE blobs (sha256 TEXT PRIMARY KEY, bytes INTEGER NOT NULL);",
    )?;
    let transaction = connection.transaction()?;
    let mut blobs = BTreeMap::new();
    let mut prepared = 0;
    let scope = batch["identity_scope"]
        .as_str()
        .unwrap_or("legacy-unassigned");
    ensure!(
        matches!(
            scope,
            "source-manifest" | "explicit-file-list" | "legacy-unassigned"
        ),
        "Unknown preparation identity scope"
    );
    for (index, row) in rows.iter().enumerate() {
        ensure!(
            row["index"].as_u64() == Some(index as u64),
            "Preparation input indices are inconsistent"
        );
        let source_id = match row["source_id"].as_str() {
            Some(id) if !id.is_empty() => id.to_owned(),
            _ if scope == "legacy-unassigned" => format!("legacy-unassigned:{index}"),
            _ => anyhow::bail!("Preparation outcome lacks a source ID"),
        };
        let (payload, provenance) = match row["status"].as_str() {
            Some("prepared") => {
                let directory = fs::canonicalize(source.join(format!("input-{index:06}")))?;
                ensure!(
                    directory.starts_with(source),
                    "Prepared input directory escapes its source"
                );
                let (entry, provenance) =
                    prepared_entry(&directory, row, &source_id, target, &mut blobs, batch)?;
                prepared += 1;
                (entry, Some(provenance))
            }
            Some("failed") => (
                json!({"source_id":source_id,"name":row["name"],"status":"failed","error":row["error"]}),
                None,
            ),
            _ => anyhow::bail!("Unsupported preparation input outcome"),
        };
        let provenance_id = if let Some(provenance) = provenance {
            let bytes = serde_json::to_vec(&provenance)?;
            let id = artifact_cache::sha256(&bytes);
            transaction.execute(
                "INSERT OR IGNORE INTO provenance VALUES (?1,?2)",
                params![id, String::from_utf8(bytes)?],
            )?;
            Some(id)
        } else {
            None
        };
        let generation = artifact_cache::sha256(&serde_json::to_vec(&(&payload, &provenance_id))?);
        let payload = serde_json::to_string(&payload)?;
        transaction.execute(
            "INSERT INTO entries VALUES (?1,?2,?3,?4,?5)",
            params![
                source_id,
                generation,
                row["status"].as_str(),
                provenance_id,
                payload
            ],
        )?;
    }
    ensure!(
        batch["failed"].as_u64() == Some((rows.len() - prepared) as u64),
        "Preparation failure counts are inconsistent"
    );
    for (hash, bytes) in &blobs {
        transaction.execute(
            "INSERT INTO blobs VALUES (?1,?2)",
            params![hash, i64::try_from(*bytes)?],
        )?;
    }
    transaction.commit()?;
    connection.close().map_err(|(_, error)| error)?;
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&database)?
        .sync_all()?;
    Ok(Header {
        version: 1,
        policy: POLICY.into(),
        database_sha256: artifact_cache::hash_file(&database)?,
        entries: rows.len(),
        prepared,
        failed: rows.len() - prepared,
        blobs: blobs.len(),
        identity_scope: scope.into(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        producer_error: batch["manager_error"].clone(),
    })
}

fn prepared_entry(
    directory: &Path,
    row: &Value,
    source_id: &str,
    target: &Path,
    blobs: &mut BTreeMap<String, u64>,
    batch: &Value,
) -> Result<(Value, Value)> {
    let receipt: Value =
        serde_json::from_reader(fs::File::open(confined_file(directory, "prepared.json")?)?)?;
    ensure!(
        receipt["version"] == 1 && receipt["operation"] == "check-performance",
        "Unsupported prepared input receipt"
    );
    if row["source_id"].is_string() {
        ensure!(
            receipt["source_id"] == source_id,
            "Prepared source identity disagrees with its outcome"
        );
    }
    ensure!(
        matches!(
            receipt["native_result"]["status"].as_str(),
            Some("optimization_confirmed" | "no_optimization_needed")
        ),
        "Prepared input lacks native completion evidence"
    );
    let source_hash = super::normalized_sha(
        receipt["source_sha256"]
            .as_str()
            .context("Baseline hash missing")?,
    )?;
    let (baseline, baseline_bytes) = if receipt["baseline"].is_object() {
        ensure!(
            receipt["baseline"]["file"] == "baseline.xlsx"
                && receipt["baseline"]["sha256"] == source_hash,
            "Invalid retained baseline receipt"
        );
        (
            confined_file(directory, "baseline.xlsx")?,
            Some(
                receipt["baseline"]["bytes"]
                    .as_u64()
                    .context("Baseline length missing")?,
            ),
        )
    } else {
        ensure!(
            receipt["baseline"].is_null(),
            "Unsupported baseline receipt"
        );
        (
            fs::canonicalize(
                receipt["source"]
                    .as_str()
                    .context("Legacy baseline source missing")?,
            )?,
            None,
        )
    };
    let baseline = copy_blob(&baseline, &source_hash, baseline_bytes, target, blobs)?;
    let controls = receipt["artifacts"]
        .as_object()
        .context("Prepared control receipts missing")?;
    ensure!(
        controls.len() == CONTROLS.len(),
        "Unsupported prepared state set"
    );
    let mut states = BTreeMap::from([("baseline".to_owned(), baseline)]);
    for name in CONTROLS {
        let control = controls
            .get(name)
            .context("Required prepared control is missing")?;
        let hash =
            super::normalized_sha(control["sha256"].as_str().context("Control hash missing")?)?;
        let bytes = control["bytes"]
            .as_u64()
            .context("Control length missing")?;
        let blob = copy_blob(
            &confined_file(directory, name)?,
            &hash,
            Some(bytes),
            target,
            blobs,
        )?;
        states.insert(name.trim_end_matches(".xlsx").to_owned(), blob);
    }
    let entry = json!({
        "source_id":source_id, "name":receipt.get("name").unwrap_or(&row["name"]),
        "status":"prepared", "source_sha256":source_hash, "raw_source":receipt["raw_source"],
        "states":states, "native_result":receipt["native_result"], "measurements":receipt["measurements"],
        "visual_validation":"pending"
    });
    let provenance = json!({"environment":receipt["environment"],"generation_recipe":receipt["generation_recipe"],
        "producer_sha256":batch["producer_sha256"]});
    Ok((entry, provenance))
}

fn copy_blob(
    source: &Path,
    hash: &str,
    expected_bytes: Option<u64>,
    target: &Path,
    blobs: &mut BTreeMap<String, u64>,
) -> Result<Blob> {
    let metadata = fs::metadata(source)?;
    ensure!(
        metadata.is_file(),
        "Workbook artifact is not a regular file"
    );
    let bytes = metadata.len();
    ensure!(
        expected_bytes.is_none_or(|expected| expected == bytes),
        "Workbook artifact length differs from its receipt"
    );
    ensure!(
        artifact_cache::hash_file(source)? == hash,
        "Workbook artifact hash differs from its receipt"
    );
    if let Some(existing) = blobs.get(hash) {
        ensure!(*existing == bytes, "Blob lengths disagree");
    } else {
        let path = target.join("blobs").join(format!("{hash}.xlsx"));
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        std::io::copy(&mut fs::File::open(source)?, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        drop(output);
        ensure!(
            artifact_cache::hash_file(&path)? == hash,
            "Workbook changed during package copy"
        );
        blobs.insert(hash.to_owned(), bytes);
    }
    Ok(Blob {
        sha256: hash.to_owned(),
        bytes,
    })
}

fn confined_file(root: &Path, name: &str) -> Result<PathBuf> {
    let root = fs::canonicalize(root)?;
    let path = root.join(name);
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file() && !metadata.is_symlink(),
        "Artifact is not a regular file"
    );
    let resolved = fs::canonicalize(path)?;
    ensure!(resolved.starts_with(root), "Artifact escapes its directory");
    Ok(resolved)
}
