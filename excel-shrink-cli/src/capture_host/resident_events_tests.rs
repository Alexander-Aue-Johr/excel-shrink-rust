use super::*;
use crate::capture_host::resident::{self, Client, Dispatch};
use crate::capture_host::{read_json, write_json};
use axum::body::Bytes;
use axum::extract::Path as RoutePath;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::tungstenite::Message as ClientMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Fixture {
    host: Arc<Host>,
    directory: PathBuf,
    url: String,
    server: tokio::task::JoinHandle<()>,
    _watcher: RecommendedWatcher,
}

impl Fixture {
    async fn new(paused: bool) -> Self {
        let directory =
            std::env::temp_dir().join(format!("resident-events-{}", uuid::Uuid::new_v4()));
        let control = directory.join("control");
        std::fs::create_dir_all(&control).unwrap();
        std::fs::create_dir_all(directory.join("clients")).unwrap();
        write_json(
            &control.join("control.json"),
            &json!({"paused":paused}),
            false,
        )
        .await
        .unwrap();
        write_json(
            &directory.join("clients/one.json"),
            &json!({"client_id":"one", "workbook":"test.xlsx"}),
            false,
        )
        .await
        .unwrap();
        let mut client = Client::new();
        client.token = "worker-token".into();
        client.expected_workbook = Some("test.xlsx".into());
        client.configured_at = Instant::now() - Duration::from_secs(3);
        client.ready_at = Some("ready".into());
        client.startup_sampled = true;
        let mut peer = Client::new();
        peer.ready_at = Some("peer-ready".into());
        peer.startup_sampled = true;
        let host = Arc::new(Host {
            session: directory.clone(),
            launcher_token: Some("launcher-token".into()),
            control_root: Some(control),
            primary_clients: vec!["one".into(), "peer".into()],
            clients: RwLock::new(HashMap::from([
                ("one".into(), Arc::new(Mutex::new(client))),
                ("peer".into(), Arc::new(Mutex::new(peer))),
            ])),
            dispatch: Mutex::new(Dispatch {
                resetting: HashSet::new(),
                paused: true,
            }),
            events: Changes::new(),
            coordinator_token: Some("coordinator-token".into()),
            diagnostics: None,
        });
        let watcher = watch_files(&host).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/events", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/events", get(upgrade))
            .with_state(host.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            host,
            directory,
            url,
            server,
            _watcher: watcher,
        }
    }

    async fn connect(&self, authentication: Value) -> Socket {
        let (mut socket, _) = tokio_tungstenite::connect_async(&self.url).await.unwrap();
        send_client(&mut socket, authentication).await;
        socket
    }

    async fn worker(&self) -> Socket {
        let mut socket = self
            .connect(json!({"role":"worker","client":"one","token":"worker-token"}))
            .await;
        assert_eq!(read(&mut socket).await["type"], "changed");
        socket
    }

    async fn command(&self, id: &str) {
        write_json(
            &self.directory.join(format!("commands/one/{id}.json")),
            &json!({"id":id,"kind":"release"}),
            false,
        )
        .await
        .unwrap();
    }

    async fn complete(&self, id: &str, result: Value) -> resident::JsonResponse {
        let mut headers = HeaderMap::new();
        headers.insert("X-Client-Token", "worker-token".parse().unwrap());
        resident::result(
            State(self.host.clone()),
            RoutePath(("one".into(), id.into())),
            headers,
            Bytes::from(serde_json::to_vec(&result).unwrap()),
        )
        .await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        // Only this unique test directory is removed; no corpus paths are used.
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

async fn send_client(socket: &mut Socket, value: Value) {
    socket
        .send(ClientMessage::Text(value.to_string().into()))
        .await
        .unwrap();
}

async fn read(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                ClientMessage::Text(text) => return serde_json::from_str(&text).unwrap(),
                ClientMessage::Ping(bytes) => {
                    socket.send(ClientMessage::Pong(bytes)).await.unwrap()
                }
                other => panic!("Unexpected websocket message: {other:?}"),
            }
        }
    })
    .await
    .expect("Resident event did not arrive")
}

async fn next_job(socket: &mut Socket) -> Value {
    send_client(socket, json!({"type":"next"})).await;
    loop {
        let value = read(socket).await;
        if value["type"] != "changed" {
            return value;
        }
    }
}

#[tokio::test]
async fn file_publication_wakes_an_idle_worker_without_polling() {
    let fixture = Fixture::new(false).await;
    let mut worker = fixture.worker().await;
    assert_eq!(next_job(&mut worker).await["type"], "idle");
    fixture.command("first").await;
    assert_eq!(read(&mut worker).await["type"], "changed");
    let job = next_job(&mut worker).await;
    assert_eq!(job["type"], "job");
    assert_eq!(job["command"]["id"], "first");
}

#[tokio::test]
async fn reconnect_replays_active_job_and_identical_result_retry_is_idempotent() {
    let fixture = Fixture::new(false).await;
    fixture.command("first").await;
    let mut worker = fixture.worker().await;
    assert_eq!(next_job(&mut worker).await["command"]["id"], "first");
    worker.close(None).await.unwrap();
    let mut replacement = fixture.worker().await;
    assert_eq!(next_job(&mut replacement).await["command"]["id"], "first");
    assert_eq!(
        fixture
            .host
            .find_client("one")
            .await
            .unwrap()
            .lock()
            .await
            .command_count,
        1
    );
    let result = json!({"ok":true,"elapsed_ms":4});
    let _ = fixture.complete("first", result.clone()).await.unwrap();
    assert_eq!(
        fixture.complete("first", result.clone()).await.unwrap().0["repeated"],
        true
    );
    assert_eq!(
        fixture
            .complete("first", json!({"ok":true,"elapsed_ms":5}))
            .await
            .unwrap_err()
            .status
            .as_u16(),
        409
    );
    assert_eq!(
        read_json(&fixture.directory.join("responses/one/first.json"))
            .await
            .unwrap(),
        result
    );
}

#[tokio::test]
async fn control_file_resume_wakes_paused_worker() {
    let fixture = Fixture::new(true).await;
    fixture.command("first").await;
    let mut worker = fixture.worker().await;
    assert_eq!(next_job(&mut worker).await["status"]["paused"], true);
    write_json(
        &fixture.directory.join("control/control.json"),
        &json!({"paused":false}),
        true,
    )
    .await
    .unwrap();
    assert_eq!(read(&mut worker).await["type"], "changed");
    assert_eq!(next_job(&mut worker).await["command"]["id"], "first");
}

#[tokio::test]
async fn coordinator_is_notified_only_after_result_is_published() {
    let fixture = Fixture::new(false).await;
    fixture.command("first").await;
    let mut worker = fixture.worker().await;
    next_job(&mut worker).await;
    let mut coordinator = fixture
        .connect(json!({"role":"coordinator","token":"coordinator-token"}))
        .await;
    assert_eq!(read(&mut coordinator).await["type"], "changed");
    let _ = fixture.complete("first", json!({"ok":true})).await.unwrap();
    assert_eq!(read(&mut coordinator).await["type"], "changed");
    assert!(fixture.directory.join("responses/one/first.json").exists());
}

#[tokio::test]
async fn wrong_token_and_retired_worker_cannot_dispatch() {
    let fixture = Fixture::new(false).await;
    let mut rejected = fixture
        .connect(json!({"role":"worker","client":"one","token":"wrong"}))
        .await;
    assert_eq!(read(&mut rejected).await["type"], "error");
    let mut worker = fixture.worker().await;
    let mut headers = HeaderMap::new();
    headers.insert("X-Launcher-Token", "launcher-token".parse().unwrap());
    let _ = resident::reset(
        State(fixture.host.clone()),
        RoutePath("one".into()),
        headers,
        Bytes::from_static(br#"{"stage":"prepare"}"#),
    )
    .await
    .unwrap();
    assert_eq!(read(&mut worker).await["type"], "error");
}

#[tokio::test]
async fn watcher_failure_is_reported_instead_of_waiting_forever() {
    let fixture = Fixture::new(false).await;
    let mut worker = fixture.worker().await;
    fixture.host.events.failed("test watcher failure".into());
    let error = read(&mut worker).await;
    assert_eq!(error["type"], "error");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("watcher failure"));
    let health = resident::health(State(fixture.host.clone())).await.0;
    assert_eq!(health["errors"][0]["code"], "EVENT_WATCH_FAILED");
}

#[tokio::test]
async fn startup_quiet_period_wakes_once_without_a_job_poll_timer() {
    let fixture = Fixture::new(false).await;
    fixture.host.clients.write().await.remove("one");
    fixture.command("first").await;
    let query = HashMap::from([
        ("client".into(), "one".into()),
        ("document".into(), "document".into()),
        ("run".into(), "run".into()),
        ("request".into(), "request".into()),
    ]);
    let config = resident::configure(State(fixture.host.clone()), axum::extract::Query(query))
        .await
        .unwrap()
        .0;
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Client-Token",
        config["client_token"].as_str().unwrap().parse().unwrap(),
    );
    let ready = resident::ready(
        State(fixture.host.clone()),
        axum::extract::Query(HashMap::from([("client".into(), "one".into())])),
        headers,
        Bytes::from_static(br#"{"workbook":"test.xlsx"}"#),
    )
    .await
    .unwrap()
    .0;
    let mut launcher = HeaderMap::new();
    launcher.insert("X-Launcher-Token", "launcher-token".parse().unwrap());
    let _ = resident::startup_sampled(
        State(fixture.host.clone()),
        RoutePath("one".into()),
        launcher,
        Bytes::from(serde_json::to_vec(&ready).unwrap()),
    )
    .await
    .unwrap();
    let mut worker = fixture
        .connect(json!({"role":"worker","client":"one","token":config["client_token"]}))
        .await;
    assert_eq!(read(&mut worker).await["type"], "changed");
    assert_eq!(next_job(&mut worker).await["status"]["settling"], true);
    // This receive is driven only by the one quiet-period expiry, not requests.
    assert_eq!(read(&mut worker).await["type"], "changed");
    assert_eq!(next_job(&mut worker).await["command"]["id"], "first");
}

#[tokio::test]
async fn later_dispatch_does_not_wait_for_a_peers_upload_lock() {
    let fixture = Fixture::new(false).await;
    fixture.command("first").await;
    let mut worker = fixture.worker().await;
    assert_eq!(next_job(&mut worker).await["command"]["id"], "first");
    let _ = fixture.complete("first", json!({"ok":true})).await.unwrap();
    let peer = fixture.host.find_client("peer").await.unwrap();
    let _upload = peer.lock().await;
    fixture.command("second").await;
    let job = tokio::time::timeout(Duration::from_secs(1), next_job(&mut worker))
        .await
        .expect("A ready peer's upload blocked job dispatch");
    assert_eq!(job["command"]["id"], "second");
}

#[tokio::test]
async fn result_retry_recovers_publication_before_in_memory_completion() {
    let fixture = Fixture::new(false).await;
    fixture.command("first").await;
    let mut worker = fixture.worker().await;
    next_job(&mut worker).await;
    let result = json!({"ok":false,"error":{"code":"CAPTURE_FAILED","message":"test failure"}});
    // Simulate cancellation after the durable result was committed, including
    // the error state that precedes publication of a failed result.
    fixture
        .host
        .find_client("one")
        .await
        .unwrap()
        .lock()
        .await
        .error = result["error"].clone();
    write_json(
        &fixture.directory.join("responses/one/first.json"),
        &result,
        false,
    )
    .await
    .unwrap();
    let _ = fixture.complete("first", result.clone()).await.unwrap();
    assert_eq!(
        fixture.complete("first", result).await.unwrap().0["repeated"],
        true
    );
    assert!(fixture
        .host
        .find_client("one")
        .await
        .unwrap()
        .lock()
        .await
        .active_command
        .is_none());
}
