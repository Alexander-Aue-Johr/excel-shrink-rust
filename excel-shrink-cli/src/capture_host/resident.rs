//! HTTP transport for resident Office.js clients. Each client serializes its own
//! evidence; the dispatch gate only coordinates admission and launcher resets.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{ensure, Context};
use axum::body::Bytes;
use axum::extract::{Path as RoutePath, Query, Request, State};
use axum::http::{HeaderMap, Method};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

use super::{
    now_iso, now_ms, read_json, serve, with_web, write_bytes, write_json, ApiError, HttpResult,
};

#[path = "resident_telemetry.rs"]
mod telemetry;
use telemetry::WorkerTelemetry;
#[path = "resident_events.rs"]
mod events;

const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
const STARTUP_QUIET_PERIOD: Duration = Duration::from_secs(2);
type JsonResponse = HttpResult<Json<Value>>;
type SharedClient = Arc<Mutex<Client>>;
type Parameters = Query<HashMap<String, String>>;

struct Host {
    session: PathBuf,
    launcher_token: Option<String>,
    control_root: Option<PathBuf>,
    primary_clients: Vec<String>,
    clients: RwLock<HashMap<String, SharedClient>>,
    dispatch: Mutex<Dispatch>,
    events: events::Changes,
    coordinator_token: Option<String>,
}

struct Dispatch {
    resetting: HashSet<String>,
    paused: bool,
}

struct Client {
    token: String,
    document_id: String,
    run_id: String,
    expected_workbook: Option<String>,
    ready_at: Option<String>,
    startup_sampled: bool,
    seen_at: String,
    last_activity_at: String,
    last_heartbeat_at: Option<String>,
    configured_at: Instant,
    data_started: bool,
    active_command: Option<String>,
    active_payload: Value,
    error: Value,
    command_count: u64,
    heartbeat_count: u64,
    last_result: Option<(String, String)>,
    telemetry: WorkerTelemetry,
}

impl Client {
    fn new() -> Self {
        Self {
            token: String::new(),
            document_id: String::new(),
            run_id: String::new(),
            expected_workbook: None,
            ready_at: None,
            startup_sampled: false,
            seen_at: now_iso(),
            last_activity_at: now_iso(),
            last_heartbeat_at: None,
            configured_at: Instant::now(),
            data_started: false,
            active_command: None,
            active_payload: Value::Null,
            error: Value::Null,
            command_count: 0,
            heartbeat_count: 0,
            last_result: None,
            telemetry: WorkerTelemetry::new(),
        }
    }

    fn snapshot(&self) -> Value {
        json!({
            "seen_at": self.seen_at,
            "last_activity_at": self.last_activity_at,
            "last_heartbeat_at": self.last_heartbeat_at,
            "data_started": self.data_started,
            "active_command": self.active_command,
            "command_count": self.command_count,
            "heartbeat_count": self.heartbeat_count,
            "ready_at": self.ready_at,
            "startup_sampled": self.startup_sampled,
            "runtime": {"document_id": self.document_id, "run_id": self.run_id},
            "error": self.error,
            "monitor": self.telemetry.snapshot(),
        })
    }
}

pub async fn run(settings_path: &Path) -> anyhow::Result<()> {
    let settings = read_json(settings_path).await?;
    let session = std::path::absolute(
        settings["session"]
            .as_str()
            .context("Missing session path")?,
    )?;
    let web = PathBuf::from(settings["web"].as_str().context("Missing web path")?);
    let port = settings["port"].as_u64().context("Missing host port")?;
    ensure!(
        (1024..=65535).contains(&port) && web.is_dir(),
        "Invalid host settings"
    );
    tokio::fs::create_dir_all(&session).await?;
    let coordinator_token = match read_json(&session.join("coordinator-token.json")).await {
        Ok(value) => value["token"]
            .as_str()
            .filter(|token| !token.is_empty())
            .map(str::to_owned),
        Err(_) if !session.join("coordinator-token.json").exists() => None,
        Err(error) => return Err(error),
    };
    let primary_clients = json_files(&session.join("clients"))
        .await?
        .into_iter()
        .filter_map(|path| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .collect();
    let host = Arc::new(Host {
        session,
        launcher_token: settings["launcher_token"].as_str().map(str::to_owned),
        control_root: std::env::var_os("EXCEL_SHRINK_CONTROL_ROOM_DIR")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from),
        primary_clients,
        clients: RwLock::new(HashMap::new()),
        dispatch: Mutex::new(Dispatch {
            resetting: HashSet::new(),
            paused: true,
        }),
        events: events::Changes::new(),
        coordinator_token,
    });
    let _watcher = events::watch_files(&host)?;
    write_json(
        &host.session.join("events.json"),
        &json!({
            "version": 1, "url": format!("ws://127.0.0.1:{port}/events")
        }),
        true,
    )
    .await?;
    let router = Router::new()
        .route("/events", get(events::upgrade))
        .route("/health", get(health))
        .route("/config", get(configure))
        .route("/ready", post(ready))
        .route("/heartbeat", get(heartbeat).post(heartbeat))
        .route("/job", get(next_job))
        .route("/launcher/startup-sampled/{id}", post(startup_sampled))
        .route("/launcher/reset/{id}", post(reset))
        .route("/image/{id}/{command}/{sheet}/{tile}", post(image))
        .route("/result/{id}/{command}", post(result))
        .with_state(host);
    serve(with_web(router, &web, MAX_IMAGE_BYTES), port as u16).await
}

impl Host {
    async fn find_client(&self, id: &str) -> HttpResult<SharedClient> {
        self.clients
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| ApiError::bad_request("Unknown or retired resident client session."))
    }

    async fn authenticated(
        &self,
        id: &str,
        headers: &HeaderMap,
    ) -> HttpResult<OwnedMutexGuard<Client>> {
        safe_id(id)?;
        let shared = self.find_client(id).await?;
        let client = shared.clone().lock_owned().await;
        // Reset/configuration can retire a session while a request body is being
        // read or while this lock is awaited. Authenticate at the mutation boundary.
        let registered = self.clients.read().await;
        if !registered
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, &shared))
            || client.token.is_empty()
            || header(headers, "X-Client-Token") != Some(client.token.as_str())
        {
            return Err(ApiError::bad_request(
                "Unknown or retired resident client session.",
            ));
        }
        Ok(client)
    }

    fn authenticate_launcher(&self, headers: &HeaderMap) -> HttpResult<()> {
        if self
            .launcher_token
            .as_deref()
            .filter(|token| !token.is_empty())
            != header(headers, "X-Launcher-Token").filter(|token| !token.is_empty())
            || self
                .launcher_token
                .as_deref()
                .unwrap_or_default()
                .is_empty()
        {
            return Err(ApiError::forbidden("Invalid launcher token."));
        }
        Ok(())
    }

    fn response_path(&self, client: &str, command: &str) -> PathBuf {
        self.session
            .join("responses")
            .join(client)
            .join(format!("{command}.json"))
    }

    fn image_path(&self, client: &str, command: &str, sheet: i64, tile: i64) -> PathBuf {
        self.session
            .join("images")
            .join(client)
            .join(command)
            .join(format!("s{sheet:04}-t{tile:05}.png"))
    }

    async fn write_response(&self, client: &str, command: &str, value: &Value) -> HttpResult<()> {
        let path = self.response_path(client, command);
        if !path.try_exists()? {
            write_json(&path, value, false).await?;
            self.events.changed();
        }
        Ok(())
    }
}

async fn health(State(host): State<Arc<Host>>) -> Json<Value> {
    let clients = host.clients.read().await.clone();
    let mut snapshots = serde_json::Map::new();
    let mut errors = Vec::new();
    if let Some(error) = host.events.error() {
        errors.push(json!({"code": "EVENT_WATCH_FAILED", "message": error}));
    }
    for (id, shared) in clients {
        let client = shared.lock().await;
        if !client.error.is_null() {
            errors.push(json!({"client_id": id, "code": client.error["code"], "message": client.error["message"]}));
        }
        snapshots.insert(id, client.snapshot());
    }
    let mut seen: Vec<_> = snapshots.keys().cloned().collect();
    seen.sort();
    Json(json!({"ok": true, "seen": seen, "errors": errors, "clients": snapshots}))
}

async fn configure(State(host): State<Arc<Host>>, Query(query): Parameters) -> JsonResponse {
    let id = query_id(&query, "client")?;
    let document = query_id(&query, "document")?;
    let run = query_id(&query, "run")?;
    let request = query_id(&query, "request")?;
    let dispatch = host.dispatch.lock().await;
    if dispatch.resetting.contains(id) {
        return Err(ApiError::conflict("Excel client is being reset."));
    }
    let path = host.session.join("clients").join(format!("{id}.json"));
    if !path.try_exists()? {
        return Err(ApiError::not_found("Unknown resident client."));
    }
    let mut config = read_json(&path).await?;
    if config["client_id"].as_str() != Some(id) {
        return Err(ApiError::bad_request(
            "Client configuration identity differs.",
        ));
    }
    let shared = host
        .clients
        .write()
        .await
        .entry(id.to_owned())
        .or_insert_with(|| Arc::new(Mutex::new(Client::new())))
        .clone();
    drop(dispatch);
    let mut client = shared.lock().await;
    if !host
        .clients
        .read()
        .await
        .get(id)
        .is_some_and(|current| Arc::ptr_eq(current, &shared))
    {
        return Err(ApiError::conflict("Excel client is being reset."));
    }
    let same_document = client.document_id == document;
    let same_run = same_document && client.run_id == run;
    let outcome = if same_run {
        "repeated_request"
    } else if same_document {
        "duplicate_run"
    } else {
        "new_document"
    };
    client
        .telemetry
        .configuration_attempt(document, run, request, outcome);
    if same_run {
        config["client_token"] = json!(client.token);
        return Ok(Json(config));
    }
    if same_document {
        return Err(ApiError::conflict(
            "This document already has a resident worker run.",
        ));
    }
    if client.data_started {
        if client.error.is_null() {
            client.error = failure("TASKPANE_CONTEXT_CHANGED", "A different taskpane document requested configuration after work started; the cause is not yet known.");
        }
        client.telemetry.restarted();
        host.events.changed();
        if let Some(command) = &client.active_command {
            host.write_response(id, command, &json!({"ok": false, "error": client.error}))
                .await?;
        }
        return Err(ApiError::conflict(
            "Resident taskpane document changed after work started.",
        ));
    }
    client.token = uuid::Uuid::new_v4().simple().to_string();
    client.document_id = document.to_owned();
    client.run_id = run.to_owned();
    client.expected_workbook = config["workbook"].as_str().map(str::to_owned);
    client.ready_at = None;
    client.startup_sampled = false;
    client.configured_at = Instant::now();
    client.last_activity_at = now_iso();
    client.telemetry.configured();
    host.events.changed();
    let changes = host.events.clone();
    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_QUIET_PERIOD).await;
        changes.changed();
    });
    config["client_token"] = json!(client.token);
    Ok(Json(config))
}

async fn ready(
    State(host): State<Arc<Host>>,
    Query(query): Parameters,
    headers: HeaderMap,
    body: Bytes,
) -> JsonResponse {
    let id = query_id(&query, "client")?;
    let observation = parse_object(&body)?;
    let mut client = host.authenticated(id, &headers).await?;
    if !client.error.is_null() {
        return Err(ApiError::conflict("Resident client has failed."));
    }
    if observation["ok"] == false {
        client.error = observation
            .get("error")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| {
                failure(
                    "ADDIN_INITIALIZATION_FAILED",
                    "Workbook verification failed.",
                )
            });
        let error = client.error.clone();
        client.telemetry.finished(false, &error);
        host.events.changed();
        return Ok(Json(json!({"ok": false})));
    }
    if observation["workbook"].as_str() != client.expected_workbook.as_deref() {
        return Err(ApiError::bad_request(
            "Ready workbook identity differs from the staged workbook.",
        ));
    }
    if client.ready_at.is_none() {
        let stamp = now_iso();
        client.ready_at = Some(stamp.clone());
        client.last_activity_at = stamp;
        client.telemetry.ready();
        host.events.changed();
    }
    Ok(Json(json!({"ok": true, "ready_at": client.ready_at})))
}

async fn startup_sampled(
    State(host): State<Arc<Host>>,
    RoutePath(id): RoutePath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> JsonResponse {
    safe_id(&id)?;
    host.authenticate_launcher(&headers)?;
    let observation = parse_object(&body)?;
    let shared = host
        .clients
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(|| ApiError::not_found("Unknown resident client."))?;
    let mut client = shared.lock().await;
    if client.ready_at.is_none() || observation["ready_at"].as_str() != client.ready_at.as_deref() {
        return Err(ApiError::conflict(
            "Startup sample belongs to a different ready observation.",
        ));
    }
    client.startup_sampled = true;
    host.events.changed();
    Ok(Json(json!({"ok": true})))
}

async fn heartbeat(
    State(host): State<Arc<Host>>,
    Query(query): Parameters,
    request: Request,
) -> JsonResponse {
    let id = query_id(&query, "client")?;
    let headers = request.headers().clone();
    let sample = if request.method() == Method::POST {
        let length =
            header(&headers, "Content-Length").and_then(|value| value.parse::<usize>().ok());
        if !length.is_some_and(|length| length <= 8192) {
            return Err(ApiError::bad_request(
                "Pane diagnostics must be a bounded snapshot.",
            ));
        }
        let body = axum::body::to_bytes(request.into_body(), 8192)
            .await
            .map_err(ApiError::bad_request)?;
        Some(parse_object(&body)?)
    } else {
        None
    };
    let mut client = host.authenticated(id, &headers).await?;
    let stamp = now_iso();
    client.last_heartbeat_at = Some(stamp.clone());
    client.last_activity_at = stamp;
    client.heartbeat_count += 1;
    if let Some(sample) = sample {
        client.telemetry.sample_pane(&sample);
    }
    Ok(Json(json!({"ok": true})))
}

async fn next_job(
    State(host): State<Arc<Host>>,
    Query(query): Parameters,
    headers: HeaderMap,
) -> JsonResponse {
    let id = query_id(&query, "client")?;
    dispatch_job(&host, id, &headers, false).await.map(Json)
}

async fn dispatch_job(
    host: &Host,
    id: &str,
    headers: &HeaderMap,
    replay_active: bool,
) -> HttpResult<Value> {
    let mut dispatch = host.dispatch.lock().await;
    let mut client = host.authenticated(id, headers).await?;
    if !dispatch.resetting.is_empty() {
        return Ok(json!({"idle": true, "resetting_client": true}));
    }
    if !client.error.is_null() {
        return Err(ApiError::conflict(client.error.to_string()));
    }
    client.last_activity_at = now_iso();
    // Polling an active command must not turn its telemetry back into idle,
    // including while dispatch of subsequent work is paused.
    if client.active_command.is_some() {
        if replay_active {
            return Ok(client.active_payload.clone());
        }
        return Err(ApiError::conflict("A command is already in progress."));
    }
    if client.ready_at.is_none() || !client.startup_sampled {
        client.telemetry.transition(
            "waiting_for_startup_sample",
            Some("launcher"),
            "Waiting for workbook verification and the launcher's memory sample.",
        );
        return Ok(json!({"idle": true, "waiting_for_startup_sample": true}));
    }
    if let Some(root) = &host.control_root {
        // Once this barrier passed, resetting a primary client requires a new
        // group. Later jobs must not wait for peer locks held by image uploads.
        if !client.data_started {
            for peer in &host.primary_clients {
                if peer == id {
                    continue;
                }
                let shared = host.clients.read().await.get(peer).cloned();
                let sampled = match shared {
                    Some(shared) => shared.lock().await.startup_sampled,
                    None => false,
                };
                if !sampled {
                    client.telemetry.transition(
                        "waiting_for_peers",
                        Some("primary_addins"),
                        "Waiting for the other primary add-ins to connect.",
                    );
                    return Ok(json!({"idle": true, "waiting_for_peers": true}));
                }
            }
        }
        // A transient control-file read cannot silently resume a paused run.
        if let Ok(controls) = read_json(&root.join("control.json")).await {
            if let Some(paused) = controls["paused"].as_bool() {
                dispatch.paused = paused;
            }
        }
    }
    if host.control_root.is_some() && dispatch.paused && !client.data_started {
        return Ok(paused_job(&mut client));
    }
    if !client.data_started && client.configured_at.elapsed() < STARTUP_QUIET_PERIOD {
        client.telemetry.transition(
            "settling",
            Some("startup_quiet_period"),
            "Waiting for the startup quiet period.",
        );
        return Ok(json!({"idle": true, "settling": true}));
    }
    for path in json_files(&host.session.join("commands").join(id)).await? {
        let command_id = path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ApiError::bad_request("Invalid command filename."))?;
        safe_id(command_id)?;
        if host.response_path(id, command_id).try_exists()? {
            continue;
        }
        let command = read_json(&path).await?;
        if command["id"].as_str() != Some(command_id) {
            return Err(ApiError::bad_request("Command filename and id differ."));
        }
        if !matches!(
            command["kind"].as_str(),
            Some(
                "capture"
                    | "plan"
                    | "capture_images"
                    | "capture_hashes"
                    | "images"
                    | "upload_retained"
                    | "discard_retained"
                    | "release"
                    | "diagnosis"
            )
        ) {
            return Err(ApiError::bad_request("Unknown resident command."));
        }
        // Pausing stops new Office captures. Already captured PNGs may still
        // be uploaded and released so a paused group can drain its buffers.
        if host.control_root.is_some() && dispatch.paused && !drains_retained_images(&command) {
            return Ok(paused_job(&mut client));
        }
        let receipt = host
            .session
            .join("dispatched")
            .join(id)
            .join(format!("{command_id}.json"));
        write_json(&receipt, &json!({"dispatched_unix_ms": now_ms()}), false).await?;
        client.active_command = Some(command_id.to_owned());
        client.active_payload = command.clone();
        client.data_started = true;
        client.command_count += 1;
        client.telemetry.started(&command);
        host.events.changed();
        return Ok(command);
    }
    if host.control_root.is_some() && dispatch.paused {
        return Ok(paused_job(&mut client));
    }
    client.telemetry.transition(
        "waiting_for_job",
        Some("coordinator"),
        "Waiting for the next coordinator command.",
    );
    Ok(json!({"idle": true}))
}

fn drains_retained_images(command: &Value) -> bool {
    matches!(
        command["kind"].as_str(),
        Some("upload_retained" | "discard_retained" | "release")
    )
}

fn paused_job(client: &mut Client) -> Value {
    client.telemetry.transition(
        "paused",
        Some("capture_pause"),
        "Capture dispatch is paused by the user.",
    );
    json!({"idle": true, "paused": true})
}

async fn reset(
    State(host): State<Arc<Host>>,
    RoutePath(id): RoutePath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> JsonResponse {
    safe_id(&id)?;
    host.authenticate_launcher(&headers)?;
    let request = parse_object(&body)?;
    let mut dispatch = host.dispatch.lock().await;
    if !host
        .session
        .join("clients")
        .join(format!("{id}.json"))
        .try_exists()?
    {
        return Err(ApiError::not_found("Unknown resident client."));
    }
    if request["stage"] == "ready" {
        dispatch.resetting.remove(&id);
        host.events.changed();
        return Ok(Json(json!({"ok": true})));
    }
    if request["stage"] != "prepare" {
        return Err(ApiError::bad_request("Unknown reset stage."));
    }
    let peers: Vec<_> = host.clients.read().await.values().cloned().collect();
    for peer in peers {
        if peer.lock().await.data_started {
            return Err(ApiError::conflict(
                "Comparison has started; restart the group to discard its prior evidence.",
            ));
        }
    }
    dispatch.resetting.insert(id.clone());
    host.clients.write().await.remove(&id);
    host.events.changed();
    Ok(Json(json!({"ok": true})))
}

async fn image(
    State(host): State<Arc<Host>>,
    RoutePath((id, command, sheet, tile)): RoutePath<(String, String, i64, i64)>,
    request: Request,
) -> JsonResponse {
    safe_id(&command)?;
    // Acquire the client before receiving its PNG. Concurrent uploads from this
    // client must not queue additional full-size image buffers in the host.
    let mut client = host.authenticated(&id, request.headers()).await?;
    let image_first = client.active_payload["kind"] == "capture_images";
    if client.active_command.as_deref() != Some(&command)
        || (!image_first
            && !matches!(
                client.active_payload["kind"].as_str(),
                Some("images" | "upload_retained")
            ))
    {
        return Err(ApiError::conflict(
            "Image does not belong to the active evidence command.",
        ));
    }
    if !client.error.is_null() {
        return Err(ApiError::conflict("Resident client has failed."));
    }
    let expected = requested_tile(&client.active_payload, sheet, tile)?;
    let expected_sha = expected["sha256"].as_str();
    if !image_first
        && (!expected_sha.is_some_and(valid_sha)
            || header(request.headers(), "X-Sha256") != expected_sha)
    {
        return Err(ApiError::bad_request(
            "Image checksum header differs from request.",
        ));
    }
    let receiving = Instant::now();
    let bytes = axum::body::to_bytes(request.into_body(), MAX_IMAGE_BYTES)
        .await
        .map_err(ApiError::bad_request)?;
    let upload_receive_ms = receiving.elapsed().as_secs_f64() * 1000.0;
    let publishing = Instant::now();
    let (width, height) = png_size(&bytes)?;
    let sha = format!("{:x}", Sha256::digest(&bytes));
    if expected["width"].as_i64() != Some(width)
        || expected["height"].as_i64() != Some(height)
        || (!image_first && expected_sha != Some(sha.as_str()))
    {
        return Err(ApiError::bad_request(
            "Evidence image differs from the offered checksum or dimensions.",
        ));
    }
    let path = host.image_path(&id, &command, sheet, tile);
    if path.try_exists()? {
        return Err(ApiError::bad_request("Duplicate evidence image."));
    }
    write_bytes(&path, &bytes, false).await?;
    let mut receipt = expected.clone();
    client.last_activity_at = now_iso();
    client.telemetry.image_received(&receipt);
    receipt["ok"] = json!(true);
    receipt["image_path"] = json!(path);
    receipt["sha256"] = json!(sha);
    receipt["png_bytes"] = json!(bytes.len());
    receipt["received_unix_ms"] = json!(now_ms());
    receipt["timings"] = json!({
        "upload_receive_ms": upload_receive_ms,
        "upload_validate_publish_ms": publishing.elapsed().as_secs_f64() * 1000.0,
    });
    if image_first || client.active_payload["kind"] == "upload_retained" {
        let path = host
            .session
            .join("receipts")
            .join(&id)
            .join(&command)
            .join(format!("s{sheet:04}-t{tile:05}.json"));
        write_json(&path, &receipt, false).await?;
        host.events.changed();
    }
    Ok(Json(receipt))
}

async fn result(
    State(host): State<Arc<Host>>,
    RoutePath((id, command)): RoutePath<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> JsonResponse {
    safe_id(&command)?;
    let result = parse_object(&body)?;
    let mut client = host.authenticated(&id, &headers).await?;
    let result_digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&result)?));
    // A connection can disappear after publication but before the HTTP ACK.
    // Retry only the identical last result from this authenticated pane session.
    if let Some((completed, digest)) = &client.last_result {
        if completed == &command {
            if digest != &result_digest {
                return Err(ApiError::conflict(
                    "Result retry differs from the published result.",
                ));
            }
            return Ok(Json(json!({"ok": true, "repeated": true})));
        }
    }
    if client.active_command.as_deref() != Some(&command) {
        return Err(ApiError::conflict("Command is no longer active."));
    }
    let response_path = host.response_path(&id, &command);
    let already_published = response_path.try_exists()?;
    if already_published && read_json(&response_path).await? != result {
        return Err(ApiError::conflict(
            "Result retry differs from the published result.",
        ));
    }
    if !client.error.is_null() && !already_published {
        return Err(ApiError::conflict("Resident client has failed."));
    }
    let ok = result["ok"]
        .as_bool()
        .ok_or_else(|| ApiError::bad_request("Missing result status."))?;
    // Publication can finish before a canceled request updates client state.
    // Its consumer may already have removed equal PNGs, so only a new result
    // validates evidence again. The existing result must match exactly above.
    if !already_published && ok && client.active_payload["kind"] == "capture_hashes" {
        validate_hashes(&client.active_payload, &result)?;
    }
    if !already_published
        && ok
        && matches!(
            client.active_payload["kind"].as_str(),
            Some("images" | "capture_images" | "upload_retained")
        )
    {
        validate_receipts(&host, &id, &command, &client.active_payload, &result).await?;
    }
    if !ok {
        client.error = result
            .get("error")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| failure("CAPTURE_FAILED", "Capture failed."));
    }
    host.write_response(&id, &command, &result).await?;
    client.last_result = Some((command.clone(), result_digest));
    if result["pane_resources"].is_object() {
        client.telemetry.sample_pane(&result["pane_resources"]);
    }
    let error = client.error.clone();
    client.telemetry.finished(ok, &error);
    client.active_command = None;
    client.active_payload = Value::Null;
    client.last_activity_at = now_iso();
    host.events.changed();
    Ok(Json(json!({"ok": true})))
}

fn validate_hashes(payload: &Value, result: &Value) -> HttpResult<()> {
    let requested = payload["tiles"]
        .as_array()
        .ok_or_else(|| ApiError::bad_request("No requested checksum tiles."))?;
    let tiles = result["tiles"]
        .as_array()
        .ok_or_else(|| ApiError::bad_request("Missing checksum tiles."))?;
    if requested.is_empty() || requested.len() > 16 || tiles.len() != requested.len() {
        return Err(ApiError::bad_request("Incomplete checksum result."));
    }
    let mut used = HashSet::new();
    for tile in tiles {
        let sheet = tile["sheet_index"]
            .as_i64()
            .filter(|index| *index >= 0)
            .ok_or_else(|| ApiError::bad_request("Invalid checksum sheet."))?;
        let index = tile["tile_index"]
            .as_i64()
            .filter(|index| *index >= 0)
            .ok_or_else(|| ApiError::bad_request("Invalid checksum tile."))?;
        let expected = requested_tile(payload, sheet, index)?;
        if !used.insert((sheet, index))
            || !tile["sha256"].as_str().is_some_and(valid_sha)
            || !tile["png_bytes"]
                .as_u64()
                .is_some_and(|bytes| bytes > 0 && bytes <= MAX_IMAGE_BYTES as u64)
            || [
                "sheet_name",
                "range",
                "core_rows",
                "x",
                "y",
                "width",
                "height",
                "capture_pixels",
                "capture_right_column",
            ]
            .iter()
            .any(|key| expected[*key].is_null() || tile[*key] != expected[*key])
        {
            return Err(ApiError::bad_request(
                "Checksum tile differs from the capture plan.",
            ));
        }
    }
    Ok(())
}

async fn validate_receipts(
    host: &Host,
    id: &str,
    command: &str,
    payload: &Value,
    result: &Value,
) -> HttpResult<()> {
    let requested = payload["tiles"]
        .as_array()
        .ok_or_else(|| ApiError::bad_request("No requested evidence tiles."))?;
    let images = result["images"]
        .as_array()
        .ok_or_else(|| ApiError::bad_request("Missing evidence receipts."))?;
    if images.len() != requested.len() {
        return Err(ApiError::bad_request("Incomplete evidence result."));
    }
    let mut used = HashSet::new();
    for image in images {
        let sheet = image["sheet_index"]
            .as_i64()
            .ok_or_else(|| ApiError::bad_request("Invalid receipt sheet."))?;
        let tile = image["tile_index"]
            .as_i64()
            .ok_or_else(|| ApiError::bad_request("Invalid receipt tile."))?;
        let expected = requested_tile(payload, sheet, tile)?;
        let path = host.image_path(id, command, sheet, tile);
        if !used.insert(path.clone())
            || !path.try_exists()?
            || image["image_path"].as_str() != path.to_str()
            || image["width"] != expected["width"]
            || image["height"] != expected["height"]
        {
            return Err(ApiError::bad_request("Invalid evidence receipt."));
        }
        let sha = if payload["kind"] == "capture_images" {
            format!("{:x}", Sha256::digest(tokio::fs::read(&path).await?))
        } else {
            expected["sha256"].as_str().unwrap_or_default().to_owned()
        };
        if image["sha256"].as_str() != Some(&sha) {
            return Err(ApiError::bad_request("Invalid evidence receipt."));
        }
    }
    Ok(())
}

fn requested_tile(payload: &Value, sheet: i64, tile: i64) -> HttpResult<&Value> {
    let tiles = payload["tiles"]
        .as_array()
        .ok_or_else(|| ApiError::bad_request("No requested evidence tiles."))?;
    let mut matches = tiles.iter().filter(|item| {
        item["sheet_index"].as_i64() == Some(sheet) && item["tile_index"].as_i64() == Some(tile)
    });
    let expected = matches
        .next()
        .ok_or_else(|| ApiError::bad_request("Unrequested evidence tile."))?;
    if matches.next().is_some() {
        return Err(ApiError::bad_request("Duplicate requested evidence tile."));
    }
    Ok(expected)
}

fn png_size(bytes: &[u8]) -> HttpResult<(i64, i64)> {
    if bytes.len() < 45
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || &bytes[12..16] != b"IHDR"
        || &bytes[bytes.len() - 8..bytes.len() - 4] != b"IEND"
    {
        return Err(ApiError::bad_request("Invalid or truncated evidence PNG."));
    }
    let width = i64::from(u32::from_be_bytes(bytes[16..20].try_into().unwrap()));
    let height = i64::from(u32::from_be_bytes(bytes[20..24].try_into().unwrap()));
    if width < 1 || height < 1 || width > 30_000 || height > 30_000 || width * height > 64_000_000 {
        return Err(ApiError::bad_request(
            "Evidence PNG exceeds the tile budget.",
        ));
    }
    Ok((width, height))
}

async fn json_files(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut entries = match tokio::fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut paths = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_file()
            && entry.path().extension().is_some_and(|ext| ext == "json")
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn parse_object(body: &[u8]) -> HttpResult<Value> {
    let value: Value = serde_json::from_slice(body)?;
    if !value.is_object() {
        return Err(ApiError::bad_request("Expected a JSON object."));
    }
    Ok(value)
}

fn safe_id(value: &str) -> HttpResult<()> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 96
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-'))
    {
        return Err(ApiError::bad_request("Invalid resident identifier."));
    }
    Ok(())
}

fn query_id<'a>(query: &'a HashMap<String, String>, name: &str) -> HttpResult<&'a str> {
    let value = query.get(name).map(String::as_str).unwrap_or_default();
    safe_id(value)?;
    Ok(value)
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn valid_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
}

fn failure(code: &str, message: &str) -> Value {
    json!({"code": code, "message": message})
}

#[cfg(test)]
mod diagnosis_dispatch_tests {
    use super::*;

    #[tokio::test]
    async fn diagnosis_commands_reach_ready_clients_through_the_real_dispatch_gate() {
        let root =
            std::env::temp_dir().join(format!("diagnosis-dispatch-{}", uuid::Uuid::new_v4()));
        let command_path = root.join("commands/original/000008.json");
        std::fs::create_dir_all(command_path.parent().unwrap()).unwrap();
        std::fs::write(
            &command_path,
            serde_json::to_vec(&json!({
                "id":"000008","kind":"diagnosis","operation":"recalculate"
            }))
            .unwrap(),
        )
        .unwrap();
        let mut client = Client::new();
        client.token = "fixture-token".into();
        client.ready_at = Some(now_iso());
        client.startup_sampled = true;
        client.configured_at = Instant::now() - Duration::from_secs(10);
        let shared = Arc::new(Mutex::new(client));
        let host = Host {
            session: root.clone(),
            launcher_token: None,
            control_root: None,
            primary_clients: vec!["original".into()],
            clients: RwLock::new(HashMap::from([("original".into(), shared.clone())])),
            dispatch: Mutex::new(Dispatch {
                resetting: HashSet::new(),
                paused: false,
            }),
            events: events::Changes::new(),
            coordinator_token: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert("X-Client-Token", "fixture-token".parse().unwrap());
        let result = dispatch_job(&host, "original", &headers, false)
            .await
            .expect("the Rust host must dispatch the diagnostic command");
        assert_eq!(result["kind"], "diagnosis");
        assert_eq!(result["operation"], "recalculate");
        assert!(root.join("dispatched/original/000008.json").is_file());
        assert_eq!(shared.lock().await.command_count, 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
