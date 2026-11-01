//! Explicit, single-sheet inspections do not mutate the corpus queue or verdict.
use super::*;

/// Start at most one additional inspection session to bound its Excel footprint.
pub(super) async fn start(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
    payload: Result<Json<Value>, JsonRejection>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    let body = object_payload(payload)?;
    if body.as_object().unwrap().keys().any(|key| key != "sheet") {
        return Err(ApiError::bad_request("Unknown sheet capture option"));
    }
    let sheet = body["sheet"]
        .as_u64()
        .filter(|index| *index > 0)
        .ok_or_else(|| ApiError::bad_request("A one-based sheet index is required"))?;
    let mut active = room.manual_capture.lock().await;
    if let Some(child) = active.as_mut() {
        if child.try_wait()?.is_none() {
            return Err(ApiError::conflict(
                "Another manual sheet capture is still running",
            ));
        }
    }
    let request = format!("manual-{}", uuid::Uuid::new_v4().simple());
    let directory = confined_path(&room.root, &format!("manual-captures/{request}"))?;
    let input_directory = directory.join("inputs");
    let copy_root = room.root.clone();
    let workbook_id = id.clone();
    let (original, variant) = tokio::task::spawn_blocking(move || {
        inspection::copy_comparison_pair(&copy_root, &workbook_id, &input_directory)
    })
    .await
    .context("Copy manual inspection inputs")??;
    let mut command = std::process::Command::new(std::env::current_exe()?);
    let controls = read_json(&room.root.join("control.json")).await?;
    command
        .args(["regression", "capture-sheet", "--original"])
        .arg(original)
        .arg("--variant")
        .arg(variant)
        .arg("--sheet")
        .arg(sheet.to_string())
        .arg("--output")
        .arg(directory.join("capture"))
        .env("EXCEL_SHRINK_CONTROL_ROOM_DIR", &room.root)
        .env("EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID", &request)
        .env(
            "EXCEL_SHRINK_HIDE_EXCEL",
            if controls["hide_excel"] == true {
                "1"
            } else {
                "0"
            },
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::fs::File::create(directory.join("stdout.log"))?)
        .stderr(std::fs::File::create(directory.join("stderr.log"))?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let child = command.spawn().context("Start manual sheet capture")?;
    let metadata = json!({"id":request,"parent_group":id,"sheet":sheet,"pid":child.id(),"started_at":now_ms()});
    *active = Some(child);
    write_json(&directory.join("request.json"), &metadata, true).await?;
    let index_path = confined_path(&room.root, &format!("groups/{id}/manual-captures.json"))?;
    let mut index = read_json(&index_path).await.unwrap_or(json!([]));
    index
        .as_array_mut()
        .context("Invalid manual capture index")?
        .push(metadata);
    write_json(&index_path, &index, true).await?;
    Ok(Json(json!({"ok":true,"id":request})))
}

/// Serve a separate inspection snapshot and its terminal result.
pub(super) async fn state(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    let directory = confined_path(&room.root, &format!("manual-captures/{id}"))?;
    let request = read_json(&directory.join("request.json")).await?;
    let mut snapshot = read_json(&room.root.join("groups").join(&id).join("state.json"))
        .await
        .unwrap_or(json!({"id":id,"sheets":[],"status":"opening"}));
    snapshot["manual_inspection"] = json!(true);
    snapshot["id"] = json!(id);
    snapshot["workbook"] = json!(format!("Manual inspection · sheet {}", request["sheet"]));
    snapshot["parent_group"] = request["parent_group"].clone();
    if let Ok(mut launcher) =
        read_json(&room.root.join("groups").join(&id).join("launcher.json")).await
    {
        check_clients(&mut launcher);
        snapshot["launcher"] = launcher;
    }
    if let Ok(result) = read_json(&directory.join("capture/inspection-result.json")).await {
        snapshot["status"] = result["status"].clone();
        snapshot["inspection_result"] = result;
    }
    Ok(Json(snapshot))
}
