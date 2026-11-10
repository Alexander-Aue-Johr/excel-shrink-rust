//! Read-only access to bounded, persisted capture receipts from the current run.
use super::*;

pub(super) async fn enrich_heartbeats(value: &mut Value) {
    let Some(root) = value["report_directory"].as_str().map(PathBuf::from) else {
        return;
    };
    let Some(lanes) = value["lanes"].as_array_mut() else {
        return;
    };
    for lane in lanes {
        let directory = match lane["id"].as_str() {
            Some("O" | "OR") => "resident-O",
            Some("SD" | "SDR") => "resident-SD",
            Some("CP" | "CPR") => "resident-CP",
            _ => continue,
        };
        // Metrics can be absent during startup or after publication. Only a
        // verified client PID and creation time may contribute a heartbeat.
        if let Ok(metrics) = read_small_json(
            &root.join(directory).join("session/metrics.json"),
            4 * 1024 * 1024,
        )
        .await
        {
            crate::capture_host::control_room::apply_addin_sample(lane, &metrics["clients"]["one"]);
        }
    }
}

pub(super) async fn asset(RoutePath(path): RoutePath<String>) -> Response {
    match crate::validation_report::live_asset(&path) {
        Some((mime, body)) => ([(header::CONTENT_TYPE, mime)], body).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn snapshot(room: &Room, run: &str) -> Result<Value> {
    let (path, previous) = {
        let progress = room
            .progress
            .lock()
            .map_err(|_| anyhow::anyhow!("Progress lock poisoned"))?;
        (
            progress.telemetry_path.clone(),
            progress
                .comparisons
                .iter()
                .find(|value| value["run_id"] == run)
                .cloned(),
        )
    };
    if let Some(path) = path.filter(|path| path.is_file()) {
        let value = read_small_json(&path, 4 * 1024 * 1024).await?;
        if value["run_id"] == run {
            return Ok(value);
        }
    }
    previous.context("This live comparison is no longer in the recent inspection window")
}

pub(super) async fn catalog(
    State(room): State<Arc<Room>>,
    RoutePath(run): RoutePath<String>,
) -> ApiResult {
    let mut value = snapshot(&room, &run).await?;
    if let Some(lanes) = value["lanes"].as_array_mut() {
        for lane in lanes {
            if let Some(captures) = lane["recent_captures"].as_array_mut() {
                for capture in captures {
                    let id = capture["id"]
                        .as_str()
                        .context("Missing capture receipt ID")?;
                    capture["url"] = json!(format!("/api/live-atlas/{run}/images/{id}"));
                    capture.as_object_mut().unwrap().remove("image");
                }
            }
        }
    }
    let lanes: Vec<_> = value["lanes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|lane| json!({"id":lane["id"],"recent_captures":lane["recent_captures"]}))
        .collect();
    Ok(Json(
        json!({"run_id":value["run_id"],"workbook":value["workbook"],
        "status":value["status"],"revision":value["revision"],"lanes":lanes}),
    ))
}

pub(super) async fn image(
    State(room): State<Arc<Room>>,
    RoutePath((run, id)): RoutePath<(String, String)>,
) -> Response {
    let result = async {
        let value = snapshot(&room, &run).await?;
        let capture = value["lanes"]
            .as_array()
            .context("Missing capture lanes")?
            .iter()
            .flat_map(|lane| lane["recent_captures"].as_array().into_iter().flatten())
            .find(|capture| capture["id"] == id)
            .context("Capture receipt has expired")?;
        let recorded = PathBuf::from(capture["image"].as_str().context("Missing capture image")?);
        let raw_root = PathBuf::from(
            value["report_directory"]
                .as_str()
                .context("Missing capture directory")?,
        );
        let report = room
            .config
            .lock()
            .map_err(|_| anyhow::anyhow!("Configuration lock poisoned"))?
            .report_dir
            .clone()
            .canonicalize()?;
        let path = if recorded.is_file() {
            let root = raw_root.canonicalize()?;
            let path = recorded.canonicalize()?;
            ensure!(
                root.starts_with(&report) && path.starts_with(&root),
                "Capture escapes the active report directory"
            );
            path
        } else {
            let pair_id = raw_root.file_name().context("Missing live pair identity")?;
            let pair_root = report.join("pairs").join(pair_id).canonicalize()?;
            ensure!(
                pair_root.starts_with(&report),
                "Published capture escapes the report"
            );
            let published =
                read_small_json(&pair_root.join("report.json"), 128 * 1024 * 1024).await?;
            let receipt = published["capture_receipts"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|receipt| receipt["id"] == id)
                .context("Capture has no retained image")?;
            let path = pair_root
                .join(
                    receipt["image"]
                        .as_str()
                        .context("Missing retained capture path")?,
                )
                .canonicalize()?;
            ensure!(
                path.starts_with(&pair_root),
                "Retained image escapes its package"
            );
            path
        };
        let metadata = fs::metadata(&path)?;
        ensure!(
            metadata.len() <= 64 * 1024 * 1024,
            "Capture exceeds the live image byte limit"
        );
        let bytes = tokio::fs::read(path).await?;
        let format = image::guess_format(&bytes)?;
        let mime = match format {
            image::ImageFormat::Png => "image/png",
            image::ImageFormat::Jpeg => "image/jpeg",
            _ => anyhow::bail!("Unsupported live capture encoding"),
        };
        Ok::<_, anyhow::Error>(
            (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                bytes,
            )
                .into_response(),
        )
    }
    .await;
    result.unwrap_or_else(|error| ApiError(error).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(root: &Path, capture: &Path) -> Arc<Room> {
        let mut config = RunConfig::default();
        config.report_dir = root.into();
        let mut progress = LiveProgress::default();
        progress
            .comparisons
            .push_back(json!({"run_id":"live-test","report_directory":root,
            "lanes":[{"recent_captures":[{"id":"receipt","image":capture}]}]}));
        Arc::new(Room {
            application: Application::DeltaSieve,
            port: 1234,
            token: "test".into(),
            config: Mutex::new(config),
            progress: Arc::new(Mutex::new(progress)),
            active: AtomicBool::new(false),
            report: Mutex::new(None),
            reports: Mutex::new(BTreeMap::new()),
            picker: Mutex::new(()),
            study: Mutex::new(None),
            study_defaults: crate::validation_study::Configuration::default(),
            preparation_job: Mutex::new(None),
            corpus_job: Mutex::new(None),
        })
    }

    #[tokio::test]
    async fn live_receipts_hide_paths_and_only_serve_confined_images() -> Result<()> {
        let parent = std::env::temp_dir().join(format!("live-atlas-{}", uuid::Uuid::new_v4()));
        let root = parent.join("report");
        fs::create_dir_all(&root)?;
        let path = root.join("capture.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])).save(&path)?;
        let allowed = room(&root, &path);
        let catalog = catalog(State(allowed.clone()), RoutePath("live-test".into()))
            .await
            .unwrap()
            .0;
        assert!(catalog["report_directory"].is_null());
        assert!(catalog["lanes"][0]["recent_captures"][0]["image"].is_null());
        assert_eq!(
            image(
                State(allowed),
                RoutePath(("live-test".into(), "receipt".into()))
            )
            .await
            .status(),
            StatusCode::OK
        );
        let outside = parent.join("outside.png");
        fs::copy(&path, &outside)?;
        assert_eq!(
            image(
                State(room(&root, &outside)),
                RoutePath(("live-test".into(), "receipt".into()))
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let published = root.join("pairs/report");
        fs::create_dir_all(published.join("evidence"))?;
        fs::copy(&path, published.join("evidence/capture.png"))?;
        fs::write(
            published.join("report.json"),
            serde_json::to_vec(&json!({
                "capture_receipts":[{"id":"receipt","image":"evidence/capture.png"}]
            }))?,
        )?;
        fs::remove_file(&path)?;
        assert_eq!(
            image(
                State(room(&root, &path)),
                RoutePath(("live-test".into(), "receipt".into()))
            )
            .await
            .status(),
            StatusCode::OK
        );
        fs::remove_dir_all(parent)?;
        Ok(())
    }
}
