use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Json, Router,
    body::Body,
    extract::{Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_server::tls_rustls::RustlsConfig;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use excel_com::{ComApartment, OwnedApplication, SaveChanges, WorkbookCloseOptions};
use image::{ImageReader, Rgba, RgbaImage};
use rcgen::generate_simple_self_signed;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    env,
    fs,
    io::{Cursor, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinHandle;

#[derive(Clone)]
struct Config {
    experiment: String,
    strategy: String,
    host_kind: String,
    static_root: PathBuf,
    file_list: PathBuf,
    output: PathBuf,
    image_root: PathBuf,
    batch_size: usize,
    overflow_width_px: usize,
    overflow_height_px: usize,
    wait_for_start: bool,
    extended_error_logging: bool,
    office_js_debug: bool,
    capture_run_id: String,
}

#[derive(Clone, Serialize)]
struct EventEntry {
    sequence: i64,
    timestamp_unix_ms: i64,
    stage: String,
    detail: Value,
}

#[derive(Clone, Serialize)]
struct ProbeResult {
    workbook: String,
    probe_id: String,
    sheet: String,
    axis: String,
    start_index: usize,
    extent_units: usize,
    changed_pixel: i64,
    changed_points: f64,
    before_png: String,
    after_png: String,
    xor_png: String,
}

struct Shared {
    config: Config,
    expected: Vec<String>,
    expected_set: HashSet<String>,
    results: Mutex<HashMap<String, Value>>,
    errors: Mutex<Vec<String>>,
    events: Mutex<Vec<EventEntry>>,
    sequence: AtomicI64,
    last_activity: AtomicI64,
    probes: Mutex<HashMap<String, JoinHandle<Result<ProbeResult, String>>>>,
    com: ComController,
    progress_gate: Mutex<()>,
    capture_owners: Mutex<HashMap<String,String>>,
}

#[derive(Clone)]
struct ComController {
    sender: mpsc::Sender<ComCommand>,
}

enum ComCommand {
    Open {
        path: PathBuf,
        response: mpsc::Sender<Result<String, String>>,
    },
    Close {
        response: mpsc::Sender<Result<(), String>>,
    },
    Shutdown,
}

#[derive(Deserialize)]
struct StatusQuery {
    workbook: Option<String>,
    after: Option<i64>,
}

#[derive(Deserialize)]
struct ProbeStatusQuery {
    workbook: String,
}

#[derive(Deserialize)]
struct BeaconQuery {
    stage: Option<String>,
    detail: Option<String>,
}

#[derive(Deserialize)]
struct ControlOpen {
    path: String,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

impl Shared {
    fn record(&self, stage: impl Into<String>, detail: Value) {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let timestamp = now_ms();
        self.last_activity.store(timestamp, Ordering::SeqCst);
        let stage = stage.into();
        eprintln!("[{timestamp}] {stage} {detail}");
        let entry=EventEntry {sequence,timestamp_unix_ms:timestamp,stage,detail};
        let mut events=self.events.lock().unwrap();
        let path=self.config.output.with_extension("json.events.jsonl");
        if let Ok(mut file)=fs::OpenOptions::new().create(true).append(true).open(path) {
            if let Ok(line)=serde_json::to_string(&entry) { let _=writeln!(file,"{line}"); let _=file.flush(); }
        }
        events.push(entry);
    }

    fn error(&self, message: impl Into<String>) {
        let message = message.into();
        self.errors.lock().unwrap().push(message.clone());
        self.record("server_error", json!({ "message": message }));
    }
}

impl ComController {
    fn start() -> Self {
        let (sender, receiver) = mpsc::channel::<ComCommand>();

        thread::spawn(move || {
            let apartment = match ComApartment::sta() {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("COM STA initialization failed: {error:#}");
                    return;
                }
            };

            let excel = match OwnedApplication::new(&apartment) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("Excel startup failed: {error:#}");
                    return;
                }
            };

            let _ = excel.set_visible(true);
            let _ = excel.set_display_alerts(false);
            let _ = excel.set_enable_events(true);

            let mut current: Option<excel_com::Workbook> = None;

            while let Ok(command) = receiver.recv() {
                match command {
                    ComCommand::Open { path, response } => {
                        if let Some(workbook) = current.take() {
                            let _ = workbook.close(WorkbookCloseOptions {
                                save_changes: SaveChanges::Discard,
                                ..WorkbookCloseOptions::new()
                            });
                        }

                        let result = (|| -> Result<String> {
                            let workbooks = excel.workbooks()?;
                            let workbook = workbooks
                                .open_without_link_updates(&path, true)
                                .with_context(|| format!("open {}", path.display()))?;
                            let full_name = workbook.full_name()?;
                            current = Some(workbook);
                            Ok(full_name)
                        })()
                        .map_err(|error| format!("{error:#}"));

                        let _ = response.send(result);
                    }
                    ComCommand::Close { response } => {
                        let result = if let Some(workbook) = current.take() {
                            workbook
                                .close(WorkbookCloseOptions {
                                    save_changes: SaveChanges::Discard,
                                    ..WorkbookCloseOptions::new()
                                })
                                .map_err(|error| format!("{error:#}"))
                        } else {
                            Ok(())
                        };
                        let _ = response.send(result);
                    }
                    ComCommand::Shutdown => break,
                }
            }

            if let Some(workbook) = current.take() {
                let _ = workbook.close(WorkbookCloseOptions {
                    save_changes: SaveChanges::Discard,
                    ..WorkbookCloseOptions::new()
                });
            }

            let _ = excel.quit_and_wait(Duration::from_secs(30));
        });

        Self { sender }
    }

    fn open(&self, path: PathBuf) -> Result<String> {
        let (tx, rx) = mpsc::channel();
        self.sender
            .send(ComCommand::Open { path, response: tx })
            .context("send COM open command")?;
        rx.recv_timeout(Duration::from_secs(180))
            .context("wait for Excel COM open")?
            .map_err(|message| anyhow!(message))
    }

    fn close(&self) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.sender
            .send(ComCommand::Close { response: tx })
            .context("send COM close command")?;
        rx.recv_timeout(Duration::from_secs(60))
            .context("wait for Excel COM close")?
            .map_err(|message| anyhow!(message))
    }
}

fn required(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("missing environment variable {name}"))
}

fn env_usize(name: &str, fallback: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(fallback)
}

// Source Context patch: manual task-pane start configuration.
// Source Context patch: Office.js diagnostic configuration.
fn env_bool(name: &str, fallback: bool) -> bool {
    let Ok(value) = env::var(name) else {
        return fallback;
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => fallback,
    }
}

fn safe_file_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => ch,
        })
        .collect()
}

fn read_png_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    if bytes.len() < 24 {
        bail!("PNG payload is too short: decoded_bytes={}", bytes.len());
    }
    if &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        let prefix_hex = bytes
            .iter()
            .take(24)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        bail!(
            "payload is not PNG: decoded_bytes={}, byte_prefix_hex={prefix_hex}",
            bytes.len()
        );
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    Ok((width, height))
}

fn analyze_probe(payload: Value, image_root: &Path) -> Result<ProbeResult> {
    let workbook = payload["workbook"]
        .as_str()
        .context("probe workbook")?
        .to_owned();
    let probe_id = payload["probe_id"]
        .as_str()
        .unwrap_or("probe")
        .to_owned();
    let sheet = payload["sheet"].as_str().context("probe sheet")?.to_owned();
    let axis = payload["axis"].as_str().context("probe axis")?.to_owned();
    let start_index = payload["start_index"]
        .as_u64()
        .context("probe start_index")? as usize;
    let before = BASE64.decode(payload["before_base64"].as_str().context("before_base64")?)?;
    let after = BASE64.decode(payload["after_base64"].as_str().context("after_base64")?)?;
    let total_points = payload["total_points"].as_f64().unwrap_or(0.0);
    let boundaries: Vec<f64> = payload["boundaries_points"]
        .as_array()
        .context("boundaries_points")?
        .iter()
        .filter_map(Value::as_f64)
        .collect();

    let before_image = ImageReader::new(Cursor::new(&before))
        .with_guessed_format()?
        .decode()?
        .to_rgba8();
    let after_image = ImageReader::new(Cursor::new(&after))
        .with_guessed_format()?
        .decode()?
        .to_rgba8();

    if before_image.dimensions() != after_image.dimensions() {
        bail!("probe image dimensions differ");
    }

    let diagnostic_dir = image_root
        .join(safe_file_name(&workbook))
        .join("_overflow-probes");
    fs::create_dir_all(&diagnostic_dir)?;

    let stem = format!(
        "{}-{}-{}",
        safe_file_name(&probe_id),
        safe_file_name(&sheet),
        safe_file_name(&axis)
    );
    let before_file_name = format!("{stem}-before.png");
    let after_file_name = format!("{stem}-after.png");
    let xor_file_name = format!("{stem}-xor.png");
    fs::write(diagnostic_dir.join(&before_file_name), &before)?;
    fs::write(diagnostic_dir.join(&after_file_name), &after)?;

    let (width, height) = before_image.dimensions();
    let mut xor_image = RgbaImage::new(width, height);
    let mut rightmost = -1_i64;
    let mut bottommost = -1_i64;

    for y in 0..height {
        for x in 0..width {
            let before_pixel = before_image.get_pixel(x, y);
            let after_pixel = after_image.get_pixel(x, y);
            let changed = before_pixel != after_pixel;

            xor_image.put_pixel(
                x,
                y,
                Rgba([
                    before_pixel[0] ^ after_pixel[0],
                    before_pixel[1] ^ after_pixel[1],
                    before_pixel[2] ^ after_pixel[2],
                    if changed { 255 } else { 0 },
                ]),
            );

            if changed {
                rightmost = rightmost.max(x as i64);
                bottommost = bottommost.max(y as i64);
            }
        }
    }

    xor_image.save(diagnostic_dir.join(&xor_file_name))?;

    let changed_pixel = if axis == "horizontal" { rightmost } else { bottommost };
    let pixel_extent = if axis == "horizontal" { width } else { height };
    let changed_points = if changed_pixel < 0 || pixel_extent == 0 {
        -1.0
    } else {
        (changed_pixel as f64 + 1.0) * total_points / pixel_extent as f64
    };

    let mut extent_units = 0;
    if changed_points >= 0.0 {
        for (index, boundary) in boundaries.iter().enumerate() {
            extent_units = index + 1;
            if *boundary + 0.01 >= changed_points {
                break;
            }
        }
    }

    let relative = |file_name: &str| {
        format!(
            "{}/_overflow-probes/{}",
            safe_file_name(&workbook),
            file_name
        )
    };
    let before_png = relative(&before_file_name);
    let after_png = relative(&after_file_name);
    let xor_png = relative(&xor_file_name);

    Ok(ProbeResult {
        workbook,
        probe_id,
        sheet,
        axis,
        start_index,
        extent_units,
        changed_pixel,
        changed_points,
        before_png,
        after_png,
        xor_png,
    })
}

async fn health() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}

async fn config(State(state): State<Arc<Shared>>) -> impl IntoResponse {
    Json(json!({
        "experiment": state.config.experiment,
        "strategy": state.config.strategy,
        "host_kind": state.config.host_kind,
        "batch_size": state.config.batch_size,
        "overflow_probe_width_px": state.config.overflow_width_px,
        "overflow_probe_height_px": state.config.overflow_height_px,
        "wait_for_start": state.config.wait_for_start,
        "extended_error_logging": state.config.extended_error_logging,
        "office_js_debug": state.config.office_js_debug,
        "capture_run_id": state.config.capture_run_id
    }))
}

async fn status(
    State(state): State<Arc<Shared>>,
    Query(query): Query<StatusQuery>,
) -> impl IntoResponse {
    let after = query.after.unwrap_or(0);
    let events: Vec<EventEntry> = state
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|entry| entry.sequence > after)
        .cloned()
        .collect();
    let done = match query.workbook {
        Some(ref workbook) => state.results.lock().unwrap().contains_key(workbook),
        None => state.results.lock().unwrap().len() == state.expected.len(),
    };

    Json(json!({
        "done": done,
        "last_activity_unix_ms": state.last_activity.load(Ordering::SeqCst),
        "last_sequence": state.sequence.load(Ordering::SeqCst),
        "events": events,
        "errors": state.errors.lock().unwrap().clone()
    }))
}

async fn reset_diagnostics(State(state): State<Arc<Shared>>) -> impl IntoResponse {
    state.errors.lock().unwrap().clear();
    state.events.lock().unwrap().clear();

    // Keep retries isolated just like the ASP.NET host. Dropping JoinHandles
    // does not cancel already-running analysis tasks, but it removes their
    // logical keys so the next workbook attempt can submit fresh probes.
    state.probes.lock().unwrap().clear();

    Json(json!({ "ok": true }))
}

async fn client_event(
    State(state): State<Arc<Shared>>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let stage = payload["stage"].as_str().unwrap_or("client_event").to_owned();
    state.record(stage, payload["detail"].clone());
    Json(json!({ "ok": true }))
}

async fn beacon(
    State(state): State<Arc<Shared>>,
    Query(query): Query<BeaconQuery>,
) -> impl IntoResponse {
    let detail = query
        .detail
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .unwrap_or(Value::Null);
    state.record(query.stage.unwrap_or_else(|| "beacon".into()), detail);
    StatusCode::NO_CONTENT
}

async fn image_batch(
    State(state): State<Arc<Shared>>,
    Json(payload): Json<Value>,
) -> Response {
    let result = (|| -> Result<Value> {
        let workbook = payload["workbook"].as_str().context("workbook")?;
        if !state.expected_set.contains(workbook) { bail!("Unexpected image workbook"); }
        let batch_index = payload["batch_index"].as_u64().unwrap_or(0);
        let images = payload["images"].as_array().context("images")?;
        let workbook_dir = state.config.image_root.join(safe_file_name(workbook));
        fs::create_dir_all(&workbook_dir)?;

        let mut persisted = Vec::new();

        for image in images {
            let id = image["id"].as_str().context("image id")?;
            if id.is_empty() || id.len()>100 || !id.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'-' || b==b'_') {bail!("Unsafe image ID");}
            let sheet = image["sheet"].as_str().context("image sheet")?;
            let address = image["address"].as_str().context("image address")?;
            let base64 = image["base64"].as_str().context("image base64")?;
            let base64_prefix: String = base64.chars().take(32).collect();

            state.record("image_batch_item_received", json!({
                "workbook": workbook,
                "batch_index": batch_index,
                "id": id,
                "sheet": sheet,
                "address": address,
                "base64_chars": base64.len(),
                "base64_prefix": base64_prefix
            }));

            let bytes = BASE64
                .decode(base64)
                .with_context(|| {
                    format!(
                        "decode image base64 workbook={workbook:?}, batch={batch_index}, id={id}, sheet={sheet:?}, address={address}, base64_chars={}",
                        base64.len()
                    )
                })?;
            let (width, height) = read_png_dimensions(&bytes).with_context(|| {
                format!(
                    "validate PNG workbook={workbook:?}, batch={batch_index}, id={id}, sheet={sheet:?}, address={address}, base64_chars={}, decoded_bytes={}",
                    base64.len(),
                    bytes.len()
                )
            })?;

            state.record("image_payload_validated", json!({
                "workbook": workbook,
                "batch_index": batch_index,
                "id": id,
                "sheet": sheet,
                "address": address,
                "base64_chars": base64.len(),
                "decoded_bytes": bytes.len(),
                "png_width": width,
                "png_height": height
            }));

            let file_name = format!("{id}.png");
            let path = workbook_dir.join(&file_name);
            fs::write(&path, &bytes)?;

            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let sha = format!("{:x}", hasher.finalize());

            persisted.push(json!({
                "id": id,
                "workbook": workbook,
                "sheet": sheet,
                "address": address,
                "relative_png": format!("{}/{}", safe_file_name(workbook), file_name),
                "png_bytes": bytes.len(),
                "png_width": width,
                "png_height": height,
                "sha256": sha
            }));
        }

        state.record("image_batch_persisted", json!({"workbook":workbook,"batch_index":batch_index,"captures":persisted}));
        write_output(&state)?;
        state.record("image_batch_received", json!({
            "workbook": workbook,
            "batch_index": batch_index,
            "image_count": images.len()
        }));

        Ok(json!({ "captures": persisted }))
    })();

    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => {
            state.error(format!("{error:#}"));
            (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{error:#}") }))).into_response()
        }
    }
}

async fn probe_batch(
    State(state): State<Arc<Shared>>,
    Json(payload): Json<Value>,
) -> Response {
    let workbook = match payload["workbook"].as_str() {
        Some(value) => value.to_owned(),
        None => return (StatusCode::BAD_REQUEST, "probe workbook missing").into_response(),
    };
    let sheet = match payload["sheet"].as_str() {
        Some(value) => value.to_owned(),
        None => return (StatusCode::BAD_REQUEST, "probe sheet missing").into_response(),
    };
    let axis = match payload["axis"].as_str() {
        Some(value) => value.to_owned(),
        None => return (StatusCode::BAD_REQUEST, "probe axis missing").into_response(),
    };
    let key = format!("{workbook}\n{sheet}\n{axis}");

    {
        let probes = state.probes.lock().unwrap();
        if probes.contains_key(&key) {
            drop(probes);
            state.record("probe_duplicate_ignored", json!({
                "workbook": workbook,
                "sheet": sheet,
                "axis": axis
            }));
            return Json(json!({
                "accepted": true,
                "duplicate": true
            })).into_response();
        }
    }

    let cloned = payload.clone();
    let image_root = state.config.image_root.clone();
    let handle = tokio::task::spawn_blocking(move || {
        analyze_probe(cloned, &image_root).map_err(|error| format!("{error:#}"))
    });

    state.probes.lock().unwrap().insert(key, handle);

    state.record("probe_received", json!({
        "workbook": workbook,
        "sheet": sheet,
        "axis": axis
    }));

    Json(json!({
        "accepted": true,
        "duplicate": false
    })).into_response()
}

async fn probe_status(
    State(state): State<Arc<Shared>>,
    Query(query): Query<ProbeStatusQuery>,
) -> Response {
    let prefix = format!("{}\n", query.workbook);

    let keys: Vec<String> = state
        .probes
        .lock()
        .unwrap()
        .keys()
        .filter(|key| key.starts_with(&prefix))
        .cloned()
        .collect();

    let mut completed = 0usize;
    {
        let probes = state.probes.lock().unwrap();
        for key in &keys {
            if probes.get(key).map(|handle| handle.is_finished()).unwrap_or(false) {
                completed += 1;
            }
        }
    }

    let done = !keys.is_empty() && completed == keys.len();
    if !done {
        return Json(json!({
            "done": false,
            "completed": completed,
            "total": keys.len(),
            "results": []
        })).into_response();
    }

    let mut handles = Vec::new();
    {
        let mut probes = state.probes.lock().unwrap();
        for key in &keys {
            if let Some(handle) = probes.remove(key) {
                handles.push(handle);
            }
        }
    }

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(result)) => results.push(result),
            Ok(Err(message)) => {
                state.error(message.clone());
                return (StatusCode::INTERNAL_SERVER_ERROR, message).into_response();
            }
            Err(error) => {
                let message = error.to_string();
                state.error(message.clone());
                return (StatusCode::INTERNAL_SERVER_ERROR, message).into_response();
            }
        }
    }

    Json(json!({
        "done": true,
        "completed": results.len(),
        "total": results.len(),
        "results": results
    })).into_response()
}

async fn result(
    State(state): State<Arc<Shared>>,
    Json(payload): Json<Value>,
) -> Response {
    let workbook = match payload["workbook"].as_str() {
        Some(value) => value.to_owned(),
        None => return (StatusCode::BAD_REQUEST, "result workbook missing").into_response(),
    };

    if !state.expected_set.contains(&workbook) {
        return (
            StatusCode::BAD_REQUEST,
            format!("unexpected workbook result: {workbook}"),
        ).into_response();
    }

    state.results.lock().unwrap().insert(workbook.clone(), payload);
    if let Err(error) = write_output(&state) {
        state.error(format!("{error:#}"));
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{error:#}"),
        ).into_response();
    }

    state.record("result_received", json!({ "workbook": workbook }));
    Json(json!({ "ok": true })).into_response()
}

fn write_output(state: &Shared) -> Result<()> {
    let results = state.results.lock().unwrap();
    let mut workbooks = serde_json::Map::new();

    for name in &state.expected {
        if let Some(value) = results.get(name) {
            workbooks.insert(name.clone(), value.clone());
        }
    }

    let document = json!({
        "_meta": {
            "experiment": state.config.experiment,
            "strategy": state.config.strategy,
            "host_kind": state.config.host_kind,
            "batch_size": state.config.batch_size,
            "completed_workbooks": results.len(),
            "expected_workbooks": state.expected.len(),
            "generated_unix_ms": now_ms()
        },
        "errors": state.errors.lock().unwrap().clone(),
        "workbooks": workbooks
    });

    if let Some(parent) = state.config.output.parent() {
        fs::create_dir_all(parent)?;
    }

    let temporary=state.config.output.with_extension("json.new");
    fs::write(&temporary, serde_json::to_string_pretty(&document)? + "\n")?;
    fs::rename(&temporary,&state.config.output)?;

    Ok(())
}

async fn client_error(
    State(state): State<Arc<Shared>>,
    body: String,
) -> impl IntoResponse {
    state.errors.lock().unwrap().push(body.clone());
    state.record("client_error", json!({ "message": body }));
    Json(json!({ "ok": true }))
}

async fn control_open(
    State(state): State<Arc<Shared>>,
    Json(payload): Json<ControlOpen>,
) -> Response {
    let path = PathBuf::from(payload.path);
    let controller = state.com.clone();
    let result = tokio::task::spawn_blocking(move || controller.open(path)).await;

    match result {
        Ok(Ok(full_name)) => {
            state.record("excel_com_opened", json!({ "full_name": full_name }));
            Json(json!({ "ok": true, "full_name": full_name })).into_response()
        }
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{error:#}") })),
        ).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        ).into_response(),
    }
}

async fn control_close(State(state): State<Arc<Shared>>) -> Response {
    let controller = state.com.clone();
    let result = tokio::task::spawn_blocking(move || controller.close()).await;

    match result {
        Ok(Ok(())) => {
            state.record("excel_com_closed", Value::Null);
            Json(json!({ "ok": true })).into_response()
        }
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{error:#}") })),
        ).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        ).into_response(),
    }
}

async fn static_file(
    State(state): State<Arc<Shared>>,
    relative: &'static str,
    content_type: &'static str,
) -> Response {
    let path = state.config.static_root.join(relative);
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let mut response = Response::new(Body::from(bytes));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type),
            );
            response
        }
        Err(error) => (
            StatusCode::NOT_FOUND,
            format!("static file error: {error}"),
        ).into_response(),
    }
}

async fn taskpane(State(state): State<Arc<Shared>>) -> Response {
    state.record("taskpane_requested", Value::Null);
    static_file(State(state), "taskpane.html", "text/html; charset=utf-8").await
}

async fn mode_js(State(state): State<Arc<Shared>>) -> Response {
    let relative = if state.config.experiment == "range-image" {
        "modes/range-image.js"
    } else if state.config.experiment == "range-image-height-limit" {
        "modes/range-image-height-limit.js"
    } else {
        "modes/shape-bounds.js"
    };
    let path = state.config.static_root.join(relative);
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let mut response = Response::new(Body::from(bytes));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/javascript; charset=utf-8"),
            );
            response
        }
        Err(error) => (StatusCode::NOT_FOUND, error.to_string()).into_response(),
    }
}

async fn capture_claim(State(state): State<Arc<Shared>>, Json(payload): Json<Value>) -> Response {
    let workbook=payload["workbook"].as_str().unwrap_or("");
    let client=payload["client_id"].as_str().unwrap_or("");
    if !state.expected_set.contains(workbook) || client.is_empty() || client.len()>200 {return (StatusCode::BAD_REQUEST,"invalid claim").into_response();}
    let mut owners=state.capture_owners.lock().unwrap();
    let owner=owners.entry(workbook.to_owned()).or_insert_with(||client.to_owned());
    Json(json!({"acquired":owner.as_str()==client})).into_response()
}
async fn capture_progress(State(state): State<Arc<Shared>>, Json(payload): Json<Value>) -> Response {
    let result=(|| -> Result<()> {
        let workbook=payload["workbook"].as_str().context("workbook")?;
        if !state.expected_set.contains(workbook) {bail!("Unexpected workbook");}
        let _gate=state.progress_gate.lock().unwrap();
        let dir=state.config.image_root.join("_progress"); fs::create_dir_all(&dir)?;
        let path=dir.join(format!("{}.progress.json",safe_file_name(workbook)));
        let temp=path.with_extension("json.new");
        fs::write(&temp,serde_json::to_vec_pretty(&payload)?)?;
        fs::rename(&temp,&path)?; Ok(())
    })();
    match result {Ok(())=>Json(json!({"ok":true})).into_response(),Err(e)=>(StatusCode::BAD_REQUEST,format!("{e:#}")).into_response()}
}
async fn capture_guard(State(state): State<Arc<Shared>>, request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if state.config.experiment=="range-image" && request.method()==axum::http::Method::POST &&
        matches!(request.uri().path(),"/capture-claim"|"/capture-progress"|"/image-batch"|"/result"|"/client-event"|"/error"|"/probe-batch") &&
        request.headers().get("X-VV-Run-Id").and_then(|x|x.to_str().ok())!=Some(state.config.capture_run_id.as_str()) {
        return (StatusCode::CONFLICT,"stale-capture-run").into_response();
    }
    next.run(request).await
}

fn router(state: Arc<Shared>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/config", get(config))
        .route("/status", get(status))
        .route("/reset-diagnostics", post(reset_diagnostics))
        .route("/client-event", post(client_event))
        .route("/client-beacon", get(beacon))
        .route("/image-batch", post(image_batch))
        .route("/capture-progress", post(capture_progress))
        .route("/capture-claim", post(capture_claim))
        .route("/probe-batch", post(probe_batch))
        .route("/probe-status", get(probe_status))
        .route("/result", post(result))
        .route("/error", post(client_error))
        .route("/control/open", post(control_open))
        .route("/control/close", post(control_close))
        .route("/taskpane.html", get(taskpane))
        .route("/taskpane.css", get(|state| static_file(state, "taskpane.css", "text/css; charset=utf-8")))
        .route("/bootstrap.js", get(|state| static_file(state, "bootstrap.js", "application/javascript; charset=utf-8")))
        .route("/shell.js", get(|state| static_file(state, "shell.js", "application/javascript; charset=utf-8")))
        .route("/commands.html", get(|state| static_file(state, "commands.html", "text/html; charset=utf-8")))
        .route("/visual-plan.json", get(|state| static_file(state, "visual-plan.json", "application/json; charset=utf-8")))
        .route("/mode.js", get(mode_js))
        .route("/safe-capture.js", get(|state| static_file(state,"safe-capture.js","application/javascript; charset=utf-8")))
        .layer(axum::middleware::from_fn_with_state(state.clone(),capture_guard))
        .layer(axum::extract::DefaultBodyLimit::max(128 * 1024 * 1024))
        .with_state(state)
}

fn load_config() -> Result<Config> {
    Ok(Config {
        experiment: required("VV_EXPERIMENT")?,
        strategy: env::var("VV_STRATEGY").unwrap_or_else(|_| "A".into()),
        host_kind: env::var("VV_HOST_KIND").unwrap_or_else(|_| "rust".into()),
        static_root: PathBuf::from(required("VV_STATIC_ROOT")?),
        file_list: PathBuf::from(required("VV_FILE_LIST")?),
        output: PathBuf::from(required("VV_OUTPUT_PATH")?),
        image_root: PathBuf::from(required("VV_IMAGE_ROOT")?),
        batch_size: env_usize("VV_BATCH_SIZE", 8),
        overflow_width_px: env_usize("VV_OVERFLOW_WIDTH_PX", 1920),
        overflow_height_px: env_usize("VV_OVERFLOW_HEIGHT_PX", 1080),
        wait_for_start: env_bool("VV_WAIT_FOR_START", false),
        extended_error_logging: env_bool("VV_EXTENDED_ERROR_LOGGING", false),
        office_js_debug: env_bool("VV_OFFICE_JS_DEBUG", false),
        capture_run_id: env::var("VV_CAPTURE_RUN_ID").unwrap_or_default(),
    })
}

fn ensure_cert(directory: &Path) -> Result<()> {
    fs::create_dir_all(directory)?;
    let cert_pem = directory.join("localhost-cert.pem");
    let key_pem = directory.join("localhost-key.pem");
    let cert_der = directory.join("localhost-cert.der");

    if cert_pem.exists() && key_pem.exists() && cert_der.exists() {
        println!("{}", cert_der.display());
        return Ok(());
    }

    let certified = generate_simple_self_signed(vec!["localhost".to_owned()])?;
    fs::write(&cert_pem, certified.cert.pem())?;
    fs::write(&key_pem, certified.key_pair.serialize_pem())?;
    fs::write(&cert_der, certified.cert.der().as_ref())?;
    println!("{}", cert_der.display());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.get(1).map(String::as_str) == Some("ensure-cert") {
        let directory = args.get(2).context("usage: host ensure-cert <directory>")?;
        ensure_cert(Path::new(directory))?;
        return Ok(());
    }

    if args.get(1).map(String::as_str) != Some("serve") {
        bail!("usage: visual-validation-office-host <ensure-cert DIR|serve CERT_PEM KEY_PEM>");
    }

    let cert_pem = PathBuf::from(args.get(2).context("missing cert PEM path")?);
    let key_pem = PathBuf::from(args.get(3).context("missing key PEM path")?);

    let config = load_config()?;
    let expected_paths = fs::read_to_string(&config.file_list)?;
    let expected: Vec<String> = expected_paths
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            Path::new(line)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(line)
                .to_owned()
        })
        .collect();

    let state = Arc::new(Shared {
        config: config.clone(),
        expected_set: expected.iter().cloned().collect(),
        expected,
        results: Mutex::new(HashMap::new()),
        errors: Mutex::new(Vec::new()),
        events: Mutex::new(Vec::new()),
        sequence: AtomicI64::new(0),
        last_activity: AtomicI64::new(now_ms()),
        probes: Mutex::new(HashMap::new()),
        com: ComController::start(),
        progress_gate: Mutex::new(()),
        capture_owners: Mutex::new(HashMap::new()),
    });

    state.record("server_starting", json!({
        "experiment": config.experiment,
        "strategy": config.strategy
    }));

    write_output(&state)?;
    let app = router(state.clone());
    let tls = RustlsConfig::from_pem_file(cert_pem, key_pem).await?;

    let https = axum_server::bind_rustls(
        SocketAddr::from(([127, 0, 0, 1], 3000)),
        tls,
    )
    .serve(app.clone().into_make_service());

    let http = axum_server::bind(SocketAddr::from(([127, 0, 0, 1], 3001)))
        .serve(app.into_make_service());

    state.record("server_ready", json!({
        "https": "https://localhost:3000",
        "http": "http://127.0.0.1:3001"
    }));

    tokio::select! {
        result = https => result?,
        result = http => result?,
    }

    let _ = state.com.sender.send(ComCommand::Shutdown);
    Ok(())
}
