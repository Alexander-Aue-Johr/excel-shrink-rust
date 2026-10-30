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
    // Full historical sweeps remain valid. Sampled sweeps must match the
    // current selection settings, otherwise a larger requested sample is lost.
    let controls = bundle.parent().and_then(Path::parent)
        .and_then(|root| fs::read(root.join("control-room/control.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null);
    for sheet in value["screen"]["sheets"].as_array().into_iter().flatten() {
        let coverage = &sheet["capture_coverage"];
        if coverage["omitted_tiles"].as_u64().unwrap_or(0) == 0 {
            continue;
        }
        for (control, field, env, default) in [
            ("tall_sheet_threshold", "threshold", "EXCEL_SHRINK_TALL_SHEET_THRESHOLD", 40),
            ("tall_sheet_edge_tiles", "edge_tiles", "EXCEL_SHRINK_TALL_SHEET_EDGE_TILES", 2),
            ("crc_equal_sheet_threshold_mib", "crc_equal_threshold_mib", "EXCEL_SHRINK_CRC_EQUAL_SHEET_THRESHOLD_MIB", 256),
        ] {
            let requested = controls[control].as_u64()
                .or_else(|| std::env::var(env).ok()?.parse::<u64>().ok())
                .unwrap_or(default);
            if coverage[field].as_u64() != Some(requested) {
                return Ok(None);
            }
        }
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
    let directory = job.work.join("preparation-capture");
    fs::create_dir_all(&directory)?;
    artifact_cache::atomic_write(
        &directory.join("recalc-inputs.json"),
        &serde_json::to_vec(&json!({"variants":{},"shrink_identity":record["shrink_identity"]}))?,
    )?;
    let mut session = crate::regression::office_capture::ResidentCapture::start_screening(
        &original,
        profile,
        &variant,
        &directory,
    )?;
    let captured = session.screen_twice(profile);
    let finished = session.finish();
    let mut screen = captured?;
    let metrics = finished?;
    let mut startup = serde_json::Map::new();
    for logical in ["original", profile] {
        let physical = metrics["capture_client_aliases"][logical]
            .as_str().unwrap_or(logical);
        let client = &metrics["clients"][physical];
        startup.insert(logical.to_owned(), json!({
            "method":client["startup_measurement_method"],
            "seconds":client["startup_seconds"],
        }));
    }
    screen["diagnostic_timings"]["startup"] = Value::Object(startup);
    screen["diagnostic_timings"]["coordinator"] = metrics["pipeline_timings"].clone();
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
    fn sampled_cache_is_invalidated_when_selection_settings_change() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "sampled-sweep-cache")?;
        fs::create_dir_all(root.join("prepared"))?;
        fs::create_dir_all(root.join("control-room"))?;
        let bundle = root.join("prepared/bundle.zip");
        fs::write(&bundle, b"bundle")?;
        let cache = json!({"bundle_sha256":artifact_cache::hash_file(&bundle)?,
            "screen":{"profile":"default","sheets":[{"capture_coverage":{
                "omitted_tiles":100,"threshold":40,"edge_tiles":2,"crc_equal_threshold_mib":256
            }}]}});
        fs::write(cache_path(&bundle), serde_json::to_vec(&cache)?)?;
        let controls = root.join("control-room/control.json");
        fs::write(&controls, br#"{"tall_sheet_threshold":40,"tall_sheet_edge_tiles":2,"crc_equal_sheet_threshold_mib":256}"#)?;
        assert!(cached_exists(&bundle, Some("default"))?);
        fs::write(&controls, br#"{"tall_sheet_threshold":40,"tall_sheet_edge_tiles":3,"crc_equal_sheet_threshold_mib":256}"#)?;
        assert!(!cached_exists(&bundle, Some("default"))?);
        fs::remove_dir_all(root)?;
        Ok(())
    }

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
