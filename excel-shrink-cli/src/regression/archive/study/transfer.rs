//! Portable completed results; imported evidence is verified before publication.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};

const VERSION: u32 = 1;
const MANIFEST_LIMIT: u64 = 16 * 1024 * 1024;
const ENTRY_LIMIT: u64 = 256 * 1024 * 1024;
const TOTAL_LIMIT: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    package_id: String,
    created_unix_ms: u64,
    records: Vec<String>,
    #[serde(default)]
    excluded_missing_source_fingerprint: Vec<String>,
    sources: BTreeMap<String, String>,
    entries: Vec<Entry>,
}

/// Export completed durable records, their referenced PNG evidence and immutable
/// machine/run manifests. Prepared bundles and private request files are excluded.
#[cfg(test)]
fn export_package(output: &Path, package: &Path) -> Result<Value> {
    let lock = output_lock(output)?;
    export_package_locked(output, package, &lock)
}

pub(super) fn export_package_locked(
    output: &Path,
    package: &Path,
    _lock: &fs::File,
) -> Result<Value> {
    ensure!(
        !package.exists(),
        "export package already exists: {}",
        package.display()
    );
    let mut paths = BTreeSet::new();
    let mut records = Vec::new();
    let mut excluded_missing_source_fingerprint = Vec::new();
    let mut sources = BTreeMap::new();
    for entry in fs::read_dir(output.join("records"))? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let record: Value = serde_json::from_slice(&fs::read(&path)?)?;
        if !completed(&record) {
            continue;
        }
        let (identity, key, corpus) = validate_record(&record)?;
        if record["source_fingerprint"].as_str().is_none() {
            // An unchanged corpus URL does not prove which historical ZIP was
            // measured. Preserve this omission explicitly instead of guessing.
            excluded_missing_source_fingerprint.push(identity.to_owned());
            continue;
        }
        ensure!(
            path.file_name().and_then(|v| v.to_str()) == Some(&format!("{key}.json")),
            "record filename does not match identity"
        );
        if let Some(fingerprint) = record["source_fingerprint"].as_str() {
            ensure!(hash(fingerprint), "invalid source catalog fingerprint");
            if let Some(old) = sources.insert(corpus.clone(), fingerprint.to_owned()) {
                ensure!(
                    old.is_empty() || old == fingerprint,
                    "conflicting source catalogs in output"
                );
            }
        } else {
            sources.entry(corpus.clone()).or_insert_with(String::new);
        }
        records.push(identity.to_owned());
        paths.insert(format!("records/{key}.json"));
        evidence_paths(&record, &mut paths)?;
        add_metadata(output, &record, &mut paths)?;
    }
    ensure!(!records.is_empty(), "no completed records with a source catalog fingerprint to export; {} historical records excluded", excluded_missing_source_fingerprint.len());
    let mut entries = Vec::new();
    let mut total = 0u64;
    for name in paths {
        let path = checked_path(output, &name)?;
        let bytes = fs::metadata(&path)?.len();
        ensure!(
            bytes <= ENTRY_LIMIT,
            "result entry exceeds export size limit"
        );
        total = total.checked_add(bytes).context("package size overflow")?;
        ensure!(total <= TOTAL_LIMIT, "result package exceeds size limit");
        entries.push(Entry {
            path: name,
            bytes,
            sha256: artifact_cache::hash_file(&path)?,
        });
    }
    let manifest = Manifest {
        schema_version: VERSION,
        package_id: provenance::new_uuid()?,
        created_unix_ms: provenance::now_ms(),
        records,
        excluded_missing_source_fingerprint,
        sources,
        entries,
    };
    let pending =
        artifact_cache::unique_path(package.parent().context("package has no parent")?, "export")?;
    let saved = (|| -> Result<()> {
        let mut zip = zip::ZipWriter::new(
            fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&pending)?,
        );
        let options = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(true);
        zip.start_file("result-package.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        for entry in &manifest.entries {
            zip.start_file(&entry.path, options)?;
            std::io::copy(
                &mut fs::File::open(checked_path(output, &entry.path)?)?,
                &mut zip,
            )?;
        }
        zip.finish()?.sync_all()?;
        fs::rename(&pending, package)?;
        Ok(())
    })();
    if saved.is_err() {
        let _ = fs::remove_file(&pending);
    }
    saved?;
    Ok(
        json!({"package":package,"sha256":artifact_cache::hash_file(package)?,
        "records":manifest.records.len(),"entries":manifest.entries.len(),"payload_bytes":total,
        "excluded_missing_source_fingerprint":manifest.excluded_missing_source_fingerprint}),
    )
}

/// `expected_sources` is the explicit corpus whitelist from the study request.
/// An empty fingerprint means that corpus is known but its archive is unavailable
/// here. Catalog/source hashes are still checked against any existing records.
/// Nonidentical existing results are retained as conflicts, never overwritten.
#[cfg(test)]
fn import_package(
    output: &Path,
    package: &Path,
    expected_sources: &BTreeMap<String, String>,
) -> Result<Value> {
    let lock = output_lock(output)?;
    import_package_locked(output, package, expected_sources, &lock)
}

pub(super) fn import_package_locked(
    output: &Path,
    package: &Path,
    expected_sources: &BTreeMap<String, String>,
    _lock: &fs::File,
) -> Result<Value> {
    let package_hash = artifact_cache::hash_file(package)?;
    let destination = output.join("imports").join(&package_hash);
    let receipt_path = destination.join("receipt.json");
    if receipt_path.is_file() {
        let saved: Manifest =
            serde_json::from_slice(&fs::read(destination.join("result-package.json"))?)?;
        validate_sources(&saved.sources, expected_sources)?;
        let receipt: Value = serde_json::from_slice(&fs::read(&receipt_path)?)?;
        ensure!(
            receipt["package_sha256"] == package_hash,
            "invalid import receipt"
        );
        return Ok(receipt);
    }
    let stage = artifact_cache::unique_path(&output.join("imports"), "verify")?;
    fs::create_dir(&stage)?;
    let verified = verify_package(package, &stage, expected_sources);
    let manifest = match verified {
        Ok(manifest) => manifest,
        Err(error) => {
            fs::remove_dir_all(&stage)?;
            return Err(error);
        }
    };
    // A known corpus with a different catalog must not be silently combined,
    // even when this package contains different members of that corpus.
    if output.join("records").is_dir() {
        for entry in fs::read_dir(output.join("records"))? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let existing: Value = serde_json::from_slice(&fs::read(path)?)?;
            if let (Some(corpus), Some(fingerprint)) = (
                existing["corpus_id"].as_str(),
                existing["source_fingerprint"].as_str(),
            ) {
                if let Some(incoming) = manifest.sources.get(corpus).filter(|s| !s.is_empty()) {
                    if incoming != fingerprint {
                        fs::remove_dir_all(&stage)?;
                        anyhow::bail!(
                            "package catalog conflicts with existing results for corpus {corpus}"
                        );
                    }
                }
            }
        }
    }
    // Keep the verified incoming snapshot, including conflict evidence, together.
    // Its record bytes are never rewritten to pretend this machine measured them.
    if destination.exists() {
        // Recover an interrupted import only after verifying its durable snapshot.
        for entry in &manifest.entries {
            ensure!(
                artifact_cache::hash_file(&checked_path(&destination, &entry.path)?)?
                    == entry.sha256,
                "unfinished import snapshot was modified; review {}",
                destination.display()
            );
        }
        fs::remove_dir_all(&stage)?;
    } else {
        fs::rename(&stage, &destination)?;
    }
    let mut added = 0u64;
    let mut identical = 0u64;
    let mut conflicts = Vec::new();
    for identity in &manifest.records {
        let key = &artifact_cache::sha256(identity.as_bytes())[..20];
        let relative = format!("records/{key}.json");
        let incoming: Value = serde_json::from_slice(&fs::read(destination.join(&relative))?)?;
        let target = checked_path(output, &relative)?;
        let mut reason = None;
        if target.is_file() {
            let existing: Value = serde_json::from_slice(&fs::read(&target)?)?;
            if existing == incoming {
                identical += 1;
                continue;
            }
            reason = Some(
                if existing["source_sha256"].is_string()
                    && incoming["source_sha256"].is_string()
                    && existing["source_sha256"] != incoming["source_sha256"]
                {
                    "source workbook SHA-256 differs"
                } else if existing["source_fingerprint"].is_string()
                    && incoming["source_fingerprint"].is_string()
                    && existing["source_fingerprint"] != incoming["source_fingerprint"]
                {
                    "source catalog fingerprint differs"
                } else {
                    "different record already exists; explicit review required"
                },
            );
        }
        let mut needed = BTreeSet::new();
        evidence_paths(&incoming, &mut needed)?;
        add_metadata(&destination, &incoming, &mut needed)?;
        if reason.is_none() {
            for name in &needed {
                let path = checked_path(output, name)?;
                if path.exists()
                    && artifact_cache::hash_file(&path)?
                        != artifact_cache::hash_file(&destination.join(name))?
                {
                    reason = Some("referenced evidence or immutable metadata conflicts");
                    break;
                }
            }
        }
        if let Some(reason) = reason {
            conflicts.push(json!({"identity":identity,"reason":reason,"incoming_record":format!("imports/{package_hash}/{relative}")}));
            continue;
        }
        for name in needed {
            copy_new_or_identical(&destination.join(&name), &checked_path(output, &name)?)?;
        }
        copy_new_or_identical(&destination.join(&relative), &target)?;
        added += 1;
    }
    let receipt = json!({"schema_version":VERSION,"package_id":manifest.package_id,
        "package_sha256":package_hash,"imported_unix_ms":provenance::now_ms(),
        "added":added,"identical":identical,"conflicts":conflicts,
        "verified_snapshot":format!("imports/{package_hash}"),
        "excluded_missing_source_fingerprint":manifest.excluded_missing_source_fingerprint});
    artifact_cache::atomic_write(&receipt_path, &serde_json::to_vec_pretty(&receipt)?)?;
    Ok(receipt)
}

fn verify_package(
    package: &Path,
    stage: &Path,
    expected: &BTreeMap<String, String>,
) -> Result<Manifest> {
    let mut zip = ZipArchive::new(fs::File::open(package)?)?;
    let manifest: Manifest = {
        let entry = zip.by_name("result-package.json")?;
        ensure!(entry.size() <= MANIFEST_LIMIT, "package manifest too large");
        serde_json::from_reader(entry)?
    };
    ensure!(
        manifest.schema_version == VERSION,
        "unsupported result package schema"
    );
    ensure!(
        provenance::valid_uuid(&manifest.package_id),
        "invalid package UUID"
    );
    let mut names = BTreeSet::new();
    let mut lower = BTreeSet::new();
    let mut total = 0u64;
    for entry in &manifest.entries {
        relative(&entry.path)?;
        ensure!(
            entry.path.starts_with("records/")
                || entry.path.starts_with("evidence/")
                || entry.path.starts_with("metadata/runs/")
                || entry.path.starts_with("metadata/machines/"),
            "unexpected package payload"
        );
        ensure!(
            hash(&entry.sha256) && entry.bytes <= ENTRY_LIMIT,
            "invalid entry digest or size"
        );
        ensure!(
            names.insert(entry.path.clone()) && lower.insert(entry.path.to_lowercase()),
            "duplicate or case-colliding entry"
        );
        total = total
            .checked_add(entry.bytes)
            .context("package size overflow")?;
        ensure!(total <= TOTAL_LIMIT, "package exceeds import size limit");
    }
    ensure!(
        zip.len() == manifest.entries.len() + 1,
        "ZIP contains unlisted or duplicate entries"
    );
    let mut actual_names = BTreeSet::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i)?;
        ensure!(
            actual_names.insert(entry.name().to_owned()),
            "duplicate ZIP member"
        );
        ensure!(
            entry.name() == "result-package.json" || names.contains(entry.name()),
            "unlisted ZIP member"
        );
        ensure!(
            !entry.is_dir() && entry.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
            "ZIP links/directories are not permitted"
        );
    }
    validate_sources(&manifest.sources, expected)?;
    for entry in &manifest.entries {
        let mut input = zip.by_name(&entry.path)?;
        ensure!(
            input.size() == entry.bytes,
            "ZIP entry size differs from manifest"
        );
        let path = checked_path(stage, &entry.path)?;
        fs::create_dir_all(path.parent().unwrap())?;
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?;
        let mut digest = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            ensure!(bytes <= entry.bytes, "ZIP entry exceeds declared size");
            digest.update(&buffer[..n]);
            output.write_all(&buffer[..n])?;
        }
        ensure!(
            bytes == entry.bytes && artifact_cache::hex(&digest.finalize()) == entry.sha256,
            "entry SHA-256 mismatch: {}",
            entry.path
        );
    }
    let mut ids = BTreeSet::new();
    let mut required = BTreeSet::new();
    for id in &manifest.records {
        ensure!(ids.insert(id), "duplicate record identity");
        let key = artifact_cache::sha256(id.as_bytes());
        let name = format!("records/{}.json", &key[..20]);
        ensure!(names.contains(&name), "missing declared record");
        let record: Value = serde_json::from_slice(&fs::read(stage.join(&name))?)?;
        let (actual, _, corpus) = validate_record(&record)?;
        ensure!(
            actual == id && completed(&record),
            "incomplete record or identity mismatch"
        );
        let fingerprint = manifest
            .sources
            .get(&corpus)
            .context("record corpus absent from package source catalog")?;
        ensure!(
            record["source_fingerprint"]
                .as_str()
                .is_none_or(|s| s == fingerprint),
            "record source catalog mismatch"
        );
        required.insert(name);
        evidence_paths(&record, &mut required)?;
        add_metadata(stage, &record, &mut required)?;
    }
    ensure!(!ids.is_empty(), "empty result package");
    ensure!(
        required == names,
        "package has missing or unreferenced payload files"
    );
    fs::write(
        stage.join("result-package.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

fn validate_sources(
    sources: &BTreeMap<String, String>,
    expected: &BTreeMap<String, String>,
) -> Result<()> {
    for (id, fingerprint) in sources {
        ensure!(hash(id), "invalid corpus identity");
        let trusted = expected
            .get(id)
            .context("package corpus is not in the requested corpus whitelist")?;
        ensure!(
            fingerprint.is_empty() || hash(fingerprint),
            "invalid source catalog fingerprint"
        );
        ensure!(
            trusted.is_empty() || trusted == fingerprint,
            "package source catalog differs from local archive"
        );
    }
    Ok(())
}

fn completed(record: &Value) -> bool {
    record["complete"] == true
        && matches!(
            record["status"].as_str(),
            Some("unchanged" | "differences" | "screened-equal" | "skipped")
        )
}

fn validate_record(record: &Value) -> Result<(&str, String, String)> {
    let id = record["identity"]
        .as_str()
        .context("record identity missing")?;
    let mut parts = id.splitn(3, ':');
    let corpus = parts.next().context("missing corpus identity")?;
    let index = parts.next().context("missing archive index")?;
    let member = parts.next().context("missing archive member")?;
    ensure!(
        hash(corpus) && index.parse::<usize>().is_ok() && !member.is_empty(),
        "invalid workbook identity"
    );
    ensure!(
        record["corpus_id"] == corpus,
        "record corpus ID does not match identity"
    );
    if let Some(name) = record["member_name"].as_str() {
        ensure!(name == member, "record member does not match identity");
    }
    for field in ["source_sha256", "baseline_sha256", "source_fingerprint"] {
        if let Some(value) = record[field].as_str() {
            ensure!(hash(value), "invalid {field}");
        }
    }
    Ok((
        id,
        artifact_cache::sha256(id.as_bytes())[..20].into(),
        corpus.into(),
    ))
}

fn evidence_paths(value: &Value, paths: &mut BTreeSet<String>) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key.ends_with("_image") {
                    if let Some(path) = value.as_str() {
                        relative(path)?;
                        ensure!(
                            path.starts_with("evidence/") && path.to_lowercase().ends_with(".png"),
                            "image evidence must be a relative evidence PNG"
                        );
                        paths.insert(path.to_owned());
                    }
                } else {
                    evidence_paths(value, paths)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                evidence_paths(value, paths)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn add_metadata(output: &Path, record: &Value, paths: &mut BTreeSet<String>) -> Result<()> {
    let mut references = Vec::new();
    provenance::collect_references(record, &mut references);
    for reference in references {
        let id = reference["run_id"]
            .as_str()
            .context("missing run reference")?;
        ensure!(provenance::valid_uuid(id), "invalid run reference UUID");
        let name = format!("metadata/runs/{id}.json");
        let run: Value = serde_json::from_slice(&fs::read(checked_path(output, &name)?)?)?;
        ensure!(
            run["run_id"] == id && run["machine_id"] == reference["machine_id"],
            "run reference mismatch"
        );
        if let Some(exe) = run["executable_sha256"].as_str() {
            ensure!(hash(exe), "invalid executable hash");
        }
        let machine = run["machine_manifest"]
            .as_str()
            .context("missing machine manifest")?;
        ensure!(
            machine.starts_with("metadata/machines/") && machine.ends_with(".json"),
            "invalid machine manifest path"
        );
        if reference["machine_manifest"].is_string() {
            ensure!(
                reference["machine_manifest"] == machine,
                "machine reference mismatch"
            );
        }
        let snapshot: Value = serde_json::from_slice(&fs::read(checked_path(output, machine)?)?)?;
        ensure!(
            snapshot["machine_id"] == run["machine_id"],
            "machine identity mismatch"
        );
        let expected = machine
            .rsplit('-')
            .next()
            .and_then(|v| v.strip_suffix(".json"))
            .context("missing machine snapshot hash")?;
        ensure!(
            hash(expected) && artifact_cache::sha256(&serde_json::to_vec(&snapshot)?) == expected,
            "machine snapshot content hash mismatch"
        );
        paths.insert(name);
        paths.insert(machine.to_owned());
    }
    Ok(())
}

fn hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn relative(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && !value.contains(['\\', ':', '\0', '<', '>', '"', '|', '?', '*'])
            && !value.starts_with('/'),
        "unsafe result path"
    );
    for component in value.split('/') {
        ensure!(
            !component.is_empty()
                && component != "."
                && component != ".."
                && !component.ends_with([' ', '.'])
                && !component.chars().any(|c| c.is_control()),
            "unsafe result path component"
        );
        let stem = component.split('.').next().unwrap().to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit()),
            "reserved Windows path component"
        );
    }
    Ok(())
}

fn checked_path(root: &Path, name: &str) -> Result<PathBuf> {
    relative(name)?;
    let mut current = root.to_owned();
    for component in name.split('/') {
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "result path traverses a symlink"
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    metadata.file_attributes() & 0x400 == 0,
                    "result path traverses a Windows reparse point"
                );
            }
        }
    }
    Ok(current)
}

fn copy_new_or_identical(source: &Path, target: &Path) -> Result<()> {
    if target.exists() {
        ensure!(
            artifact_cache::hash_file(source)? == artifact_cache::hash_file(target)?,
            "conflicting import artifact"
        );
        return Ok(());
    }
    fs::create_dir_all(target.parent().context("no result parent")?)?;
    // The output lock excludes other study/import writers. Atomic replacement
    // publishes only complete files, preserving a resumable per-record journal.
    artifact_cache::atomic_write(target, &fs::read(source)?)
}

#[cfg(test)]
fn output_lock(output: &Path) -> Result<fs::File> {
    fs::create_dir_all(output)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(output.join(".run.lock"))?;
    lock.try_lock()
        .context("report directory is in use; export/import between study invocations")?;
    Ok(lock)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(label: &str) -> Result<(PathBuf, Value, BTreeMap<String, String>)> {
        let output = super::super::tests::test_directory(label)?;
        let corpus = "a".repeat(64);
        let fingerprint = "b".repeat(64);
        let identity = format!("{corpus}:0:example.xlsx");
        let record = json!({"identity":identity,"corpus_id":corpus,"source_fingerprint":fingerprint,
            "member_name":"example.xlsx","complete":true,"status":"differences","whitespace_diagnosis":{"status":"not_investigated"},
            "results":[{"original_image":"evidence/file/original.png","shrunk_image":"evidence/file/shrunk.png"}]});
        fs::create_dir_all(output.join("evidence/file"))?;
        fs::write(output.join("evidence/file/original.png"), b"original image")?;
        fs::write(output.join("evidence/file/shrunk.png"), b"changed image")?;
        write_record(&record_path(&output, &identity), &record)?;
        Ok((output, record, BTreeMap::from([(corpus, fingerprint)])))
    }

    #[test]
    fn package_audit_roundtrip_keeps_its_proof_without_inventing_visual_checks() -> Result<()> {
        let (source, mut record, expected) = fixture("transfer-package-audit")?;
        let target = super::super::tests::test_directory("transfer-package-audit-target")?;
        record["status"] = json!("unchanged");
        record["capture_mode"] = json!("package-audit");
        record["package_identity"] = json!({"version":1,"status":"equivalent",
            "method":"streamed SHA-256 and length of every uncompressed ZIP member",
            "original":{"archive_sha256":"c".repeat(64),"parts":{"xl/worksheets/sheet1.xml":{"bytes":123,"sha256":"d".repeat(64)}}}});
        record["results"] = json!(prepared::PROFILES.map(|profile| json!({
            "profile":profile,"status":"package unchanged","checks_complete":false,
            "package_checks_complete":true,"sheets":[],"failures":[]
        })));
        let identity = record["identity"].as_str().unwrap();
        write_record(&record_path(&source, identity), &record)?;
        let package = source.join("package-audit.zip");
        export_package(&source, &package)?;
        let receipt = import_package(&target, &package, &expected)?;
        assert_eq!(receipt["added"], 1);
        let imported: Value = serde_json::from_slice(&fs::read(record_path(&target, identity))?)?;
        assert_eq!(imported, record);
        assert!(!super::super::verified_unchanged(&imported));
        fs::remove_dir_all(source)?;
        fs::remove_dir_all(target)?;
        Ok(())
    }

    #[test]
    fn completed_results_roundtrip_idempotently_and_conflicts_keep_original() -> Result<()> {
        let (source, record, expected) = fixture("transfer-source")?;
        let target = super::super::tests::test_directory("transfer-target")?;
        let package = source.join("results.zip");
        export_package(&source, &package)?;
        let receipt = import_package(&target, &package, &expected)?;
        assert_eq!(receipt["added"], 1);
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(record_path(
                &target,
                record["identity"].as_str().unwrap()
            ))?)?,
            record
        );
        assert_eq!(
            fs::read(target.join("evidence/file/shrunk.png"))?,
            b"changed image"
        );
        assert_eq!(import_package(&target, &package, &expected)?, receipt);
        let mut changed = record.clone();
        changed["review_note"] = json!("different measurements");
        write_record(
            &record_path(&source, record["identity"].as_str().unwrap()),
            &changed,
        )?;
        let other = source.join("other.zip");
        export_package(&source, &other)?;
        let conflict = import_package(&target, &other, &expected)?;
        assert_eq!(conflict["added"], 0);
        assert_eq!(conflict["conflicts"].as_array().unwrap().len(), 1);
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(record_path(
                &target,
                record["identity"].as_str().unwrap()
            ))?)?,
            record
        );
        fs::remove_dir_all(source)?;
        fs::remove_dir_all(target)?;
        Ok(())
    }

    #[test]
    fn wrong_catalog_or_corpus_is_rejected_before_import() -> Result<()> {
        let (source, _, expected) = fixture("transfer-catalog")?;
        let target = super::super::tests::test_directory("transfer-catalog-target")?;
        let package = source.join("results.zip");
        export_package(&source, &package)?;
        assert!(import_package(&target, &package, &BTreeMap::new()).is_err());
        let wrong = expected
            .keys()
            .map(|k| (k.clone(), "c".repeat(64)))
            .collect();
        assert!(import_package(&target, &package, &wrong).is_err());
        let known = expected
            .keys()
            .map(|k| (k.clone(), String::new()))
            .collect();
        assert_eq!(import_package(&target, &package, &known)?["added"], 1);
        fs::remove_dir_all(source)?;
        fs::remove_dir_all(target)?;
        Ok(())
    }

    #[test]
    fn incomplete_records_are_excluded_and_absolute_evidence_is_rejected() -> Result<()> {
        let (source, mut record, _) = fixture("transfer-incomplete")?;
        record["complete"] = json!(false);
        write_record(
            &record_path(&source, record["identity"].as_str().unwrap()),
            &record,
        )?;
        assert!(export_package(&source, &source.join("empty.zip")).is_err());
        record["complete"] = json!(true);
        record["results"][0]["original_image"] = json!("C:/secret.png");
        write_record(
            &record_path(&source, record["identity"].as_str().unwrap()),
            &record,
        )?;
        assert!(export_package(&source, &source.join("unsafe.zip")).is_err());
        for path in [
            "../escape",
            "/absolute",
            "evidence/a/../b.png",
            "evidence/a\\b.png",
            "evidence/a.png:stream",
            "evidence/CON.png",
            "evidence/a. /b",
        ] {
            assert!(relative(path).is_err(), "{path}");
        }
        fs::remove_dir_all(source)?;
        Ok(())
    }

    #[test]
    fn altered_image_payload_fails_before_any_record_is_published() -> Result<()> {
        let (source, _, expected) = fixture("transfer-hash")?;
        let target = super::super::tests::test_directory("transfer-hash-target")?;
        let package = source.join("results.zip");
        export_package(&source, &package)?;
        let changed = source.join("tampered.zip");
        let mut input = ZipArchive::new(fs::File::open(&package)?)?;
        let mut output = zip::ZipWriter::new(fs::File::create(&changed)?);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for i in 0..input.len() {
            let mut entry = input.by_index(i)?;
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            if entry.name() == "evidence/file/shrunk.png" {
                bytes[0] ^= 1;
            }
            output.start_file(entry.name(), options)?;
            output.write_all(&bytes)?;
        }
        output.finish()?;
        assert!(format!(
            "{:#}",
            import_package(&target, &changed, &expected).unwrap_err()
        )
        .contains("SHA-256 mismatch"));
        assert_eq!(fs::read_dir(target.join("records"))?.count(), 0);
        fs::remove_dir_all(source)?;
        fs::remove_dir_all(target)?;
        Ok(())
    }

    #[test]
    fn historical_records_without_catalog_fingerprints_are_explicitly_excluded() -> Result<()> {
        let (source, _, mut expected) = fixture("transfer-legacy-catalog")?;
        let target = super::super::tests::test_directory("transfer-legacy-catalog-target")?;
        let legacy_corpus = "e".repeat(64);
        let legacy_id = format!("{legacy_corpus}:1:historical.xlsx");
        let legacy = json!({"identity":legacy_id,"corpus_id":legacy_corpus,
            "member_name":"historical.xlsx","complete":true,"status":"skipped","results":[]});
        expected.insert(legacy_corpus.clone(), "f".repeat(64));
        write_record(&record_path(&source, &legacy_id), &legacy)?;
        let package = source.join("results.zip");
        let exported = export_package(&source, &package)?;
        assert_eq!(exported["records"], 1);
        assert_eq!(
            exported["excluded_missing_source_fingerprint"],
            json!([legacy_id])
        );
        let imported = import_package(&target, &package, &expected)?;
        assert_eq!(imported["added"], 1);
        assert_eq!(
            imported["excluded_missing_source_fingerprint"],
            json!([legacy_id])
        );
        assert!(!record_path(&target, &legacy_id).exists());
        assert!(record_path(&source, &legacy_id).is_file());
        assert!(
            validate_sources(&BTreeMap::from([(legacy_corpus, String::new())]), &expected).is_err()
        );
        fs::remove_dir_all(source)?;
        fs::remove_dir_all(target)?;
        Ok(())
    }

    #[test]
    fn machine_and_run_manifests_follow_results_without_reattributing_them() -> Result<()> {
        let (source, mut record, expected) = fixture("transfer-metadata")?;
        let target = super::super::tests::test_directory("transfer-metadata-target")?;
        let machine = json!({"schema_version":1,"machine_id":"c".repeat(64),"label":"Laptop"});
        let snapshot_hash = artifact_cache::sha256(&serde_json::to_vec(&machine)?);
        let machine_path = format!(
            "metadata/machines/{}-{snapshot_hash}.json",
            machine["machine_id"].as_str().unwrap()
        );
        let run_id = provenance::new_uuid()?;
        let run = json!({"schema_version":1,"run_id":run_id,"machine_id":machine["machine_id"],"machine_manifest":machine_path,"executable_sha256":"d".repeat(64)});
        artifact_cache::atomic_write(
            &source.join(&machine_path),
            &serde_json::to_vec_pretty(&machine)?,
        )?;
        artifact_cache::atomic_write(
            &source.join(format!("metadata/runs/{run_id}.json")),
            &serde_json::to_vec_pretty(&run)?,
        )?;
        provenance::stamp(&mut record, &run, provenance::Stage::Comparison);
        write_record(
            &record_path(&source, record["identity"].as_str().unwrap()),
            &record,
        )?;
        let package = source.join("results.zip");
        export_package(&source, &package)?;
        assert_eq!(import_package(&target, &package, &expected)?["added"], 1);
        assert_eq!(
            fs::read(target.join(&machine_path))?,
            fs::read(source.join(&machine_path))?
        );
        let imported: Value = serde_json::from_slice(&fs::read(record_path(
            &target,
            record["identity"].as_str().unwrap(),
        ))?)?;
        assert_eq!(imported["provenance"]["comparison"]["run_id"], run_id);
        let mut altered = machine;
        altered["label"] = json!("Changed hardware snapshot");
        artifact_cache::atomic_write(
            &source.join(&machine_path),
            &serde_json::to_vec_pretty(&altered)?,
        )?;
        assert!(format!(
            "{:#}",
            export_package(&source, &source.join("invalid-metadata.zip")).unwrap_err()
        )
        .contains("snapshot content hash mismatch"));
        fs::remove_dir_all(source)?;
        fs::remove_dir_all(target)?;
        Ok(())
    }
}
