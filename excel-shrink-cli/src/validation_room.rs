//! Local configuration and compact progress for the standalone Observatory.
//! Completed evidence uses exactly the same files as an exported static report.

use crate::validation::{self, LiveProgress, RunConfig};
use anyhow::{ensure, Context, Result};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path as RoutePath, Query, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

const HTML: &str = include_str!("../office-capture/control-room/sheet-scope.html");
const JS: &str = include_str!("../office-capture/control-room/sheet-scope.js");
const CSS: &str = include_str!("../office-capture/control-room/sheet-scope.css");

struct Room {
    port: u16,
    token: String,
    config: Mutex<RunConfig>,
    progress: Arc<Mutex<LiveProgress>>,
    active: AtomicBool,
    report: Mutex<Option<(String, PathBuf)>>,
    reports: Mutex<BTreeMap<String, PathBuf>>,
    picker: Mutex<()>,
}

type ApiResult = std::result::Result<Json<Value>, ApiError>;
#[derive(Debug)]
struct ApiError(anyhow::Error);
impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":format!("{:#}",self.0)})),
        )
            .into_response()
    }
}

pub(crate) fn serve(config: RunConfig, port: u16, open: bool) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(serve_async(config, port, open))
}

async fn serve_async(config: RunConfig, port: u16, open: bool) -> Result<()> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let port = listener.local_addr()?.port();
    let room = Arc::new(Room {
        port,
        token: uuid::Uuid::new_v4().to_string(),
        config: Mutex::new(config),
        progress: Arc::new(Mutex::new(LiveProgress::default())),
        active: AtomicBool::new(false),
        report: Mutex::new(None),
        reports: Mutex::new(BTreeMap::new()),
        picker: Mutex::new(()),
    });
    let web_root = validation::capture::capture_directory()?.join("control-room");
    let routes = Router::new()
        .route("/", get(|| async { Html(HTML) }))
        .route(
            "/sheet-scope.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    JS,
                )
            }),
        )
        .route(
            "/sheet-scope.css",
            get(|| async { ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], CSS) }),
        )
        .route("/api/session", get(session))
        .route("/api/state", get(state))
        .route("/api/browse", get(browse))
        .route("/api/pick", post(pick))
        .route("/api/workbooks", get(workbooks))
        .route("/api/run", post(start))
        .route("/api/study/resume", post(resume_study))
        .route("/api/open", post(open_report))
        .route("/api/export", post(export_report))
        .route("/reports/{id}/{*path}", get(report_asset))
        .nest_service(
            "/observatory-assets",
            tower_http::services::ServeDir::new(web_root),
        )
        .layer(DefaultBodyLimit::max(512 * 1024))
        .with_state(room.clone())
        .layer(middleware::from_fn_with_state(room, local_request));
    let url = format!("http://deltasieve.localhost:{port}/");
    eprintln!("DeltaSieve Observatory: {url}");
    if open {
        validation::open_browser(std::ffi::OsStr::new(&url))?;
    }
    axum::serve(listener, routes).await?;
    Ok(())
}

/// Same-origin reads and a per-process token protect local filesystem/process APIs.
async fn local_request(State(room): State<Arc<Room>>, request: Request, next: Next) -> Response {
    let mut response = if allowed_headers(request.headers(), room.port)
        && (matches!(request.method().as_str(), "GET" | "HEAD")
            || request
                .headers()
                .get("x-sheet-scope-token")
                .and_then(|v| v.to_str().ok())
                == Some(room.token.as_str()))
    {
        next.run(request).await
    } else {
        (
            StatusCode::FORBIDDEN,
            "Only this local Observatory session is allowed",
        )
            .into_response()
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("referrer-policy", "no-referrer".parse().unwrap());
    response
}

fn allowed_headers(headers: &HeaderMap, port: u16) -> bool {
    let expected = [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("deltasieve.localhost:{port}"),
    ];
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    if !expected.iter().any(|value| value == host) {
        return false;
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        if origin.to_str().ok() != Some(format!("http://{host}").as_str()) {
            return false;
        }
    }
    headers
        .get("sec-fetch-site")
        .is_none_or(|value| value != "cross-site")
}

async fn session(State(room): State<Arc<Room>>) -> ApiResult {
    let config = room
        .config
        .lock()
        .map_err(|_| anyhow::anyhow!("Configuration lock poisoned"))?
        .clone();
    let working_directory = std::env::current_dir().map_err(anyhow::Error::from)?;
    let browse_directory = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| working_directory.clone());
    Ok(Json(
        json!({"token":room.token,"config":config,"working_directory":working_directory,
        "browse_directory":browse_directory,"native_picker":cfg!(windows),
        "logical_processors":std::thread::available_parallelism().map_or(1, usize::from),
        "processor_schema":crate::cli_metadata::describe(),"shrink_preset":validation::excel_shrink_preset(1)}),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum PickerKind {
    Folder,
    File,
    Program,
}

#[derive(Deserialize)]
struct PickRequest {
    kind: PickerKind,
    #[serde(default)]
    initial_path: String,
}

/// Open one native picker without blocking status polling.
async fn pick(State(room): State<Arc<Room>>, Json(request): Json<PickRequest>) -> ApiResult {
    if !cfg!(windows) {
        return Err(anyhow::anyhow!("Native file dialogs require Windows").into());
    }
    let result = tokio::task::spawn_blocking(move || -> Result<Value> {
        let _picker = room
            .picker
            .try_lock()
            .map_err(|_| anyhow::anyhow!("A file dialog is already open"))?;
        let script = validation::capture::capture_directory()?.join("select-path.ps1");
        let kind = match request.kind {
            PickerKind::Folder => "folder",
            PickerKind::File => "file",
            PickerKind::Program => "program",
        };
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-STA",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(script)
            .args(["-Kind", kind, "-InitialPath", &request.initial_path])
            .stdin(std::process::Stdio::null())
            .output()?;
        ensure!(
            output.status.success(),
            "File dialog failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    })
    .await
    .map_err(anyhow::Error::from)??;
    Ok(Json(result))
}

#[derive(Deserialize)]
struct WorkbookQuery {
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    search: String,
    #[serde(default)]
    filter: String,
}

async fn workbooks(State(room): State<Arc<Room>>, Query(query): Query<WorkbookQuery>) -> ApiResult {
    let progress = room
        .progress
        .lock()
        .map_err(|_| anyhow::anyhow!("Progress lock poisoned"))?;
    let search = query.search.to_lowercase();
    let matches = |record: &&Value| {
        let status = record["status"].as_str().unwrap_or_default();
        let differences = record["different_regions"].as_u64().unwrap_or(0);
        record["name"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase()
            .contains(&search)
            && match query.filter.as_str() {
                "differences" => differences > 0,
                "equal" => status == "completed_with_limitations" && differences == 0,
                "failed" => matches!(status, "failed" | "missing"),
                "reused" => status == "reused",
                "pending" => matches!(status, "pending" | "running"),
                _ => true,
            }
    };
    let total = progress.workbooks.values().filter(matches).count();
    let rows: Vec<_> = progress
        .workbooks
        .values()
        .filter(matches)
        .skip(query.offset)
        .take(100)
        .cloned()
        .collect();
    Ok(Json(
        json!({"total":total,"offset":query.offset,"rows":rows}),
    ))
}

async fn state(State(room): State<Arc<Room>>) -> ApiResult {
    // The paginated workbook index stays in memory; polling serializes only the
    // bounded status fields instead of cloning the complete historical index.
    let (progress, telemetry_path, comparisons) = {
        let progress = room
            .progress
            .lock()
            .map_err(|_| anyhow::anyhow!("Progress lock poisoned"))?;
        (
            serde_json::to_value(&*progress).map_err(anyhow::Error::from)?,
            progress.telemetry_path.clone(),
            progress.comparisons.clone(),
        )
    };
    let telemetry = if let Some(path) = &telemetry_path {
        read_small_json(path, 4 * 1024 * 1024)
            .await
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let report_base = room
        .report
        .lock()
        .map_err(|_| anyhow::anyhow!("Report lock poisoned"))?
        .as_ref()
        .map(|(id, _)| format!("/reports/{id}"));
    Ok(Json(
        json!({"active":room.active.load(Ordering::Acquire),"progress":progress,"telemetry":telemetry,
        "comparisons":comparisons,"report_base":report_base}),
    ))
}

async fn read_small_json(path: &Path, limit: u64) -> Result<Value> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path).await?;
    ensure!(
        file.metadata().await?.len() <= limit,
        "JSON snapshot exceeds its reading bound"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).await?;
    ensure!(
        bytes.len() as u64 <= limit,
        "JSON snapshot grew beyond its reading bound"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

#[derive(Deserialize)]
struct Browse {
    path: Option<String>,
    #[serde(default)]
    offset: usize,
}

async fn browse(Query(query): Query<Browse>) -> ApiResult {
    let result = tokio::task::spawn_blocking(move || -> Result<Value> {
        let path = match query.path.filter(|path| !path.trim().is_empty()) {
            Some(path) => PathBuf::from(path), None => std::env::current_dir()?,
        };
        let path = fs::canonicalize(path)?;
        let path = if path.is_file() { path.parent().context("File has no parent")?.to_owned() } else { path };
        let mut entries = Vec::new();
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) { continue; }
            entries.push(json!({"name":entry.file_name().to_string_lossy(),
                "path":entry.path(),"directory":kind.is_dir()}));
            ensure!(entries.len() <= 100_000, "Directory contains more than 100,000 entries; enter a narrower path");
        }
        entries.sort_by(|a,b| b["directory"].as_bool().cmp(&a["directory"].as_bool())
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str())));
        let total = entries.len();
        let page: Vec<_> = entries.into_iter().skip(query.offset).take(200).collect();
        Ok(json!({"path":path,"parent":path.parent(),"entries":page,"total":total,"offset":query.offset}))
    }).await.map_err(anyhow::Error::from)??;
    Ok(Json(result))
}

async fn start(State(room): State<Arc<Room>>, Json(mut config): Json<RunConfig>) -> ApiResult {
    config.resolve_paths(&std::env::current_dir().map_err(anyhow::Error::from)?)?;
    config.validate()?;
    let reservation = reserve(&room)?;
    *room
        .config
        .lock()
        .map_err(|_| anyhow::anyhow!("Configuration lock poisoned"))? = config.clone();
    *room
        .progress
        .lock()
        .map_err(|_| anyhow::anyhow!("Progress lock poisoned"))? = LiveProgress::default();
    select_report(&room, config.report_dir.clone())?;
    let task_room = room.clone();
    tokio::spawn(async move {
        let _reservation = reservation;
        let progress = task_room.progress.clone();
        let result =
            tokio::task::spawn_blocking(move || validation::execute(config, Some(progress))).await;
        let error = match result {
            Ok(Ok(_)) => None,
            Ok(Err(error)) => Some(format!("{error:#}")),
            Err(error) => Some(format!("Validation worker stopped unexpectedly: {error}")),
        };
        if let Some(error) = error {
            if let Ok(mut progress) = task_room.progress.lock() {
                progress.error = Some(error);
                progress.state = validation::RunState::Failed;
            }
        }
    });
    Ok(Json(json!({"accepted":true})))
}

#[derive(Deserialize)]
struct ResumeStudy {
    receipt: PathBuf,
}

/// Keep the selected study on its recorded runner while preserving its results.
async fn resume_study(State(room): State<Arc<Room>>, Json(request): Json<ResumeStudy>) -> ApiResult {
    let reservation = reserve(&room)?;
    let (mut child, url) = tokio::task::spawn_blocking(move || {
        crate::validation_study::resume(request.receipt)
    }).await.map_err(anyhow::Error::from)??;
    let pid = child.id();
    let progress = room.progress.clone();
    tokio::task::spawn_blocking(move || {
        let _reservation = reservation;
        let error = match child.wait() {
            Ok(status) if status.success() => None,
            Ok(status) => Some(format!("Study runner exited with {status}. Inspect its web-launches logs.")),
            Err(error) => Some(format!("Could not observe study runner exit: {error}")),
        };
        if let Some(error) = error {
            if let Ok(mut progress) = progress.lock() {
                progress.error = Some(error);
                progress.state = validation::RunState::Failed;
            }
        }
    });
    Ok(Json(json!({"pid":pid,"url":url})))
}

/// Keep start, open and export mutually exclusive, including failures and panics.
struct Reservation(Arc<Room>);
impl Drop for Reservation {
    fn drop(&mut self) {
        self.0.active.store(false, Ordering::Release);
    }
}

fn reserve(room: &Arc<Room>) -> Result<Reservation> {
    ensure!(
        !room.active.swap(true, Ordering::AcqRel),
        "Another Observatory operation is active"
    );
    Ok(Reservation(room.clone()))
}

fn select_report(room: &Room, root: PathBuf) -> Result<String> {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let mut registry = room
        .reports
        .lock()
        .map_err(|_| anyhow::anyhow!("Report registry lock poisoned"))?;
    // Stable roots keep previously opened tabs from silently switching reports.
    // Registry entries contain only paths, not report contents or image buffers.
    if registry.len() >= 64 {
        if let Some(oldest) = registry.keys().next().cloned() {
            registry.remove(&oldest);
        }
    }
    registry.insert(id.clone(), root.clone());
    *room
        .report
        .lock()
        .map_err(|_| anyhow::anyhow!("Report lock poisoned"))? = Some((id.clone(), root));
    Ok(format!("/reports/{id}"))
}

#[derive(Deserialize)]
struct OpenReport {
    path: PathBuf,
}
async fn open_report(State(room): State<Arc<Room>>, Json(request): Json<OpenReport>) -> ApiResult {
    let _reservation = reserve(&room)?;
    let (root, run) = tokio::task::spawn_blocking(move || -> Result<(PathBuf, Value)> {
        let root = validation::report_root(&request.path)?;
        let run = validation::read_run(&root)?;
        crate::validation_report::write_site(&root, &run)?;
        Ok((root, run))
    })
    .await
    .map_err(anyhow::Error::from)??;
    let pairs = run["pairs"]
        .as_array()
        .context("Report has no pair index")?;
    let recent = pairs.iter().rev().take(20).cloned().collect();
    let progress = LiveProgress {
        state: match run["status"].as_str() {
            Some("completed_with_limitations" | "completed_with_errors") => {
                validation::RunState::Completed
            }
            Some("failed") => validation::RunState::Failed,
            _ => validation::RunState::Idle,
        },
        report_dir: Some(root.clone()),
        current: Some(format!(
            "Saved report · {}",
            run["status"].as_str().unwrap_or("unknown")
        )),
        total: run["summary"]["total"]
            .as_u64()
            .unwrap_or(pairs.len() as u64) as usize,
        completed: pairs
            .iter()
            .filter(|pair| pair["status"] != "running")
            .count(),
        failed: pairs
            .iter()
            .filter(|pair| matches!(pair["status"].as_str(), Some("failed" | "missing")))
            .count(),
        reused: pairs
            .iter()
            .filter(|pair| pair["status"] == "reused")
            .count(),
        different_pairs: pairs
            .iter()
            .filter(|pair| pair["different_regions"].as_u64().unwrap_or(0) > 0)
            .count(),
        checked_equal_pairs: pairs
            .iter()
            .filter(|pair| {
                pair["status"] == "completed_with_limitations"
                    && pair["different_regions"].as_u64() == Some(0)
            })
            .count(),
        workbooks: pairs
            .iter()
            .filter_map(|pair| pair["id"].as_str().map(|id| (id.to_owned(), pair.clone())))
            .collect(),
        recent,
        ..LiveProgress::default()
    };
    *room
        .progress
        .lock()
        .map_err(|_| anyhow::anyhow!("Progress lock poisoned"))? = progress;
    let base = select_report(&room, root)?;
    Ok(Json(
        json!({"url":format!("{base}/index.html"),"report_base":base}),
    ))
}

#[derive(Deserialize)]
struct ExportReport {
    output: PathBuf,
}
async fn export_report(
    State(room): State<Arc<Room>>,
    Json(request): Json<ExportReport>,
) -> ApiResult {
    let _reservation = reserve(&room)?;
    let (_, root) = room
        .report
        .lock()
        .map_err(|_| anyhow::anyhow!("Report lock poisoned"))?
        .clone()
        .context("Open a report first")?;
    let index = tokio::task::spawn_blocking(move || {
        crate::validation_report::export_site(&root, &request.output)
    })
    .await
    .map_err(anyhow::Error::from)??;
    Ok(Json(json!({"index":index})))
}

/// Only published report assets are served, never the run's working spreadsheets.
async fn report_asset(
    State(room): State<Arc<Room>>,
    RoutePath((id, relative)): RoutePath<(String, String)>,
) -> Response {
    let result = async {
        let root = room
            .reports
            .lock()
            .map_err(|_| anyhow::anyhow!("Report registry lock poisoned"))?
            .get(&id)
            .cloned()
            .context("This report is no longer registered in this Observatory session")?;
        let path = Path::new(&relative);
        ensure!(
            !relative.contains(['\\', ':'])
                && path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "Invalid report asset path"
        );
        ensure!(
            !matches!(
                path.components()
                    .next()
                    .and_then(|part| part.as_os_str().to_str()),
                Some("work" | "records")
            ),
            "Working data is not a published report asset"
        );
        let root = tokio::fs::canonicalize(root).await?;
        let file = tokio::fs::canonicalize(root.join(path)).await?;
        ensure!(file.starts_with(&root), "Asset leaves the selected report");
        let content_type = match file.extension().and_then(|extension| extension.to_str()) {
            Some("html") => "text/html; charset=utf-8",
            Some("js") => "text/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("json") => "application/json",
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("log" | "txt") => "text/plain; charset=utf-8",
            _ => anyhow::bail!("Unsupported report asset type"),
        };
        // ServeFile streams large evidence; do not buffer full captures in HTTP handlers.
        let response = tower_http::services::ServeFile::new(file)
            .try_call(axum::http::Request::new(Body::empty()))
            .await?;
        let mut response = response.into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type.parse()?);
        Ok::<Response, anyhow::Error>(response)
    }
    .await;
    match result {
        Ok(response) => response,
        Err(error) => ApiError(error).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn workbook_pages_filter_the_index_while_status_polling_stays_bounded() {
        let mut progress = LiveProgress::default();
        for index in 0..250 {
            progress.workbooks.insert(
                format!("{index:03}"),
                json!({
                    "id":format!("{index:03}"),"name":format!("workbook-{index:03}.xlsx"),
                    "status":"completed_with_limitations","different_regions":index % 2,
                }),
            );
        }
        let room = Arc::new(Room {
            port: 1234,
            token: "test".into(),
            config: Mutex::new(RunConfig::default()),
            progress: Arc::new(Mutex::new(progress)),
            active: AtomicBool::new(false),
            report: Mutex::new(None),
            reports: Mutex::new(BTreeMap::new()),
            picker: Mutex::new(()),
        });
        let page = workbooks(
            State(room.clone()),
            Query(WorkbookQuery {
                offset: 100,
                search: String::new(),
                filter: "differences".into(),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(page["total"], 125);
        assert_eq!(page["rows"].as_array().unwrap().len(), 25);
        let snapshot = state(State(room)).await.unwrap().0;
        assert!(snapshot["progress"].get("workbooks").is_none());
        assert!(snapshot["progress"]["recent"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn local_api_rejects_foreign_hosts_and_cross_origin_requests() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:1234".parse().unwrap());
        assert!(allowed_headers(&headers, 1234));
        headers.insert(header::HOST, "deltasieve.localhost:1234".parse().unwrap());
        headers.insert(
            header::ORIGIN,
            "http://deltasieve.localhost:1234".parse().unwrap(),
        );
        assert!(allowed_headers(&headers, 1234));
        headers.insert(header::ORIGIN, "https://example.com".parse().unwrap());
        assert!(!allowed_headers(&headers, 1234));
        headers.remove(header::ORIGIN);
        headers.insert(header::HOST, "example.com:1234".parse().unwrap());
        assert!(!allowed_headers(&headers, 1234));
        headers.insert(header::HOST, "127.0.0.1:1234".parse().unwrap());
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(!allowed_headers(&headers, 1234));
    }
}
