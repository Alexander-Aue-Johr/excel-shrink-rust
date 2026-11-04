//! Reuse only the converted original; prior Shrink variants are never restored.
use super::*;

/// Restore a source-matched, hash-verified XLSX from an earlier preparation bundle.
/// Missing bundles return false; inconsistent metadata or bytes fail explicitly.
pub(super) fn restore(
    directory: &Path,
    identity: &str,
    source_sha256: &str,
    output: &Path,
) -> Result<bool> {
    let name = format!("{}.zip", &artifact_cache::sha256(identity.as_bytes())[..20]);
    let path = directory.join(name);
    if !path.is_file() {
        return Ok(false);
    }
    let mut zip = ZipArchive::new(fs::File::open(path)?)?;
    let metadata: Value = serde_json::from_reader(zip.by_name("preparation.json")?)?;
    ensure!(
        metadata["identity"] == identity,
        "converted-original identity mismatch"
    );
    ensure!(
        metadata["source_sha256"] == source_sha256,
        "converted-original source hash mismatch"
    );
    let member = metadata["converted_member"]
        .as_str()
        .context("missing converted original member")?;
    let expected = metadata["converted_sha256"]
        .as_str()
        .context("missing converted original hash")?;
    // Copy a single named entry to a fixed destination; never extract archive paths.
    std::io::copy(&mut zip.by_name(member)?, &mut fs::File::create(output)?)?;
    ensure!(
        artifact_cache::hash_file(output)? == expected,
        "converted-original content hash mismatch"
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restores_only_verified_original_and_rejects_stale_source() -> Result<()> {
        let root = super::super::super::tests::test_directory("converted-original-cache")?;
        let identity = "fixture:1:book.xls";
        let bytes = b"converted original bytes";
        let bundle = root.join(format!(
            "{}.zip",
            &artifact_cache::sha256(identity.as_bytes())[..20]
        ));
        let mut zip = zip::ZipWriter::new(fs::File::create(bundle)?);
        let options = zip::write::FileOptions::default();
        zip.start_file("converted.xlsx", options)?;
        zip.write_all(bytes)?;
        zip.start_file("default.xlsx", options)?;
        zip.write_all(b"obsolete shrink result")?;
        zip.start_file("preparation.json", options)?;
        zip.write_all(&serde_json::to_vec(
            &json!({"identity":identity,"source_sha256":"source",
            "converted_member":"converted.xlsx","converted_sha256":artifact_cache::sha256(bytes)}),
        )?)?;
        zip.finish()?;
        let output = root.join("restored.xlsx");
        assert!(restore(&root, identity, "source", &output)?);
        assert_eq!(fs::read(&output)?, bytes);
        assert!(restore(&root, identity, "changed", &output).is_err());
        assert!(!restore(&root, "missing", "source", &output)?);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
