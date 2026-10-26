//! Bounded wake-ups for resident jobs and their durable results. Reconnection
//! reconciles files; notifications deliberately do not form an in-memory log.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{json, Value};
use tokio::sync::watch;

use super::{dispatch_job, safe_id, Host};

#[derive(Clone, Default)]
struct Change {
    revision: u64,
    error: Option<String>,
}

#[derive(Clone)]
pub(super) struct Changes(watch::Sender<Change>);

impl Changes {
    pub fn new() -> Self {
        Self(watch::channel(Change::default()).0)
    }

    pub fn changed(&self) {
        self.0
            .send_modify(|change| change.revision = change.revision.wrapping_add(1));
    }

    fn failed(&self, message: String) {
        self.0.send_modify(|change| {
            change.revision = change.revision.wrapping_add(1);
            change.error = Some(message);
        });
    }

    pub fn error(&self) -> Option<String> {
        self.0.borrow().error.clone()
    }
}

pub(super) fn watch_files(host: &Host) -> anyhow::Result<RecommendedWatcher> {
    let commands = host.session.join("commands");
    std::fs::create_dir_all(&commands)?;
    let control = host
        .control_root
        .as_ref()
        .map(|root| root.join("control.json"));
    let changes = host.events.clone();
    let observed_commands = commands.clone();
    let observed_control = control.clone();
    let mut watcher =
        notify::recommended_watcher(move |event: notify::Result<Event>| match event {
            Ok(event) if event.need_rescan() => changes.changed(),
            Ok(event)
                if !event.kind.is_access()
                    && event.paths.iter().any(|path| {
                        (path.starts_with(&observed_commands)
                            && path
                                .extension()
                                .is_some_and(|extension| extension == "json"))
                            || observed_control.as_ref() == Some(path)
                    }) =>
            {
                changes.changed()
            }
            Ok(_) => {}
            Err(error) => changes.failed(format!("Resident event watcher failed: {error}")),
        })?;
    watcher.watch(&commands, RecursiveMode::Recursive)?;
    if let Some(control) = control {
        watcher.watch(
            control.parent().context("Control path has no parent")?,
            RecursiveMode::NonRecursive,
        )?;
    }
    Ok(watcher)
}

pub(super) async fn upgrade(State(host): State<Arc<Host>>, socket: WebSocketUpgrade) -> Response {
    socket
        .max_message_size(16 * 1024)
        .max_frame_size(16 * 1024)
        .on_upgrade(move |mut socket| async move {
            if let Err(error) = connected(&host, &mut socket).await {
                let _ = send(
                    &mut socket,
                    json!({
                    "type": "error", "code": "EVENT_PROTOCOL_FAILED", "message": format!("{error:#}")
                    }),
                )
                .await;
                let _ = socket
                    .send(Message::Close(Some(CloseFrame {
                        code: 1008,
                        reason: "Resident event session ended".into(),
                    })))
                    .await;
            }
        })
}

enum Participant {
    Coordinator,
    Worker { id: String, headers: HeaderMap },
}

async fn authenticate(host: &Host, value: &Value) -> anyhow::Result<Participant> {
    let token = value["token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .context("Missing event session token")?;
    match value["role"].as_str() {
        Some("worker") => {
            let id = value["client"]
                .as_str()
                .context("Missing worker identity")?;
            safe_id(id).map_err(|error| anyhow::anyhow!(error.message))?;
            let mut headers = HeaderMap::new();
            headers.insert("X-Client-Token", token.parse()?);
            drop(
                host.authenticated(id, &headers)
                    .await
                    .map_err(|error| anyhow::anyhow!(error.message))?,
            );
            Ok(Participant::Worker {
                id: id.to_owned(),
                headers,
            })
        }
        Some("coordinator") if host.coordinator_token.as_deref() == Some(token) => {
            Ok(Participant::Coordinator)
        }
        _ => bail!("Unknown or retired event session"),
    }
}

async fn connected(host: &Host, socket: &mut WebSocket) -> anyhow::Result<()> {
    // Subscribe first: a publication during authentication must remain visible.
    let mut changes = host.events.0.subscribe();
    let first = tokio::time::timeout(Duration::from_secs(10), socket.recv())
        .await?
        .context("Event connection closed before authentication")??;
    let Message::Text(first) = first else {
        bail!("Expected event authentication")
    };
    let participant = authenticate(host, &serde_json::from_str::<Value>(&first)?).await?;
    send_change(socket, &mut changes).await?;
    loop {
        tokio::select! {
            changed = changes.changed() => {
                changed.context("Resident event source closed")?;
                if let Participant::Worker { id, headers } = &participant {
                    drop(host.authenticated(id, headers).await.map_err(|error| anyhow::anyhow!(error.message))?);
                }
                send_change(socket, &mut changes).await?;
            }
            message = socket.recv() => {
                let Some(message) = message else { return Ok(()) };
                match message? {
                    Message::Text(text) => {
                        let request: Value = serde_json::from_str(&text)?;
                        match (&participant, request["type"].as_str()) {
                            (Participant::Coordinator, Some("wake")) => host.events.changed(),
                            (Participant::Worker { id, headers }, Some("next")) => {
                                // Do not acknowledge a revision that arrived during dispatch;
                                // it may be the publication that makes this idle worker runnable.
                                let revision = changes.borrow().revision;
                                let command = dispatch_job(host, id, headers, true).await
                                    .map_err(|error| anyhow::anyhow!(error.message))?;
                                if command["idle"] == true {
                                    send(socket, json!({"type":"idle", "revision":revision, "status":command})).await?;
                                } else {
                                    send(socket, json!({"type":"job", "revision":revision, "command":command})).await?;
                                }
                            }
                            _ => bail!("Unexpected resident event message"),
                        }
                    }
                    Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await?,
                    Message::Pong(_) => {},
                    Message::Close(_) => return Ok(()),
                    Message::Binary(_) => bail!("Images must use the bounded HTTP upload"),
                }
            }
        }
    }
}

async fn send_change(
    socket: &mut WebSocket,
    changes: &mut watch::Receiver<Change>,
) -> anyhow::Result<()> {
    let change = changes.borrow_and_update().clone();
    if let Some(error) = change.error {
        bail!("{error}")
    }
    send(
        socket,
        json!({"type":"changed", "revision":change.revision}),
    )
    .await
}

async fn send(socket: &mut WebSocket, message: Value) -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(10),
        socket.send(Message::Text(message.to_string().into())),
    )
    .await
    .context("Event receiver stopped reading")??;
    Ok(())
}

#[cfg(test)]
#[path = "resident_events_tests.rs"]
mod tests;
