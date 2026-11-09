//! Import externally extracted candidates without claiming local reconstruction.

use super::*;
use crate::validation_sources::{
    hash_file, relative_name, require_regular_file, require_regular_source,
};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use zip::{write::FileOptions, CompressionMethod, ZipWriter};

#[derive(Deserialize)]
struct Handover {
    schema: u32,
    source_item: String,
    candidate_files: usize,
    candidate_bytes: u64,
}

#[derive(Deserialize)]
struct Candidate {
    sha256: String,
    bytes: u64,
    path: String,
}

/// Publish the receipt last. An interrupted import retains its incomplete directory
/// for inspection, and a new invocation must choose a new output directory.
pub(crate) fn import_handover(handover: &Path, output: &Path) -> Result<Value> {
    require_regular_source(handover)?;
    let handover = fs::canonicalize(handover)?;
    let output = std::path::absolute(output)?;
    ensure!(handover.is_dir(), "Choose a handover directory");
    ensure!(!output.exists(), "Choose a new import output directory");
    verify_sources(&handover)?;
    let summary: Handover =
        serde_json::from_slice(&fs::read(confined_file(&handover, "handover.json")?)?)?;
    ensure!(
        summary.schema == 1 && summary.source_item == "edrm.enron.email.data.set.v2.xml",
        "Unsupported Enron handover identity"
    );
    let candidates = read_candidates(&handover, &summary)?;
    verify_code_receipts(&handover)?;
    let parent = fs::canonicalize(
        output
            .parent()
            .context("Import output needs a parent directory")?,
    )?;
    ensure!(
        !parent.starts_with(&handover),
        "Import output must be outside the handover"
    );
    fs::create_dir(&output)?;
    let output = fs::canonicalize(&output)?;
    ensure!(
        !output.starts_with(&handover),
        "Import output must be outside the handover"
    );

    let archive = output.join("candidates.zip");
    let pending = output.join("candidates.zip.partial");
    write_candidates(&handover, &candidates, &pending)?;
    fs::rename(&pending, &archive)?;
    // Preserve the external ledger, direct occurrences, review cases and recipe.
    // Only the candidate archive is submitted to the ordinary corpus runner.
    freeze_metadata(&handover, &output.join("handover"))?;
    let receipt = json!({
        "version":1,"profile":"enron-edrm-v2","provenance":"externally_generated",
        "verification":"candidate_sha256_and_frozen_source_files",
        "local_reconstruction":false,"candidate_count":candidates.len(),
        "candidate_bytes":summary.candidate_bytes,
        "archive":{"path":"candidates.zip","sha256":hash_file(&archive)?,
            "bytes":fs::metadata(&archive)?.len()},
        "selection_unit":"distinct_candidate_sha256",
        "occurrence_manifest":"handover/manifests/candidate-locations.jsonl",
        "candidate_manifest":"handover/manifests/candidates.jsonl",
        "openability_verified":false,
        "member_policy":"candidates/<sha256>.xls; original candidate bytes unchanged"
    });
    crate::regression::artifact_cache::atomic_write(
        &output.join("import.json"),
        &serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(receipt)
}

fn confined_file(root: &Path, name: &str) -> Result<PathBuf> {
    let name = relative_name(name)?;
    let path = root.join(name);
    require_regular_file(&path)?;
    let canonical = fs::canonicalize(&path)?;
    ensure!(
        canonical.starts_with(root),
        "Handover file escapes its root"
    );
    Ok(canonical)
}

fn verify_sources(root: &Path) -> Result<()> {
    let supplied: Value =
        serde_json::from_slice(&fs::read(confined_file(root, "manifests/sources.json")?)?)?;
    let pinned: Value =
        serde_json::from_str(include_str!("../../../profiles/corpora/enron-edrm-v2.json"))?;
    ensure!(
        supplied["item"] == pinned["source"]["identifier"],
        "Enron source item differs"
    );
    let rows = supplied["archives"]
        .as_array()
        .context("Missing source archives")?;
    let expected = pinned["archives"]
        .as_array()
        .context("Missing pinned source archives")?;
    ensure!(
        rows.len() == expected.len(),
        "Enron source archive count differs"
    );
    let mut names = BTreeSet::new();
    for row in rows {
        let name = row["name"]
            .as_str()
            .context("Missing source archive name")?;
        ensure!(names.insert(name), "Duplicate source archive name");
        let reference = expected
            .iter()
            .find(|entry| entry["name"] == name)
            .context("Unknown source archive")?;
        ensure!(
            row["url"] == reference["url"]
                && row["size"] == reference["bytes"]
                && row["sha1"] == reference["sha1"]
                && row["md5"] == reference["md5"],
            "Enron source identity differs for {name}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> Result<()> {
        for directory in [
            "manifests",
            "receipts",
            "scripts/pipeline",
            "database",
            "artifacts",
        ] {
            fs::create_dir_all(root.join(directory))?;
        }
        let pinned: Value =
            serde_json::from_str(include_str!("../../../profiles/corpora/enron-edrm-v2.json"))?;
        let archives: Vec<Value> = pinned["archives"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                json!({"name":row["name"],"url":row["url"],"size":row["bytes"],
                "sha1":row["sha1"],"md5":row["md5"]})
            })
            .collect();
        fs::write(
            root.join("manifests/sources.json"),
            serde_json::to_vec(&json!({
                "item":"edrm.enron.email.data.set.v2.xml","archives":archives
            }))?,
        )?;
        fs::write(root.join("artifacts/candidate.bin"), b"candidate")?;
        let sha256 = hash_file(&root.join("artifacts/candidate.bin"))?;
        fs::write(
            root.join("manifests/candidates.jsonl"),
            serde_json::to_vec(&json!({
                "sha256":sha256,"bytes":9,"path":"artifacts/candidate.bin","kind":"ole",
                "inspection_state":"review"
            }))?,
        )?;
        fs::write(
            root.join("manifests/candidate-locations.jsonl"),
            b"retained external occurrence evidence",
        )?;
        fs::write(
            root.join("scripts/pipeline/recipe.py"),
            b"# Frozen external recipe\n",
        )?;
        fs::write(
            root.join("receipts/code-files.json"),
            serde_json::to_vec(&json!([{
                "path":"recipe.py","sha256":hash_file(&root.join("scripts/pipeline/recipe.py"))?
            }]))?,
        )?;
        fs::write(
            root.join("handover.json"),
            serde_json::to_vec(&json!({
                "schema":1,"source_item":"edrm.enron.email.data.set.v2.xml",
                "candidate_files":1,"candidate_bytes":9
            }))?,
        )?;
        fs::write(root.join("README.md"), b"External extraction fixture")?;
        fs::write(
            root.join("verify_handover.py"),
            b"# External verifier fixture",
        )?;
        fs::write(
            root.join("database/snapshot.sqlite"),
            b"opaque retained ledger",
        )?;
        Ok(())
    }

    #[test]
    fn import_preserves_bytes_review_evidence_and_deterministic_archive_identity() -> Result<()> {
        let root = std::env::temp_dir().join(format!("enron-import-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        fixture(&source)?;
        let first = import_handover(&source, &root.join("first"))?;
        let second = import_handover(&source, &root.join("second"))?;
        assert_eq!(first["archive"]["sha256"], second["archive"]["sha256"]);
        assert_eq!(first["openability_verified"], false);
        assert_eq!(first["provenance"], "externally_generated");
        let mut archive = zip::ZipArchive::new(fs::File::open(root.join("first/candidates.zip"))?)?;
        assert_eq!(archive.len(), 1);
        let mut bytes = Vec::new();
        archive.by_index(0)?.read_to_end(&mut bytes)?;
        assert_eq!(bytes, b"candidate");
        assert_eq!(
            fs::read(root.join("first/handover/manifests/candidates.jsonl"))?,
            fs::read(source.join("manifests/candidates.jsonl"))?
        );
        assert!(import_handover(&source, &root.join("first")).is_err());
        drop(archive);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn changed_candidates_leave_no_success_receipt() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("enron-import-corrupt-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        fixture(&source)?;
        fs::write(source.join("artifacts/candidate.bin"), b"different")?;
        let output = root.join("output");
        assert!(import_handover(&source, &output).is_err());
        assert!(!output.join("import.json").exists());
        assert!(output.join("candidates.zip.partial").exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn changed_source_collection_is_rejected_before_output_creation() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("enron-import-source-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        fixture(&source)?;
        fs::write(
            source.join("manifests/sources.json"),
            b"{\"item\":\"different\",\"archives\":[]}",
        )?;
        let output = root.join("output");
        assert!(import_handover(&source, &output).is_err());
        assert!(!output.exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}

fn read_candidates(root: &Path, summary: &Handover) -> Result<Vec<Candidate>> {
    let path = confined_file(root, "manifests/candidates.jsonl")?;
    let reader = BufReader::new(fs::File::open(path)?);
    let mut candidates = Vec::new();
    let mut hashes = BTreeSet::new();
    let mut total = 0u64;
    for line in reader.lines() {
        let candidate: Candidate = serde_json::from_str(&line?)?;
        ensure!(
            valid_sha256(&candidate.sha256)
                && candidate.sha256 == candidate.sha256.to_ascii_lowercase()
                && hashes.insert(candidate.sha256.clone()),
            "Invalid or duplicate candidate SHA-256"
        );
        let path = confined_file(root, &candidate.path)?;
        ensure!(
            fs::metadata(path)?.len() == candidate.bytes,
            "Candidate length differs"
        );
        total = total
            .checked_add(candidate.bytes)
            .context("Candidate byte count overflow")?;
        candidates.push(candidate);
    }
    ensure!(
        !candidates.is_empty()
            && candidates.len() == summary.candidate_files
            && total == summary.candidate_bytes,
        "Candidate inventory totals differ"
    );
    candidates.sort_by(|left, right| left.sha256.cmp(&right.sha256));
    Ok(candidates)
}

fn verify_code_receipts(root: &Path) -> Result<()> {
    let receipts: Vec<Value> =
        serde_json::from_slice(&fs::read(confined_file(root, "receipts/code-files.json")?)?)?;
    ensure!(!receipts.is_empty(), "Missing extraction code receipts");
    for receipt in receipts {
        let name = receipt["path"]
            .as_str()
            .context("Missing frozen code path")?;
        let path = confined_file(root, &format!("scripts/pipeline/{}", relative_name(name)?))?;
        ensure!(
            hash_file(&path)? == receipt["sha256"],
            "Frozen extraction code differs for {name}"
        );
    }
    Ok(())
}

fn write_candidates(root: &Path, candidates: &[Candidate], output: &Path) -> Result<()> {
    let mut zip = ZipWriter::new(fs::File::create_new(output)?);
    let options = FileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default())
        .large_file(true);
    let mut buffer = vec![0u8; 128 * 1024];
    for (index, candidate) in candidates.iter().enumerate() {
        let path = confined_file(root, &candidate.path)?;
        let mut source = fs::File::open(path)?;
        zip.start_file(format!("candidates/{}.xls", candidate.sha256), options)?;
        let mut digest = Sha256::new();
        let mut bytes = 0u64;
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
            zip.write_all(&buffer[..count])?;
            bytes += count as u64;
        }
        ensure!(
            bytes == candidate.bytes && format!("{:x}", digest.finalize()) == candidate.sha256,
            "Candidate changed or SHA-256 differs for {}",
            candidate.sha256
        );
        if (index + 1) % 1000 == 0 {
            eprintln!(
                "Verified and imported {}/{} candidates",
                index + 1,
                candidates.len()
            );
        }
    }
    zip.finish()?.sync_all()?;
    Ok(())
}

fn freeze_metadata(source: &Path, output: &Path) -> Result<()> {
    fs::create_dir(output)?;
    for name in ["handover.json", "README.md", "verify_handover.py"] {
        fs::copy(confined_file(source, name)?, output.join(name))?;
    }
    let mut identities = Vec::new();
    for directory in ["manifests", "receipts", "scripts", "database"] {
        let root = source.join(directory);
        require_regular_source(&root)?;
        for entry in walkdir::WalkDir::new(&root).follow_links(false) {
            let entry = entry?;
            require_regular_source(entry.path())?;
            let relative = entry.path().strip_prefix(source)?;
            let target = output.join(relative);
            if entry.file_type().is_dir() {
                fs::create_dir_all(target)?;
                continue;
            }
            require_regular_file(entry.path())?;
            fs::copy(entry.path(), &target)?;
            let hash = hash_file(&target)?;
            ensure!(
                hash_file(entry.path())? == hash,
                "Handover metadata changed during copy"
            );
            identities
                .push(json!({"path":relative.to_string_lossy().replace('\\', "/"),"sha256":hash}));
        }
    }
    fs::write(
        output.join("received-files.json"),
        serde_json::to_vec_pretty(&identities)?,
    )?;
    Ok(())
}
