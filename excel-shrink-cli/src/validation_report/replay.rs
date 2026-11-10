//! Verify compressed progress history before copying it into a static report.
use super::evidence;
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};

const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_COMPRESSED_BYTES: u64 = 513 * 1024;
const MAX_DECODED_BYTES: u64 = 512 * 1024 + 64;

/// Read one bounded replay chunk for the replay-contract tests.
#[cfg(test)]
pub(super) fn read_chunk(path: &Path) -> Result<Value> {
    let bytes = read_bounded(path, MAX_COMPRESSED_BYTES)?;
    decode_chunk(&bytes)
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Replay file exceeds its read budget"
    );
    Ok(bytes)
}

fn decode_chunk(bytes: &[u8]) -> Result<Value> {
    let mut decoder = flate2::bufread::GzDecoder::new(bytes);
    let mut decoded = Vec::new();
    decoder
        .by_ref()
        .take(MAX_DECODED_BYTES + 1)
        .read_to_end(&mut decoded)?;
    ensure!(
        decoded.len() as u64 <= MAX_DECODED_BYTES,
        "Replay decoded chunk is too large"
    );
    ensure!(
        decoder.get_ref().is_empty(),
        "Replay chunk has trailing compressed data"
    );
    Ok(serde_json::from_slice(&decoded)?)
}

/// Copy only complete verified history. Older or interrupted histories remain explicitly unavailable.
pub(super) fn copy_channel(source: &Path, destination: &Path) -> Result<(Value, Vec<String>)> {
    if !source.join("replay/manifest.json").exists() {
        return Ok((
            json!({"available":false,"reason":"No recorded replay channel"}),
            Vec::new(),
        ));
    }
    let manifest_path = evidence::confined_file(source, "replay/manifest.json")?;
    ensure!(
        manifest_path.metadata()?.len() <= MAX_MANIFEST_BYTES,
        "Replay manifest is too large"
    );
    let manifest_bytes = read_bounded(&manifest_path, MAX_MANIFEST_BYTES)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)?;
    ensure!(
        manifest["version"] == 1
            && manifest["channel"] == "run-progress"
            && manifest["encoding"] == "gzip-json-checkpoints"
            && manifest["clock"] == "elapsed-monotonic-milliseconds",
        "Unsupported replay channel"
    );
    if manifest["complete"] == false {
        return Ok((
            json!({"available":false,"reason":"Recording is incomplete; retained history requires recovery"}),
            Vec::new(),
        ));
    }
    ensure!(
        manifest["complete"] == true,
        "Replay completeness is missing"
    );
    let chunks = manifest["chunks"]
        .as_array()
        .context("Replay chunk index is missing")?;
    let target = destination.join("replay");
    fs::create_dir(&target)?;
    let mut sequence = 0u64;
    let mut last_at = 0u64;
    let mut audit_files = Vec::new();
    for (index, entry) in chunks.iter().enumerate() {
        let name = format!("chunk-{index:06}.json.gz");
        ensure!(
            entry["file"] == name,
            "Replay chunk filename is not canonical"
        );
        let relative = format!("replay/{name}");
        let path = evidence::confined_file(source, &relative)?;
        let length = path.metadata()?.len();
        ensure!(
            length <= MAX_COMPRESSED_BYTES && entry["compressed_bytes"] == length,
            "Replay compressed size differs"
        );
        let bytes = read_bounded(&path, MAX_COMPRESSED_BYTES)?;
        ensure!(
            bytes.len() as u64 == length,
            "Replay chunk size changed while reading"
        );
        ensure!(
            entry["sha256"] == format!("{:x}", Sha256::digest(&bytes)),
            "Replay chunk checksum differs"
        );
        let value = decode_chunk(&bytes)?;
        let events = value
            .as_array()
            .context("Replay chunk is not an event array")?;
        ensure!(
            !events.is_empty() && events.len() <= 32 && entry["events"] == events.len(),
            "Replay chunk event count differs"
        );
        ensure!(
            entry["first_sequence"] == sequence && entry["first_at_ms"] == events[0]["at_ms"],
            "Replay chunk start differs"
        );
        for event in events {
            ensure!(
                serde_json::to_vec(event)?.len() <= 64 * 1024,
                "Replay checkpoint is too large"
            );
            let at = event["at_ms"]
                .as_u64()
                .context("Replay timestamp is invalid")?;
            ensure!(
                event["sequence"] == sequence && at >= last_at && event["state"].is_object(),
                "Replay event order or state is invalid"
            );
            sequence = sequence
                .checked_add(1)
                .context("Replay sequence overflow")?;
            last_at = at;
        }
        ensure!(entry["last_at_ms"] == last_at, "Replay chunk end differs");
        evidence::write_new(&target.join(&name), &bytes)?;
        audit_files.push(relative);
    }
    ensure!(
        manifest["events"] == sequence && manifest["duration_ms"] == last_at,
        "Replay manifest totals differ"
    );
    evidence::write_new(&target.join("manifest.json"), &manifest_bytes)?;
    audit_files.push("replay/manifest.json".into());
    Ok((
        json!({"available":true,"manifest":"replay/manifest.json","channel":"run-progress"}),
        audit_files,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    fn fixture() -> Result<(std::path::PathBuf, std::path::PathBuf)> {
        let root = std::env::temp_dir().join(format!("replay-export-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let target = root.join("export");
        fs::create_dir_all(source.join("replay"))?;
        fs::create_dir(&target)?;
        let events = json!([{"sequence":0,"at_ms":10,"state":{"state":"preparing"}},
            {"sequence":1,"at_ms":20,"state":{"state":"failed","error":"C:\\private\\book.xlsx"}}]);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&serde_json::to_vec(&events)?)?;
        let bytes = encoder.finish()?;
        fs::write(source.join("replay/chunk-000000.json.gz"), &bytes)?;
        fs::write(
            source.join("replay/manifest.json"),
            serde_json::to_vec(&json!({
                "version":1,"channel":"run-progress","encoding":"gzip-json-checkpoints","complete":true,
            "clock":"elapsed-monotonic-milliseconds",
                "events":2,"duration_ms":20,"chunks":[{"file":"chunk-000000.json.gz","events":2,
                    "first_sequence":0,"first_at_ms":10,"last_at_ms":20,"compressed_bytes":bytes.len(),
                    "sha256":format!("{:x}",Sha256::digest(&bytes))}]
            }))?,
        )?;
        Ok((source.canonicalize()?, target.canonicalize()?))
    }

    #[test]
    fn verified_channel_is_copied_and_compressed_metadata_is_audited() -> Result<()> {
        let (source, target) = fixture()?;
        let (availability, files) = copy_channel(&source, &target)?;
        assert_eq!(availability["available"], true);
        let review = super::super::publication::audit(&target, &files)?;
        assert!(review["metadata_finding_count"].as_u64().unwrap() > 0);
        Ok(())
    }

    #[test]
    fn corruption_and_path_changes_are_rejected() -> Result<()> {
        let (source, target) = fixture()?;
        fs::write(source.join("replay/chunk-000000.json.gz"), b"corrupt")?;
        assert!(copy_channel(&source, &target).is_err());
        let (source, target) = fixture()?;
        let path = source.join("replay/manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&path)?)?;
        manifest["chunks"][0]["file"] = json!("../outside.json.gz");
        fs::write(path, serde_json::to_vec(&manifest)?)?;
        assert!(copy_channel(&source, &target).is_err());
        Ok(())
    }

    #[test]
    fn expansion_beyond_the_budget_is_rejected() -> Result<()> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&vec![b' '; MAX_DECODED_BYTES as usize + 1])?;
        assert!(decode_chunk(&encoder.finish()?).is_err());
        Ok(())
    }

    #[test]
    fn valid_checksums_do_not_hide_invalid_event_order() -> Result<()> {
        let (source, target) = fixture()?;
        let path = source.join("replay/chunk-000000.json.gz");
        let mut events = read_chunk(&path)?;
        events[1]["sequence"] = json!(7);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&serde_json::to_vec(&events)?)?;
        let bytes = encoder.finish()?;
        fs::write(path, &bytes)?;
        let path = source.join("replay/manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&path)?)?;
        manifest["chunks"][0]["compressed_bytes"] = json!(bytes.len());
        manifest["chunks"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(&bytes)));
        fs::write(path, serde_json::to_vec(&manifest)?)?;
        assert!(copy_channel(&source, &target)
            .unwrap_err()
            .to_string()
            .contains("event order"));
        Ok(())
    }

    #[test]
    fn missing_and_incomplete_histories_are_explicitly_unavailable() -> Result<()> {
        let (source, target) = fixture()?;
        let path = source.join("replay/manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&path)?)?;
        manifest["complete"] = json!(false);
        fs::write(&path, serde_json::to_vec(&manifest)?)?;
        assert_eq!(copy_channel(&source, &target)?.0["available"], false);
        assert!(!target.join("replay").exists());
        fs::remove_file(path)?;
        assert_eq!(copy_channel(&source, &target)?.0["available"], false);
        Ok(())
    }

    #[test]
    fn ordinary_export_preserves_the_recorded_channel_without_changing_its_source() -> Result<()> {
        let (source, target) = fixture()?;
        let run = json!({"schema":1,"tool":"DeltaSieve","status":"failed","index_complete":true,
            "pairs":[],"summary":{"processed_pairs":0}});
        let before = serde_json::to_vec(&run)?;
        fs::write(source.join("run.json"), &before)?;
        let website = target.join("website");
        super::super::site::export_site(&source, &website)?;
        let exported: Value = serde_json::from_slice(&fs::read(website.join("run.json"))?)?;
        assert_eq!(exported["status"], "failed");
        assert_eq!(exported["replay"]["available"], true);
        assert!(website.join("replay/chunk-000000.json.gz").is_file());
        assert_eq!(fs::read(source.join("run.json"))?, before);
        Ok(())
    }
}
