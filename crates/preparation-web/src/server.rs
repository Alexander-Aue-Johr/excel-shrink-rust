use anyhow::{ensure, Context, Result};
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use corpus_runner::{
    artifacts,
    memory::MemoryReserve,
    package_sizes::PackageSizes,
    telemetry::{Event, JsonLines, Observer, Operation},
};
use excel_automation::{context::RunContext, conversion, pool::performance};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

const HISTORY_LIMIT: usize = 512;
const JOB_LIMIT: usize = 256;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    pub inputs: Vec<PathBuf>,
    pub output: Option<PathBuf>,
    pub workers: usize,
    pub recursive: bool,
    pub reserve_mib: Option<u64>,
    pub source_manifest: Option<PathBuf>,
    pub reuse_from: Option<PathBuf>,
    #[serde(default)]
    pub max_active_workbooks: Option<usize>,
    #[serde(default)]
    pub parallel_range_chunk_size: Option<usize>,
}

type ShrinkRunner = Arc<dyn Fn(Defaults, RunContext) -> Result<()> + Send + Sync>;

#[derive(Clone, Serialize)]
struct WorkTask {
    id: u64,
    workbook_id: String,
    sheet: String,
    worker: Option<usize>,
    phase: String,
    chunk_index: Option<usize>,
    chunk_count: usize,
    range: Option<String>,
    started_ms: f64,
    ended_ms: Option<f64>,
}

#[derive(Clone, Serialize)]
struct Job {
    id: String,
    name: String,
    worker: Option<usize>,
    started_ms: u64,
    ended_ms: Option<u64>,
    status: String,
    before: Option<PackageSizes>,
    after: Option<PackageSizes>,
    source_bytes: Option<u64>,
    error: Option<String>,
    sheet_names: Value,
    sheet_order: Value,
}

#[derive(Default, Serialize)]
struct Counts {
    total: usize,
    prepared: usize,
    failed: usize,
    reused: usize,
    before_bytes: u64,
    after_bytes: u64,
}

struct RunState {
    id: String,
    status: &'static str,
    error: Option<String>,
    defaults: Defaults,
    counts: Counts,
    jobs: VecDeque<Job>,
    tasks: VecDeque<WorkTask>,
    events: VecDeque<Value>,
    next: u64,
    started: Instant,
    elapsed_ms: u64,
    stop: Option<Arc<AtomicBool>>,
}

impl RunState {
    fn new(defaults: Defaults) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            status: "idle",
            error: None,
            defaults,
            counts: Counts::default(),
            jobs: VecDeque::new(),
            tasks: VecDeque::new(),
            events: VecDeque::new(),
            next: 0,
            started: Instant::now(),
            elapsed_ms: 0,
            stop: None,
        }
    }

    fn active(&self) -> bool {
        matches!(self.status, "starting" | "running" | "stopping")
    }

    fn accept(&mut self, event: &Event) {
        let task_time = event
            .elapsed_micros
            .map(|micros| micros as f64 / 1000.0)
            .unwrap_or(event.elapsed_millis as f64);
        if let Some(id) = &event.workbook_id {
            if event.event_type == "work.started" {
                while self.tasks.len() >= 4096 {
                    let Some(position) = self.tasks.iter().position(|task| task.ended_ms.is_some())
                    else {
                        break;
                    };
                    self.tasks.remove(position);
                }
                self.tasks.push_back(WorkTask {
                    id: event.payload["task_id"].as_u64().unwrap(),
                    workbook_id: id.clone(),
                    sheet: event.payload["sheet"].as_str().unwrap().into(),
                    worker: event.worker_index,
                    phase: event.payload["phase"].as_str().unwrap().into(),
                    chunk_index: event.payload["chunk_index"]
                        .as_u64()
                        .map(|index| index as usize),
                    chunk_count: event.payload["chunk_count"].as_u64().unwrap_or(1) as usize,
                    range: event.payload["range"].as_str().map(str::to_owned),
                    started_ms: task_time,
                    ended_ms: None,
                });
            }
        }
        if event.event_type == "work.finished" {
            if let Some(task) = self
                .tasks
                .iter_mut()
                .find(|task| Some(task.id) == event.payload["task_id"].as_u64())
            {
                task.ended_ms = Some(task_time);
            }
        }
        if event.event_type == "run.started" {
            self.counts.total = event.payload["total"].as_u64().unwrap_or(0) as usize;
            self.status = if self.status == "stopping" {
                "stopping"
            } else {
                "running"
            };
        }
        if event.event_type == "workbook.reused" {
            self.counts.prepared += 1;
            self.counts.reused += 1;
        }
        if let Some(id) = &event.workbook_id {
            if event.event_type == "job.started" {
                while self.jobs.len() >= JOB_LIMIT {
                    let Some(position) = self.jobs.iter().position(|job| job.ended_ms.is_some())
                    else {
                        break;
                    };
                    self.jobs.remove(position);
                }
                self.jobs.push_back(Job {
                    id: id.clone(),
                    name: event.payload["name"].as_str().unwrap_or(id).to_owned(),
                    worker: event.worker_index,
                    started_ms: event.elapsed_millis,
                    ended_ms: None,
                    status: "running".into(),
                    before: None,
                    after: None,
                    source_bytes: None,
                    error: None,
                    sheet_names: Value::Null,
                    sheet_order: Value::Null,
                });
            }
            if let Some(job) = self.jobs.iter_mut().find(|job| job.id == *id) {
                match event.event_type.as_str() {
                    "workbook.metadata" => {
                        job.sheet_names = event.payload["sheet_names"].clone();
                        job.sheet_order = event.payload["sheet_order"].clone();
                        job.before = serde_json::from_value(event.payload["package"].clone()).ok();
                        job.source_bytes = event.payload["file_bytes"].as_u64();
                    }
                    "job.completed" | "job.failed" => {
                        job.ended_ms = Some(event.elapsed_millis);
                        job.status = if event.event_type == "job.completed" {
                            "prepared"
                        } else {
                            "failed"
                        }
                        .into();
                        job.error = event.payload["error"].as_str().map(str::to_owned);
                        let sizes = &event.payload["archive_sizes"];
                        if let Ok(before) =
                            serde_json::from_value(sizes["before_optimization"].clone())
                        {
                            job.before = Some(before);
                        }
                        job.after = serde_json::from_value(sizes["after_optimization"].clone())
                            .or_else(|_| serde_json::from_value(sizes["converted"].clone()))
                            .ok();
                        if job.status == "prepared" {
                            self.counts.prepared += 1;
                            if let (Some(before), Some(after)) = (&job.before, &job.after) {
                                self.counts.before_bytes += before.file_bytes;
                                self.counts.after_bytes += after.file_bytes;
                            }
                        } else {
                            self.counts.failed += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        self.elapsed_ms = event.elapsed_millis;
        self.events
            .push_back(json!({"cursor": self.next, "event": event}));
        self.next += 1;
        while self.events.len() > HISTORY_LIMIT {
            self.events.pop_front();
        }
    }
}

#[derive(Clone)]
struct App {
    operation: Operation,
    token: String,
    authority: String,
    run: Arc<Mutex<RunState>>,
    shrink_runner: Option<ShrinkRunner>,
}

struct RunObserver {
    state: Arc<Mutex<RunState>>,
    log: JsonLines,
}

impl Observer for RunObserver {
    fn publish(&self, event: &Event) -> Result<()> {
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("Dashboard state poisoned"))?
            .accept(event);
        self.log.publish(event)
    }
}

pub fn serve(operation: Operation, defaults: Defaults, port: u16) -> Result<()> {
    serve_inner(operation, defaults, port, None, false)
}

pub fn serve_with_browser(
    operation: Operation,
    defaults: Defaults,
    port: u16,
    open_browser: bool,
) -> Result<()> {
    serve_inner(operation, defaults, port, None, open_browser)
}

pub fn serve_sheetshrink(
    defaults: Defaults,
    port: u16,
    runner: impl Fn(Defaults, RunContext) -> Result<()> + Send + Sync + 'static,
) -> Result<()> {
    serve_inner(
        Operation::SheetShrink,
        defaults,
        port,
        Some(Arc::new(runner)),
        false,
    )
}

fn serve_inner(
    operation: Operation,
    defaults: Defaults,
    port: u16,
    shrink_runner: Option<ShrinkRunner>,
    open_browser: bool,
) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(async {
            let listener =
                tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
            let authority = listener.local_addr()?.to_string();
            let app = App {
                operation,
                token: uuid::Uuid::new_v4().to_string(),
                authority: authority.clone(),
                run: Arc::new(Mutex::new(RunState::new(defaults))),
                shrink_runner,
            };
            let routes = Router::new()
                .route("/", get(index))
                .route(
                    "/app.js",
                    get(|| async {
                        (
                            [(header::CONTENT_TYPE, "text/javascript")],
                            include_str!("../assets/app.js"),
                        )
                    }),
                )
                .route(
                    "/app.css",
                    get(|| async {
                        (
                            [(header::CONTENT_TYPE, "text/css")],
                            include_str!("../assets/app.css"),
                        )
                    }),
                )
                .route("/api/state", get(state))
                .route("/api/atlas", get(atlas))
                .route("/api/replay", get(replay))
                .route("/api/start", post(start))
                .route("/api/stop", post(stop))
                .route("/api/workers", post(workers))
                .layer(axum::middleware::from_fn_with_state(
                    app.clone(),
                    check_host,
                ))
                .with_state(app);
            println!("Preparation Observatory: http://{authority}/");
            if open_browser {
                use std::os::windows::process::CommandExt;
                // Dispatch the loopback URL through the user's Windows URL association.
                let launcher = std::path::PathBuf::from(
                    std::env::var_os("SystemRoot").context("SystemRoot is unavailable")?,
                )
                .join("System32/rundll32.exe");
                std::process::Command::new(launcher)
                    .args([
                        "url.dll,FileProtocolHandler",
                        &format!("http://{authority}/"),
                    ])
                    .creation_flags(0x0800_0000)
                    .spawn()
                    .context("Could not launch the default browser")?;
            }
            axum::serve(listener, routes).await?;
            Ok(())
        })
}

async fn check_host(
    State(app): State<App>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        != Some(app.authority.as_str())
    {
        return (StatusCode::FORBIDDEN, "Invalid local host").into_response();
    }
    next.run(request).await
}

async fn index(State(app): State<App>) -> Response {
    let title = match app.operation {
        Operation::SheetShrink => "SheetShrink",
        Operation::XlsxConvert => "XLSX Convert",
        Operation::ExcelRebuild => "Excel Rebuild",
        Operation::CheckPerformance => "Excel Check Performance",
    };
    let html = include_str!("../assets/index.html")
        .replace("__TOKEN__", &app.token)
        .replace("__APPLICATION__", title);
    ([(header::CACHE_CONTROL, "no-store"),
      (header::CONTENT_SECURITY_POLICY, "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; font-src 'self'; frame-ancestors 'none'")], Html(html)).into_response()
}

#[derive(Default, Deserialize)]
struct Cursor {
    run: Option<String>,
    after: Option<u64>,
}

#[derive(Default, Deserialize)]
struct AtlasQuery {
    run: String,
    #[serde(default)]
    page: usize,
    page_size: Option<usize>,
}

#[derive(Deserialize)]
struct ReplayQuery {
    run: String,
    #[serde(default)]
    after: u64,
}

/// Read bounded pages from the durable log after writers have finished.
/// Byte offsets avoid rescanning preceding events and bind to the current run.
async fn replay(State(app): State<App>, Query(query): Query<ReplayQuery>) -> Response {
    let defaults = {
        let run = app.run.lock().unwrap();
        if query.run != run.id || run.active() {
            return rejected("Replay requires the current completed or stopped run");
        }
        run.defaults.clone()
    };
    let result = tokio::task::spawn_blocking(move || replay_page(defaults, query)).await;
    match result {
        Ok(Ok(page)) => Json(page).into_response(),
        Ok(Err(error)) => rejected(error),
        Err(error) => rejected(error),
    }
}

/// Resume at a byte cursor returned by the preceding page, without rescanning it.
fn replay_page(defaults: Defaults, query: ReplayQuery) -> Result<Value> {
    let output = defaults.output.context("No recorded output directory")?;
    let mut name = output.into_os_string();
    name.push(".events.jsonl");
    let file = std::fs::File::open(PathBuf::from(name))?;
    let length = file.metadata()?.len();
    ensure!(
        query.after <= length,
        "Replay cursor is beyond the event log"
    );
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(query.after))?;
    let mut events = Vec::new();
    let mut line = String::new();
    for _ in 0..1000 {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let event: Event = serde_json::from_str(&line).context("Invalid recorded event")?;
        events.push(event);
    }
    let next = reader.stream_position()?;
    Ok(json!({"run_id": query.run, "events": events, "next": next, "complete": next == length}))
}

// Read the durable history on demand. Ordinary status polling keeps its bounded
// history, and only the requested atlas page enters the browser.
fn atlas_page(defaults: Defaults, query: AtlasQuery) -> Result<Value> {
    let page_size = query.page_size.unwrap_or(7).clamp(1, 24);
    let Some(output) = defaults.output.as_ref() else {
        return Ok(json!({"run_id": query.run, "total": 0, "page": 0, "jobs": []}));
    };
    let mut log_name = output.as_os_str().to_os_string();
    log_name.push(".events.jsonl");
    let log_path = PathBuf::from(log_name);
    if !log_path.exists() {
        return Ok(json!({"run_id": query.run, "total": 0, "page": 0, "jobs": []}));
    }
    let mut sizes = HashMap::new();
    for line in BufReader::new(std::fs::File::open(&log_path)?).split(b'\n') {
        let line = line?;
        // A writer may currently be appending its final line.
        let event = match serde_json::from_slice::<Event>(&line) {
            Ok(event) => event,
            Err(error) if error.is_eof() => continue,
            Err(error) => return Err(error).context("Read atlas event history"),
        };
        let Some(id) = event.workbook_id else {
            continue;
        };
        match event.event_type.as_str() {
            "job.started" => {
                sizes.entry(id).or_insert(0);
            }
            "workbook.metadata" => {
                sizes.insert(
                    id,
                    event.payload["package"]["uncompressed_payload_bytes"]
                        .as_u64()
                        .unwrap_or(0),
                );
            }
            "job.completed" => {
                if let Some(bytes) = event.payload["archive_sizes"]["before_optimization"]
                    ["uncompressed_payload_bytes"]
                    .as_u64()
                {
                    sizes.insert(id, bytes);
                }
            }
            _ => {}
        }
    }
    let mut ranked: Vec<_> = sizes.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let total = ranked.len();
    let page = query.page.min(total.saturating_sub(1) / page_size);
    let ids: HashSet<_> = ranked
        .iter()
        .skip(page * page_size)
        .take(page_size)
        .map(|(id, _)| id.clone())
        .collect();
    let mut history = RunState::new(defaults);
    for line in BufReader::new(std::fs::File::open(&log_path)?).split(b'\n') {
        let line = line?;
        let event = match serde_json::from_slice::<Event>(&line) {
            Ok(event) => event,
            Err(error) if error.is_eof() => continue,
            Err(error) => return Err(error).context("Read atlas event history"),
        };
        if event
            .workbook_id
            .as_ref()
            .is_some_and(|id| ids.contains(id))
            && matches!(
                event.event_type.as_str(),
                "job.started" | "workbook.metadata" | "job.completed" | "job.failed"
            )
        {
            history.accept(&event);
        }
    }
    let mut jobs: Vec<_> = history.jobs.into_iter().collect();
    jobs.sort_by_key(|job| ranked.iter().position(|(id, _)| id == &job.id));
    Ok(json!({"run_id": query.run, "total": total, "page": page, "jobs": jobs}))
}

async fn atlas(State(app): State<App>, Query(query): Query<AtlasQuery>) -> Response {
    let defaults = {
        let run = app.run.lock().unwrap();
        if query.run != run.id {
            return rejected("The run has changed. Refresh the atlas");
        }
        run.defaults.clone()
    };
    match tokio::task::spawn_blocking(move || atlas_page(defaults, query)).await {
        Ok(Ok(page)) => ([(header::CACHE_CONTROL, "no-store")], Json(page)).into_response(),
        Ok(Err(error)) => rejected(error),
        Err(error) => rejected(error),
    }
}

async fn state(State(app): State<App>, Query(cursor): Query<Cursor>) -> Response {
    let run = app.run.lock().unwrap();
    let first = run
        .events
        .front()
        .and_then(|event| event["cursor"].as_u64())
        .unwrap_or(run.next);
    let reset = cursor.run.as_deref() != Some(&run.id)
        || cursor
            .after
            .is_none_or(|after| after < first || after > run.next);
    let events: Vec<_> = if reset {
        Vec::new()
    } else {
        run.events
            .iter()
            .filter(|event| event["cursor"].as_u64().unwrap() >= cursor.after.unwrap())
            .cloned()
            .collect()
    };
    let elapsed_ms = if run.active() {
        run.started.elapsed().as_millis() as u64
    } else {
        run.elapsed_ms
    };
    ([(header::CACHE_CONTROL, "no-store")], Json(json!({
        "run_id": run.id, "operation": app.operation, "status": run.status, "error": run.error,
            "defaults": if reset { Some(&run.defaults) } else { None }, "workers": run.defaults.workers,
            "counts": run.counts, "elapsed_ms": elapsed_ms,
        "next": run.next, "reset": reset, "jobs": if reset { Some(&run.jobs) } else { None }, "events": events,
            "tasks": if reset { Some(&run.tasks) } else { None },
        "history_limit": JOB_LIMIT,
    }))).into_response()
}

fn authorize(app: &App, headers: &HeaderMap) -> Result<()> {
    ensure!(
        headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            == Some(app.authority.as_str()),
        "Invalid local host"
    );
    ensure!(
        headers
            .get("x-observatory-token")
            .and_then(|value| value.to_str().ok())
            == Some(app.token.as_str()),
        "Invalid request token"
    );
    if let Some(origin) = headers.get(header::ORIGIN) {
        ensure!(
            origin.to_str()? == format!("http://{}", app.authority),
            "Invalid origin"
        );
    }
    Ok(())
}

fn rejected(error: impl std::fmt::Display) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": error.to_string()})),
    )
        .into_response()
}

async fn start(
    State(app): State<App>,
    headers: HeaderMap,
    Json(defaults): Json<Defaults>,
) -> Response {
    if let Err(error) = authorize(&app, &headers) {
        return rejected(error);
    }
    if defaults.workers == 0 || defaults.workers > 256 {
        return rejected("Choose between 1 and 256 workers");
    }
    if matches!(app.operation, Operation::SheetShrink) {
        let logical = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1);
        if defaults.workers > logical {
            return rejected(format!(
                "SheetShrink supports up to {logical} Rayon threads"
            ));
        }
        if defaults.max_active_workbooks == Some(0) {
            return rejected("Active workbook limit must be positive");
        }
    }
    if let Some(mib) = defaults.reserve_mib {
        if let Err(error) = MemoryReserve::from_mib(mib) {
            return rejected(error);
        }
    }
    let Some(output) = defaults.output.as_ref() else {
        return rejected("Choose a new output directory");
    };
    if output.exists() {
        return rejected("Output directory already exists. Choose a new directory");
    }
    if !output.parent().is_some_and(|parent| parent.is_dir()) {
        return rejected("Output parent directory must exist");
    }
    if defaults.inputs.is_empty() && defaults.source_manifest.is_none() {
        return rejected("Choose input files or a source manifest");
    }
    if !defaults.inputs.is_empty() && defaults.source_manifest.is_some() {
        return rejected("Choose either input paths or a source manifest");
    }
    if !matches!(app.operation, Operation::CheckPerformance)
        && (defaults.source_manifest.is_some() || defaults.reuse_from.is_some())
    {
        return rejected("This operation accepts input paths and a new output directory");
    }
    let mut log_name = output.as_os_str().to_os_string();
    log_name.push(".events.jsonl");
    let log_path = PathBuf::from(log_name);
    let mut run = app.run.lock().unwrap();
    if run.active() {
        return rejected("A preparation is already running");
    }
    let log = match JsonLines::create(&log_path) {
        Ok(log) => log,
        Err(error) => return rejected(error),
    };
    *run = RunState::new(defaults.clone());
    run.status = "starting";
    let mut context = RunContext::observed(
        app.operation,
        Arc::new(RunObserver {
            state: Arc::clone(&app.run),
            log,
        }),
    );
    context.memory_reserve = defaults
        .reserve_mib
        .map(MemoryReserve::from_mib)
        .transpose()
        .unwrap();
    run.stop = Some(context.stop_handle());
    drop(run);
    tokio::spawn(async move {
        let operation = app.operation;
        let runner = app.shrink_runner.clone();
        let result = tokio::task::spawn_blocking(move || match runner {
            Some(runner) => runner(defaults, context),
            None => execute(operation, defaults, context),
        })
        .await;
        let mut run = app.run.lock().unwrap();
        run.elapsed_ms = run.started.elapsed().as_millis() as u64;
        let stopped = run
            .stop
            .as_ref()
            .is_some_and(|stop| stop.load(Ordering::Acquire));
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(format!("{error:#}")),
            Err(error) => Some(error.to_string()),
        };
        run.status = if stopped {
            "stopped"
        } else if error.is_some() {
            "failed"
        } else {
            "completed"
        };
        run.error = error;
        let elapsed = run.elapsed_ms;
        for job in run.jobs.iter_mut().filter(|job| job.ended_ms.is_none()) {
            job.ended_ms = Some(elapsed);
            job.status = "interrupted".into();
        }
        run.stop = None;
        for task in run.tasks.iter_mut().filter(|task| task.ended_ms.is_none()) {
            task.ended_ms = Some(elapsed as f64);
        }
    });
    Json(json!({"accepted": true})).into_response()
}

fn execute(operation: Operation, defaults: Defaults, context: RunContext) -> Result<()> {
    let output = defaults.output.context("Output missing")?;
    match operation {
        Operation::XlsxConvert => conversion::run(
            conversion::Options {
                inputs: defaults.inputs,
                output,
                workers: defaults.workers,
                recursive: defaults.recursive,
            },
            context,
        ),
        Operation::ExcelRebuild => conversion::rebuild(
            conversion::Options {
                inputs: defaults.inputs,
                output,
                workers: defaults.workers,
                recursive: defaults.recursive,
            },
            context,
        ),
        Operation::CheckPerformance => {
            let inputs = if defaults.inputs.is_empty() {
                Vec::new()
            } else {
                corpus_runner::sources::expand_paths(
                    &defaults.inputs,
                    &["xlsx"],
                    defaults.recursive,
                )?
            };
            performance::run_with_context(
                performance::Args {
                    input: inputs,
                    source_manifest: defaults.source_manifest,
                    output,
                    workers: defaults.workers,
                    reuse_from: defaults.reuse_from,
                },
                context,
            )
        }
        Operation::SheetShrink => anyhow::bail!("This host serves native Excel preparation"),
    }
}

async fn stop(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Err(error) = authorize(&app, &headers) {
        return rejected(error);
    }
    let mut run = app.run.lock().unwrap();
    if let Some(stop) = &run.stop {
        stop.store(true, Ordering::Release);
        run.status = "stopping";
    }
    Json(json!({"accepted": true})).into_response()
}

#[derive(Deserialize)]
struct Workers {
    workers: usize,
}

async fn workers(
    State(app): State<App>,
    headers: HeaderMap,
    Json(request): Json<Workers>,
) -> Response {
    if matches!(app.operation, Operation::SheetShrink) {
        return rejected("Rayon threads are configured before a SheetShrink run");
    }
    if let Err(error) = authorize(&app, &headers) {
        return rejected(error);
    }
    if request.workers == 0 || request.workers > 256 {
        return rejected("Choose between 1 and 256 workers");
    }
    let mut run = app.run.lock().unwrap();
    if run.status != "running" {
        return rejected("Wait for the preparation to start");
    }
    let output = run.defaults.output.as_ref().unwrap();
    if let Err(error) = artifacts::atomic_write(
        &output.join("control.json"),
        &serde_json::to_vec(&json!({"version": 1, "workers": request.workers})).unwrap(),
    ) {
        return rejected(error);
    }
    run.defaults.workers = request.workers;
    Json(json!({"accepted": true})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App {
            operation: Operation::XlsxConvert,
            token: "test-token".into(),
            authority: "127.0.0.1:12345".into(),
            run: Arc::new(Mutex::new(RunState::new(Defaults {
                inputs: Vec::new(),
                output: None,
                workers: 2,
                recursive: false,
                reserve_mib: None,
                source_manifest: None,
                reuse_from: None,
                max_active_workbooks: None,
                parallel_range_chunk_size: None,
            }))),
            shrink_runner: None,
        }
    }

    fn event(sequence: u64, kind: &str, id: &str) -> Event {
        Event {
            version: 1,
            sequence,
            elapsed_millis: sequence,
            elapsed_micros: None,
            operation: Operation::XlsxConvert,
            event_type: kind.into(),
            workbook_id: Some(id.into()),
            worker_index: Some(0),
            payload: json!({"name": id}),
        }
    }

    #[test]
    fn replay_pages_preserve_all_events_and_reject_invalid_cursors() {
        use std::io::Write;
        let directory = std::env::temp_dir().join(format!("replay-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("result.events.jsonl");
        let mut log = std::fs::File::create(&path).unwrap();
        for index in 0..1007 {
            writeln!(
                log,
                "{}",
                serde_json::to_string(&event(index, "work.started", "sheet-ÃŽÂ±")).unwrap()
            )
            .unwrap();
        }
        drop(log);
        let app = app();
        let mut defaults = app.run.lock().unwrap().defaults.clone();
        defaults.output = Some(directory.join("result"));
        let query = |after| ReplayQuery {
            run: "test".into(),
            after,
        };
        let first = replay_page(defaults.clone(), query(0)).unwrap();
        assert_eq!(first["events"].as_array().unwrap().len(), 1000);
        assert_eq!(first["complete"], false);
        let second = replay_page(defaults.clone(), query(first["next"].as_u64().unwrap())).unwrap();
        assert_eq!(second["events"].as_array().unwrap().len(), 7);
        assert_eq!(second["events"][0]["sequence"], 1000);
        assert_eq!(second["complete"], true);
        let end = second["next"].as_u64().unwrap();
        assert!(replay_page(defaults.clone(), query(end + 1)).is_err());
        assert!(replay_page(defaults.clone(), query(1)).is_err());
        assert!(replay_page(defaults.clone(), query(end)).unwrap()["events"]
            .as_array()
            .unwrap()
            .is_empty());
        std::fs::write(&path, "{incomplete event").unwrap();
        assert!(replay_page(defaults, query(0)).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[tokio::test]
    async fn replay_rejects_active_runs_and_stale_run_ids() {
        let app = app();
        let id = {
            let mut run = app.run.lock().unwrap();
            run.status = "running";
            run.id.clone()
        };
        let response = replay(State(app.clone()), Query(ReplayQuery { run: id, after: 0 })).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        app.run.lock().unwrap().status = "completed";
        let response = replay(
            State(app),
            Query(ReplayQuery {
                run: "stale".into(),
                after: 0,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn atlas_pages_include_jobs_evicted_from_live_history() {
        use std::io::Write;
        let directory = std::env::temp_dir().join(format!("atlas-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let output = directory.join("result");
        let mut log = std::fs::File::create(directory.join("result.events.jsonl")).unwrap();
        for index in 0..300 {
            let id = format!("book-{index:03}");
            let start = event(index * 2, "job.started", &id);
            writeln!(log, "{}", serde_json::to_string(&start).unwrap()).unwrap();
            let mut metadata = event(index * 2 + 1, "workbook.metadata", &id);
            metadata.payload = json!({"package": {"file_bytes": 1000, "compressed_payload_bytes": 900, "uncompressed_payload_bytes": 300 - index, "worksheets": []}});
            writeln!(log, "{}", serde_json::to_string(&metadata).unwrap()).unwrap();
        }
        drop(log);
        let app = app();
        let mut defaults = app.run.lock().unwrap().defaults.clone();
        defaults.output = Some(output);
        let page = atlas_page(
            defaults.clone(),
            AtlasQuery {
                run: "test".into(),
                page: 0,
                page_size: Some(7),
            },
        )
        .unwrap();
        assert_eq!(page["total"], 300);
        assert_eq!(page["jobs"].as_array().unwrap().len(), 7);
        assert_eq!(page["jobs"][0]["id"], "book-000");
        let last = atlas_page(
            defaults,
            AtlasQuery {
                run: "test".into(),
                page: usize::MAX,
                page_size: Some(7),
            },
        )
        .unwrap();
        assert_eq!(last["page"], 42);
        assert_eq!(last["jobs"].as_array().unwrap().len(), 6);
        assert_eq!(last["jobs"][5]["id"], "book-299");
        std::fs::remove_file(directory.join("result.events.jsonl")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn history_eviction_retains_global_counts() {
        let app = app();
        let mut state = app.run.lock().unwrap();
        for index in 0..700 {
            let id = index.to_string();
            state.accept(&event(index * 2, "job.started", &id));
            state.accept(&event(index * 2 + 1, "job.completed", &id));
        }
        assert_eq!(state.counts.prepared, 700);
        assert_eq!(state.jobs.len(), JOB_LIMIT);
        assert_eq!(state.events.len(), HISTORY_LIMIT);
        assert_eq!(
            state.events.front().unwrap()["cursor"],
            1400 - HISTORY_LIMIT
        );
    }

    #[test]
    fn mutations_require_local_authority_token_and_origin() {
        let app = app();
        let mut headers = HeaderMap::new();
        assert!(authorize(&app, &headers).is_err());
        headers.insert(header::HOST, "127.0.0.1:12345".parse().unwrap());
        headers.insert("x-observatory-token", "test-token".parse().unwrap());
        assert!(authorize(&app, &headers).is_ok());
        headers.insert(header::ORIGIN, "https://unrelated.example".parse().unwrap());
        assert!(authorize(&app, &headers).is_err());
        headers.insert(header::ORIGIN, "http://127.0.0.1:12345".parse().unwrap());
        assert!(authorize(&app, &headers).is_ok());
        headers.insert(header::HOST, "unrelated.example".parse().unwrap());
        assert!(authorize(&app, &headers).is_err());
    }

    #[test]
    fn chunk_events_retain_fractional_millisecond_times_without_counting_workbooks() {
        let app = app();
        let mut state = app.run.lock().unwrap();
        let mut start = event(0, "work.started", "book");
        start.elapsed_micros = Some(125);
        start.payload = json!({"task_id": 7, "sheet": "xl/worksheets/sheet1.xml", "phase": "chunk", "chunk_index": 2, "chunk_count": 4});
        state.accept(&start);
        let mut end = event(1, "work.finished", "book");
        end.elapsed_micros = Some(875);
        end.payload = json!({"task_id": 7});
        state.accept(&end);
        assert_eq!(state.tasks.len(), 1);
        assert_eq!(
            state.tasks[0].ended_ms.unwrap() - state.tasks[0].started_ms,
            0.75
        );
        assert_eq!(state.tasks[0].chunk_index, Some(2));
        assert_eq!(state.counts.prepared, 0);
        assert!(state.jobs.is_empty());
    }
}
