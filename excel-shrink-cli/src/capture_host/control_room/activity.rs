//! Activity for runners started before explicit per-attempt telemetry existed.
use super::snapshots::{read_object, safe_id, SNAPSHOT_LIMIT};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;

fn current_attempt(group: &Value, queue: &Value) -> bool {
    let Some(started) = queue["runner_started_unix_ms"].as_u64() else {
        return false;
    };
    group["run_id"] == queue["run_id"]
        && group["attempt_id"].is_string()
        && group["started_unix_ms"]
            .as_u64()
            .is_some_and(|time| time >= started)
}

fn terminal(group: &Value) -> bool {
    group["active"] != true
        && matches!(
            group["status"].as_str(),
            Some("unchanged" | "differences" | "skipped" | "failed" | "interrupted")
        )
}

pub(super) async fn augment_progress(queue: &mut Value, root: &Path) -> Result<()> {
    if !queue["progress"].is_object()
        || queue["progress"]["activity"].is_object()
        || !queue["runner_started_unix_ms"].is_u64()
    {
        return Ok(());
    }
    let groups: Vec<_> = queue["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|group| current_attempt(group, queue))
        .collect();
    let finished: Vec<_> = groups
        .iter()
        .copied()
        .filter(|group| terminal(group))
        .collect();
    let compared = finished
        .iter()
        .filter(|group| matches!(group["status"].as_str(), Some("unchanged" | "differences")))
        .count();
    let skipped = finished
        .iter()
        .filter(|group| group["status"] == "skipped")
        .count();
    let mut activity = json!({"source":"observed_current_run", "started_attempts":groups.len(),
        "finished_attempts":finished.len(), "compared_attempts":compared, "skipped_attempts":skipped,
        "interrupted_attempts":finished.len()-compared-skipped, "retried_attempts":null,
        "attempts_per_hour":null, "recent_outcomes":[]});

    // Queue rows retain one attempt per workbook. They cannot prove the full
    // retry count or a throughput rate. Inspect only the most recently saved
    // terminal record; file metadata is enough to choose that candidate.
    let mut latest = None;
    if let Some(output) = root.parent() {
        for group in finished {
            let Some(id) = group["id"].as_str().filter(|id| safe_id(id).is_ok()) else {
                continue;
            };
            let path = output.join("records").join(format!("{id}.json"));
            let Ok(modified) = tokio::fs::metadata(&path)
                .await
                .and_then(|metadata| metadata.modified())
            else {
                continue;
            };
            if latest.as_ref().is_none_or(|(time, _, _)| modified > *time) {
                latest = Some((modified, group.clone(), path));
            }
        }
    }
    if let Some((_, group, path)) = latest {
        if let Ok(record) = read_object(&path, SNAPSHOT_LIMIT).await {
            if record["control_room_attempt"]["id"] == group["attempt_id"] {
                let id = group["id"].as_str().unwrap();
                let coordinator = read_object(
                    &root.join("groups").join(id).join("coordinator.json"),
                    2 * 1024 * 1024,
                )
                .await
                .ok();
                let at = coordinator
                    .filter(|value| {
                        value["attempt_id"] == group["attempt_id"]
                            && value["last_event"]["kind"] == "record_saved"
                    })
                    .map(|value| value["last_event"]["at_unix_ms"].clone());
                let reason: String = record["error"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(600)
                    .collect();
                activity["recent_outcomes"] = json!([{"workbook":group["workbook"],
                    "status":group["status"], "reason":reason, "retried":null,
                    "group_id":group["id"], "attempt_id":group["attempt_id"], "at_unix_ms":at}]);
            }
        }
    }
    queue["progress"]["activity"] = activity;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fallback_counts_only_attempts_since_this_runner_started() -> Result<()> {
        let mut queue = json!({"run_id":"run", "runner_started_unix_ms":100,
        "progress":{"counts":{"completed":7}}, "groups":[
            {"run_id":"run","attempt_id":"old","started_unix_ms":50,"status":"unchanged"},
            {"run_id":"other","attempt_id":"other","started_unix_ms":110,"status":"unchanged"},
            {"run_id":"run","attempt_id":"retry","started_unix_ms":110,"status":"skipped","active":false},
            {"run_id":"run","attempt_id":"active","started_unix_ms":120,"status":"active","active":true}
        ]});
        augment_progress(&mut queue, Path::new("missing/control-room")).await?;
        let activity = &queue["progress"]["activity"];
        assert_eq!(activity["started_attempts"], 2);
        assert_eq!(activity["finished_attempts"], 1);
        assert_eq!(activity["skipped_attempts"], 1);
        assert_eq!(activity["compared_attempts"], 0);
        assert!(activity["retried_attempts"].is_null());
        assert!(activity["attempts_per_hour"].is_null());
        assert_eq!(queue["progress"]["counts"]["completed"], 7);
        Ok(())
    }

    #[tokio::test]
    async fn exact_runner_activity_is_never_replaced() -> Result<()> {
        let mut queue = json!({"progress":{"activity":{"source":"runner","finished_attempts":20}}});
        let original = queue.clone();
        augment_progress(&mut queue, Path::new("missing/control-room")).await?;
        assert_eq!(queue, original);
        Ok(())
    }

    #[tokio::test]
    async fn missing_runner_start_does_not_claim_zero_observed_attempts() -> Result<()> {
        let mut queue = json!({"progress":{"counts":{"completed":7}}});
        let original = queue.clone();
        augment_progress(&mut queue, Path::new("missing/control-room")).await?;
        assert_eq!(queue, original);
        Ok(())
    }
}
