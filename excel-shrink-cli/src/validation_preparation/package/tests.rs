use super::*;

/// Mock native receipts exercise transfer semantics without making rendering claims.
fn fixture() -> Result<(PathBuf, PathBuf)> {
    let root = std::env::temp_dir().join(format!("preparation-package-{}", uuid::Uuid::new_v4()));
    let source = root.join("source");
    fs::create_dir_all(&source)?;
    let mut rows = Vec::new();
    for index in 0..2 {
        let directory = source.join(format!("input-{index:06}"));
        fs::create_dir(&directory)?;
        fs::write(directory.join("baseline.xlsx"), b"baseline bytes")?;
        let baseline_hash = artifact_cache::hash_file(&directory.join("baseline.xlsx"))?;
        let mut artifacts = BTreeMap::new();
        for name in CONTROLS {
            let data = if name == "after-optimization.xlsx" {
                b"optimized".as_slice()
            } else {
                b"saved".as_slice()
            };
            fs::write(directory.join(name), data)?;
            artifacts.insert(name,json!({"sha256":artifact_cache::hash_file(&directory.join(name))?,"bytes":data.len()}));
        }
        let id = format!("source:'{index}");
        artifact_cache::atomic_write(
            &directory.join("prepared.json"),
            &serde_json::to_vec(&json!({
                "version":1,"operation":"check-performance","source_id":id,"name":"../../display-only.xlsx",
                "source":"external-source-was-moved.xlsx","source_sha256":baseline_hash,
                "baseline":{"file":"baseline.xlsx","sha256":baseline_hash,"bytes":b"baseline bytes".len()},
                "artifacts":artifacts,"native_result":{"status":"optimization_confirmed","method":"unit-fixture"},
                "measurements":{"worker_jobs_completed":index+1},"environment":{"machine":"producer"},
                "generation_recipe":{"fixture":"v1"}
            }))?,
        )?;
        rows.push(json!({"index":index,"source_id":id,"status":"prepared","name":"display"}));
    }
    rows.push(json!({"index":2,"source_id":"failed-source","status":"failed","name":"failed","error":"fixture error"}));
    artifact_cache::atomic_write(
        &source.join("preparation.json"),
        &serde_json::to_vec(&json!({
            "version":1,"operation":"check-performance","total":3,"failed":1,"identity_scope":"source-manifest",
            "producer_sha256":"a".repeat(64),"inputs":rows
        }))?,
    )?;
    Ok((root, source))
}

fn open(root: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        root.join("metadata.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}

#[test]
fn export_deduplicates_bytes_and_preserves_occurrences_failures_and_provenance() -> Result<()> {
    let (root, source) = fixture()?;
    let target = root.join("package");
    let header = export(&source, &target)?;
    assert_eq!(
        (header.entries, header.prepared, header.failed, header.blobs),
        (3, 2, 1, 3)
    );
    assert_eq!(
        header.database_sha256,
        artifact_cache::hash_file(&target.join("metadata.sqlite"))?
    );
    assert!(!target.join("incomplete.json").exists());
    let connection = open(&target)?;
    assert_eq!(
        connection.query_row("SELECT count(*) FROM provenance", [], |row| row
            .get::<_, i64>(0))?,
        1
    );
    assert_eq!(
        connection.query_row("SELECT count(*) FROM entries", [], |row| row
            .get::<_, i64>(0))?,
        3
    );
    let payload: String = connection.query_row(
        "SELECT payload FROM entries WHERE source_id=?1",
        ["source:'0"],
        |row| row.get(0),
    )?;
    let entry: Value = serde_json::from_str(&payload)?;
    assert_eq!(entry["visual_validation"], "pending");
    assert_eq!(entry["name"], "../../display-only.xlsx");
    assert_eq!(entry["states"].as_object().unwrap().len(), 4);
    for file in fs::read_dir(target.join("blobs"))? {
        let file = file?;
        let name = file
            .path()
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(name.len(), 64);
        assert!(name.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    Ok(())
}

#[test]
fn corruption_leaves_an_explicit_incomplete_export() -> Result<()> {
    let (root, source) = fixture()?;
    fs::write(
        source.join("input-000000/after-optimization.xlsx"),
        b"changed",
    )?;
    let target = root.join("package");
    assert!(export(&source, &target).is_err());
    assert!(!target.join("manifest.json").exists());
    let failure: Value = serde_json::from_slice(&fs::read(target.join("incomplete.json"))?)?;
    assert_eq!(failure["state"], "incomplete");
    Ok(())
}

#[test]
fn an_existing_package_is_preserved_and_nested_destinations_are_rejected() -> Result<()> {
    let (root, source) = fixture()?;
    let target = root.join("package");
    export(&source, &target)?;
    let before = fs::read(target.join("manifest.json"))?;
    assert!(export(&source, &target).is_err());
    assert_eq!(before, fs::read(target.join("manifest.json"))?);
    assert!(export(&source, &source.join("nested")).is_err());
    assert!(!source.join("nested").exists());
    Ok(())
}

#[test]
fn legacy_receipts_use_a_verified_available_source_baseline() -> Result<()> {
    let (root, source) = fixture()?;
    let legacy = root.join("legacy.xlsx");
    fs::write(&legacy, b"baseline bytes")?;
    let path = source.join("input-000000/prepared.json");
    let mut receipt: Value = serde_json::from_slice(&fs::read(&path)?)?;
    receipt.as_object_mut().unwrap().remove("baseline");
    receipt["source"] = json!(legacy);
    artifact_cache::atomic_write(&path, &serde_json::to_vec(&receipt)?)?;
    fs::remove_file(source.join("input-000000/baseline.xlsx"))?;
    assert_eq!(export(&source, &root.join("package"))?.prepared, 2);
    Ok(())
}
