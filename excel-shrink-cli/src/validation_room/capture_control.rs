//! Adjust package admission for the currently observed standalone comparison.
use super::*;
use crate::capture_settings;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    run_id: String,
    geometry_batch_ranges: Option<usize>,
    capture_batch_ranges: Option<usize>,
    capture_batch_megapixels: Option<usize>,
}

pub(super) async fn current(State(room): State<Arc<Room>>) -> ApiResult {
    Ok(Json(current_settings(&room)?))
}

fn current_settings(room: &Room) -> Result<Value> {
    let progress = room
        .progress
        .lock()
        .map_err(|_| anyhow::anyhow!("Progress lock poisoned"))?;
    if !room.active.load(Ordering::Acquire) {
        return Ok(json!({"available":false}));
    }
    let Some(path) = progress.telemetry_path.as_deref() else {
        return Ok(json!({"available":false}));
    };
    let telemetry = read_controls(path)?;
    if telemetry["status"] != "running" {
        return Ok(json!({"available":false}));
    }
    let root = progress
        .report_dir
        .as_deref()
        .context("No active report directory")?;
    let controls = read_controls(&root.join("logs/validation/control.json"))?;
    Ok(json!({"available":true,"run_id":telemetry["run_id"],
        "geometry_batch_ranges":capture_settings::batch_size(&controls["geometry_batch_ranges"],
            capture_settings::DEFAULT_GEOMETRY_RANGES, capture_settings::MAX_GEOMETRY_RANGES)?,
        "capture_batch_ranges":capture_settings::batch_size(&controls["capture_batch_ranges"],
            capture_settings::DEFAULT_CAPTURE_RANGES, capture_settings::MAX_CAPTURE_RANGES)?,
        "capture_batch_megapixels":capture_settings::capture_batch_megapixels(&controls["capture_batch_megapixels"])?}))
}

pub(super) async fn update(
    State(room): State<Arc<Room>>,
    Json(request): Json<Request>,
) -> ApiResult {
    Ok(Json(apply(&room, &request)?))
}

fn apply(room: &Room, request: &Request) -> Result<Value> {
    // Serialize updates with comparison transitions and other control requests.
    let progress = room
        .progress
        .lock()
        .map_err(|_| anyhow::anyhow!("Progress lock poisoned"))?;
    ensure!(
        room.active.load(Ordering::Acquire),
        "No validation run is active"
    );
    let telemetry = progress
        .telemetry_path
        .as_deref()
        .context("The active comparison has no telemetry yet")?;
    let current = read_controls(telemetry)?;
    ensure!(
        current["run_id"] == request.run_id,
        "Comparison identity changed"
    );
    ensure!(
        current["status"] == "running",
        "Comparison is already terminal"
    );
    let root = progress
        .report_dir
        .as_deref()
        .context("No active report directory")?;
    let path = root.join("logs/validation/control.json");
    let controls = merge(read_controls(&path)?, request)?;
    crate::regression::artifact_cache::atomic_write(&path, &serde_json::to_vec(&controls)?)?;
    Ok(json!({"accepted":true,"run_id":request.run_id,
        "scope":"active_report","controls":controls,
        "effective":"next_package_boundary"}))
}

fn read_controls(path: &Path) -> Result<Value> {
    match fs::metadata(path) {
        Ok(metadata) => ensure!(
            metadata.len() <= 4 * 1024 * 1024,
            "Control or telemetry file exceeds 4 MiB"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(error) => return Err(error).context("Inspect capture controls"),
    }
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(value.is_object(), "Capture controls must be an object");
    Ok(value)
}

fn merge(mut controls: Value, request: &Request) -> Result<Value> {
    ensure!(
        request.geometry_batch_ranges.is_some()
            || request.capture_batch_ranges.is_some()
            || request.capture_batch_megapixels.is_some(),
        "At least one batch setting is required"
    );
    for (key, value, default, maximum) in [
        (
            "geometry_batch_ranges",
            request.geometry_batch_ranges,
            capture_settings::DEFAULT_GEOMETRY_RANGES,
            capture_settings::MAX_GEOMETRY_RANGES,
        ),
        (
            "capture_batch_ranges",
            request.capture_batch_ranges,
            capture_settings::DEFAULT_CAPTURE_RANGES,
            capture_settings::MAX_CAPTURE_RANGES,
        ),
    ] {
        if let Some(value) = value {
            capture_settings::batch_size(&json!(value), default, maximum)?;
            controls[key] = json!(value);
        }
    }
    if let Some(value) = request.capture_batch_megapixels {
        capture_settings::capture_batch_megapixels(&json!(value))?;
        controls["capture_batch_megapixels"] = json!(value);
    }
    Ok(controls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn updates_require_the_current_live_comparison() -> Result<()> {
        let root = std::env::temp_dir().join(format!("capture-control-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let telemetry = root.join("telemetry.json");
        fs::write(&telemetry, br#"{"run_id":"current","status":"running"}"#)?;
        let room = Arc::new(Room {
            application: Application::DeltaSieve,
            port: 1234,
            token: "test".into(),
            config: Mutex::new(RunConfig::default()),
            progress: Arc::new(Mutex::new(LiveProgress {
                report_dir: Some(root.clone()),
                telemetry_path: Some(telemetry.clone()),
                ..LiveProgress::default()
            })),
            active: AtomicBool::new(true),
            report: Mutex::new(None),
            reports: Mutex::new(BTreeMap::new()),
            picker: Mutex::new(()),
            study: Mutex::new(None),
            study_defaults: crate::validation_study::Configuration::default(),
            preparation_job: Mutex::new(None),
            corpus_job: Mutex::new(None),
        });
        let request = |run: &str| -> Result<Request> {
            Ok(serde_json::from_value(
                json!({"run_id":run,"capture_batch_ranges":4,
                "capture_batch_megapixels":128}),
            )?)
        };
        assert!(update(State(room.clone()), Json(request("stale")?))
            .await
            .is_err());
        let path = root.join("logs/validation/control.json");
        assert!(!path.exists());
        assert_eq!(current_settings(&room)?["capture_batch_megapixels"], 32);
        let response = update(State(room.clone()), Json(request("current")?))
            .await
            .map_err(|error| error.0)?;
        assert_eq!(response.0["accepted"], true);
        let before = fs::read(&path)?;
        assert_eq!(read_controls(&path)?["capture_batch_megapixels"], 128);
        assert_eq!(current_settings(&room)?["capture_batch_megapixels"], 128);
        fs::write(&telemetry, br#"{"run_id":"current","status":"completed"}"#)?;
        assert!(update(State(room.clone()), Json(request("current")?))
            .await
            .is_err());
        room.active.store(false, Ordering::Release);
        assert_eq!(current_settings(&room)?["available"], false);
        assert!(update(State(room), Json(request("current")?))
            .await
            .is_err());
        assert_eq!(fs::read(&path)?, before);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn independent_updates_preserve_other_controls_and_validate_limits() -> Result<()> {
        let request: Request = serde_json::from_value(json!({"run_id":"pair",
            "capture_batch_megapixels":128}))?;
        let current = json!({"geometry_batch_ranges":256,"capture_batch_ranges":4,"paused":false});
        let changed = merge(current.clone(), &request)?;
        assert_eq!(changed["geometry_batch_ranges"], 256);
        assert_eq!(changed["capture_batch_ranges"], 4);
        assert_eq!(changed["paused"], false);
        assert_eq!(changed["capture_batch_megapixels"], 128);
        for patch in [
            json!({}),
            json!({"capture_batch_ranges":0}),
            json!({"capture_batch_ranges":65}),
            json!({"geometry_batch_ranges":4097}),
            json!({"capture_batch_megapixels":31}),
            json!({"capture_batch_megapixels":257}),
        ] {
            let mut value = patch;
            value["run_id"] = json!("pair");
            let request: Request = serde_json::from_value(value)?;
            assert!(merge(current.clone(), &request).is_err());
        }
        assert!(serde_json::from_value::<Request>(json!({"run_id":"pair","workers":8})).is_err());
        Ok(())
    }
}
