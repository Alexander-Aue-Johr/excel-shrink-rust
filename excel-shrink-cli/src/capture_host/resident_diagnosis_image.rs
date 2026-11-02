//! Binary images for targeted diagnosis commands, owned by an authenticated resident.
use super::*;
use std::io::Cursor;

/// Publish one image for the active capture command; identical retries are safe.
/// The browser cannot choose a local destination or upload for a peer command.
pub(super) async fn upload(
    State(host): State<Arc<Host>>,
    RoutePath((id, command)): RoutePath<(String, String)>,
    request: Request,
) -> JsonResponse {
    safe_id(&id)?;
    safe_id(&command)?;
    let mut client = host.authenticated(&id, request.headers()).await?;
    if client.active_command.as_deref() != Some(&command)
        || client.active_payload["kind"] != "diagnosis"
        || client.active_payload["operation"] != "targeted"
        || !matches!(
            client.active_payload["request"]["kind"].as_str(),
            Some("capture" | "capture_chart")
        )
        || !client.error.is_null()
    {
        return Err(ApiError::conflict(
            "Image does not belong to an active targeted capture.",
        ));
    }
    let bytes = axum::body::to_bytes(request.into_body(), MAX_IMAGE_BYTES)
        .await
        .map_err(ApiError::bad_request)?;
    let reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(ApiError::bad_request)?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
    ) {
        return Err(ApiError::bad_request("Expected a PNG or JPEG capture."));
    }
    let (width, height) = reader.into_dimensions().map_err(ApiError::bad_request)?;
    if width == 0 || height == 0 {
        return Err(ApiError::bad_request("Empty capture."));
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let path = host
        .session
        .join("diagnosis-images")
        .join(&id)
        .join(format!("{command}.image"));
    if path.try_exists()? {
        let previous = tokio::fs::read(&path).await?;
        if previous.as_slice() != bytes.as_ref() {
            return Err(ApiError::conflict(
                "Image retry differs from the published capture.",
            ));
        }
    } else {
        write_bytes(&path, &bytes, false).await?;
    }
    client.last_activity_at = now_iso();
    Ok(Json(
        json!({"image_path":path,"sha256":sha256,"image_bytes":bytes.len(),"width":width,"height":height}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn authenticates_binds_validates_and_retries_binary_images() -> anyhow::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("diagnosis-image-{}", uuid::Uuid::new_v4()));
        let mut client = Client::new();
        client.token = "worker-token".into();
        client.active_command = Some("capture-1".into());
        client.active_payload =
            json!({"kind":"diagnosis","operation":"targeted","request":{"kind":"capture"}});
        let host = Arc::new(Host {
            session: directory.clone(),
            launcher_token: None,
            control_root: None,
            primary_clients: vec!["one".into()],
            clients: RwLock::new(HashMap::from([(
                "one".into(),
                Arc::new(Mutex::new(client)),
            )])),
            dispatch: Mutex::new(Dispatch {
                resetting: HashSet::new(),
                paused: false,
            }),
            events: events::Changes::new(),
            coordinator_token: None,
        });
        let mut bytes = Cursor::new(Vec::new());
        image::RgbImage::new(2, 3).write_to(&mut bytes, image::ImageFormat::Png)?;
        let bytes = bytes.into_inner();
        let request = |token: &str, bytes: Vec<u8>| {
            Request::builder()
                .header("X-Client-Token", token)
                .body(axum::body::Body::from(bytes))
                .unwrap()
        };
        let args = || RoutePath(("one".into(), "capture-1".into()));
        assert!(
            upload(State(host.clone()), args(), request("wrong", bytes.clone()))
                .await
                .is_err()
        );
        assert!(upload(
            State(host.clone()),
            RoutePath(("one".into(), "other".into())),
            request("worker-token", bytes.clone())
        )
        .await
        .is_err());
        assert!(upload(
            State(host.clone()),
            args(),
            request("worker-token", b"invalid".to_vec())
        )
        .await
        .is_err());
        let first = upload(
            State(host.clone()),
            args(),
            request("worker-token", bytes.clone()),
        )
        .await
        .unwrap()
        .0;
        let second = upload(
            State(host.clone()),
            args(),
            request("worker-token", bytes.clone()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(first, second);
        assert_eq!(
            tokio::fs::read(first["image_path"].as_str().unwrap()).await?,
            bytes
        );
        let mut changed = bytes.clone();
        changed.extend_from_slice(b"different encoding");
        assert!(
            upload(State(host), args(), request("worker-token", changed))
                .await
                .is_err()
        );
        tokio::fs::remove_dir_all(directory).await?;
        Ok(())
    }
}
