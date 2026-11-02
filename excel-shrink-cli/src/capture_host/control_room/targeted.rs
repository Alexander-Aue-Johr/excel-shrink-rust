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
    for name in &names {
        if let Some(snapshot) =
            read_snapshot(&root.join(name), &mut diagnostics, SNAPSHOT_LIMIT).await
        {
            comparisons.push(snapshot);
        }
    }
    let mut comparisons = retain_recent(comparisons);
    enrich_addin_heartbeats(&room, &mut comparisons, &mut diagnostics).await;
    Ok(Json(
        json!({"comparisons":comparisons,"retained_limit":100,"diagnostics":diagnostics}),
    ))
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
    let Some(queue) =
        read_snapshot(&room.root.join("queue.json"), diagnostics, SNAPSHOT_LIMIT).await
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
