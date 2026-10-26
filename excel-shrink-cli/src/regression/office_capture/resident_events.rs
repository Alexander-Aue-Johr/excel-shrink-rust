//! Wake the synchronous comparison loop when its resident host publishes evidence.
//! Notices are coalesced hints: durable receipts/results remain authoritative, so
//! reconnecting never requires replaying an unbounded history of messages.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message;

const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
struct SessionRejected(String);

impl std::fmt::Display for SessionRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SessionRejected {}

pub(super) struct ResidentEvents {
    notices: mpsc::Receiver<()>,
    notifier: mpsc::SyncSender<()>,
    commands: watch::Sender<u64>,
    shutdown: watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
    status: Arc<Mutex<Value>>,
}

impl ResidentEvents {
    pub fn start(directory: &Path, token: String) -> Result<Self> {
        let (notice_tx, notices) = mpsc::sync_channel(1);
        let notifier = notice_tx.clone();
        let (commands, command_rx) = watch::channel(0);
        let (shutdown, mut shutdown_rx) = watch::channel(false);
        let status = Arc::new(Mutex::new(json!({
            "method": "websocket", "connected": false,
            "connections": 0, "notices_received": 0, "last_error": null
        })));
        let state = Arc::clone(&status);
        let directory = directory.to_owned();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let thread = std::thread::Builder::new()
            .name("resident-events".into())
            .spawn(move || {
                runtime.block_on(async {
                    tokio::select! {
                        result = connect_loop(directory, token, command_rx, notice_tx.clone(), Arc::clone(&state)) => {
                            if let Err(error) = result {
                                let mut state = state.lock().unwrap();
                                state["fatal_error"] = json!(format!("{error:#}"));
                                state["connected"] = json!(false);
                                let _ = notice_tx.try_send(());
                            }
                        }
                        _ = shutdown_rx.changed() => {}
                    }
                });
            })?;
        Ok(Self {
            notices,
            notifier,
            commands,
            shutdown,
            thread: Some(thread),
            status,
        })
    }

    pub fn command_published(&self) {
        self.commands
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    /// Local image processing uses the same coalesced wakeup as host evidence.
    pub fn wake_sender(&self) -> mpsc::SyncSender<()> {
        self.notifier.clone()
    }

    pub fn snapshot(&self) -> Value {
        self.status.lock().unwrap().clone()
    }

    /// The one-second timeout checks process exit, user restart and deadlines.
    /// Normal job/result delivery wakes immediately, including notices received
    /// between scanning the durable files and entering this wait.
    pub fn wait(&self) -> Result<()> {
        if let Some(error) = self.snapshot()["fatal_error"].as_str() {
            bail!("resident event connection: {error}");
        }
        match self.notices.recv_timeout(HEALTH_CHECK_INTERVAL) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("resident event listener stopped unexpectedly")
            }
        }
    }
}

impl Drop for ResidentEvents {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn connect_loop(
    directory: PathBuf,
    token: String,
    mut commands: watch::Receiver<u64>,
    notices: mpsc::SyncSender<()>,
    status: Arc<Mutex<Value>>,
) -> Result<()> {
    // Subscribe before checking the endpoint file. The launcher can publish it
    // at any time; a file event arriving during the read stays pending.
    let (files, mut changed) = watch::channel(None::<String>);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if match event {
            Ok(event) => {
                event.need_rescan()
                    || event
                        .paths
                        .iter()
                        .any(|path| path.file_name().is_some_and(|name| name == "events.json"))
            }
            Err(error) => {
                files.send_modify(|failure| *failure = Some(error.to_string()));
                false
            }
        } {
            files.send_modify(|_| {});
        }
    })?;
    watcher.watch(&directory, RecursiveMode::NonRecursive)?;
    let endpoint = directory.join("events.json");
    let url = loop {
        if let Some(error) = changed.borrow_and_update().as_ref() {
            bail!("Resident endpoint watcher failed: {error}");
        }
        match tokio::fs::read(&endpoint).await {
            Ok(bytes) => {
                let settings: Value = serde_json::from_slice(&bytes)?;
                let url = settings["url"]
                    .as_str()
                    .context("Missing resident event URL")?;
                anyhow::ensure!(
                    url.starts_with("ws://127.0.0.1:"),
                    "Resident events must use loopback"
                );
                break url.to_owned();
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                changed
                    .changed()
                    .await
                    .context("Resident endpoint watcher stopped")?;
            }
            Err(error) => return Err(error.into()),
        }
    };
    drop(watcher);
    let mut backoff = Duration::from_millis(250);
    let mut outage_started = None;
    loop {
        let outcome = receive(&url, &token, &mut commands, &notices, &status).await;
        if outcome
            .as_ref()
            .is_err_and(|error| error.is::<SessionRejected>())
        {
            return outcome;
        }
        {
            let mut state = status.lock().unwrap();
            if state["connected"] == true {
                outage_started = None;
                backoff = Duration::from_millis(250);
            }
            state["connected"] = json!(false);
            state["last_error"] = json!(outcome.err().map(|error| format!("{error:#}")));
        }
        let since = outage_started.get_or_insert_with(std::time::Instant::now);
        anyhow::ensure!(
            since.elapsed() < Duration::from_secs(60),
            "Resident event connection could not recover within 60 seconds: {}",
            status.lock().unwrap()["last_error"]
        );
        let _ = notices.try_send(());
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

async fn receive(
    url: &str,
    token: &str,
    commands: &mut watch::Receiver<u64>,
    notices: &mpsc::SyncSender<()>,
    status: &Arc<Mutex<Value>>,
) -> Result<()> {
    let (mut socket, _) =
        tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(url))
            .await
            .context("Resident event connection timed out")??;
    socket
        .send(Message::Text(
            json!({"role":"coordinator", "token":token})
                .to_string()
                .into(),
        ))
        .await?;
    // Sending a wake on every connection reconciles commands written during an
    // outage. The native watcher covers callers which only publish command files.
    socket
        .send(Message::Text(json!({"type":"wake"}).to_string().into()))
        .await?;
    let authentication_deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
    let mut authenticated = false;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(authentication_deadline), if !authenticated => {
                bail!("Resident event authentication timed out");
            }
            update = commands.changed() => {
                update.context("Resident command notifier stopped")?;
                socket.send(Message::Text(json!({"type":"wake"}).to_string().into())).await?;
            }
            message = socket.next() => {
                match message.context("Resident event socket closed")?? {
                    Message::Text(text) => {
                        let event: Value = serde_json::from_str(&text)?;
                        if event["type"] == "error" {
                            return Err(SessionRejected(format!("Resident event host rejected connection: {}", event["message"])).into());
                        }
                        if event["type"] == "changed" {
                            authenticated = true;
                            let mut state = status.lock().unwrap();
                            if state["connected"] != true {
                                state["connections"] = json!(state["connections"].as_u64().unwrap_or(0) + 1);
                            }
                            state["connected"] = json!(true);
                            state["last_error"] = Value::Null;
                            state["notices_received"] = json!(state["notices_received"].as_u64().unwrap_or(0) + 1);
                            let _ = notices.try_send(());
                        }
                    }
                    Message::Ping(_) => { socket.flush().await?; }
                    Message::Close(Some(frame)) if frame.code == CloseCode::Policy => {
                        return Err(SessionRejected("Resident event session was retired".into()).into());
                    }
                    Message::Close(_) => bail!("Resident event socket closed"),
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_coalesce_without_losing_a_wake_before_wait() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.try_send(()).unwrap();
        assert!(matches!(
            sender.try_send(()),
            Err(mpsc::TrySendError::Full(()))
        ));
        receiver.recv_timeout(Duration::from_millis(10)).unwrap();
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn dropping_listener_cancels_endpoint_wait() -> Result<()> {
        let root = crate::regression::artifact_cache::unique_path(
            &std::env::temp_dir(),
            "resident-event-drop",
        )?;
        std::fs::create_dir_all(&root)?;
        let events = ResidentEvents::start(&root, "unused".into())?;
        drop(events);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn rejected_sessions_stop_without_reconnecting() -> Result<()> {
        use tokio_tungstenite::tungstenite::protocol::CloseFrame;

        let rejections = [
            Message::Text(
                json!({"type":"error", "code":"SESSION_RETIRED", "message":"Token retired"})
                    .to_string()
                    .into(),
            ),
            Message::Close(Some(CloseFrame {
                code: CloseCode::Policy,
                reason: "Session retired".into(),
            })),
        ];
        for rejection in rejections {
            let directory = crate::regression::artifact_cache::unique_path(
                &std::env::temp_dir(),
                "resident-event-rejected",
            )?;
            std::fs::create_dir_all(&directory)?;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let url = format!("ws://{}/events", listener.local_addr()?);
            std::fs::write(
                directory.join("events.json"),
                serde_json::to_vec(&json!({"url":url}))?,
            )?;
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await?;
                let mut socket = tokio_tungstenite::accept_async(stream).await?;
                let authentication = socket.next().await.context("Missing authentication")??;
                let authentication: Value = serde_json::from_str(authentication.to_text()?)?;
                assert_eq!(
                    authentication,
                    json!({"role":"coordinator", "token":"retired"})
                );
                let wake = socket
                    .next()
                    .await
                    .context("Missing initial reconciliation")??;
                assert_eq!(
                    serde_json::from_str::<Value>(wake.to_text()?)?,
                    json!({"type":"wake"})
                );
                socket.send(rejection).await?;
                Ok::<_, anyhow::Error>(())
            });
            let (_commands, command_rx) = watch::channel(0);
            let (notices, _received) = mpsc::sync_channel(1);
            let status = Arc::new(Mutex::new(json!({"connected":false})));
            let error = tokio::time::timeout(
                Duration::from_secs(3),
                connect_loop(
                    directory.clone(),
                    "retired".into(),
                    command_rx,
                    notices,
                    status,
                ),
            )
            .await
            .context("A rejected session entered the reconnect loop")?
            .expect_err("Retired session unexpectedly succeeded");
            assert!(error.is::<SessionRejected>(), "{error:#}");
            server.await??;
            std::fs::remove_dir_all(directory)?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn server_that_never_confirms_authentication_times_out() -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("ws://{}/events", listener.local_addr()?);
        let (stop_server, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            let mut socket = tokio_tungstenite::accept_async(stream).await?;
            socket.next().await.context("Missing authentication")??;
            socket
                .next()
                .await
                .context("Missing initial reconciliation")??;
            // Keep the transport open but send no authentication response.
            let _ = stopped.await;
            Ok::<_, anyhow::Error>(())
        });
        let (_commands, mut command_rx) = watch::channel(0);
        let (notices, _received) = mpsc::sync_channel(1);
        let status = Arc::new(Mutex::new(json!({"connected":false})));
        let error = tokio::time::timeout(
            CONNECT_TIMEOUT + Duration::from_secs(3),
            receive(&url, "valid", &mut command_rx, &notices, &status),
        )
        .await
        .context("An unresponsive server bypassed the authentication deadline")?
        .expect_err("Silent server unexpectedly authenticated");
        assert!(format!("{error:#}").contains("authentication timed out"));
        assert!(!error.is::<SessionRejected>());
        assert_eq!(status.lock().unwrap()["connected"], false);
        let _ = stop_server.send(());
        server.await??;
        Ok(())
    }
}
