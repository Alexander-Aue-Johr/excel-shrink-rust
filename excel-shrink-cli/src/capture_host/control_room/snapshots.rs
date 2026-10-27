use super::platform;
use crate::capture_host::now_iso;
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

pub(super) const SNAPSHOT_LIMIT: u64 = 32 * 1024 * 1024;
const MONITOR_LIMIT: u64 = 2 * 1024 * 1024;

pub(super) async fn read_object(path: &Path, maximum: u64) -> Result<Value> {
    // Producers publish snapshots by atomic rename. Rust's Windows file reads
    // allow delete sharing so a producer can replace the snapshot during a read.
    let file = tokio::fs::File::open(path).await?;
    ensure!(
        file.metadata().await?.len() <= maximum,
        "Snapshot exceeds the {} KiB metadata limit.",
        maximum / 1024
    );
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes).await?;
    ensure!(
        bytes.len() as u64 <= maximum,
        "Snapshot exceeds the {} KiB metadata limit.",
        maximum / 1024
    );
    let value: Value = serde_json::from_slice(&bytes)?;
    ensure!(value.is_object(), "Expected an object snapshot.");
    Ok(value)
}

pub(super) async fn read_snapshot(
    path: &Path,
    diagnostics: &mut Vec<Value>,
    maximum: u64,
) -> Option<Value> {
    match read_object(path, maximum).await {
        Ok(value) => Some(value),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            None
        }
        Err(error) => {
            diagnostics.push(json!({"file": path.file_name().and_then(|name| name.to_str()), "error": error.to_string()}));
            None
        }
    }
}

pub(super) fn copy_fields(source: &Value, fields: &[&str]) -> Value {
    let mut result = json!({});
    for field in fields {
        if !source[field].is_null() {
            result[field] = source[field].clone();
        }
    }
    result
}

fn compact_monitor(value: &Value) -> Value {
    let mut result = copy_fields(
        value,
        &[
            "attempt_id",
            "pid",
            "process_started_unix_ms",
            "phase",
            "state",
            "state_since",
            "state_since_unix_ms",
            "last_progress_unix_ms",
            "role",
            "workbook",
            "wait_reason",
            "last_progress_at",
            "last_progress",
            "last_event",
            "last_success",
            "active_job",
            "uploaded_images",
            "completed_commands",
            "sheet_name",
            "sheet_index",
            "profile",
            "pane_resources",
        ],
    );
    let events = value["events"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    result["events"] = json!(&events[events.len().saturating_sub(24)..]);
    result
}

fn inspect_process(value: &Value, pid_field: &str, time_field: &str) -> Option<bool> {
    platform::process_alive(value[pid_field].as_i64()?, value[time_field].as_i64()?)
}

fn check_process(value: &mut Value) {
    value["process_alive"] = json!(inspect_process(value, "pid", "process_started_unix_ms"));
    value["process_checked_at"] = json!(now_iso());
}

pub(super) fn check_clients(launcher: &mut Value) {
    if let Some(clients) = launcher["clients"].as_object_mut() {
        for client in clients.values_mut().filter(|value| value.is_object()) {
            check_process(client);
        }
    }
}

pub(super) fn check_runner(queue: &mut Value, diagnostics: &mut Vec<Value>) {
    let alive = queue["runner_started_unix_ms"]
        .as_i64()
        .filter(|time| *time > 0)
        .and_then(|_| inspect_process(queue, "runner_pid", "runner_started_unix_ms"));
    queue["runner_alive"] = json!(alive);
    if queue["running"] != true || alive == Some(true) {
        return;
    }
    // Presentation only. Never overwrite the scheduler's durable queue based
    // on a process check, or infer a completed scientific result from an exit.
    queue["runner_reported_running"] = json!(true);
    queue["running"] = json!(false);
    queue["phase"] = json!(if alive == Some(false) {
        "interrupted"
    } else {
        "owner_unverified"
    });
    let reason = if alive == Some(false) {
        "Runner exited or its process id now belongs to a different process."
    } else {
        "Runner process identity could not be verified."
    };
    diagnostics.push(json!({"file": "queue.json", "error": reason}));
    if alive == Some(false) {
        if let Some(groups) = queue["groups"].as_array_mut() {
            for group in groups.iter_mut().filter(|group| group["active"] == true) {
                group["active"] = json!(false);
                group["status"] = json!("interrupted");
            }
        }
    }
}

pub(super) async fn worker_group(
    root: &Path,
    summary: &Value,
    diagnostics: &mut Vec<Value>,
) -> Result<Value> {
    let id = safe_id(required_string(summary, "id")?)?;
    let mut result = copy_fields(
        summary,
        &[
            "id",
            "workbook",
            "corpus",
            "status",
            "active",
            "attempt_id",
            "started_unix_ms",
            "cleanup_pending",
            "error",
            "capture_mode",
        ],
    );
    let attempt = summary["attempt_id"].as_str();
    let directory = confined_path(root, &format!("groups/{id}"))?;
    if let Some(coordinator) = read_snapshot(
        &directory.join("coordinator.json"),
        diagnostics,
        MONITOR_LIMIT,
    )
    .await
    {
        if attempt.is_none() || coordinator["attempt_id"].as_str() == attempt {
            let mut monitor = compact_monitor(&coordinator);
            check_process(&mut monitor);
            result["coordinator"] = monitor;
        }
    }
    let launcher_name = if summary["capture_mode"] == "classic" {
        "classic.json"
    } else {
        "launcher.json"
    };
    let launcher = read_snapshot(&directory.join(launcher_name), diagnostics, MONITOR_LIMIT).await;
    let mut clients = Vec::new();
    if let Some(mut launcher) =
        launcher.filter(|launcher| attempt.is_none() || launcher["attempt_id"].as_str() == attempt)
    {
        check_clients(&mut launcher);
        if let Some(processes) = launcher["clients"].as_object() {
            for (client_id, client) in processes.iter().filter(|(_, value)| value.is_object()) {
                let mut item = copy_fields(
                    client,
                    &[
                        "pid",
                        "process_started_unix_ms",
                        "process_alive",
                        "status",
                        "stage",
                        "stage_since",
                        "requested_at",
                        "opened_at",
                        "hide_excel",
                        "window_mode",
                        "heartbeat_count",
                        "last_heartbeat_at",
                        "active_command",
                        "error",
                        "last_launcher_event",
                        "last_launcher_success",
                        "runtime",
                    ],
                );
                item["client_id"] = json!(client_id);
                item["worker_id"] = json!(format!(
                    "{id}/{}/{client_id}/{}/{}",
                    attempt.unwrap_or("legacy"),
                    client["pid"],
                    client["process_started_unix_ms"]
                ));
                if client["monitor"].is_object() {
                    item["monitor"] = compact_monitor(&client["monitor"]);
                }
                clients.push(item);
            }
        }
        result["launcher_status"] = launcher["status"].clone();
        result["launcher_sampled_unix_ms"] = launcher["sampled_unix_ms"].clone();
    } else {
        result["launcher_pending"] = json!(true);
    }
    result["clients"] = json!(clients);
    Ok(result)
}

pub(super) async fn reconcile_groups(
    root: &Path,
    queue: &mut Value,
    diagnostics: &mut Vec<Value>,
) -> Result<()> {
    let Some(groups) = queue["groups"].as_array_mut() else {
        return Ok(());
    };
    let mut reconciled = false;
    for group in groups.iter_mut().filter(|group| {
        group["active"] == true
            && group["status"] == "waiting_cleanup"
            && group["cleanup_pending"]["state"] != "waiting"
            && group["attempt_id"]
                .as_str()
                .is_some_and(|attempt| !attempt.is_empty())
    }) {
        let worker = worker_group(root, group, diagnostics).await?;
        let coordinator = &worker["coordinator"];
        let status = &coordinator["last_event"]["details"]["status"];
        if coordinator["phase"] != "finished"
            || coordinator["last_event"]["kind"] != "record_saved"
            || coordinator["process_alive"] != false
            || !matches!(
                status.as_str(),
                Some("unchanged" | "differences" | "skipped" | "failed")
            )
            || worker["clients"].as_array().is_some_and(|clients| {
                clients
                    .iter()
                    .any(|client| client["process_alive"] != false)
            })
        {
            continue;
        }
        group["scheduler_status"] = group["status"].clone();
        group["status"] = status.clone();
        group["active"] = json!(false);
        group["completion_reconciled"] = json!(true);
        reconciled = true;
    }
    if reconciled {
        let open = groups
            .iter()
            .filter(|group| group["active"] == true)
            .count();
        let cleanup = groups
            .iter()
            .filter(|group| group["active"] == true && group["status"] == "waiting_cleanup")
            .count();
        queue["scheduler_open_groups"] = queue["open_groups"].clone();
        queue["open_groups"] = json!(open);
        queue["cleanup_groups"] = json!(cleanup);
        diagnostics.push(json!("Completed workers were still marked active by the scheduler; display reconciled from saved-result and process evidence. Scheduling has not been repaired."));
    }
    Ok(())
}

pub(super) async fn recent_commands(
    directory: &Path,
    group_id: &str,
    diagnostics: &mut Vec<Value>,
) -> Result<Vec<Value>> {
    let mut commands = Vec::new();
    for path in command_files(directory).await? {
        let Some(command) = read_snapshot(&path, diagnostics, SNAPSHOT_LIMIT).await else {
            continue;
        };
        if command["group_id"] == group_id {
            commands.push(command);
            if commands.len() == 10 {
                break;
            }
        }
    }
    commands.sort_by_key(|command| {
        std::cmp::Reverse(command["requested_unix_ms"].as_i64().unwrap_or(0))
    });
    Ok(commands)
}

pub(super) async fn command_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = match tokio::fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        let metadata = tokio::fs::symlink_metadata(entry.path()).await?;
        if metadata.is_file() && !platform::is_link(&metadata) {
            files.push((metadata.modified()?, entry.path()));
        }
    }
    files.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    Ok(files.into_iter().map(|(_, path)| path).collect())
}

pub(super) fn safe_id(value: &str) -> Result<&str> {
    let bytes = value.as_bytes();
    ensure!(
        !bytes.is_empty()
            && bytes.len() <= 128
            && bytes[0].is_ascii_alphanumeric()
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-'),
        "Invalid comparison group id."
    );
    Ok(value)
}

pub(super) fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .with_context(|| format!("Missing {field}."))
}

pub(super) fn confined_path(root: &Path, relative: &str) -> Result<PathBuf> {
    ensure!(
        !relative.trim().is_empty()
            && !relative.contains(['\\', ':'])
            && !Path::new(relative).is_absolute(),
        "Invalid relative asset path."
    );
    let mut path = root.to_path_buf();
    for segment in relative.split('/') {
        ensure!(
            !matches!(segment, "" | "." | "..")
                && !segment.ends_with(['.', ' '])
                && !segment
                    .chars()
                    .any(|character| character < ' ' || "<>\"|?*".contains(character)),
            "Invalid asset path segment."
        );
        path.push(segment);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if platform::is_link(&metadata) => {
                bail!("Linked assets are not served by the control room.")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}
