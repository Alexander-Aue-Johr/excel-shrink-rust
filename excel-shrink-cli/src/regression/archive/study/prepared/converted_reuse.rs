//! Reuse only the pre-shrink conversion from a previous, explicitly selected run.
use super::*;

/// Restore a converted input only when both its source and its own bytes match.
/// Shrink variants and baseline repairs are never selected as conversion inputs.
pub(super) fn restore(
    bundle: &Path,
    identity: &str,
    source_sha256: &str,
    output: &Path,
) -> Result<Option<Value>> {
    if !bundle.exists() {
        return Ok(None);
    }
    let mut zip = ZipArchive::new(fs::File::open(bundle)?)?;
    let metadata: Value = serde_json::from_reader(zip.by_name("preparation.json")?)?;
    ensure!(
        metadata["identity"] == identity,
        "conversion bundle identity mismatch"
    );
    ensure!(
        metadata["source_sha256"] == source_sha256,
        "conversion source checksum mismatch"
    );
    let member = metadata["converted_member"]
        .as_str()
        .context("bundle has no pre-shrink converted input")?;
    let expected = metadata["converted_sha256"]
        .as_str()
        .context("bundle has no converted input checksum")?;
    let pending = output.with_extension("pending.xlsx");
    let result = (|| -> Result<()> {
        std::io::copy(&mut zip.by_name(member)?, &mut fs::File::create(&pending)?)?;
        ensure!(
            artifact_cache::hash_file(&pending)? == expected,
            "converted input checksum mismatch"
        );
        if output.exists() {
            fs::remove_file(output)?;
        }
        fs::rename(&pending, output)?;
        Ok(())
    })();
    if result.is_err() && pending.exists() {
        fs::remove_file(&pending)?;
    }
    result?;
    Ok(Some(json!({"conversion":metadata["conversion"],
        "bundle":bundle,"converted_sha256":expected})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_conversion_and_rejects_changed_sources_or_corrupt_bytes() -> Result<()> {
        let root = super::super::super::tests::test_directory("converted-reuse")?;
        let bundle = root.join("previous.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&bundle)?);
        let options = zip::write::FileOptions::default();
        zip.start_file("converted.xlsx", options)?;
        zip.write_all(b"converted original")?;
        zip.start_file("default.xlsx", options)?;
        zip.write_all(b"old shrink output must not be reused")?;
        zip.start_file("preparation.json", options)?;
        zip.write_all(&serde_json::to_vec(
            &json!({"identity":"example","source_sha256":"source",
            "converted_member":"converted.xlsx",
            "converted_sha256":artifact_cache::sha256(b"converted original"),"conversion":{}}),
        )?)?;
        zip.finish()?;
        let output = root.join("restored.xlsx");
        assert!(restore(&root.join("missing.zip"), "example", "source", &output)?.is_none());
        assert!(restore(&bundle, "different", "source", &output).is_err());
        assert!(restore(&bundle, "example", "different", &output).is_err());
        assert!(!output.exists());
        restore(&bundle, "example", "source", &output)?.context("not restored")?;
        assert_eq!(fs::read(&output)?, b"converted original");
        fs::write(&output, b"incomplete earlier attempt")?;
        restore(&bundle, "example", "source", &output)?.context("retry not restored")?;
        assert_eq!(fs::read(&output)?, b"converted original");
        fs::remove_file(&output)?;
        let mut zip = zip::ZipWriter::new_append(
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&bundle)?,
        )?;
        zip.start_file("preparation.json", options)?;
        zip.write_all(&serde_json::to_vec(
            &json!({"identity":"example","source_sha256":"source",
            "converted_member":"default.xlsx","converted_sha256":"invalid"}),
        )?)?;
        zip.finish()?;
        assert!(restore(&bundle, "example", "source", &output).is_err());
        assert!(!output.exists());
        assert!(!output.with_extension("pending.xlsx").exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
