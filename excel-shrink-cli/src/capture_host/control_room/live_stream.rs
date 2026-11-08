//! Bounded live snapshots followed by keyed RFC 6902 updates per browser connection.
use super::*;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};

static CONNECTIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

pub(super) async fn connect(
    State(room): State<Arc<Room>>,
    upgrade: WebSocketUpgrade,
) -> HttpResult<Response> {
    let permit = CONNECTIONS
        .try_acquire()
        .map_err(|_| ApiError::conflict("Live status connection limit reached"))?;
    Ok(upgrade.on_upgrade(move |socket| async move {
        let _permit = permit;
        if let Err(error) = serve_connection(socket, room).await {
            tracing::warn!(%error, "Live status connection ended");
        }
    }))
}

async fn snapshot(room: Arc<Room>) -> Result<Value> {
    let Json(queue) = state(
        State(room.clone()),
        Query(StateOptions {
            group_scope: Some("live".into()),
            selected: None,
        }),
    )
    .await
    .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let Json(targeted) = targeted::comparisons(State(room))
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let mut comparisons = serde_json::Map::new();
    for item in targeted["comparisons"].as_array().into_iter().flatten() {
        if let Some(id) = item["run_id"].as_str() {
            validate_comparison_id(id)?;
            comparisons.insert(id.into(), item.clone());
        }
    }
    Ok(json!({"queue":queue,"comparisons":comparisons,
        "retired_run_ids":targeted["retired_run_ids"],"diagnostics":targeted["diagnostics"]}))
}

fn validate_comparison_id(id: &str) -> Result<()> {
    if uuid::Uuid::parse_str(id).is_ok() {
        return Ok(());
    }
    let (timestamp, uuid) = id.split_once('-').context("Invalid comparison identity")?;
    ensure!(!timestamp.is_empty() && timestamp.len() <= 20
        && timestamp.bytes().all(|byte| byte.is_ascii_digit()), "Invalid comparison timestamp");
    uuid::Uuid::parse_str(uuid)?;
    Ok(())
}

/// Keyed comparison objects prevent one changing lane from replacing all history.
fn patch(previous: &Value, next: &Value) -> Vec<Value> {
    let mut operations = Vec::new();
    for key in ["queue", "retired_run_ids", "diagnostics"] {
        if previous[key] != next[key] {
            operations.push(json!({"op":"replace","path":format!("/{key}"),"value":next[key]}));
        }
    }
    let old = previous["comparisons"]
        .as_object()
        .expect("normalized snapshot");
    let new = next["comparisons"]
        .as_object()
        .expect("normalized snapshot");
    for id in old.keys().filter(|id| !new.contains_key(*id)) {
        operations.push(json!({"op":"remove","path":format!("/comparisons/{id}")}));
    }
    for (id, item) in new {
        if old.get(id) != Some(item) {
            operations.push(
                json!({"op":if old.contains_key(id) {"replace"} else {"add"},
                "path":format!("/comparisons/{id}"),"value":item}),
            );
        }
    }
    operations
}

async fn serve_connection(mut socket: WebSocket, room: Arc<Room>) -> Result<()> {
    let mut previous = None;
    let mut sequence = 0u64;
    let mut interval = tokio::time::interval(Duration::from_secs(3));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    _ => return Ok(()),
                }
            }
            _ = interval.tick() => {
                let next = snapshot(room.clone()).await?;
                sequence += 1;
                let message = match &previous {
                    None => json!({"sequence":sequence,"snapshot":next}),
                    Some(old) => json!({"sequence":sequence,"patch":patch(old, &next)}),
                };
                tokio::time::timeout(Duration::from_secs(10), socket.send(Message::Text(message.to_string().into())))
                    .await.context("Live status browser stopped reading")??;
                previous = Some(next);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_existing_timestamped_comparison_identities() {
        assert!(validate_comparison_id("1794182130052-0873afcd-70cd-4fdc-8917-9628978fd1ee").is_ok());
        assert!(validate_comparison_id("0873afcd-70cd-4fdc-8917-9628978fd1ee").is_ok());
        assert!(validate_comparison_id("__proto__").is_err());
    }

    #[test]
    fn updates_only_changed_comparisons_and_removes_retired_history() {
        let old = json!({"queue":{},"comparisons":{"a":{"value":1},"b":{"value":2}},"retired_run_ids":[],"diagnostics":[]});
        let new = json!({"queue":{},"comparisons":{"a":{"value":1},"c":{"value":3}},"retired_run_ids":[],"diagnostics":[]});
        assert_eq!(
            patch(&old, &new),
            vec![
                json!({"op":"remove","path":"/comparisons/b"}),
                json!({"op":"add","path":"/comparisons/c","value":{"value":3}})
            ]
        );
        assert!(patch(&new, &new).is_empty());
    }
}
