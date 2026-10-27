//! Browser-facing control plane. The scheduler owns scientific work; this host
//! reads its snapshots and writes explicit, attempt-scoped control requests.

mod activity;
mod inspection;
mod platform;
mod snapshots;

use super::{now_iso, now_ms, read_json, serve, with_web, write_json, ApiError, HttpResult};
use anyhow::{ensure, Context, Result};
use axum::{
    body::Body,
    extract::{rejection::JsonRejection, Path as RoutePath, Query, Request, State},
    http::{HeaderMap, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use snapshots::*;
use std::{collections::HashMap, path::Path, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tower_http::services::ServeFile;

struct Room {
    root: PathBuf,
    port: u16,
    version: String,
    started: String,
    initial_controls: Value,
    control_gate: Mutex<()>,
}

pub(super) async fn run(settings_path: &Path) -> Result<()> {
    let settings = read_json(settings_path).await?;
    let root = std::path::absolute(required_string(&settings, "root")?)?;
    let web = std::path::absolute(required_string(&settings, "web_root")?)?;
    let port = settings["port"]
        .as_u64()
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port >= 1024)
        .context("Invalid control room port.")?;
    ensure!(web.is_dir(), "Invalid control room web root.");
    tokio::fs::create_dir_all(root.join("groups")).await?;
    tokio::fs::create_dir_all(confined_path(&root, "commands")?).await?;
    let mut initial = defaults();
    if settings["initial_controls"].is_object() {
        initial = merge_controls(&initial, &settings["initial_controls"])?;
    }
    let control_path = root.join("control.json");
    if control_path.exists() {
        initial = merge_controls(
            &defaults(),
            &read_object(&control_path, SNAPSHOT_LIMIT).await?,
        )?;
    } else {
        write_json(&control_path, &initial, true).await?;
    }
    let room = Arc::new(Room {
        root,
        port,
        version: settings["host_version"]
            .as_str()
            .unwrap_or("unspecified")
            .to_owned(),
        started: now_iso(),
        initial_controls: initial,
        control_gate: Mutex::new(()),
    });
    let routes = Router::new()
        .route("/health", get(health))
        .route("/api/state", get(state))
        .route("/api/workers", get(workers))
        .route("/api/groups/{id}", get(group))
        .route("/api/run/stop", post(stop))
        .route("/api/control", post(control))
        .route("/api/groups/{id}/restart", post(restart))
        .route("/api/groups/{id}/inspect", post(inspect))
        .route("/api/commands/{id}", get(command))
        .route("/data/{*path}", get(asset))
        .with_state(room.clone());
    let app = with_web(routes, &web, 16 * 1024)
        .layer(middleware::from_fn_with_state(room, enforce_local_request));
    serve(app, port).await
}

async fn enforce_local_request(
    State(room): State<Arc<Room>>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = if local_request(request.headers(), room.port) {
        next.run(request).await
    } else {
        ApiError::forbidden("Only the local control room origin is allowed.").into_response()
    };
    response
        .headers_mut()
        .insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

fn local_request(headers: &HeaderMap, port: u16) -> bool {
    let Some(host) = headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<axum::http::uri::Authority>().ok())
    else {
        return false;
    };
    let host_name = host.host().trim_matches(['[', ']']);
    if host.as_str().contains('@')
        || !matches!(
            host_name.to_ascii_lowercase().as_str(),
            "localhost" | "127.0.0.1" | "::1"
        )
        || host.port_u16() != Some(port)
    {
        return false;
    }
    if let Some(origin) = headers.get("origin") {
        let Ok(origin) = origin.to_str() else {
            return false;
        };
        if !origin.is_empty() {
            let Ok(uri) = origin.parse::<Uri>() else {
                return false;
            };
            if uri.scheme_str() != Some("http")
                || uri.port_u16() != Some(port)
                || !uri.host().is_some_and(|name| {
                    name.trim_matches(['[', ']'])
                        .eq_ignore_ascii_case(host_name)
                })
                || uri
                    .authority()
                    .is_some_and(|authority| authority.as_str().contains('@'))
                || !matches!(uri.path(), "" | "/")
                || uri.query().is_some()
                || origin.contains('#')
            {
                return false;
            }
        }
    }
    headers
        .get("sec-fetch-site")
        .is_none_or(|value| value != "cross-site")
}

async fn health(State(room): State<Arc<Room>>) -> Json<Value> {
    Json(
        json!({"ok": true, "kind": "excel-shrink-control-room", "root": room.root,
        "port": room.port, "host_version": room.version, "started_at": room.started, "live": true}),
    )
}

async fn read_queue(room: &Room, diagnostics: &mut Vec<Value>) -> Result<Value> {
    let mut queue = read_snapshot(&room.root.join("queue.json"), diagnostics, SNAPSHOT_LIMIT)
        .await
        .unwrap_or(json!({}));
    check_runner(&mut queue, diagnostics);
    reconcile_groups(&room.root, &mut queue, diagnostics).await?;
    Ok(queue)
}

async fn state(State(room): State<Arc<Room>>) -> HttpResult<Json<Value>> {
    let mut diagnostics = Vec::new();
    let mut queue = read_queue(&room, &mut diagnostics).await?;
    queue["update_ready"] =
        json!(queue["runner_alive"] == false && platform::output_idle(&room.root));
    if queue["groups"].is_null() {
        queue["groups"] = json!([]);
    }
    queue["controls"] = read_controls(&room, &mut diagnostics).await;
    let stop = read_snapshot(
        &room.root.join("stop-request.json"),
        &mut diagnostics,
        SNAPSHOT_LIMIT,
    )
    .await;
    queue["stop_requested"] = json!(active_stop_requested(&queue, stop.as_ref()));
    queue.as_object_mut().unwrap().remove("progress");
    if let Some(progress) = read_snapshot(
        &room.root.join("progress.json"),
        &mut diagnostics,
        64 * 1024,
    )
    .await
    {
        if !queue["run_id"].is_null() && progress["run_id"] == queue["run_id"] {
            queue["progress"] = progress;
        }
    }
    activity::augment_progress(&mut queue, &room.root).await?;
    queue["memory"] = platform::memory_snapshot();
    queue["server"] =
        json!({"started_at": room.started, "utc": now_iso(), "port": room.port, "live": true});
    queue["diagnostics"] = json!(diagnostics);
    Ok(Json(queue))
}

async fn workers(
    State(room): State<Arc<Room>>,
    Query(query): Query<HashMap<String, String>>,
) -> HttpResult<Json<Value>> {
    let offset = query
        .get("offset")
        .filter(|value| !value.is_empty())
        .map(|value| value.parse::<i32>())
        .transpose()
        .map_err(|_| ApiError::bad_request("Worker page offset must be a nonnegative integer."))?
        .unwrap_or(0);
    let offset = usize::try_from(offset)
        .map_err(|_| ApiError::bad_request("Worker page offset must be a nonnegative integer."))?;
    let mut diagnostics = Vec::new();
    let queue = read_queue(&room, &mut diagnostics).await?;
    let all = queue["groups"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let mut selected: Vec<_> = all.iter().filter(|group| group["active"] == true).collect();
    if query
        .get("include_recent")
        .is_some_and(|value| value == "true")
    {
        let mut recent: Vec<_> = all
            .iter()
            .filter(|group| {
                group["active"] != true
                    && group["run_id"] == queue["run_id"]
                    && matches!(
                        group["status"].as_str(),
                        Some(
                            "unchanged"
                                | "differences"
                                | "skipped"
                                | "failed"
                                | "interrupted"
                                | "complete"
                                | "completed"
                        )
                    )
            })
            .collect();
        recent
            .sort_by_key(|group| std::cmp::Reverse(group["started_unix_ms"].as_i64().unwrap_or(0)));
        selected.extend(recent.into_iter().take(8));
    }
    const PAGE_SIZE: usize = 64;
    let mut groups = Vec::new();
    for summary in selected.iter().skip(offset).take(PAGE_SIZE) {
        groups.push(worker_group(&room.root, summary, &mut diagnostics).await?);
    }
    Ok(Json(
        json!({"groups": groups, "offset": offset, "total_groups": selected.len(),
        "next_offset": offset.checked_add(PAGE_SIZE).filter(|next| *next < selected.len()),
        "active_groups": all.iter().filter(|group| group["active"] == true).count(),
        "running": queue["running"], "runner_alive": queue["runner_alive"],
        "sampled_at": now_iso(), "diagnostics": diagnostics}),
    ))
}

async fn group(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    let mut diagnostics = Vec::new();
    let queue = read_queue(&room, &mut diagnostics).await?;
    let summary = queue["groups"]
        .as_array()
        .and_then(|groups| groups.iter().find(|group| group["id"] == id))
        .cloned();
    let scheduled_status = summary
        .as_ref()
        .and_then(|group| group["status"].as_str())
        .map(str::to_owned);
    let scheduled_attempt = summary
        .as_ref()
        .and_then(|group| group["attempt_id"].as_str())
        .map(str::to_owned);
    let directory = confined_path(&room.root, &format!("groups/{id}"))?;
    let mut detail = read_snapshot(
        &directory.join("state.json"),
        &mut diagnostics,
        SNAPSHOT_LIMIT,
    )
    .await;
    if detail.is_none() {
        detail = read_snapshot(
            &directory.join("session.json"),
            &mut diagnostics,
            SNAPSHOT_LIMIT,
        )
        .await;
    }
    if summary.is_some()
        && scheduled_attempt.as_deref()
            != detail
                .as_ref()
                .and_then(|detail| detail["attempt_id"].as_str())
    {
        detail = None;
    }
    if summary.is_none() && detail.is_none() {
        return Err(ApiError::not_found("Unknown comparison group."));
    }
    let available = detail.is_some();
    let capture_status = detail
        .as_ref()
        .map(|detail| detail["status"].clone())
        .unwrap_or(Value::Null);
    let mut merged = summary.unwrap_or(json!({}));
    if let Some(Value::Object(detail)) = detail {
        if detail
            .get("id")
            .is_some_and(|value| !value.is_null() && value != &id)
        {
            return Err(ApiError::bad_request(
                "Group snapshot identity differs from its directory.",
            ));
        }
        merged.as_object_mut().unwrap().extend(detail);
    }
    if let Some(attempt) = &scheduled_attempt {
        merged["attempt_id"] = json!(attempt);
    }
    if scheduled_status
        .as_deref()
        .is_some_and(scheduler_status_wins)
    {
        merged["capture_status"] = capture_status;
        merged["status"] = json!(scheduled_status);
    }
    let displayed_attempt =
        scheduled_attempt.or_else(|| merged["attempt_id"].as_str().map(str::to_owned));
    let launcher_name = if merged["capture_mode"] == "classic" {
        "classic.json"
    } else {
        "launcher.json"
    };
    let launcher = read_snapshot(
        &directory.join(launcher_name),
        &mut diagnostics,
        SNAPSHOT_LIMIT,
    )
    .await;
    if displayed_attempt.is_some()
        && displayed_attempt.as_deref()
            != launcher
                .as_ref()
                .and_then(|launcher| launcher["attempt_id"].as_str())
    {
        merged["launcher_pending"] = json!(true);
    } else if let Some(mut launcher) = launcher {
        check_clients(&mut launcher);
        merged["launcher"] = launcher;
    }
    merged["restart_commands"] =
        json!(recent_commands(&room.root.join("commands"), &id, &mut diagnostics).await?);
    merged["client_reset_commands"] =
        json!(recent_commands(&directory.join("client-resets"), &id, &mut diagnostics).await?);
    let record_path = room
        .root
        .parent()
        .context("Missing study directory.")?
        .join("records")
        .join(format!("{id}.json"));
    let record = read_snapshot(&record_path, &mut diagnostics, SNAPSHOT_LIMIT)
        .await
        .unwrap_or(json!({}));
    if record["cleanup_pending"].is_object() {
        merged["cleanup_pending"] = record["cleanup_pending"].clone();
    }
    attach_outcome(&mut merged, &record, displayed_attempt.as_deref());
    let inspection_root = room.root.clone();
    let inspection_id = id.clone();
    let profiles = tokio::task::spawn_blocking(move || {
        inspection::available_profiles(&inspection_root, &inspection_id)
    })
    .await
    .context("Read inspection variants")?;
    merged["inspection_profiles"] = match profiles {
        Ok(profiles) => json!(profiles),
        Err(error) => {
            diagnostics.push(json!({"file":"prepared workbook bundle","error":error.to_string()}));
            json!([])
        }
    };
    merged["measurements"] = measurements(&record, &merged, displayed_attempt.as_deref());
    merged["id"] = json!(id);
    merged["available"] = json!(available);
    merged["diagnostics"] = json!(diagnostics);
    Ok(Json(merged))
}

fn attach_outcome(group: &mut Value, record: &Value, attempt: Option<&str>) {
    // A live record can still hold the previous attempt's result while a retry
    // is starting. Only attach a result belonging to the displayed attempt.
    if record["control_room_attempt"]["id"].as_str() != attempt
        || !matches!(
            group["status"].as_str(),
            Some("skipped" | "failed" | "unchanged" | "differences")
        )
        || group["status"] != record["status"]
    {
        return;
    }
    group["outcome_error"] = record["error"].clone();
    group["error"] = record["error"].clone();
    if record["capture_mode"] == "package-audit" {
        group["outcome_method"] = json!("package-equivalence");
    }
}

async fn inspect(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
    payload: Result<Json<Value>, JsonRejection>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    let body = object_payload(payload)?;
    if body.as_object().unwrap().keys().any(|key| key != "profile") {
        return Err(ApiError::bad_request("Unknown inspection option."));
    }
    let profile = required_string(&body, "profile")?.to_owned();
    if !inspection::PROFILES.contains(&profile.as_str()) {
        return Err(ApiError::bad_request("Unknown inspection profile."));
    }
    let queue = read_queue(&room, &mut Vec::new()).await?;
    if !queue["groups"]
        .as_array()
        .is_some_and(|groups| groups.iter().any(|group| group["id"] == id))
    {
        return Err(ApiError::not_found("Unknown comparison group."));
    }
    let root = room.root.clone();
    let requested_profile = profile.clone();
    let path =
        tokio::task::spawn_blocking(move || inspection::open_copy(&root, &id, &requested_profile))
            .await
            .context("Open inspection copy")??;
    Ok(Json(
        json!({"ok":true,"status":"launch_requested","profile":profile,"path":path}),
    ))
}

fn scheduler_status_wins(status: &str) -> bool {
    matches!(
        status,
        "queued"
            | "preparing"
            | "restart_pending"
            | "complete"
            | "completed"
            | "failed"
            | "skipped"
            | "unchanged"
            | "differences"
            | "waiting_cleanup"
            | "cleanup_blocked"
            | "restart_waiting_cleanup"
            | "interrupted"
            | "retrying_classic"
    )
}

fn measurements(record: &Value, group: &Value, attempt: Option<&str>) -> Value {
    let mut measurements = json!({});
    for profile in record["results"].as_array().into_iter().flatten() {
        let Some(name) = profile["profile"].as_str() else {
            continue;
        };
        measurements[name] = json!({"bytes": profile["shrunk_bytes"], "open_seconds": profile["shrunk_open_seconds"],
            "memory_bytes": profile["shrunk_memory_bytes"], "shrink_seconds": profile["shrink_seconds"]});
        if measurements["original"].is_null() {
            measurements["original"] = json!({"bytes": profile["original_bytes"], "open_seconds": profile["original_open_seconds"],
                "memory_bytes": profile["original_memory_bytes"]});
        }
        for field in ["open_seconds", "memory_bytes"] {
            if measurements["original"][field].is_null() {
                measurements["original"][field] = profile[format!("original_{field}")].clone();
            }
        }
        let native = &profile["native_startup"];
        if native["method"] == "launch-to-addin-ready-v1"
            && (attempt.is_none() || native["attempt_id"].as_str() == attempt)
        {
            measurements[name]["startup"] =
                native_startup(&native["shrunk"], native["method"].as_str());
            if measurements["original"]["startup"].is_null() {
                measurements["original"]["startup"] =
                    native_startup(&native["original"], native["method"].as_str());
            }
        }
    }
    // Historical COM fields remain distinct from attempt-bound native startup.
    if let Some(clients) = group["launcher"]["clients"].as_object() {
        for (name, measurement) in measurements.as_object_mut().unwrap() {
            let physical = group["aliases"][name].as_str().unwrap_or(name);
            let Some(client) = clients.get(physical) else {
                continue;
            };
            let sample = json!({"seconds": client["startup_seconds"], "private_bytes": client["startup_memory_private_bytes"],
                "working_set_bytes": client["startup_memory_working_set_bytes"], "sampled_unix_ms": client["startup_sampled_unix_ms"],
                "ready_at": client["ready_at"], "pid": client["pid"], "process_started_unix_ms": client["process_started_unix_ms"], "client_id": physical});
            measurement["startup"] =
                native_startup(&sample, client["startup_measurement_method"].as_str());
        }
    }
    measurements
}

fn native_startup(sample: &Value, method: Option<&str>) -> Value {
    if method != Some("launch-to-addin-ready-v1")
        || !sample.is_object()
        || [
            "seconds",
            "sampled_unix_ms",
            "ready_at",
            "pid",
            "process_started_unix_ms",
        ]
        .iter()
        .any(|field| sample[field].is_null())
    {
        return Value::Null;
    }
    let mut result = sample.clone();
    result["method"] = json!(method);
    result
}

async fn stop(State(room): State<Arc<Room>>) -> HttpResult<Json<Value>> {
    let _guard = room.control_gate.lock().await;
    let mut queue = read_object(&room.root.join("queue.json"), SNAPSHOT_LIMIT).await?;
    check_runner(&mut queue, &mut Vec::new());
    if queue["running"] != true || queue["runner_alive"] != true {
        return Err(ApiError::conflict(
            "There is no verified active runner to stop.",
        ));
    }
    if queue["stop_supported"] != true {
        return Err(ApiError::conflict(
            "This runner predates orderly stopping. Stop it from the updated corpus configurator.",
        ));
    }
    write_json(
        &room.root.join("stop-request.json"),
        &json!({"runner_pid": queue["runner_pid"],
        "runner_started_unix_ms": queue["runner_started_unix_ms"]}),
        true,
    )
    .await?;
    let path = room.root.join("control.json");
    let current = merge_controls(&defaults(), &read_object(&path, SNAPSHOT_LIMIT).await?)?;
    write_json(
        &path,
        &merge_controls(&current, &json!({"target_groups": 0, "paused": false}))?,
        true,
    )
    .await?;
    Ok(Json(
        json!({"ok": true, "message": "Stopping after active workbooks and Excel cleanup finish."}),
    ))
}

async fn control(
    State(room): State<Arc<Room>>,
    payload: Result<Json<Value>, JsonRejection>,
) -> HttpResult<Json<Value>> {
    let patch = object_payload(payload)?;
    let _guard = room.control_gate.lock().await;
    let path = room.root.join("control.json");
    let current = merge_controls(&defaults(), &read_object(&path, SNAPSHOT_LIMIT).await?)?;
    let mut next = merge_controls(&current, &patch)?;
    let stop_path = room.root.join("stop-request.json");
    let queue_path = room.root.join("queue.json");
    let mut queue = if stop_path.exists() && queue_path.exists() {
        read_object(&queue_path, SNAPSHOT_LIMIT).await?
    } else {
        json!({})
    };
    let stop = if stop_path.exists() {
        Some(read_object(&stop_path, SNAPSHOT_LIMIT).await?)
    } else {
        None
    };
    check_runner(&mut queue, &mut Vec::new());
    if active_stop_requested(&queue, stop.as_ref()) {
        next["target_groups"] = json!(0);
        next["paused"] = json!(false);
    }
    write_json(&path, &next, true).await?;
    Ok(Json(json!({"ok": true, "controls": next})))
}

fn object_payload(payload: Result<Json<Value>, JsonRejection>) -> HttpResult<Value> {
    let Json(value) = payload.map_err(|error| {
        let status = match error.status() {
            StatusCode::UNSUPPORTED_MEDIA_TYPE => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            StatusCode::PAYLOAD_TOO_LARGE => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::BAD_REQUEST,
        };
        ApiError::new(status, error.body_text())
    })?;
    if !value.is_object() {
        return Err(ApiError::bad_request("Expected a JSON object."));
    }
    Ok(value)
}

async fn restart(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
    payload: Result<Json<Value>, JsonRejection>,
) -> HttpResult<Response> {
    safe_id(&id)?;
    let body = object_payload(payload)?;
    if body.as_object().unwrap().keys().any(|key| {
        !matches!(
            key.as_str(),
            "client_id" | "reason" | "hard_reset" | "expected_attempt_id"
        )
    }) {
        return Err(ApiError::bad_request("Unknown restart option."));
    }
    let hard_reset = if body["hard_reset"].is_null() {
        false
    } else {
        body["hard_reset"]
            .as_bool()
            .context("hard_reset must be a boolean.")?
    };
    let client_id = optional_string(&body, "client_id")?
        .map(safe_id)
        .transpose()?;
    let reason =
        optional_string(&body, "reason")?.unwrap_or("Restart requested in the control room.");
    if reason.chars().count() > 1000 {
        return Err(ApiError::bad_request(
            "Restart reason exceeds 1000 characters.",
        ));
    }
    let _guard = room.control_gate.lock().await;
    let mut diagnostics = Vec::new();
    let queue = read_queue(&room, &mut diagnostics).await?;
    if queue["running"] != true || queue["runner_alive"] != true {
        return Err(ApiError::conflict(
            "Start the corpus runner before requesting a restart.",
        ));
    }
    let group = queue["groups"]
        .as_array()
        .and_then(|groups| groups.iter().find(|group| group["id"] == id))
        .ok_or_else(|| ApiError::not_found("Unknown comparison group."))?;
    if !body["expected_attempt_id"].is_null() && body["expected_attempt_id"] != group["attempt_id"]
    {
        return Err(ApiError::conflict(
            "This Excel group has already been replaced. Refresh the view before retrying.",
        ));
    }
    let run_id = required_string(&queue, "run_id")?;
    if group["run_id"] != run_id || group["restart_eligible"] == false {
        return Err(ApiError::conflict("This group belongs to an earlier run; select it in a new corpus run to compare it again."));
    }
    if let Some(client_id) = client_id {
        let launcher = read_snapshot(
            &confined_path(&room.root, &format!("groups/{id}/launcher.json"))?,
            &mut diagnostics,
            SNAPSHOT_LIMIT,
        )
        .await
        .unwrap_or(json!({}));
        let client = launcher["clients"]
            .get(client_id)
            .ok_or_else(|| ApiError::not_found("Unknown Excel client in this group."))?;
        if hard_reset
            && launcher["client_reset_supported"] == true
            && group["active"] == true
            && launcher["attempt_id"] == group["attempt_id"]
            && launcher["status"] == "running"
            && !client["pid"].is_null()
            && client["closed"] != true
        {
            let reset =
                reset_client(&room, group, client_id, client, reason, &mut diagnostics).await?;
            return Ok(accepted_restart("client", reset));
        }
    }
    if hard_reset && client_id.is_none() {
        return Err(ApiError::bad_request(
            "Hard reset requires an Excel client.",
        ));
    }
    for pending in recent_commands(&room.root.join("commands"), &id, &mut diagnostics).await? {
        if pending["run_id"] == run_id
            && pending["attempt_id"] == group["attempt_id"]
            && matches!(
                pending["status"].as_str(),
                Some("pending" | "restarting" | "queued")
            )
        {
            return Ok(accepted_restart("group", pending));
        }
    }
    let command_id = uuid::Uuid::new_v4().simple().to_string();
    let command = json!({"id": command_id, "type": "restart_group", "group_id": id,
        "run_id": run_id, "client_id": client_id, "reason": reason, "attempt_id": group["attempt_id"],
        "requested_unix_ms": now_ms(), "status": "pending", "restart_scope": "group",
        "message": "The whole comparison will be requeued next after safe cleanup of its current peer groups."});
    write_json(
        &confined_path(&room.root, &format!("commands/{command_id}.json"))?,
        &command,
        true,
    )
    .await?;
    Ok(accepted_restart("group", command))
}

fn accepted_restart(scope: &str, command: Value) -> Response {
    let location = format!(
        "/api/commands/{}",
        command["id"].as_str().unwrap_or_default()
    );
    let mut response = (
        StatusCode::ACCEPTED,
        Json(json!({"ok": true, "restart_scope": scope, "command": command})),
    )
        .into_response();
    if scope == "group" {
        response
            .headers_mut()
            .insert("location", location.parse().unwrap());
    }
    response
}

fn optional_string<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>> {
    if value[key].is_null() {
        return Ok(None);
    }
    Ok(Some(
        value[key]
            .as_str()
            .with_context(|| format!("{key} must be a string."))?,
    ))
}

async fn reset_client(
    room: &Room,
    group: &Value,
    client_id: &str,
    client: &Value,
    reason: &str,
    diagnostics: &mut Vec<Value>,
) -> Result<Value> {
    let id = required_string(group, "id")?;
    let directory = confined_path(&room.root, &format!("groups/{id}/client-resets"))?;
    tokio::fs::create_dir_all(&directory).await?;
    for path in command_files(&directory).await? {
        if let Some(pending) = read_snapshot(&path, diagnostics, SNAPSHOT_LIMIT).await {
            if pending["attempt_id"] == group["attempt_id"]
                && pending["client_id"] == client_id
                && matches!(pending["status"].as_str(), Some("pending" | "running"))
            {
                return Ok(pending);
            }
        }
    }
    let reset_id = uuid::Uuid::new_v4().simple().to_string();
    let controls = read_controls(room, diagnostics).await;
    let reset = json!({"id": reset_id, "type": "reset_client", "client_id": client_id, "group_id": id,
        "attempt_id": group["attempt_id"], "run_id": group["run_id"], "expected_pid": client["pid"],
        "expected_started_unix_ms": client["process_started_unix_ms"], "hide_excel": controls["hide_excel"],
        "reason": reason, "status": "pending", "requested_unix_ms": now_ms()});
    write_json(&directory.join(format!("{reset_id}.json")), &reset, true).await?;
    Ok(reset)
}

async fn command(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    let path = confined_path(&room.root, &format!("commands/{id}.json"))?;
    let value = read_snapshot(&path, &mut Vec::new(), SNAPSHOT_LIMIT)
        .await
        .ok_or_else(|| ApiError::not_found("Unknown command."))?;
    Ok(Json(value))
}

async fn asset(
    State(room): State<Arc<Room>>,
    RoutePath(path): RoutePath<String>,
    request: Request,
) -> HttpResult<Response> {
    let path = confined_path(&room.root, &path)?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "png" | "webp" | "jpg" | "jpeg") || !path.is_file() {
        return Err(ApiError::not_found("Unknown image asset."));
    }
    Ok(ServeFile::new(path).try_call(request).await?.map(Body::new))
}

fn defaults() -> Value {
    json!({"target_groups": 1, "paused": false, "open_timeout_seconds": 600, "system_reserve_mib": 1024,
        "job_timeout_seconds": 300, "equal_preview_width": 512, "capture_tile_megapixels": 32,
        "capture_batch_tiles": 1, "capture_transport": "live-images", "hide_excel": false,
        "tall_sheet_threshold": 40, "tall_sheet_edge_tiles": 2})
}

fn merge_controls(current: &Value, patch: &Value) -> Result<Value> {
    let fields = patch
        .as_object()
        .context("Expected a JSON object of control changes.")?;
    let mut next = current.clone();
    let known = defaults();
    for (key, value) in fields {
        ensure!(known.get(key).is_some(), "Unknown control '{key}'.");
        if matches!(key.as_str(), "paused" | "hide_excel") {
            ensure!(value.is_boolean(), "{key} must be a boolean.");
        } else if key == "capture_transport" {
            ensure!(
                matches!(value.as_str(), Some("live-images" | "hash-first")),
                "capture_transport must be 'live-images' or 'hash-first'."
            );
        } else {
            let integer = value
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .with_context(|| format!("{key} must be an integer."))?;
            let valid = match key.as_str() {
                "equal_preview_width" => matches!(integer, 0 | 128 | 256 | 512 | 1024),
                "target_groups" => integer >= 0,
                "capture_tile_megapixels" => (1..=32).contains(&integer),
                "capture_batch_tiles" => (1..=16).contains(&integer),
                "system_reserve_mib" => (1..=65536).contains(&integer),
                _ => integer > 0,
            };
            ensure!(valid, "Invalid value for {key}.");
        }
        next[key] = value.clone();
    }
    if next["target_groups"] == 0 {
        ensure!(
            patch["paused"] != true,
            "Finish current work at group target 0; capture dispatch cannot also be paused."
        );
        next["paused"] = json!(false);
    }
    Ok(next)
}

async fn read_controls(room: &Room, diagnostics: &mut Vec<Value>) -> Value {
    let result = async {
        merge_controls(
            &defaults(),
            &read_object(&room.root.join("control.json"), SNAPSHOT_LIMIT).await?,
        )
    }
    .await;
    match result {
        Ok(controls) => controls,
        Err(error) => {
            diagnostics.push(json!({"file": "control.json", "error": error.to_string()}));
            room.initial_controls.clone()
        }
    }
}

fn active_stop_requested(queue: &Value, stop: Option<&Value>) -> bool {
    stop.is_some_and(|stop| {
        !stop["runner_pid"].is_null()
            && queue["runner_alive"] != false
            && (queue["running"] == true || queue["runner_reported_running"] == true)
            && stop["runner_pid"] == queue["runner_pid"]
            && stop["runner_started_unix_ms"] == queue["runner_started_unix_ms"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_reason_must_belong_to_the_displayed_attempt() {
        let record = json!({"status":"skipped","error":"Excel opening timed out",
            "control_room_attempt":{"id":"current"}});
        let mut current = json!({"status":"skipped"});
        attach_outcome(&mut current, &record, Some("current"));
        assert_eq!(current["outcome_error"], "Excel opening timed out");
        let mut retry = json!({"status":"skipped"});
        attach_outcome(&mut retry, &record, Some("next"));
        assert!(retry["outcome_error"].is_null());
        let mut active = json!({"status":"capturing"});
        attach_outcome(&mut active, &record, Some("current"));
        assert!(active["outcome_error"].is_null());
    }

    #[test]
    fn capture_transport_defaults_to_live_images_and_rejects_unknown_modes() -> Result<()> {
        let current = defaults();
        assert_eq!(current["capture_transport"], "live-images");
        let selected = merge_controls(&current, &json!({"capture_transport":"hash-first"}))?;
        assert_eq!(selected["capture_transport"], "hash-first");
        assert_eq!(selected["target_groups"], current["target_groups"]);
        assert_eq!(
            selected["capture_batch_tiles"],
            current["capture_batch_tiles"]
        );
        let restored = merge_controls(&selected, &json!({"capture_transport":"live-images"}))?;
        assert_eq!(restored, current);
        for value in [
            json!("hash-only"),
            json!(""),
            json!(true),
            json!(1),
            Value::Null,
        ] {
            assert!(merge_controls(&current, &json!({"capture_transport":value})).is_err());
        }
        Ok(())
    }
}
