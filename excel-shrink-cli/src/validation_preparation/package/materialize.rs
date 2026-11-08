use super::*;
use crate::validation_preparation;

/// Publish exact paired corpus copies while keeping producer measurements in an identity index.
/// Every selected occurrence must be compatible before reserving the new destination.
pub(crate) fn materialize(package: &Path, manifest: &Path, destination: &Path) -> Result<usize> {
    let header = verify(package)?;
    ensure!(
        header.identity_scope == "source-manifest",
        "Corpus reuse requires a source-manifest package"
    );
    let inputs = validation_preparation::read(manifest)?;
    let root = fs::canonicalize(package)?;
    let connection = Connection::open_with_flags(
        confined_file(&root, "metadata.sqlite")?,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    connection.pragma_update(None, "trusted_schema", false)?;
    let mut statement = connection.prepare("SELECT generation_id, status, provenance_id, payload FROM entries WHERE source_id=?1 ORDER BY generation_id")?;
    let mut selections = Vec::new();
    for input in &inputs {
        let item = preview::inspect(&mut statement, input)?;
        ensure!(
            item.status == preview::Status::Complete,
            "Source {} is not ready for import: {:?}",
            input.source_id,
            item.status
        );
        selections.push((
            input,
            item.selected_generation
                .context("Compatible generation missing")?,
        ));
    }
    let destination = std::path::absolute(destination)?;
    let parent = fs::canonicalize(
        destination
            .parent()
            .context("Import destination has no parent")?,
    )?;
    let target = parent.join(
        destination
            .file_name()
            .context("Choose an import directory")?,
    );
    ensure!(
        !target.starts_with(&root),
        "Import destination must be outside the package"
    );
    fs::create_dir(&target).context("Choose a new import destination")?;
    let pending = target.join("incomplete.json");
    artifact_cache::atomic_write(&pending, br#"{"state":"building"}"#)?;
    let result = write_pairs(&root, &target, &connection, &selections, &inputs, &header);
    match result {
        Ok(index) => {
            fs::remove_file(&pending)?;
            artifact_cache::atomic_write(
                &target.join("import.json"),
                &serde_json::to_vec_pretty(&index)?,
            )?;
            Ok(selections.len())
        }
        Err(error) => {
            artifact_cache::atomic_write(
                &pending,
                &serde_json::to_vec(&json!({"state":"incomplete","error":format!("{error:#}")}))?,
            )?;
            Err(error)
        }
    }
}

fn write_pairs(
    root: &Path,
    target: &Path,
    connection: &Connection,
    selections: &[(&validation_preparation::Input, String)],
    inputs: &[validation_preparation::Input],
    header: &Header,
) -> Result<Value> {
    fs::create_dir(target.join("baseline"))?;
    fs::create_dir(target.join("check-performance"))?;
    let mut entries = Vec::new();
    let mut provenance = BTreeMap::<String, Value>::new();
    for (input, generation) in selections {
        let (producer, payload): (String, String) = connection.query_row(
            "SELECT provenance_id,payload FROM entries WHERE source_id=?1 AND generation_id=?2",
            params![input.source_id, generation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let payload: Value = serde_json::from_str(&payload)?;
        let name = format!(
            "{}.xlsx",
            artifact_cache::sha256(input.source_id.as_bytes())
        );
        for (folder, role) in [
            ("baseline", "baseline"),
            ("check-performance", "after-optimization"),
        ] {
            let blob: Blob = serde_json::from_value(payload["states"][role].clone())?;
            let path = confined_file(&root.join("blobs"), &format!("{}.xlsx", blob.sha256))?;
            copy_state(&path, &target.join(folder).join(&name), &blob)?;
        }
        if !provenance.contains_key(&producer) {
            let value: String = connection.query_row(
                "SELECT payload FROM provenance WHERE id=?1",
                [&producer],
                |row| row.get(0),
            )?;
            provenance.insert(producer.clone(), serde_json::from_str(&value)?);
        }
        entries.push(
            json!({"source_id":input.source_id,"name":input.name,"pair_file":name,
            "generation_id":generation,"provenance_id":producer,"prepared_evidence":payload}),
        );
    }
    // The standard comparison entry point loads these relative folders without CP generation.
    let config = json!({"source_a":"baseline","source_b":"check-performance","report_dir":"report","steps":[]});
    artifact_cache::atomic_write(
        &target.join("compare.json"),
        &serde_json::to_vec_pretty(&config)?,
    )?;
    Ok(
        json!({"version":1,"operation":"import-check-performance","package_database_sha256":header.database_sha256,
        "selection_sha256":validation_preparation::selection_hash(inputs)?,"entries":entries,"provenance":provenance,
        "visual_validation":"pending","imported_at":chrono::Utc::now().to_rfc3339()}),
    )
}

fn copy_state(source: &Path, destination: &Path, blob: &Blob) -> Result<()> {
    ensure!(
        fs::metadata(source)?.len() == blob.bytes
            && artifact_cache::hash_file(source)? == blob.sha256,
        "Package state changed before import"
    );
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    std::io::copy(&mut fs::File::open(source)?, &mut output)?;
    output.flush()?;
    output.sync_all()?;
    drop(output);
    ensure!(
        fs::metadata(destination)?.len() == blob.bytes
            && artifact_cache::hash_file(destination)? == blob.sha256,
        "Imported state failed verification"
    );
    Ok(())
}
