//! Read the dedicated comparison registry without scanning every corpus group.
use super::*;

/// Start the same CLI workflow on fresh copies, without changing the corpus queue.
pub(super) async fn start(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    let group = confined_path(&room.root, &format!("groups/{id}"))?;
    if !group.is_dir() {
        return Err(ApiError::not_found("Unknown comparison group"));
    }
    let mut diagnostics = Vec::new();
    let detail = read_snapshot(&group.join("state.json"), &mut diagnostics, SNAPSHOT_LIMIT)
        .await
        .or(read_snapshot(
            &group.join("session.json"),
            &mut diagnostics,
            SNAPSHOT_LIMIT,
        )
        .await);
    let workbook = detail
        .as_ref()
        .and_then(|detail| detail["workbook"].as_str())
        .unwrap_or(&id)
        .to_owned();
    let script = room
        .web
        .parent()
        .context("Capture script directory missing")?
        .join("run-targeted-comparison.ps1");
    if !script.is_file() {
        return Err(ApiError::bad_request(
            "Private targeted comparison launcher is unavailable",
        ));
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    let directory = confined_path(&room.root, &format!("targeted-runs/{request_id}"))?;
    let input_directory = directory.join("inputs");
    let copy_root = room.root.clone();
    let (original, default) = tokio::task::spawn_blocking(move || {
        inspection::copy_comparison_pair(&copy_root, &id, &input_directory)
    })
    .await
    .context("Copy comparison inputs")??;
    let stdout = std::fs::File::create(directory.join("launcher.stdout.log"))?;
    let stderr = std::fs::File::create(directory.join("launcher.stderr.log"))?;
    let mut command = std::process::Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .env("EXCEL_SHRINK_TARGETED_WORKBOOK_LABEL", workbook)
        .arg("-Binary")
        .arg(std::env::current_exe()?)
        .arg("-Original")
        .arg(original)
        .arg("-Default")
        .arg(default)
        .arg("-Output")
        .arg(directory.join("result"))
        .arg("-ObservatoryGroup")
        .arg(group)
        .stdin(std::process::Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW; Excel uses the private desktop.
    }
    let mut child = command
        .spawn()
        .context("Start private targeted comparison")?;
    let pid = child.id();
    std::thread::spawn(move || {
        let outcome = child
            .wait()
            .map(|status| json!({"exit_code":status.code(),"success":status.success()}))
            .unwrap_or_else(|error| json!({"success":false,"error":error.to_string()}));
        if let Err(error) =
            std::fs::write(directory.join("launcher-result.json"), outcome.to_string())
        {
            tracing::error!(%error, "Cannot record targeted launcher exit");
        }
    });
    Ok(Json(
        json!({"ok":true,"request_id":request_id,"launcher_pid":pid}),
    ))
}

/// Return the latest bounded set, including completed work across page reloads.
pub(super) async fn comparisons(State(room): State<Arc<Room>>) -> HttpResult<Json<Value>> {
    let root = room.root.join("targeted-comparisons");
    let mut entries = match tokio::fs::read_dir(&root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Json(json!({"comparisons":[]})))
        }
        Err(error) => return Err(ApiError::from(error)),
    };
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json") && entry.file_type().await?.is_file() {
            names.push(name);
        }
    }
    names.sort_unstable_by(|left, right| right.cmp(left));
    let mut diagnostics = Vec::new();
    let mut comparisons = Vec::new();
    let mut resolved_cleanup = Vec::new();
    for name in &names {
        if let Some(snapshot) = room
            .snapshots
            .snapshot(&root.join(name), &mut diagnostics, SNAPSHOT_LIMIT)
            .await
        {
            let mut snapshot = snapshot.as_ref().clone();
            let ended_without_telemetry = reconcile_saved_attempt(&room, &mut snapshot, &mut diagnostics).await;
            if ended_without_telemetry
                || reconcile_cleanup(&mut snapshot, crate::native_process::process_alive)
            {
                resolved_cleanup.push(snapshot["run_id"].clone());
            }
            comparisons.push(snapshot);
        }
    }
    let mut comparisons = retain_recent(comparisons);
    // A browser may still retain an old snapshot as active. Explicit retirement
    // removes it even when its corrected terminal state falls outside history.
    resolved_cleanup.retain(|id| !comparisons.iter().any(|item| item["run_id"] == *id));
    enrich_addin_heartbeats(&room, &mut comparisons, &mut diagnostics).await;
    Ok(Json(
        json!({"comparisons":comparisons,"retired_run_ids":resolved_cleanup,"retained_limit":100,"diagnostics":diagnostics}),
    ))
}

/// A durable final workbook record can outlive a killed comparison's last snapshot.
/// Reconcile only this study's matching workbook and positively departed processes.
async fn reconcile_saved_attempt(room: &Room, snapshot: &mut Value, diagnostics: &mut Vec<Value>) -> bool {
    if snapshot["status"] != "running" {
        return false;
    }
    let Some(study) = room.root.parent() else { return false };
    let Some(directory) = snapshot["report_directory"].as_str().map(PathBuf::from) else { return false };
    if !directory.starts_with(study.join("evidence")) {
        return false;
    }
    let Some(id) = directory.parent().and_then(Path::file_name).and_then(|name| name.to_str()) else {
        return false;
    };
    if safe_id(id).is_err() {
        return false;
    }
    let Some(record) = read_snapshot(&study.join(format!("records/{id}.json")), diagnostics, SNAPSHOT_LIMIT).await else {
        return false;
    };
    let Some(identity) = record["identity"].as_str() else { return false };
    if &crate::regression::artifact_cache::sha256(identity.as_bytes())[..20] != id {
        return false;
    }
    reconcile_ended_snapshot(snapshot, &record, crate::native_process::process_alive)
}

/// Preserve unknown/live attempts and leave the durable scientific verdict untouched.
fn reconcile_ended_snapshot(
    snapshot: &mut Value,
    record: &Value,
    mut observe: impl FnMut(i64, i64) -> Option<bool>,
) -> bool {
    if snapshot["status"] != "running" || record["complete"] != true
        || record["name"] != snapshot["workbook"] || snapshot["updated_at"].as_i64().is_none()
    {
        return false;
    }
    let Some(lanes) = snapshot["lanes"].as_array() else { return false };
    let mut observed_process = false;
    for lane in lanes {
        match (lane["pid"].as_i64(), lane["process_started_at"].as_i64()) {
            (Some(pid), Some(started)) => {
                observed_process = true;
                if observe(pid, started) != Some(false) {
                    return false;
                }
            }
            (None, _) if matches!(lane["lifecycle"].as_str(),
                Some("not_started" | "unavailable" | "skipped" | "closed")) => {}
            _ => return false,
        }
    }
    if !observed_process {
        return false;
    }
    for lane in snapshot["lanes"].as_array_mut().expect("validated lanes") {
        if lane["pid"].is_number() {
            lane["process_alive"] = json!(false);
            lane["lifecycle"] = json!("closed");
        }
    }
    snapshot["status"] = json!("attempt_ended");
    // The last observation orders history; it does not invent a process-exit time.
    snapshot["finished_at"] = snapshot["updated_at"].clone();
    snapshot["completion_observation"] = json!({
        "source":"final-workbook-record-and-pid-creation-time",
        "all_processes_exited":true, "exact_end_time_known":false,
        "recorded_status":"running", "workbook_status":record["status"]});
    if snapshot["error"].is_null() {
        snapshot["error"] = json!("This attempt ended without final telemetry. Its Excel processes have exited; the saved workbook result remains authoritative.");
    }
    true
}

/// Refresh a failed attempt's frozen cleanup observations without changing its
/// durable report. Only exact PID-and-creation-time evidence can confirm exit.
fn reconcile_cleanup(
    snapshot: &mut Value,
    mut observe: impl FnMut(i64, i64) -> Option<bool>,
) -> bool {
    if !matches!(snapshot["status"].as_str(), Some("cleanup_unconfirmed" | "failed"))
        || snapshot["updated_at"].as_i64().is_none()
    {
        return false;
    }
    let Some(lanes) = snapshot["lanes"].as_array_mut() else {
        return false;
    };
    let mut all_closed = true;
    for lane in lanes {
        match (lane["pid"].as_i64(), lane["process_started_at"].as_i64()) {
            (Some(pid), Some(started)) => {
                let alive = observe(pid, started);
                lane["process_alive"] = json!(alive);
                if alive == Some(false) {
                    lane["lifecycle"] = json!("closed");
                } else {
                    all_closed = false;
                }
            }
            (None, _) if matches!(lane["lifecycle"].as_str(),
                Some("not_started" | "unavailable" | "skipped" | "closed")) => {}
            _ => all_closed = false,
        }
    }
    if !all_closed {
        return false;
    }
    snapshot["status"] = json!("failed");
    // This is the original failure time, not a fabricated process-exit time.
    if snapshot["finished_at"].is_null() {
        snapshot["finished_at"] = snapshot["updated_at"].clone();
    }
    snapshot["cleanup_observation"] = json!({"all_processes_exited":true,"source":"pid-and-creation-time"});
    true
}

/// Load the durable portable report for this workbook, never a different live attempt.
pub(super) async fn report(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    let study = room.root.parent().context("Missing study directory.")?;
    let mut diagnostics = Vec::new();
    let record_path = confined_path(study, &format!("records/{id}.json"))?;
    let record = read_snapshot(&record_path, &mut diagnostics, SNAPSHOT_LIMIT).await
        .ok_or_else(|| ApiError::not_found("No durable result for this workbook."))?;
    let relative = record["targeted_comparison"]["targeted_report_file"].as_str()
        .filter(|path| path.starts_with("evidence/") && path.ends_with("/portable-report.json"))
        .ok_or_else(|| ApiError::not_found("No completed targeted report is available yet."))?;
    let path = confined_path(study, relative)?;
    let mut report = read_snapshot(&path, &mut diagnostics, SNAPSHOT_LIMIT).await
        .ok_or_else(|| ApiError::not_found("The retained targeted report could not be read."))?;
    report["_evidence_id"] = json!(relative);
    Ok(Json(report))
}

/// Read existing launcher observations, never manufacture beats from a live PID.
async fn enrich_addin_heartbeats(
    room: &Room,
    comparisons: &mut [Value],
    diagnostics: &mut Vec<Value>,
) {
    let Some(study) = room.root.parent() else {
        return;
    };
    let Some(queue) = room
        .snapshots
        .snapshot(&room.root.join("queue.json"), diagnostics, SNAPSHOT_LIMIT)
        .await
    else {
        return;
    };
    for comparison in comparisons
        .iter_mut()
        .filter(|item| item["finished_at"].is_null())
    {
        let workbook = comparison["workbook"].clone();
        let Some(groups) = queue["groups"].as_array() else {
            continue;
        };
        for group in groups
            .iter()
            .filter(|group| group["active"] == true && group["workbook"] == workbook)
        {
            let Some(id) = group["id"].as_str().filter(|id| safe_id(id).is_ok()) else {
                continue;
            };
            let Some(record) = read_snapshot(
                &study.join(format!("records/{id}.json")),
                diagnostics,
                SNAPSHOT_LIMIT,
            )
            .await
            else {
                continue;
            };
            let Some(directory) = record["targeted_comparison"]["report_directory"].as_str() else {
                continue;
            };
            let directory = PathBuf::from(directory);
            // Working evidence is owned by this study; a stale record cannot bind
            // another attempt's session to this card merely by matching its name.
            if !directory.starts_with(study.join("evidence")) {
                continue;
            }
            let Some(telemetry) = read_snapshot(
                &directory.join("telemetry.json"),
                diagnostics,
                SNAPSHOT_LIMIT,
            )
            .await
            else {
                continue;
            };
            if telemetry["run_id"] != comparison["run_id"] {
                continue;
            }
            let Some(lanes) = comparison["lanes"].as_array_mut() else {
                continue;
            };
            for lane in lanes {
                let Some(id) = lane["id"]
                    .as_str()
                    .filter(|id| ["O", "SD", "CP", "OR", "SDR", "CPR"].contains(id))
                else {
                    continue;
                };
                let path = directory.join(format!("resident-{id}/session/metrics.json"));
                if !path.is_file() {
                    continue;
                }
                if let Some(metrics) = read_snapshot(&path, diagnostics, SNAPSHOT_LIMIT).await {
                    apply_addin_sample(lane, &metrics["clients"]["one"]);
                }
            }
            break;
        }
    }
}

/// Bind real received counts and timestamps to the exact Excel process identity.
fn apply_addin_sample(lane: &mut Value, client: &Value) {
    if lane["pid"].as_u64().is_none()
        || lane["process_started_at"].as_i64().is_none()
        || lane["pid"] != client["pid"]
        || lane["process_started_at"] != client["process_started_unix_ms"]
    {
        return;
    }
    lane["addin_ready"] = json!(client["ready_at"].is_string());
    let Some(at) = client["last_heartbeat_at"]
        .as_str()
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
    else {
        return;
    };
    let Some(count) = client["heartbeat_count"]
        .as_u64()
        .filter(|count| *count > 0)
    else {
        return;
    };
    lane["addin_heartbeat_at"] = json!(at.timestamp_millis());
    lane["addin_heartbeat_count"] = json!(count);
}

/// Keep all active work and the most recently finished records, independent of start order.
fn retain_recent(mut comparisons: Vec<Value>) -> Vec<Value> {
    comparisons.sort_by_key(|snapshot| {
        std::cmp::Reverse(
            snapshot["finished_at"]
                .as_i64()
                .or_else(|| snapshot["started_at"].as_i64())
                .unwrap_or(0),
        )
    });
    // A slow active comparison must not disappear behind newer completed runs.
    // This dedicated registry is separate from the thousands of corpus groups.
    let mut completed = 0;
    comparisons.retain(|snapshot| {
        if snapshot["finished_at"].is_null() {
            return true;
        }
        completed += 1;
        completed <= 100
    });
    comparisons
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_record_and_departed_processes_retire_a_frozen_running_snapshot() {
        let record = json!({"name":"book.xls", "complete":true, "status":"skipped"});
        let mut snapshot = json!({"workbook":"book.xls", "status":"running", "updated_at":300,
            "error":null, "lanes":[{"pid":12,"process_started_at":100,
                "lifecycle":"open","process_alive":true}, {"lifecycle":"not_started"}]});
        assert!(reconcile_ended_snapshot(&mut snapshot, &record, |pid, started| {
            assert_eq!((pid, started), (12, 100));
            Some(false)
        }));
        assert_eq!(snapshot["status"], "attempt_ended");
        assert_eq!(snapshot["finished_at"], 300);
        assert_eq!(snapshot["lanes"][0]["lifecycle"], "closed");
        assert!(snapshot["lanes"][0]["closed_at"].is_null());
        assert_eq!(snapshot["completion_observation"]["exact_end_time_known"], false);
        assert_eq!(record["status"], "skipped");
    }

    #[test]
    fn frozen_running_snapshot_requires_both_final_record_and_exact_exit_evidence() {
        let record = json!({"name":"book.xls", "complete":true, "status":"differences"});
        let snapshot = json!({"workbook":"book.xls", "status":"running", "updated_at":300,
            "lanes":[{"pid":12,"process_started_at":100,"lifecycle":"open"}]});
        for observation in [Some(true), None] {
            let mut candidate = snapshot.clone();
            assert!(!reconcile_ended_snapshot(&mut candidate, &record, |_, _| observation));
            assert_eq!(candidate, snapshot);
        }
        for invalid in [json!({"name":"other.xls","complete":true}),
            json!({"name":"book.xls","complete":false})] {
            assert!(!reconcile_ended_snapshot(&mut snapshot.clone(), &invalid,
                |_, _| panic!("Unrelated or unfinished record")));
        }
        let mut missing_birth = snapshot.clone();
        missing_birth["lanes"][0]["process_started_at"] = Value::Null;
        assert!(!reconcile_ended_snapshot(&mut missing_birth, &record,
            |_, _| panic!("Missing process birth time")));
    }

    #[test]
    fn departed_cleanup_process_becomes_failed_history_without_erasing_the_error() {
        let mut snapshot = json!({"run_id":"old", "status":"cleanup_unconfirmed",
            "updated_at":300, "finished_at":null, "error":"open_batch timed out",
            "lanes":[{"pid":12,"process_started_at":100,"process_alive":true,"lifecycle":"failed"},
                {"pid":null,"lifecycle":"not_started"}]});
        assert!(reconcile_cleanup(&mut snapshot, |pid, started| {
            assert_eq!((pid, started), (12, 100));
            Some(false)
        }));
        assert_eq!(snapshot["status"], "failed");
        assert_eq!(snapshot["finished_at"], 300);
        assert_eq!(snapshot["error"], "open_batch timed out");
        assert_eq!(snapshot["lanes"][0]["lifecycle"], "closed");
        assert_eq!(snapshot["lanes"][0]["process_alive"], false);
        assert!(snapshot["lanes"][0]["closed_at"].is_null());
    }

    #[test]
    fn failed_timeout_receipt_refreshes_stale_liveness_and_preserves_failure_time() {
        let mut snapshot = json!({"run_id":"timeout", "status":"failed",
            "updated_at":400, "finished_at":300, "error":"capture idle timeout",
            "lanes":[{"pid":12,"process_started_at":100,"process_alive":true,"lifecycle":"failed"}]});
        assert!(reconcile_cleanup(&mut snapshot, |pid, started| {
            assert_eq!((pid, started), (12, 100));
            Some(false)
        }));
        assert_eq!(snapshot["finished_at"], 300);
        assert_eq!(snapshot["error"], "capture idle timeout");
        assert_eq!(snapshot["lanes"][0]["lifecycle"], "closed");
        assert_eq!(snapshot["lanes"][0]["process_alive"], false);
        assert_eq!(snapshot["cleanup_observation"]["all_processes_exited"], true);
    }

    #[test]
    fn live_or_unknown_cleanup_is_not_certified_closed() {
        for observation in [Some(true), None] {
            let mut snapshot = json!({"status":"cleanup_unconfirmed","updated_at":300,
                "lanes":[{"pid":12,"process_started_at":100,"lifecycle":"failed"}]});
            assert!(!reconcile_cleanup(&mut snapshot, |_, _| observation));
            assert_eq!(snapshot["status"], "cleanup_unconfirmed");
            assert!(snapshot["finished_at"].is_null());
        }
        let mut missing_birth = json!({"status":"cleanup_unconfirmed","updated_at":300,
            "lanes":[{"pid":12,"lifecycle":"failed"}]});
        assert!(!reconcile_cleanup(&mut missing_birth, |_, _| panic!("No birth time")));
    }

    #[test]
    fn current_running_work_is_not_reclassified_by_cleanup_observation() {
        let mut snapshot = json!({"status":"running","updated_at":300,
            "lanes":[{"pid":12,"process_started_at":100,"lifecycle":"opening"}]});
        let before = snapshot.clone();
        assert!(!reconcile_cleanup(&mut snapshot, |_, _| panic!("Not cleanup")));
        assert_eq!(snapshot, before);
    }

    #[test]
    fn addin_heartbeats_require_received_messages_from_the_exact_process() {
        let mut lane = json!({"pid":12,"process_started_at":100,"process_heartbeat_count":99});
        let client = json!({"pid":12,"process_started_unix_ms":100,"ready_at":"2026-11-02T12:00:00Z",
            "last_heartbeat_at":"2026-11-02T12:00:02Z","heartbeat_count":7});
        let mut other = client.clone();
        let mut unknown_birth = lane.clone();
        unknown_birth.as_object_mut().unwrap().remove("process_started_at");
        other.as_object_mut().unwrap().remove("process_started_unix_ms");
        apply_addin_sample(&mut unknown_birth, &other);
        assert!(unknown_birth["addin_heartbeat_at"].is_null());
        other = client.clone();
        other["process_started_unix_ms"] = json!(101);
        apply_addin_sample(&mut lane, &other);
        assert!(lane["addin_heartbeat_at"].is_null());
        let mut no_beat = client.clone();
        no_beat["heartbeat_count"] = json!(0);
        apply_addin_sample(&mut lane, &no_beat);
        assert!(lane["addin_heartbeat_at"].is_null());
        apply_addin_sample(&mut lane, &client);
        assert_eq!(lane["addin_heartbeat_count"], 7);
        assert_eq!(lane["addin_heartbeat_at"], 1793620802000i64);
        apply_addin_sample(&mut lane, &client);
        assert_eq!(
            lane["addin_heartbeat_count"], 7,
            "Repeated reads do not create beats"
        );
    }

    #[test]
    fn an_old_active_run_survives_a_full_completed_history() {
        let mut snapshots: Vec<_> = (0..110)
            .map(|index| json!({"run_id":index,"finished_at":index+10,"started_at":index}))
            .collect();
        snapshots.push(json!({"run_id":"old-active","started_at":0,"finished_at":null}));
        snapshots.push(json!({"run_id":"late-finisher","started_at":0,"finished_at":500}));
        let retained = retain_recent(snapshots);
        assert_eq!(retained.len(), 101);
        assert!(retained.iter().any(|item| item["run_id"] == "old-active"));
        assert!(retained
            .iter()
            .any(|item| item["run_id"] == "late-finisher"));
        assert!(!retained.iter().any(|item| item["run_id"] == 0));
    }
}
