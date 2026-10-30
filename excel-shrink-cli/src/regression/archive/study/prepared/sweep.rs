//! Hash sweeps consume durable shrink bundles; they never rerun shrinking.
use super::*;

fn cache_path(bundle: &Path) -> PathBuf {
    bundle.with_extension("capture-hashes.json")
}

/// Read a completed sweep only for the exact bundle and requested profile.
pub(super) fn cached(bundle: &Path, profile: Option<&str>) -> Result<Option<Value>> {
    let path = cache_path(bundle);
    if !path.exists() || profile.is_none() {
        return Ok(None);
    }
    let value: Value = match fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        Some(value) => value,
        None => return Ok(None),
    };
    if value["screen"]["profile"].as_str() != profile
        || value["bundle_sha256"] != artifact_cache::hash_file(bundle)?
    {
        return Ok(None);
    }
    Ok(value
        .get("screen")
        .filter(|screen| screen.is_object())
        .cloned())
}

/// Whether this bundle already has a reusable sweep for the selected profile.
pub(in super::super) fn cached_exists(bundle: &Path, profile: Option<&str>) -> Result<bool> {
    Ok(cached(bundle, profile)?.is_some())
}

/// Capture a prepared pair twice, preserving its bundle even if Excel fails.
/// Failed sweeps fall back to the normal selected-profile visual comparison.
pub(super) fn run(job: &MemberJob, record: &mut Value) -> Result<()> {
    record
        .as_object_mut()
        .context("missing workbook record")?
        .remove("hash_sweep_error");
    if let Err(error) = capture(job, record) {
        record["hash_sweep_error"] = json!(format!("{error:#}"));
        work_state::event(
            "hash_sweeping",
            "hash_sweep_failed",
            "Hash sweep failed; shrink bundle retained for visual fallback",
            false,
            None,
            json!({"error":record["hash_sweep_error"]}),
        );
        record["preparation_hash_screen"] = Value::Null;
    }
    record["prepared"] = json!(true);
    record["status"] = json!("prepared");
    record["complete"] = json!(false);
    write_record(&job.record, record)
}

fn capture(job: &MemberJob, record: &mut Value) -> Result<()> {
    let profile = job
        .run_manifest
        .as_ref()
        .and_then(|run| run["preparation_hash_profile"].as_str())
        .context("hash sweep requires a selected profile")?;
    let bundle = job.bundle.as_ref().context("missing prepared bundle")?;
    let _ = unpack(bundle, &job.work, &[profile])?;
    let original = job.work.join("original.xlsx");
    let variant = job.work.join(format!("{profile}.xlsx"));
    let bundle_sha256 = artifact_cache::hash_file(bundle)?;
    let mut session = crate::regression::office_capture::ResidentCapture::start_screening(
        &original,
        profile,
        &variant,
        &job.work.join("preparation-capture"),
    )?;
    let captured = session.screen_twice(profile);
    let finished = session.finish();
    let mut screen = captured?;
    finished?;
    screen["original_file_sha256"] = json!(artifact_cache::hash_file(&original)?);
    screen["variant_file_sha256"] = json!(artifact_cache::hash_file(&variant)?);
    ensure!(
        artifact_cache::hash_file(bundle)? == bundle_sha256,
        "preparation bundle changed during sweep"
    );
    let cache = json!({"version":1,"bundle_sha256":bundle_sha256,"screen":screen});
    artifact_cache::atomic_write(&cache_path(bundle), &serde_json::to_vec(&cache)?)?;
    record["preparation_hash_screen"] = screen;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_sweep_is_bound_to_bundle_and_profile() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "sweep-cache")?;
        fs::create_dir_all(&root)?;
        let bundle = root.join("prepared.zip");
        fs::write(&bundle, b"original bundle")?;
        let cache = json!({"version":1,"bundle_sha256":artifact_cache::hash_file(&bundle)?,
            "screen":{"profile":"default"}});
        fs::write(cache_path(&bundle), serde_json::to_vec(&cache)?)?;
        assert!(cached_exists(&bundle, Some("default"))?);
        assert!(!cached_exists(&bundle, Some("maximum-cleaning"))?);
        fs::write(&bundle, b"replacement bundle")?;
        assert!(!cached_exists(&bundle, Some("default"))?);
        fs::write(cache_path(&bundle), b"incomplete JSON")?;
        assert!(!cached_exists(&bundle, Some("default"))?);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
