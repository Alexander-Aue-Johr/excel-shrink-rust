//! Browser-facing control plane. The scheduler owns scientific work; this host
//! reads its snapshots, archives terminal display mirrors and writes explicit,
//! attempt-scoped control requests.

mod activity;
mod inspection;
mod manual_capture;
mod pixel_diagnostics;
mod formula_diagnostics;
mod platform;
mod snapshots;
mod snapshot_cache;
mod summary;
mod targeted;
mod targeted_history;
mod workbook_index;

use super::{now_iso, now_ms, read_json, serve, with_web, write_json, ApiError, HttpResult};
use anyhow::{ensure, Context, Result};
use axum::{
    body::{Body, Bytes},
    extract::{rejection::JsonRejection, Path as RoutePath, Query, Request, State},
    http::{HeaderMap, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use snapshots::*;
use std::{collections::HashMap, io::Read, path::Path, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tower_http::services::ServeFile;

struct Room {
    root: PathBuf,
    web: PathBuf,
    port: u16,
    version: String,
    started: String,
    initial_controls: Value,
    control_gate: Mutex<()>,
    manual_capture: Mutex<Option<std::process::Child>>,
    snapshots: snapshot_cache::SnapshotCache,
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
        web: web.clone(),
        port,
        version: settings["host_version"]
            .as_str()
            .unwrap_or("unspecified")
            .to_owned(),
        started: now_iso(),
        initial_controls: initial,
        control_gate: Mutex::new(()),
        manual_capture: Mutex::new(None),
        snapshots: snapshot_cache::SnapshotCache::default(),
    });
    let routes = Router::new()
        .route("/health", get(health))
        .route("/api/state", get(state))
        .route("/api/summary", get(summary::get))
        .route("/api/workbooks", get(workbooks))
        .route("/api/workers", get(workers))
        .route("/api/targeted-comparisons", get(targeted::comparisons))
        .route("/api/groups/{id}", get(group))
        .route("/api/groups/{id}/targeted-compare", post(targeted::start))
        .route("/api/groups/{id}/targeted-report", get(targeted::report))
        .route(
            "/api/groups/{id}/pixel-diagnostics",
            get(pixel_diagnostics::inspect),
        )
        .route("/api/evidence/{*path}", get(evidence_asset))
        .route("/api/groups/{id}/formula-diagnostics", get(formula_diagnostics::inspect))
        .route(
            "/api/groups/{id}/capture-sheet",
            post(manual_capture::start),
        )
        .route("/api/manual-captures/{id}", get(manual_capture::state))
        .route("/api/run/stop", post(stop))
        .route("/api/run/terminate", post(terminate))
        .route("/api/control", post(control))
        .route("/api/groups/{id}/restart", post(restart))
        .route("/api/groups/{id}/inspect", post(inspect))
        .route("/api/groups/{id}/copy-original", post(copy_original))
        .route("/api/groups/{id}/variants/{profile}", get(download_variant))
        .route("/api/commands/{id}", get(command))
        .route("/data/{*path}", get(asset))
        .with_state(room.clone());
    let history_root = room.root.clone();
    let app = with_web(routes, &web, 16 * 1024)
        .layer(middleware::from_fn_with_state(room, enforce_local_request));
    let maintenance = tokio::spawn(targeted_history::maintain(history_root));
    let result = serve(app, port).await;
    maintenance.abort();
    result
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
    let mut queue = room
        .snapshots
        .snapshot(&room.root.join("queue.json"), diagnostics, SNAPSHOT_LIMIT)
        .await
        .map(|value| value.as_ref().clone())
        .unwrap_or(json!({}));
    check_runner(&mut queue, diagnostics);
    reconcile_groups(&room.root, &mut queue, diagnostics).await?;
    Ok(queue)
}

fn capture_clients_waiting(capacity: &Value, metrics: &Value) -> Option<usize> {
    let reserved = usize::try_from(capacity["reserved_clients"].as_u64()?).ok()?;
    let clients = metrics["clients"].as_object()?;
    let started = clients
        .values()
        .filter(|client| client["pid"].as_u64().is_some())
        .count();
    (reserved <= 16 && started <= reserved).then_some(reserved - started)
}

/// Counts prepared capture workbooks that have not yet started Excel.
///
/// The count includes sessions waiting for a capture-group slot. It is a
/// demand signal, not a count of Excel processes already running.
async fn capture_opening_demand(root: &Path, queue: &Value) -> Value {
    let mut waiting_workbooks = 0;
    let mut waiting_sessions = 0;
    let mut opening_workbooks = 0;
    let mut observed_sessions = 0;
    let work = root.parent().map(|output| output.join(".work"));
    if let (Some(work), Some(groups)) = (work, queue["groups"].as_array()) {
        for group in groups {
            if group["active"] != true
                || !matches!(
                    group["status"].as_str(),
                    Some("preparing" | "hash_sweeping")
                )
                || group["run_id"] != queue["run_id"]
            {
                continue;
            }
            let Some(id) = group["id"].as_str().filter(|id| safe_id(id).is_ok()) else {
                continue;
            };
            let session = work.join(id).join("preparation-capture");
            let (Ok(capacity), Ok(metrics)) = (
                read_object(&session.join("capacity.json"), 4096).await,
                read_object(&session.join("metrics.json"), SNAPSHOT_LIMIT).await,
            ) else {
                continue;
            };
            if metrics["attempt_id"] != group["attempt_id"] {
                continue;
            }
            let Some(waiting) = capture_clients_waiting(&capacity, &metrics) else {
                continue;
            };
            observed_sessions += 1;
            waiting_workbooks += waiting;
            waiting_sessions += usize::from(waiting > 0);
            opening_workbooks += metrics["clients"]
                .as_object()
                .unwrap()
                .values()
                .filter(|client| client["status"] == "opening")
                .count();
        }
    }
    json!({
        "waiting_workbooks": waiting_workbooks,
        "waiting_sessions": waiting_sessions,
        "opening_workbooks": opening_workbooks,
        "observed_sessions": observed_sessions,
        "scope": "preparation capture sessions"
    })
}

/// Reads per-sheet capture progress from current Excel clients only.
///
/// Preparation clients publish in `.work`; comparison clients publish in the
/// control room. Attempt matching keeps old session files out of a resumed run.
async fn capture_workers(root: &Path, queue: &Value) -> Vec<Value> {
    let mut workers = Vec::new();
    let Some(groups) = queue["groups"].as_array() else {
        return workers;
    };
    for group in groups {
        if group["active"] != true || group["run_id"] != queue["run_id"] {
            continue;
        }
        let Some(id) = group["id"].as_str().filter(|id| safe_id(id).is_ok()) else {
            continue;
        };
        let session = if matches!(
            group["status"].as_str(),
            Some("preparing" | "hash_sweeping")
        ) {
            root.parent().map(|output| {
                output
                    .join(".work")
                    .join(id)
                    .join("preparation-capture/metrics.json")
            })
        } else {
            Some(root.join("groups").join(id).join("launcher.json"))
        };
        let Some(session) = session else {
            continue;
        };
        let Ok(metrics) = read_object(&session, SNAPSHOT_LIMIT).await else {
            continue;
        };
        if metrics["attempt_id"] != group["attempt_id"] {
            continue;
        }
        let Some(clients) = metrics["clients"].as_object() else {
            continue;
        };
        for (profile, client) in clients {
            if client["pid"].as_u64().is_none()
                || client["process_alive"] == false
                || client["closed"] == true
            {
                continue;
            }
            let resources = &client["monitor"]["pane_resources"];
            let sweep = &resources["sweep_progress"];
            let current = &resources["capture_progress"];
            let (progress, unit) = if sweep.is_object() {
                (sweep, "sheet passes")
            } else if current.is_object() && client["active_command"] == current["command_id"] {
                (current, "captures")
            } else {
                (&Value::Null, "sheet passes")
            };
            let current_matches = sweep.is_object()
                && current["command_id"] == sweep["command_id"]
                && current["sheet_index"] == sweep["sheet_index"];
            workers.push(json!({"group_id":id,"attempt_id":group["attempt_id"],
                "workbook":group["workbook"],"profile":profile,"pid":client["pid"],
                "process_started_unix_ms":client["process_started_unix_ms"],
                "state":client["monitor"]["state"],"sheet_name":progress["sheet_name"],
                "sheet_index":progress["sheet_index"],"completed":progress["completed"],
                "total":progress["total"],"unit":unit,"pass":progress["pass"],
                "passes":progress["passes"],"updated_unix_ms":progress["updated_unix_ms"],
                "captures_completed":if current_matches {current["completed"].clone()} else {Value::Null},
                "captures_total":if current_matches {current["total"].clone()} else {Value::Null}}));
        }
    }
    workers
}

async fn read_workbook_index(room: &Room, diagnostics: &mut Vec<Value>) -> Option<Arc<Value>> {
    room.snapshots
        .snapshot(
            &room.root.join("workbook-index.json"),
            diagnostics,
            64 * 1024 * 1024,
        )
        .await
}

async fn workbooks(
    State(room): State<Arc<Room>>,
    Query(query): Query<HashMap<String, String>>,
) -> HttpResult<Json<Value>> {
    let filter = query.get("filter").map(String::as_str).unwrap_or("all");
    if !workbook_index::FILTERS.contains(&filter) {
        return Err(ApiError::bad_request("Unknown workbook filter."));
    }
    let search = query.get("search").map(String::as_str).unwrap_or("").trim();
    if search.len() > 200 {
        return Err(ApiError::bad_request("Workbook search is too long."));
    }
    let offset = query
        .get("offset")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| ApiError::bad_request("Workbook page offset must be nonnegative."))?
        .unwrap_or(0);
    let limit = query
        .get("limit")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| ApiError::bad_request("Workbook page size must be an integer."))?
        .unwrap_or(100);
    if !(1..=100).contains(&limit) {
        return Err(ApiError::bad_request("Workbook page size must be 1..100."));
    }
    let mut diagnostics = Vec::new();
    let Some(index) = read_workbook_index(&room, &mut diagnostics).await else {
        return Ok(Json(json!({"available":false,"workbooks":[],"counts":{},
            "total":0,"offset":offset,"next_offset":null,"diagnostics":diagnostics})));
    };
    let mut response = workbook_index::page(&index, filter, search, offset, limit);
    response["available"] = json!(true);
    response["diagnostics"] = json!(diagnostics);
    Ok(Json(response))
}

#[derive(Default, serde::Deserialize)]
struct StateOptions {
    group_scope: Option<String>,
    selected: Option<String>,
}

/// Project frequent live polls while preserving the complete legacy response.
/// Corpus browsing remains available through the paged workbook index.
async fn state(
    State(room): State<Arc<Room>>,
    Query(options): Query<StateOptions>,
) -> HttpResult<Json<Value>> {
    let scope = options.group_scope.as_deref().unwrap_or("all");
    if !matches!(scope, "all" | "live") {
        return Err(ApiError::bad_request("group_scope must be all or live."));
    }
    if let Some(id) = &options.selected {
        safe_id(id)?;
    }
    let mut diagnostics = Vec::new();
    let mut queue = read_queue(&room, &mut diagnostics).await?;
    queue["update_ready"] =
        json!(queue["runner_alive"] == false && platform::output_idle(&room.root));
    if queue["groups"].is_null() {
        queue["groups"] = json!([]);
    }
    queue["controls"] = read_controls(&room, &mut diagnostics).await;
    queue["logical_cores"] = json!(logical_cores());
    let opening_demand = capture_opening_demand(&room.root, &queue).await;
    queue["excel_opening_demand"] = opening_demand;
    let workers = capture_workers(&room.root, &queue).await;
    queue["capture_workers"] = json!(workers);
    let stop = read_snapshot(
        &room.root.join("stop-request.json"),
        &mut diagnostics,
        SNAPSHOT_LIMIT,
    )
    .await;
    queue["stop_requested"] = json!(active_stop_requested(&queue, stop.as_ref()));
    let terminate = read_snapshot(
        &room.root.join("terminate-request.json"),
        &mut diagnostics,
        SNAPSHOT_LIMIT,
    )
    .await;
    queue["terminate_requested"] = json!(active_stop_requested(&queue, terminate.as_ref()));
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
    attach_active_preparation_profile(&room.root, &mut queue, &mut diagnostics).await;
    activity::augment_progress(&mut queue, &room.root).await?;
    queue["memory"] = platform::memory_snapshot();
    queue["server"] =
        json!({"started_at": room.started, "utc": now_iso(), "port": room.port, "live": true});
    queue["diagnostics"] = json!(diagnostics);
    if scope == "live" {
        retain_live_groups(&mut queue, options.selected.as_deref());
    }
    Ok(Json(queue))
}

/// Retain every active group, the selected workbook and eight recent summaries.
/// Filtering owns only this response and never changes the durable queue.
fn retain_live_groups(queue: &mut Value, selected: Option<&str>) {
    let groups = queue["groups"]
        .as_array_mut()
        .map(std::mem::take)
        .unwrap_or_default();
    let total = groups.len();
    let mut recent: Vec<_> = groups
        .iter()
        .enumerate()
        .filter(|(_, group)| {
            group["active"] != true
                && (queue["run_id"].is_null()
                    || group["run_id"].is_null()
                    || group["run_id"] == queue["run_id"])
        })
        .map(|(position, group)| {
            (group["started_unix_ms"].as_i64().unwrap_or(0), position)
        })
        .collect();
    recent.sort_unstable_by(|left, right| right.cmp(left));
    let recent: std::collections::HashSet<_> = recent
        .into_iter()
        .take(8)
        .map(|(_, position)| position)
        .collect();
    let kept: Vec<_> = groups
        .into_iter()
        .enumerate()
        .filter_map(|(position, group)| {
            (group["active"] == true
                || recent.contains(&position)
                || selected.is_some_and(|id| group["id"].as_str() == Some(id)))
            .then_some(group)
        })
        .collect();
    queue["groups_total"] = json!(total);
    queue["group_scope"] = json!("live");
    queue["groups_truncated"] = json!(kept.len() < total);
    queue["groups"] = json!(kept);
}

async fn attach_active_preparation_profile(
    root: &Path,
    queue: &mut Value,
    diagnostics: &mut Vec<Value>,
) {
    let Some(active) = queue["progress"]["active_preparation"].as_array() else {
        return;
    };
    if active.len() != 1 {
        return;
    }
    let workbook = &active[0]["workbook"];
    let Some(group) = queue["groups"].as_array().and_then(|groups| {
        groups.iter().find(|group| {
            group["active"] == true
                && group["status"] == "preparing"
                && group["workbook"] == *workbook
                && group["run_id"] == queue["run_id"]
        })
    }) else {
        return;
    };
    let Some(id) = group["id"].as_str() else {
        return;
    };
    let attempt = group["attempt_id"].clone();
    let Ok(directory) = confined_path(root, &format!("groups/{id}")) else {
        return;
    };
    let Some(coordinator) = read_snapshot(
        &directory.join("coordinator.json"),
        diagnostics,
        SNAPSHOT_LIMIT,
    )
    .await
    else {
        return;
    };
    if attempt.is_null() || coordinator["attempt_id"] != attempt {
        return;
    }
    if coordinator["phase"] != "shrinking" || coordinator["last_event"]["kind"] != "profile_started"
    {
        return;
    }
    if let Some(profile) = coordinator["last_event"]["details"]["profile"].as_str() {
        queue["progress"]["active_preparation"][0]["profile"] = json!(profile);
    }
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
                                | "crc-identical"
                                | "workbook-only"
                                | "paused-for-later"
                                | "differences"
                                | "screened"
                                | "screening-differences"
                                | "screening-failed"
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
    let queued_summary = queue["groups"]
        .as_array()
        .and_then(|groups| {
            groups.iter().find(|group| {
                group["id"] == id
                    && (group["run_id"].is_null()
                        || queue["run_id"].is_null()
                        || group["run_id"] == queue["run_id"])
            })
        })
        .cloned();
    let in_queue = queued_summary.is_some();
    let summary = if queued_summary.is_some() {
        queued_summary
    } else {
        read_workbook_index(&room, &mut diagnostics)
            .await
            .and_then(|index| {
                (queue["run_id"].is_null() || index["run_id"] == queue["run_id"])
                    .then(|| workbook_index::find(&index, &id).cloned())
                    .flatten()
            })
    };
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
    if !in_queue
        || scheduled_attempt.as_deref()
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
    merged["targeted_comparison"] = read_snapshot(
        &directory.join("targeted-comparison.json"),
        &mut diagnostics,
        SNAPSHOT_LIMIT,
    )
    .await
    .unwrap_or(Value::Null);
    merged["manual_captures"] = read_snapshot(
        &directory.join("manual-captures.json"),
        &mut diagnostics,
        SNAPSHOT_LIMIT,
    )
    .await
    .unwrap_or(json!([]));
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
    if merged["shrink_identity"].is_null() {
        merged["shrink_identity"] = record["shrink_identity"].clone();
    }
    if merged["cleanup_analysis"].is_null() {
        merged["cleanup_analysis"] = record["cleanup_analysis"].clone();
    }
    if merged["capture_deferred"].is_null() {
        merged["capture_deferred"] = record["capture_deferred"].clone();
    }
    if merged["recalc_identity_changed"].is_null() {
        merged["recalc_identity_changed"] = record["recalc_identity_changed"].clone();
    }
    if merged["baseline_repaired"].is_null() {
        merged["baseline_repaired"] = record["baseline_repaired"].clone();
    }
    if merged["source_format"].is_null() {
        merged["source_format"] = record["source_format"].clone();
    }
    if record["cleanup_pending"].is_object() {
        merged["cleanup_pending"] = record["cleanup_pending"].clone();
    }
    merged["priority_validation"] = record["priority_validation"].clone();
    merged["analysis_summary"] = record["analysis_summary"].clone();
    merged["visual_diagnoses"] = json!(record["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|profile| profile["visual_diagnosis"].is_object())
        .map(
            |profile| json!({"profile":profile["profile"],"diagnosis":profile["visual_diagnosis"]})
        )
        .collect::<Vec<_>>());
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
            Some(
                "skipped"
                    | "failed"
                    | "unchanged"
                    | "crc-identical"
                    | "workbook-only"
                    | "paused-for-later"
                    | "differences"
                    | "screened"
                    | "screened-equal"
                    | "screening-differences"
                    | "screening-failed"
            )
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
    if !known_workbook(&room, &id).await? {
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

/// Copy an immutable prepared original into inspection storage before publishing
/// a file-drop clipboard entry. This never opens Excel or changes the run queue.
async fn copy_original(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    if !known_workbook(&room, &id).await? {
        return Err(ApiError::not_found("Unknown comparison group."));
    }
    let script = room
        .web
        .parent()
        .context("Missing capture scripts")?
        .join("copy-workbook-to-clipboard.ps1");
    let root = room.root.clone();
    let path = tokio::task::spawn_blocking(move || {
        inspection::copy_original_to_clipboard(&root, &id, &script)
    })
    .await
    .context("Copy original inspection file")??;
    Ok(Json(json!({"ok":true,"path":path,"clipboard":"file-copy"})))
}

async fn known_workbook(room: &Room, id: &str) -> Result<bool> {
    let queue = read_queue(&room, &mut Vec::new()).await?;
    let queued = queue["groups"].as_array().is_some_and(|groups| {
        groups.iter().any(|group| {
            group["id"] == id
                && (group["run_id"].is_null()
                    || queue["run_id"].is_null()
                    || group["run_id"] == queue["run_id"])
        })
    });
    let indexed = if queued {
        false
    } else {
        read_workbook_index(&room, &mut Vec::new())
            .await
            .is_some_and(|index| {
                (queue["run_id"].is_null() || index["run_id"] == queue["run_id"])
                    && workbook_index::find(&index, &id).is_some()
            })
    };
    Ok(queued || indexed)
}

async fn download_variant(
    State(room): State<Arc<Room>>,
    RoutePath((id, profile)): RoutePath<(String, String)>,
) -> HttpResult<Response> {
    safe_id(&id)?;
    if !inspection::PROFILES.contains(&profile.as_str()) {
        return Err(ApiError::bad_request("Unknown inspection profile."));
    }
    if !known_workbook(&room, &id).await? {
        return Err(ApiError::not_found("Unknown comparison group."));
    }
    let root = room.root.clone();
    let requested_id = id.clone();
    let requested_profile = profile.clone();
    let (mut archive, member, size) = tokio::task::spawn_blocking(move || {
        inspection::open_variant(&root, &requested_id, &requested_profile)
    })
    .await
    .context("Open prepared workbook variant")??;
    let (sender, receiver) = tokio::sync::mpsc::channel::<std::io::Result<Bytes>>(4);
    tokio::task::spawn_blocking(move || {
        let transfer = (|| -> std::io::Result<()> {
            let mut entry = archive.by_name(&member).map_err(std::io::Error::other)?;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = entry.read(&mut buffer)?;
                if read == 0 {
                    return Ok(());
                }
                if sender
                    .blocking_send(Ok(Bytes::copy_from_slice(&buffer[..read])))
                    .is_err()
                {
                    return Ok(());
                }
            }
        })();
        if let Err(error) = transfer {
            let _ = sender.blocking_send(Err(error));
        }
    });
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|chunk| (chunk, receiver))
    });
    let filename = format!("{id}-{profile}.xlsx");
    Response::builder()
        .status(StatusCode::OK)
        .header(
            "content-type",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        )
        .header(
            "content-disposition",
            format!("attachment; filename=\"{filename}\""),
        )
        .header("content-length", size.to_string())
        .body(Body::from_stream(stream))
        .map_err(|error| ApiError::bad_request(error))
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
            | "crc-identical"
            | "workbook-only"
            | "paused-for-later"
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
    let terminate = read_snapshot(
        &room.root.join("terminate-request.json"),
        &mut Vec::new(),
        SNAPSHOT_LIMIT,
    )
    .await;
    if active_stop_requested(&queue, terminate.as_ref()) {
        return Err(ApiError::conflict(
            "This runner is already stopping immediately.",
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

async fn terminate(State(room): State<Arc<Room>>) -> HttpResult<Json<Value>> {
    let _guard = room.control_gate.lock().await;
    let mut queue = read_object(&room.root.join("queue.json"), SNAPSHOT_LIMIT).await?;
    check_runner(&mut queue, &mut Vec::new());
    if queue["running"] != true || queue["runner_alive"] != true {
        return Err(ApiError::conflict(
            "There is no verified active runner to terminate.",
        ));
    }
    if queue["terminate_supported"] != true {
        return Err(ApiError::conflict(
            "This runner does not support immediate termination.",
        ));
    }
    let pid = queue["runner_pid"]
        .as_i64()
        .ok_or_else(|| ApiError::conflict("Runner PID is unavailable."))?;
    let started = queue["runner_started_unix_ms"]
        .as_i64()
        .ok_or_else(|| ApiError::conflict("Runner creation time is unavailable."))?;
    let path = room.root.join("control.json");
    let current = merge_controls(&defaults(), &read_object(&path, SNAPSHOT_LIMIT).await?)?;
    write_json(
        &path,
        &merge_controls(&current, &json!({"target_groups": 0, "paused": false}))?,
        true,
    )
    .await?;
    let request = json!({"runner_pid": pid, "runner_started_unix_ms": started});
    write_json(&room.root.join("terminate-request.json"), &request, true).await?;

    // Workers observe the request within their progress loop and close owned
    // Excel sessions. If that bounded cleanup stalls, end only this verified
    // runner process. A reused PID can never match the saved creation time.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(120)).await;
        if let Err(error) = platform::terminate_verified_process(pid, started) {
            eprintln!("[control-room] Could not terminate stalled runner {pid}: {error}");
        }
    });
    Ok(Json(json!({"ok": true, "message":
        "Stopping active workers and owned Excel sessions now. If cleanup stalls, the verified runner will be terminated after 120 seconds."})))
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
    let terminate_path = room.root.join("terminate-request.json");
    let queue_path = room.root.join("queue.json");
    let mut queue = if (stop_path.exists() || terminate_path.exists()) && queue_path.exists() {
        read_object(&queue_path, SNAPSHOT_LIMIT).await?
    } else {
        json!({})
    };
    let stop = if stop_path.exists() {
        Some(read_object(&stop_path, SNAPSHOT_LIMIT).await?)
    } else {
        None
    };
    let terminate = if terminate_path.exists() {
        Some(read_object(&terminate_path, SNAPSHOT_LIMIT).await?)
    } else {
        None
    };
    check_runner(&mut queue, &mut Vec::new());
    if active_stop_requested(&queue, stop.as_ref())
        || active_stop_requested(&queue, terminate.as_ref())
    {
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
            "client_id" | "reason" | "hard_reset" | "expected_attempt_id" | "full_scan_deferred"
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
    let full_scan_deferred = if body["full_scan_deferred"].is_null() {
        false
    } else {
        body["full_scan_deferred"]
            .as_bool()
            .context("full_scan_deferred must be a boolean.")?
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
    if queue["phase"] == "awaiting_deferred_manual" && !full_scan_deferred {
        return Err(ApiError::conflict(
            "Only deferred full scans can start after regular comparisons finish.",
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
    if full_scan_deferred {
        if queue["phase"] != "awaiting_deferred_manual" {
            return Err(ApiError::conflict(
                "Deferred full scans can start after all regular comparisons finish.",
            ));
        }
        let pending = match &group["capture_deferred"] {
            Value::Bool(value) => *value,
            Value::Array(items) => !items.is_empty(),
            Value::Object(profiles) => profiles.values().any(|value| {
                value.as_bool() == Some(true)
                    || value.as_array().is_some_and(|items| !items.is_empty())
            }),
            _ => false,
        };
        if !pending || client_id.is_some() || hard_reset {
            return Err(ApiError::bad_request(
                "A deferred full scan requires a group with pending sheets and no client reset.",
            ));
        }
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
            if full_scan_deferred && pending["full_scan_deferred"] != true {
                return Err(ApiError::conflict(
                    "A regular restart is already pending for this group.",
                ));
            }
            if full_scan_deferred {
                resume_for_deferred_scan(&room, &queue).await?;
            }
            return Ok(accepted_restart("group", pending));
        }
    }
    if full_scan_deferred {
        resume_for_deferred_scan(&room, &queue).await?;
    }
    let command_id = uuid::Uuid::new_v4().simple().to_string();
    let command = json!({"id": command_id, "type": "restart_group", "group_id": id,
    "run_id": run_id, "client_id": client_id, "reason": reason, "attempt_id": group["attempt_id"],
    "requested_unix_ms": now_ms(), "status": "pending", "restart_scope": "group",
    "full_scan_deferred": full_scan_deferred,
    "message": if full_scan_deferred {
        "Deferred sheets will be fully captured after safe cleanup of current groups."
    } else {
        "The whole comparison will be requeued next after safe cleanup of its current peer groups."
    }});
    write_json(
        &confined_path(&room.root, &format!("commands/{command_id}.json"))?,
        &command,
        true,
    )
    .await?;
    Ok(accepted_restart("group", command))
}

async fn resume_for_deferred_scan(room: &Room, queue: &Value) -> HttpResult<()> {
    let stop = read_snapshot(
        &room.root.join("stop-request.json"),
        &mut Vec::new(),
        SNAPSHOT_LIMIT,
    )
    .await;
    let terminate = read_snapshot(
        &room.root.join("terminate-request.json"),
        &mut Vec::new(),
        SNAPSHOT_LIMIT,
    )
    .await;
    if active_stop_requested(queue, stop.as_ref())
        || active_stop_requested(queue, terminate.as_ref())
    {
        return Err(ApiError::conflict("The study runner is stopping."));
    }
    let path = room.root.join("control.json");
    let current = merge_controls(&defaults(), &read_object(&path, SNAPSHOT_LIMIT).await?)?;
    if current["target_groups"] == 0 {
        let resumed = merge_controls(&current, &json!({"target_groups": 1}))?;
        write_json(&path, &resumed, true).await?;
    }
    Ok(())
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

/// Retained targeted evidence lives beside control-room, under the study evidence tree.
async fn evidence_asset(
    State(room): State<Arc<Room>>,
    RoutePath(relative): RoutePath<String>,
    request: Request,
) -> HttpResult<Response> {
    let study = room.root.parent().context("Missing study directory.")?;
    let path = confined_path(study, &format!("evidence/{relative}"))?;
    if path.extension().and_then(|value| value.to_str()) != Some("png") || !path.is_file() {
        return Err(ApiError::not_found("Unknown evidence image."));
    }
    Ok(ServeFile::new(path).try_call(request).await?.map(Body::new))
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

fn logical_cores() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
}

fn defaults() -> Value {
    json!({"target_groups": 1, "prepare_processes": 1, "sweep_processes": 1, "shrink_threads": logical_cores(),
        "paused": false, "open_timeout_seconds": 600, "system_reserve_mib": 0,
        "job_timeout_seconds": 300, "equal_preview_width": 512, "capture_tile_megapixels": 32,
        "capture_batch_tiles": 1, "capture_context_tiles": 0, "capture_transport": "live-images", "hide_excel": false,
        "geometry_batch_ranges": crate::capture_settings::DEFAULT_GEOMETRY_RANGES,
        "capture_batch_ranges": crate::capture_settings::DEFAULT_CAPTURE_RANGES,
        "tall_sheet_threshold": 40, "tall_sheet_edge_tiles": 2,
        "crc_equal_sheet_threshold_mib": 256})
}

fn merge_controls(current: &Value, patch: &Value) -> Result<Value> {
    let fields = patch
        .as_object()
        .context("Expected a JSON object of control changes.")?;
    let mut next = current.clone();
    next.as_object_mut()
        .context("Expected current controls object.")?
        .remove("excel_process_limit");
    let known = defaults();
    for (key, value) in fields {
        // Retired settings from saved runs and already-open browser tabs are ignored.
        if key == "excel_process_limit" {
            continue;
        }
        ensure!(known.get(key).is_some(), "Unknown control '{key}'.");
        if matches!(key.as_str(), "paused" | "hide_excel") {
            ensure!(value.is_boolean(), "{key} must be a boolean.");
        } else if key == "geometry_batch_ranges" || key == "capture_batch_ranges" {
            let maximum = if key == "geometry_batch_ranges" {
                crate::capture_settings::MAX_GEOMETRY_RANGES
            } else {
                crate::capture_settings::MAX_CAPTURE_RANGES
            };
            ensure!(!value.is_null(), "Batch size must be an integer");
            crate::capture_settings::batch_size(value, 1, maximum)?;
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
                "sweep_processes" => integer > 0,
                "prepare_processes" | "shrink_threads" => {
                    integer > 0 && (integer as usize) <= logical_cores()
                }
                "capture_tile_megapixels" => (1..=64).contains(&integer),
                "capture_batch_tiles" => (1..=1024).contains(&integer),
                "capture_context_tiles" => (0..=16).contains(&integer),
                "system_reserve_mib" => (0..=65536).contains(&integer),
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
    use std::io::Write;

    #[test]
    fn targeted_package_sizes_are_independent_api_controls() -> Result<()> {
        let defaults = defaults();
        let current = merge_controls(
            &defaults,
            &json!({"geometry_batch_ranges":256,"capture_batch_ranges":3}),
        )?;
        assert_eq!(current["geometry_batch_ranges"], 256);
        assert_eq!(current["capture_batch_ranges"], 3);
        let next = merge_controls(&current, &json!({"capture_batch_ranges":2}))?;
        assert_eq!(next["geometry_batch_ranges"], 256);
        assert_eq!(next["capture_batch_ranges"], 2);
        for invalid in [json!(0), json!(-1), json!(1.5), Value::Null, json!(65)] {
            assert!(merge_controls(&current, &json!({"capture_batch_ranges":invalid})).is_err());
        }
        assert!(merge_controls(&current, &json!({"geometry_batch_ranges":4097})).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn preparation_capture_demand_counts_unstarted_excel_clients() -> Result<()> {
        let output = std::env::temp_dir().join(format!(
            "control-room-excel-demand-{}",
            uuid::Uuid::new_v4()
        ));
        let root = output.join("control-room");
        let session = output.join(".work/workbook/preparation-capture");
        std::fs::create_dir_all(&root)?;
        std::fs::create_dir_all(&session)?;
        std::fs::write(session.join("capacity.json"), br#"{"reserved_clients":2}"#)?;
        std::fs::write(
            session.join("metrics.json"),
            br#"{"attempt_id":"current","clients":{}}"#,
        )?;
        let queue = json!({"run_id":"run","groups":[{"id":"workbook","workbook":"book.xlsx","run_id":"run",
            "attempt_id":"current","active":true,"status":"preparing"}]});
        let waiting = capture_opening_demand(&root, &queue).await;
        assert_eq!(waiting["waiting_workbooks"], 2);
        assert_eq!(waiting["waiting_sessions"], 1);

        std::fs::write(
            session.join("metrics.json"),
            br#"{"attempt_id":"current","clients":{"original":{"pid":42,"status":"opening"}}}"#,
        )?;
        let opening = capture_opening_demand(&root, &queue).await;
        assert_eq!(opening["waiting_workbooks"], 1);
        assert_eq!(opening["opening_workbooks"], 1);
        assert_eq!(capture_workers(&root, &queue).await.len(), 1);

        std::fs::write(
            session.join("metrics.json"),
            br#"{"attempt_id":"current","clients":{"original":{"pid":42,"status":"running","active_command":"000002","monitor":{"state":"capturing","pane_resources":{"capture_progress":{"command_id":"000002","sheet_name":"Titel","sheet_index":1,"completed":3,"total":7,"updated_unix_ms":123}}}}}}"#,
        )?;
        let workers = capture_workers(&root, &queue).await;
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0]["sheet_name"], "Titel");
        assert_eq!(workers[0]["completed"], 3);
        assert_eq!(workers[0]["total"], 7);

        // A new command and a sheet transition must not erase process progress.
        let mut sweeping = queue.clone();
        sweeping["groups"][0]["status"] = json!("hash_sweeping");
        let mut metrics = json!({"attempt_id":"current","clients":{"original":{
            "pid":42,"process_started_unix_ms":100,"active_command":"recalc",
            "monitor":{"state":"waiting_for_job","pane_resources":{
                "sweep_progress":{"command_id":"capture-1","sheet_name":"Second",
                    "completed":1,"total":6,"pass":1,"passes":2}}}}}});
        std::fs::write(session.join("metrics.json"), serde_json::to_vec(&metrics)?)?;
        metrics["clients"]["original"]["monitor"]["pane_resources"]["capture_progress"] =
            json!({"command_id":"capture-1","sheet_index":2,"completed":3,"total":8});
        metrics["clients"]["original"]["monitor"]["pane_resources"]["sweep_progress"]
            ["sheet_index"] = json!(2);
        std::fs::write(session.join("metrics.json"), serde_json::to_vec(&metrics)?)?;
        let waiting_workers = capture_workers(&root, &sweeping).await;
        assert_eq!(waiting_workers[0]["captures_completed"], 3);
        assert_eq!(waiting_workers[0]["captures_total"], 8);
        assert_eq!(waiting_workers.len(), 1);
        assert_eq!(waiting_workers[0]["completed"], 1);
        assert_eq!(waiting_workers[0]["total"], 6);
        assert_eq!(waiting_workers[0]["sheet_name"], "Second");
        assert_eq!(waiting_workers[0]["unit"], "sheet passes");
        let progress =
            &mut metrics["clients"]["original"]["monitor"]["pane_resources"]["sweep_progress"];
        progress["sheet_name"] = json!("Third");
        progress["sheet_index"] = json!(3);
        progress["completed"] = json!(5);
        progress["pass"] = json!(2);
        std::fs::write(session.join("metrics.json"), serde_json::to_vec(&metrics)?)?;
        let next_sheet = capture_workers(&root, &sweeping).await;
        assert_eq!(next_sheet.len(), 1);
        assert!(next_sheet[0]["captures_total"].is_null());
        assert_eq!(next_sheet[0]["pid"], waiting_workers[0]["pid"]);
        assert_eq!(
            next_sheet[0]["attempt_id"],
            waiting_workers[0]["attempt_id"]
        );
        assert_eq!(next_sheet[0]["completed"], 5);
        assert_eq!(next_sheet[0]["sheet_name"], "Third");
        metrics["clients"]["original"]["closed"] = json!(true);
        std::fs::write(session.join("metrics.json"), serde_json::to_vec(&metrics)?)?;
        assert!(capture_workers(&root, &sweeping).await.is_empty());
        std::fs::write(
            session.join("metrics.json"),
            br#"{"attempt_id":"old","clients":{}}"#,
        )?;
        assert_eq!(
            capture_opening_demand(&root, &queue).await["observed_sessions"],
            0
        );
        assert!(capture_workers(&root, &queue).await.is_empty());
        std::fs::remove_dir_all(output)?;
        Ok(())
    }

    #[test]
    fn preparation_controls_are_bounded_by_available_logical_cores() -> Result<()> {
        let max = logical_cores();
        let controls = merge_controls(
            &defaults(),
            &json!({"prepare_processes": max, "shrink_threads": 1}),
        )?;
        assert_eq!(controls["prepare_processes"], max);
        assert_eq!(controls["shrink_threads"], 1);
        assert!(merge_controls(&controls, &json!({"prepare_processes": 0})).is_err());
        assert!(merge_controls(&controls, &json!({"shrink_threads": 0})).is_err());
        assert!(merge_controls(&controls, &json!({"prepare_processes": max + 1})).is_err());
        assert!(merge_controls(&controls, &json!({"shrink_threads": max + 1})).is_err());
        Ok(())
    }

    #[test]
    fn live_projection_retains_active_selected_and_recent_without_expanding_with_corpus_size() {
        let mut groups: Vec<_> = (0..9000)
            .map(|index| json!({
                "id":format!("book-{index}"), "active":false, "run_id":"run",
                "started_unix_ms":index, "status":"completed"
            }))
            .collect();
        groups[1]["active"] = json!(true);
        groups[2]["active"] = json!(true);
        let mut queue = json!({"run_id":"run", "groups":groups, "open_groups":2});
        retain_live_groups(&mut queue, Some("book-3"));
        let kept = queue["groups"].as_array().unwrap();
        assert_eq!(kept.len(), 11);
        assert_eq!(queue["groups_total"], 9000);
        assert_eq!(queue["open_groups"], 2);
        assert_eq!(queue["groups_truncated"], true);
        for id in ["book-1", "book-2", "book-3", "book-8999"] {
            assert!(kept.iter().any(|group| group["id"] == id));
        }
        assert!(!kept.iter().any(|group| group["id"] == "book-8991"));
    }

    #[tokio::test]
    async fn explicit_deferred_scan_resumes_one_group_but_never_counters_a_stop() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("control-room-deferred-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root)?;
        let path = root.join("control.json");
        let controls = merge_controls(&defaults(), &json!({"target_groups":0}))?;
        std::fs::write(&path, serde_json::to_vec(&controls)?)?;
        let room = Room {
            root: root.clone(),
            web: PathBuf::new(),
            port: 0,
            version: "test".into(),
            started: "test".into(),
            initial_controls: defaults(),
            control_gate: Mutex::new(()),
            manual_capture: Mutex::new(None),
            snapshots: snapshot_cache::SnapshotCache::default(),
        };
        let queue = json!({"running":true,"runner_alive":true,
            "runner_pid":42,"runner_started_unix_ms":100});
        resume_for_deferred_scan(&room, &queue)
            .await
            .map_err(|error| anyhow::anyhow!("resume failed: {error:?}"))?;
        assert_eq!(
            read_object(&path, SNAPSHOT_LIMIT).await?["target_groups"],
            1
        );

        std::fs::write(&path, serde_json::to_vec(&controls)?)?;
        std::fs::write(
            root.join("stop-request.json"),
            serde_json::to_vec(&json!({"runner_pid":42,"runner_started_unix_ms":100}))?,
        )?;
        assert!(resume_for_deferred_scan(&room, &queue).await.is_err());
        assert_eq!(
            read_object(&path, SNAPSHOT_LIMIT).await?["target_groups"],
            0
        );
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn download_streams_an_aliased_prepared_variant() -> Result<()> {
        let base =
            std::env::temp_dir().join(format!("control-room-download-{}", uuid::Uuid::new_v4()));
        let root = base.join("study/control-room");
        let prepared = base.join("study/prepared");
        std::fs::create_dir_all(&root)?;
        std::fs::create_dir_all(&prepared)?;
        std::fs::write(
            root.join("queue.json"),
            serde_json::to_vec(&json!({"run_id":"run-a","groups":[
                {"id":"book","run_id":"run-a","status":"prepared","active":false}
            ]}))?,
        )?;
        let mut bundle = zip::ZipWriter::new(std::fs::File::create(prepared.join("book.zip"))?);
        bundle.start_file("original.xlsx", zip::write::FileOptions::default())?;
        bundle.write_all(b"prepared bytes")?;
        bundle.start_file("preparation.json", zip::write::FileOptions::default())?;
        bundle.write_all(
            br#"{"version":2,"variants":[{"profile":"default","member":"original.xlsx"}]}"#,
        )?;
        bundle.finish()?;
        let room = Arc::new(Room {
            root,
            web: PathBuf::new(),
            port: 0,
            version: "test".into(),
            started: "test".into(),
            initial_controls: defaults(),
            control_gate: Mutex::new(()),
            manual_capture: Mutex::new(None),
            snapshots: snapshot_cache::SnapshotCache::default(),
        });
        let response = download_variant(State(room), RoutePath(("book".into(), "default".into())))
            .await
            .map_err(|error| anyhow::anyhow!("download failed: {error:?}"))?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-length"], "14");
        let body = axum::body::to_bytes(response.into_body(), 1024).await?;
        assert_eq!(&body[..], b"prepared bytes");
        std::fs::remove_dir_all(base)?;
        Ok(())
    }

    #[tokio::test]
    async fn active_preparation_profile_comes_from_the_current_worker_attempt() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("control-room-profile-{}", uuid::Uuid::new_v4()));
        let worker = root.join("groups/group-a/coordinator.json");
        std::fs::create_dir_all(worker.parent().unwrap())?;
        let mut queue = json!({"run_id":"run-a","groups":[{"id":"group-a",
            "workbook":"book.xlsx","run_id":"run-a","attempt_id":"attempt-a",
            "status":"preparing","active":true}],
            "progress":{"active_preparation":[{"id":"source-a","workbook":"book.xlsx"}]}});
        let worker_state = json!({"attempt_id":"attempt-a","phase":"shrinking",
            "last_event":{"kind":"profile_started","details":{"profile":"maximum-cleaning"}}});
        std::fs::write(&worker, serde_json::to_vec(&worker_state)?)?;
        attach_active_preparation_profile(&root, &mut queue, &mut Vec::new()).await;
        assert_eq!(
            queue["progress"]["active_preparation"][0]["profile"],
            "maximum-cleaning"
        );

        queue["progress"]["active_preparation"][0]
            .as_object_mut()
            .unwrap()
            .remove("profile");
        let mut stale = worker_state;
        stale["attempt_id"] = json!("previous-attempt");
        std::fs::write(&worker, serde_json::to_vec(&stale)?)?;
        attach_active_preparation_profile(&root, &mut queue, &mut Vec::new()).await;
        assert!(queue["progress"]["active_preparation"][0]["profile"].is_null());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

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
        for status in ["crc-identical", "paused-for-later"] {
            let record = json!({"status":status,"error":"Evidence remains incomplete",
                "control_room_attempt":{"id":"current"}});
            let mut group = json!({"status":status});
            attach_outcome(&mut group, &record, Some("current"));
            assert_eq!(group["outcome_error"], "Evidence remains incomplete");
        }
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

    #[test]
    fn retired_excel_limit_does_not_gate_or_reset_sweep_concurrency() -> Result<()> {
        let mut legacy = defaults();
        legacy["excel_process_limit"] = json!(1);
        legacy["sweep_processes"] = json!(7);
        let next = merge_controls(
            &legacy,
            &json!({"excel_process_limit":32,"sweep_processes":12}),
        )?;
        assert!(next.get("excel_process_limit").is_none());
        assert_eq!(next["sweep_processes"], 12);
        let reloaded = merge_controls(&defaults(), &legacy)?;
        assert!(reloaded.get("excel_process_limit").is_none());
        assert_eq!(reloaded["sweep_processes"], 7);
        assert!(merge_controls(&defaults(), &json!({"sweep_processes":0})).is_err());
        Ok(())
    }
    #[test]
    fn crc_equal_sheet_threshold_is_a_live_control() -> Result<()> {
        let current = defaults();
        assert_eq!(current["crc_equal_sheet_threshold_mib"], 256);
        let selected = merge_controls(&current, &json!({"crc_equal_sheet_threshold_mib": 512}))?;
        assert_eq!(selected["crc_equal_sheet_threshold_mib"], 512);
        assert!(merge_controls(&current, &json!({"crc_equal_sheet_threshold_mib": 0})).is_err());
        Ok(())
    }
}
