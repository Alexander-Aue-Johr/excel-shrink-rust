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

#[test]
fn verification_survives_relocation_without_external_sources() -> Result<()> {
    let (root, source) = fixture()?;
    let target = root.join("package");
    export(&source, &target)?;
    let relocated = root.join("relocated");
    fs::rename(&target, &relocated)?;
    fs::rename(&source, root.join("unavailable-source"))?;
    let header = verify(&relocated)?;
    assert_eq!((header.entries, header.prepared, header.failed), (3, 2, 1));
    assert!(!relocated.join("metadata.sqlite-wal").exists());
    Ok(())
}

#[test]
fn verification_rejects_corrupt_bytes_incomplete_publication_and_counts() -> Result<()> {
    let (root, source) = fixture()?;
    let target = root.join("package");
    export(&source, &target)?;
    fs::write(target.join("incomplete.json"), b"{}")?;
    assert!(verify(&target).is_err());
    fs::remove_file(target.join("incomplete.json"))?;
    let path = target.join("manifest.json");
    let mut header: Header = serde_json::from_slice(&fs::read(&path)?)?;
    header.entries += 1;
    fs::write(&path, serde_json::to_vec(&header)?)?;
    assert!(verify(&target).is_err());
    header.entries -= 1;
    fs::write(&path, serde_json::to_vec(&header)?)?;
    let blob = fs::read_dir(target.join("blobs"))?.next().unwrap()?.path();
    let mut bytes = fs::read(&blob)?;
    bytes[0] ^= 1;
    fs::write(blob, bytes)?;
    assert!(verify(&target).is_err());
    Ok(())
}

#[test]
fn verification_checks_database_hash_and_internal_generation_bindings() -> Result<()> {
    let (root, source) = fixture()?;
    let target = root.join("package");
    export(&source, &target)?;
    let database = target.join("metadata.sqlite");
    let connection = Connection::open(&database)?;
    connection.execute(
        "UPDATE entries SET generation_id='incorrect' WHERE source_id=?1",
        ["source:'0"],
    )?;
    connection.close().map_err(|(_, error)| error)?;
    assert!(verify(&target).is_err());
    let path = target.join("manifest.json");
    let mut header: Header = serde_json::from_slice(&fs::read(&path)?)?;
    header.database_sha256 = artifact_cache::hash_file(&database)?;
    fs::write(path, serde_json::to_vec(&header)?)?;
    assert!(format!("{:#}", verify(&target).unwrap_err()).contains("generation hash"));
    Ok(())
}

fn selection(root: &Path, ids: &[&str]) -> Result<PathBuf> {
    let baseline = root.join("local.xlsx");
    fs::write(&baseline, b"baseline bytes")?;
    let hash = artifact_cache::hash_file(&baseline)?;
    let inputs: Vec<_> = ids
        .iter()
        .map(|id| {
            json!({"source_id":id,"name":id,
        "baseline":"local.xlsx","baseline_sha256":hash})
        })
        .collect();
    let path = root.join("selection.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!({"version":1,"inputs":inputs}))?,
    )?;
    Ok(path)
}

#[test]
fn coverage_preview_counts_all_occurrences_and_pages_details() -> Result<()> {
    let (root, source) = fixture()?;
    let target = root.join("package");
    export(&source, &target)?;
    let manifest = selection(
        &root,
        &["source:'0", "source:'1", "failed-source", "absent"],
    )?;
    let result = serde_json::to_value(preview(&target, &manifest, 1, 1)?)?;
    assert_eq!(result["selected"], 4);
    assert_eq!(
        result["counts"],
        json!({"complete":2,"unsuccessful":1,"missing":1})
    );
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["source_id"], "source:'1");
    assert!(preview(&target, &manifest, 0, 0).is_err());
    assert!(preview(&target, &manifest, 0, 1001).is_err());
    Ok(())
}

#[test]
fn coverage_preview_distinguishes_source_and_local_baseline_mismatches() -> Result<()> {
    let (root, source) = fixture()?;
    let target = root.join("package");
    export(&source, &target)?;
    let manifest = selection(&root, &["source:'0"])?;
    let original = fs::read(&manifest)?;
    let mut input: Value = serde_json::from_slice(&original)?;
    input["inputs"][0]["baseline_sha256"] = json!("f".repeat(64));
    fs::write(&manifest, serde_json::to_vec(&input)?)?;
    assert_eq!(
        serde_json::to_value(preview(&target, &manifest, 0, 1)?)?["counts"],
        json!({"baseline_mismatch":1})
    );
    input = serde_json::from_slice(&original)?;
    input["inputs"][0]["raw_source"] = json!({"sha256":"e".repeat(64),"format":"xls"});
    fs::write(&manifest, serde_json::to_vec(&input)?)?;
    assert_eq!(
        serde_json::to_value(preview(&target, &manifest, 0, 1)?)?["counts"],
        json!({"source_mismatch":1})
    );
    fs::write(&manifest, original)?;
    fs::write(root.join("local.xlsx"), b"changed")?;
    assert_eq!(
        serde_json::to_value(preview(&target, &manifest, 0, 1)?)?["counts"],
        json!({"local_baseline_changed":1})
    );
    fs::remove_file(root.join("local.xlsx"))?;
    assert_eq!(
        serde_json::to_value(preview(&target, &manifest, 0, 1)?)?["counts"],
        json!({"local_baseline_unavailable":1})
    );
    Ok(())
}

#[test]
fn coverage_preview_retains_ambiguous_generations_without_selecting_one() -> Result<()> {
    let (root, source) = fixture()?;
    let target = root.join("package");
    export(&source, &target)?;
    let database = target.join("metadata.sqlite");
    let connection = Connection::open(&database)?;
    let (producer, payload): (Option<String>, String) = connection.query_row(
        "SELECT provenance_id,payload FROM entries WHERE source_id=?1",
        ["source:'0"],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut payload: Value = serde_json::from_str(&payload)?;
    payload["measurements"]["worker_jobs_completed"] = json!(999);
    let generation = artifact_cache::sha256(&serde_json::to_vec(&(&payload, &producer))?);
    connection.execute(
        "INSERT INTO entries VALUES (?1,?2,'prepared',?3,?4)",
        params![
            "source:'0",
            generation,
            producer,
            serde_json::to_string(&payload)?
        ],
    )?;
    connection.close().map_err(|(_, error)| error)?;
    let path = target.join("manifest.json");
    let mut header: Header = serde_json::from_slice(&fs::read(&path)?)?;
    header.database_sha256 = artifact_cache::hash_file(&database)?;
    header.entries += 1;
    header.prepared += 1;
    fs::write(path, serde_json::to_vec(&header)?)?;
    let manifest = selection(&root, &["source:'0"])?;
    let result = serde_json::to_value(preview(&target, &manifest, 0, 1)?)?;
    assert_eq!(result["counts"], json!({"ambiguous_generation":1}));
    assert_eq!(
        result["items"][0]["candidates"].as_array().unwrap().len(),
        2
    );
    let choices = ImportOptions {
        generations: BTreeMap::from([("source:'0".into(), generation)]),
        adopt_packaged_baseline: false,
    };
    assert_eq!(
        materialize(&target, &manifest, &root.join("chosen"), &choices)?,
        1
    );
    let invalid = ImportOptions {
        generations: BTreeMap::from([("source:'0".into(), "f".repeat(64))]),
        adopt_packaged_baseline: false,
    };
    assert!(materialize(&target, &manifest, &root.join("invalid-choice"), &invalid).is_err());
    assert!(!root.join("invalid-choice").exists());
    Ok(())
}

#[test]
fn import_materializes_exact_pairs_with_shared_names_and_producer_evidence() -> Result<()> {
    let (root, source) = fixture()?;
    let package = root.join("package");
    export(&source, &package)?;
    let manifest = selection(&root, &["source:'0", "source:'1"])?;
    let target = root.join("imported");
    assert_eq!(
        materialize(&package, &manifest, &target, &ImportOptions::default())?,
        2
    );
    assert!(!target.join("incomplete.json").exists());
    let index: Value = serde_json::from_slice(&fs::read(target.join("import.json"))?)?;
    assert_eq!(index["visual_validation"], "pending");
    assert_eq!(index["provenance"].as_object().unwrap().len(), 1);
    let config = crate::validation::RunConfig::from_file(&target.join("compare.json"))?;
    config.validate()?;
    let a = crate::validation_sources::discover(Path::new(&config.source_a))?;
    let b = crate::validation_sources::discover(Path::new(config.source_b.as_ref().unwrap()))?;
    assert_eq!(
        a.files.keys().collect::<Vec<_>>(),
        b.files.keys().collect::<Vec<_>>()
    );
    assert_eq!(a.files.len(), 2);
    for entry in index["entries"].as_array().unwrap() {
        let name = entry["pair_file"].as_str().unwrap();
        assert_eq!(name.len(), 69);
        assert_eq!(
            fs::read(target.join("baseline").join(name))?,
            b"baseline bytes"
        );
        assert_eq!(
            fs::read(target.join("check-performance").join(name))?,
            b"optimized"
        );
        assert_eq!(
            entry["prepared_evidence"]["measurements"]["worker_jobs_completed"]
                .as_u64()
                .is_some(),
            true
        );
    }
    let before = fs::read(target.join("import.json"))?;
    assert!(materialize(&package, &manifest, &target, &ImportOptions::default()).is_err());
    assert_eq!(before, fs::read(target.join("import.json"))?);
    Ok(())
}

#[test]
fn import_rejects_incomplete_selection_before_creating_output() -> Result<()> {
    let (root, source) = fixture()?;
    let package = root.join("package");
    export(&source, &package)?;
    let manifest = selection(&root, &["source:'0", "failed-source"])?;
    let target = root.join("imported");
    assert!(materialize(&package, &manifest, &target, &ImportOptions::default()).is_err());
    assert!(!target.exists());
    let manifest = selection(&root, &["source:'0"])?;
    assert!(materialize(
        &package,
        &manifest,
        &package.join("nested"),
        &ImportOptions::default()
    )
    .is_err());
    assert!(!package.join("nested").exists());
    Ok(())
}

#[test]
fn explicit_baseline_adoption_requires_raw_identity_for_different_conversions() -> Result<()> {
    let (root, source) = fixture()?;
    let raw = json!({"sha256":"a".repeat(64),"format":"xls","member":null});
    let receipt_path = source.join("input-000000/prepared.json");
    let mut receipt: Value = serde_json::from_slice(&fs::read(&receipt_path)?)?;
    receipt["raw_source"] = raw.clone();
    fs::write(&receipt_path, serde_json::to_vec(&receipt)?)?;
    let package = root.join("package");
    export(&source, &package)?;
    let manifest = selection(&root, &["source:'0"])?;
    fs::write(root.join("local.xlsx"), b"different conversion")?;
    let local_hash = artifact_cache::hash_file(&root.join("local.xlsx"))?;
    let mut selected: Value = serde_json::from_slice(&fs::read(&manifest)?)?;
    selected["inputs"][0]["baseline_sha256"] = json!(local_hash);
    selected["inputs"][0]["raw_source"] = raw;
    fs::write(&manifest, serde_json::to_vec(&selected)?)?;
    assert!(materialize(
        &package,
        &manifest,
        &root.join("strict"),
        &ImportOptions::default()
    )
    .is_err());
    let options = ImportOptions {
        adopt_packaged_baseline: true,
        ..Default::default()
    };
    let target = root.join("adopted");
    assert_eq!(materialize(&package, &manifest, &target, &options)?, 1);
    assert_eq!(fs::read(root.join("local.xlsx"))?, b"different conversion");
    let index: Value = serde_json::from_slice(&fs::read(target.join("import.json"))?)?;
    assert_eq!(index["entries"][0]["requested_baseline_sha256"], local_hash);
    assert_ne!(
        index["entries"][0]["prepared_evidence"]["source_sha256"],
        local_hash
    );
    selected["inputs"][0]["raw_source"]["sha256"] = json!("b".repeat(64));
    fs::write(&manifest, serde_json::to_vec(&selected)?)?;
    assert!(materialize(&package, &manifest, &root.join("wrong-raw"), &options).is_err());
    selected["inputs"][0]
        .as_object_mut()
        .unwrap()
        .remove("raw_source");
    fs::write(&manifest, serde_json::to_vec(&selected)?)?;
    assert!(materialize(&package, &manifest, &root.join("unproven-raw"), &options).is_err());
    Ok(())
}

#[test]
fn explicit_adoption_can_restore_an_unavailable_matching_baseline() -> Result<()> {
    let (root, source) = fixture()?;
    let package = root.join("package");
    export(&source, &package)?;
    let manifest = selection(&root, &["source:'0"])?;
    fs::remove_file(root.join("local.xlsx"))?;
    let options = ImportOptions {
        adopt_packaged_baseline: true,
        ..Default::default()
    };
    assert_eq!(
        materialize(&package, &manifest, &root.join("restored"), &options)?,
        1
    );
    assert!(!root.join("local.xlsx").exists());
    Ok(())
}

#[test]
fn generation_choice_files_validate_version_hashes_and_selected_sources() -> Result<()> {
    let (root, source) = fixture()?;
    let path = root.join("choices.json");
    fs::write(
        &path,
        format!(
            "\u{feff}{}",
            json!({"version":1,"generations":{"source:'0":"A".repeat(64)}})
        ),
    )?;
    let options = ImportOptions::from_arguments(Some(&path), false)?;
    assert_eq!(options.generations["source:'0"], "a".repeat(64));
    fs::write(&path, br#"{"version":2,"generations":{}}"#)?;
    assert!(ImportOptions::from_arguments(Some(&path), false).is_err());
    fs::write(&path, br#"{"version":1,"generations":{"id":"../escape"}}"#)?;
    assert!(ImportOptions::from_arguments(Some(&path), false).is_err());
    let package = root.join("package");
    export(&source, &package)?;
    let manifest = selection(&root, &["source:'1"])?;
    assert!(materialize(
        &package,
        &manifest,
        &root.join("outside-selection"),
        &options
    )
    .is_err());
    assert!(!root.join("outside-selection").exists());
    Ok(())
}
