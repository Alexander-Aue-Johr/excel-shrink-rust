use super::*;
use std::collections::BTreeSet;

/// Validate a relocated package without opening Excel or modifying package files.
/// Hashes establish transport integrity, while recorded producer provenance stays separate.
pub(crate) fn verify(package: &Path) -> Result<Header> {
    let root = fs::canonicalize(package)?;
    ensure!(
        !root.join("incomplete.json").exists(),
        "Package is incomplete"
    );
    let header: Header =
        serde_json::from_reader(fs::File::open(confined_file(&root, "manifest.json")?)?)?;
    ensure!(
        header.version == 1 && header.policy == POLICY,
        "Unsupported preparation package"
    );
    ensure!(
        matches!(
            header.identity_scope.as_str(),
            "source-manifest" | "explicit-file-list" | "legacy-unassigned"
        ),
        "Unknown package identity scope"
    );
    let database = confined_file(&root, "metadata.sqlite")?;
    ensure!(
        artifact_cache::hash_file(&database)? == header.database_sha256,
        "Package database hash mismatch"
    );
    let connection =
        Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.pragma_update(None, "trusted_schema", false)?;
    let application: u32 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    ensure!(
        application == APPLICATION_ID && version == 1,
        "Unsupported package database schema"
    );
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(integrity == "ok", "Package database integrity check failed");
    let blobs = verify_blobs(&connection, &root, header.blobs)?;
    let provenance = verify_provenance(&connection)?;
    verify_entries(&connection, &header, &blobs, &provenance)?;
    Ok(header)
}

fn verify_blobs(
    connection: &Connection,
    root: &Path,
    expected: usize,
) -> Result<BTreeMap<String, u64>> {
    let directory = fs::canonicalize(root.join("blobs"))?;
    ensure!(
        directory.starts_with(root),
        "Blob directory escapes package"
    );
    let mut statement = connection.prepare("SELECT sha256, bytes FROM blobs")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut blobs = BTreeMap::new();
    for row in rows {
        let (hash, length) = row?;
        ensure!(
            super::super::normalized_sha(&hash)? == hash,
            "Noncanonical blob hash"
        );
        let length = u64::try_from(length).context("Negative blob length")?;
        let path = confined_file(&directory, &format!("{hash}.xlsx"))?;
        ensure!(fs::metadata(&path)?.len() == length, "Blob length mismatch");
        ensure!(
            artifact_cache::hash_file(&path)? == hash,
            "Blob hash mismatch"
        );
        ensure!(
            blobs.insert(hash, length).is_none(),
            "Duplicate blob identity"
        );
    }
    ensure!(blobs.len() == expected, "Package blob count mismatch");
    Ok(blobs)
}

fn verify_provenance(connection: &Connection) -> Result<BTreeSet<String>> {
    let mut statement = connection.prepare("SELECT id, payload FROM provenance")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut identities = BTreeSet::new();
    for row in rows {
        let (id, payload) = row?;
        let _: Value = serde_json::from_str(&payload)?;
        ensure!(
            artifact_cache::sha256(payload.as_bytes()) == id,
            "Provenance hash mismatch"
        );
        ensure!(identities.insert(id), "Duplicate provenance identity");
    }
    Ok(identities)
}

fn verify_entries(
    connection: &Connection,
    header: &Header,
    blobs: &BTreeMap<String, u64>,
    provenance: &BTreeSet<String>,
) -> Result<()> {
    let mut statement = connection
        .prepare("SELECT source_id, generation_id, status, provenance_id, payload FROM entries")?;
    let mut rows = statement.query([])?;
    let mut total = 0;
    let mut prepared = 0;
    let mut referenced = BTreeSet::new();
    while let Some(row) = rows.next()? {
        let source_id: String = row.get(0)?;
        let generation: String = row.get(1)?;
        let status: String = row.get(2)?;
        let producer: Option<String> = row.get(3)?;
        let payload: Value = serde_json::from_str(&row.get::<_, String>(4)?)?;
        ensure!(
            !source_id.is_empty()
                && payload["source_id"] == source_id
                && payload["status"] == status,
            "Entry identity or status mismatch"
        );
        ensure!(
            artifact_cache::sha256(&serde_json::to_vec(&(&payload, &producer))?) == generation,
            "Entry generation hash mismatch"
        );
        match status.as_str() {
            "prepared" => {
                ensure!(
                    producer.as_ref().is_some_and(|id| provenance.contains(id)),
                    "Entry provenance missing"
                );
                verify_states(&payload, blobs, &mut referenced)?;
                prepared += 1;
            }
            "failed" => ensure!(
                producer.is_none() && payload["error"].is_string(),
                "Invalid failed outcome"
            ),
            _ => anyhow::bail!("Unsupported entry outcome"),
        }
        total += 1;
    }
    ensure!(
        total == header.entries && prepared == header.prepared && total - prepared == header.failed,
        "Package outcome counts mismatch"
    );
    ensure!(
        referenced.len() == blobs.len(),
        "Package contains unreferenced blobs"
    );
    Ok(())
}

fn verify_states(
    payload: &Value,
    blobs: &BTreeMap<String, u64>,
    referenced: &mut BTreeSet<String>,
) -> Result<()> {
    ensure!(
        matches!(
            payload["native_result"]["status"].as_str(),
            Some("optimization_confirmed" | "no_optimization_needed")
        ),
        "Native completion evidence missing"
    );
    ensure!(
        payload["visual_validation"] == "pending",
        "Unsupported preparation validation state"
    );
    let states = payload["states"]
        .as_object()
        .context("Prepared states missing")?;
    ensure!(states.len() == 4, "Unsupported prepared state set");
    for role in [
        "baseline",
        "original-saved",
        "before-optimization",
        "after-optimization",
    ] {
        let blob: Blob =
            serde_json::from_value(states.get(role).context("Required state missing")?.clone())?;
        ensure!(
            blobs.get(&blob.sha256) == Some(&blob.bytes),
            "State blob reference mismatch"
        );
        if role == "baseline" {
            ensure!(
                payload["source_sha256"] == blob.sha256,
                "Baseline identity mismatch"
            );
        }
        referenced.insert(blob.sha256);
    }
    Ok(())
}
