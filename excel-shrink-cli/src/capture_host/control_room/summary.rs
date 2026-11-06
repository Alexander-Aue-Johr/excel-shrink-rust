//! Bounded diagnostic projections for frequent operator and agent checks.
use super::*;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Default, Deserialize)]
pub(super) struct Options {
    limit: Option<usize>,
    stalled_after_seconds: Option<i64>,
}

/// Use the same snapshots and heartbeat enrichment as the Observatory, with compact responses.
pub(super) async fn get(
    State(room): State<Arc<Room>>,
    Query(options): Query<Options>,
) -> HttpResult<Json<Value>> {
    let limit = options.limit.unwrap_or(8);
    let stall = options.stalled_after_seconds.unwrap_or(120);
    if !(1..=50).contains(&limit) || !(5..=86400).contains(&stall) {
        return Err(ApiError::bad_request(
            "limit must be 1..50 and stalled_after_seconds 5..86400",
        ));
    }
    let mut diagnostics = Vec::new();
    let queue = room.snapshots.snapshot(
        &room.root.join("queue.json"),
        &mut diagnostics,
        SNAPSHOT_LIMIT,
    )
    .await;
    let progress = read_snapshot(
        &room.root.join("progress.json"),
        &mut diagnostics,
        SNAPSHOT_LIMIT,
    )
    .await;
    let Json(comparisons) = targeted::comparisons(State(room.clone())).await?;
    let mut result = project(
        queue.as_deref().unwrap_or(&Value::Null),
        progress.as_ref().unwrap_or(&Value::Null),
        &comparisons["comparisons"],
        limit,
        stall,
        now_ms(),
    );
    result["host_version"] = json!(room.version);
    result["runner_alive"] = queue
        .as_ref()
        .and_then(|queue| {
            queue["runner_pid"]
                .as_i64()
                .zip(queue["runner_started_unix_ms"].as_i64())
                .and_then(|(pid, birth)| crate::native_process::process_alive(pid, birth))
        })
        .into();
    result["diagnostics"] =
        json!({"snapshots":diagnostics,"comparisons":comparisons["diagnostics"]});
    result["control_endpoints"] = json!({"configure":"POST /api/control", "restart_group":"POST /api/groups/{id}/restart", "command_receipt":"GET /api/commands/{id}"});
    Ok(Json(result))
}

/// A completed host can still occupy a scheduler slot until its completion is consumed.
fn awaiting_cleanup(queue: &Value, snapshot: &Value) -> bool {
    queue["groups"].as_array().into_iter().flatten().any(|group| {
        group["active"] == true
            && group["status"] == "waiting_cleanup"
            && group["workbook"] == snapshot["workbook"]
            && group["started_unix_ms"].as_i64()
                .zip(snapshot["started_at"].as_i64())
                .is_some_and(|(group, comparison)| comparison >= group)
    })
}

fn project(
    queue: &Value,
    progress: &Value,
    comparisons: &Value,
    limit: usize,
    stall: i64,
    now: i64,
) -> Value {
    // Loading a resumed corpus can briefly leave the preceding run's progress file.
    let progress = if progress["run_id"] == queue["run_id"] {
        progress
    } else {
        &Value::Null
    };
    let started = queue["runner_started_unix_ms"].as_i64();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut active = Vec::new();
    for group in queue["groups"].as_array().into_iter().flatten() {
        *counts
            .entry(group["status"].as_str().unwrap_or("unknown").into())
            .or_default() += 1;
        if group["active"] == true {
            active.push(json!({"group_id":group["id"],"workbook":group["workbook"],"attempt_id":group["attempt_id"],
                "status":group["status"],"started_at":group["started_unix_ms"],
                "elapsed_seconds":group["started_unix_ms"].as_i64().map(|start| (now-start).max(0)/1000)}));
        }
    }
    let mut current: Vec<_> = comparisons
        .as_array()
        .into_iter()
        .flatten()
        .filter(|snapshot| {
            started
                .zip(snapshot["started_at"].as_i64())
                .is_some_and(|(run, attempt)| attempt >= run)
        })
        .collect();
    // Long-running attempts must remain visible even while newer groups complete.
    current.sort_by_key(|snapshot| (
        snapshot["status"] != "running" && !awaiting_cleanup(queue, snapshot),
        std::cmp::Reverse(snapshot["updated_at"].as_i64().unwrap_or(0)),
    ));
    let total = current.len();
    let groups: Vec<_> = current.into_iter().take(limit).map(|snapshot| {
        let operation = &snapshot["current_operation"];
        let last = snapshot["last_successful_operation"]["finished_at"].as_i64()
            .or_else(|| snapshot["started_at"].as_i64());
        let idle = last.map(|time| (now-time).max(0)/1000);
        let running = snapshot["status"] == "running";
        let mut processes = BTreeMap::new();
        for lane in snapshot["lanes"].as_array().into_iter().flatten() {
            if let Some(pid) = lane["pid"].as_u64() {
                processes.insert(pid, json!({"lane":lane["id"],"pid":pid,"birth":lane["process_started_at"],
                    "alive":lane["process_alive"],"lifecycle":lane["lifecycle"],
                    "addin_heartbeat_at":lane["addin_heartbeat_at"],"sheet":lane["current_sheet"],
                    "range":lane["current_range"],"captures_done":lane["captures_done"]}));
            }
        }
        json!({"comparison_id":snapshot["run_id"],"revision":snapshot["revision"],
            "workbook":snapshot["workbook"],"status":snapshot["status"],
            "execution_strategy":snapshot["execution_strategy"],"capture_policy":snapshot["capture_policy"],
            "report_directory":snapshot["report_directory"],"updated_at":snapshot["updated_at"],
            "capture_success_sequence":snapshot["capture_success_sequence"],"current_operation":operation,
            "last_successful_operation":snapshot["last_successful_operation"],"seconds_since_host_completion":idle,
            "attention": if running && operation["deadline_at"].as_i64().is_some_and(|time| time < now) {
                Some("command_deadline_elapsed")
            } else if running && idle.is_some_and(|seconds| seconds >= stall) {
                Some("long_operation_requires_inspection")
            } else if awaiting_cleanup(queue, snapshot) && idle.is_some_and(|seconds| seconds >= stall) {
                Some("completed_host_waiting_for_scheduler")
            } else { None },
            "result": {
                "after_differences":snapshot["report"]["after_differences"],
                "chart_differences":snapshot["report"]["chart_differences"],
                "check_performance_differences":snapshot["report"]["check_performance_differences"],
                "chart_comparisons":snapshot["report"]["chart_comparisons"],
                "check_performance":snapshot["report"]["check_performance"]
            },
            "processes":processes.into_values().collect::<Vec<_>>(),"error":snapshot["error"]})
    }).collect();
    json!({"run_id":queue["run_id"],"runner_pid":queue["runner_pid"],"running":queue["running"],"phase":queue["phase"],
        "queue_updated_at":queue["updated_unix_ms"],"counts":progress["counts"],"rates":progress["rates"],
        "eta":progress["eta"],"queue_statuses":counts,"active_count":active.len(),
        "active_groups":active.into_iter().take(limit).collect::<Vec<_>>(),
        "recent_outcomes":progress["activity"]["recent_outcomes"],"comparisons":groups,
        "comparisons_available":total,"comparisons_truncated":total>limit,
        "coverage_note":"Long operations are diagnostic observations; completion and correctness require retained comparison evidence."})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_hosts_still_occupying_current_scheduler_slots_require_attention() {
        let mut queue = json!({"runner_started_unix_ms":1,"groups":[{
            "active":true,"workbook":"book.xlsx","status":"waiting_cleanup","started_unix_ms":100
        }]});
        let snapshots = json!([{
            "workbook":"book.xlsx","started_at":101,"updated_at":1000,
            "status":"completed_with_limitations","last_successful_operation":{"finished_at":1000}
        }]);
        let summary = project(&queue, &Value::Null, &snapshots, 1, 120, 200_000);
        assert_eq!(summary["comparisons"][0]["attention"], "completed_host_waiting_for_scheduler");
        queue["groups"][0]["active"] = json!(false);
        assert!(project(&queue, &Value::Null, &snapshots, 1, 120, 200_000)["comparisons"][0]["attention"].is_null());
        queue["groups"][0]["active"] = json!(true);
        queue["groups"][0]["started_unix_ms"] = json!(102);
        assert!(!awaiting_cleanup(&queue, &snapshots[0]));
    }

    #[test]
    fn summaries_are_bounded_exclude_old_attempts_and_separate_liveness_from_progress() {
        let queue = json!({"runner_started_unix_ms":100,"groups":[{"active":true,"id":"a"},{"active":true,"id":"b"}]});
        let snapshots = json!([
            {"started_at":99,"status":"running"},
            {"started_at":100,"updated_at":200,"status":"running", "lanes":[{"id":"O","pid":12,"process_alive":true}],"report":{"after_differences":2}},
            {"started_at":101,"updated_at":299,"status":"completed_with_limitations"}
        ]);
        let summary = project(&queue, &Value::Null, &snapshots, 1, 120, 200_000);
        assert_eq!(summary["comparisons_available"], 2);
        assert_eq!(summary["comparisons"].as_array().unwrap().len(), 1);
        assert_eq!(summary["active_count"], 2);
        assert_eq!(summary["active_groups"].as_array().unwrap().len(), 1);
        assert_eq!(
            summary["comparisons"][0]["attention"],
            "long_operation_requires_inspection"
        );
        assert_eq!(summary["comparisons"][0]["processes"][0]["alive"], true);
        assert_eq!(summary["comparisons"][0]["result"]["after_differences"], 2);
        let stale = project(
            &json!({"run_id":"new"}),
            &json!({"run_id":"old","counts":{"completed":999}}),
            &json!([]), 1, 120, 200_000,
        );
        assert!(stale["counts"].is_null());
    }
}
