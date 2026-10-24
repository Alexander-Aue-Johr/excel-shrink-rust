//! Retry recoverable resident capture failures only after the entire batch drains.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn after_batch(
    source: &Source,
    jobs: &[scheduler::Comparison],
    output: &Path,
    records: &mut [Value],
    corpora: &[Value],
    limits: &ArchiveArgs,
    run: &Value,
    log_level: LogLevel,
) -> Result<()> {
    if run["options"]["resident_capture"] != true {
        return Ok(());
    }
    let mut classic = run.clone();
    classic["options"]["resident_capture"] = json!(false);
    classic["options"]["comparison_workers"] = json!(1);
    // No queue expansion: each original batch member receives at most one retry.
    for job in jobs {
        let previous = &records[job.position];
        let Some(reason) = recoverable_failure(previous) else {
            continue;
        };
        if !bundle_path(output, &job.id).is_file() {
            eprintln!(
                "[study] Cannot retry {}: prepared bundle is missing",
                job.id
            );
            continue;
        }
        scheduler::wait_for_classic_retry_memory(run)?;
        let initial = prepare_retry(output, &job.id, previous, run, reason)?;
        eprintln!(
            "[study] Retrying {} sequentially with classic capture after {reason}",
            job.id
        );
        let mut result = execution::run_record(
            source,
            job.index,
            &job.id,
            JobMode::Compare,
            output,
            initial,
            limits,
            &classic,
            log_level,
            None,
            |checkpoint, phase| {
                records[job.position] = checkpoint.clone();
                publish_progress(output, records, corpora, phase);
            },
        )?;
        result["comparison_fallback"]["status"] = json!(if result["status"] == "skipped" {
            "failed"
        } else {
            "complete"
        });
        result["comparison_fallback"]["total_elapsed_seconds"] = json!(
            result["comparison_fallback"]["original_attempt"]["elapsed_seconds"]
                .as_f64()
                .unwrap_or(0.0)
                + result["comparing_elapsed_seconds"].as_f64().unwrap_or(0.0)
        );
        write_record(&record_path(output, &job.id), &result)?;
        records[job.position] = result;
        publish_progress(output, records, corpora, "comparing");
    }
    Ok(())
}

pub(super) fn recoverable_failure(record: &Value) -> Option<&'static str> {
    if record["capture_mode"] != "resident"
        || record["status"] != "skipped"
        || record["complete"] != true
    {
        return None;
    }
    let profiles = record["results"].as_array();
    if profiles.is_some_and(|profiles| {
        prepared::PROFILES.iter().all(|name| {
            profiles
                .iter()
                .any(|profile| profile["profile"] == *name && profile["checks_complete"] == true)
        })
    }) {
        // Completed primary checks with an auxiliary failure are not retried.
        return None;
    }
    let messages = std::iter::once(record["error"].as_str())
        .chain(
            profiles
                .into_iter()
                .flatten()
                .filter(|profile| profile["checks_complete"] != true)
                .flat_map(|profile| profile["failures"].as_array().into_iter().flatten())
                .map(Value::as_str),
        )
        .flatten()
        .collect::<Vec<_>>();
    for message in &messages {
        for token in message.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
            match token {
                "IMAGE_BUDGET" => return Some("IMAGE_BUDGET"),
                "TASKPANE_RESTARTED" => return Some("TASKPANE_RESTARTED"),
                "InvalidOperationInCellEditMode" => return Some("InvalidOperationInCellEditMode"),
                _ => {}
            }
        }
    }
    let batch = &record["comparison_batch"];
    let reduced_budget = batch["adaptive"] == true
        && match (
            batch["resident_memory_limit_mib"].as_u64(),
            batch["configured_resident_memory_mib"].as_u64(),
        ) {
            (Some(effective), Some(configured)) => effective > 0 && effective < configured,
            _ => false,
        };
    let deterministic_failure = messages.iter().any(|message| {
        let lower = message.to_ascii_lowercase();
        lower.contains("drawing boundary is unresolved")
            || lower.contains("drawing anchor exceeds capture boundary")
    });
    if reduced_budget
        && !deterministic_failure
        && messages.iter().any(|message| !message.trim().is_empty())
    {
        // A Windows allocation limit can surface as an exit or timeout rather
        // than an explicit memory error. The existing classic retry runs only
        // after the batch drains and does not inherit the reduced resident cap.
        return Some("ADAPTIVE_MEMORY_BUDGET");
    }
    None
}

fn prepare_retry(
    output: &Path,
    id: &str,
    previous: &Value,
    run: &Value,
    reason: &str,
) -> Result<Value> {
    let key = artifact_cache::sha256(id.as_bytes());
    let run_key =
        artifact_cache::sha256(run["run_id"].as_str().context("missing run id")?.as_bytes());
    let prefix = format!("logs/{}-resident-attempt-{}", &key[..20], &run_key[..16]);
    let snapshot = format!("{prefix}.json");
    let log = format!("{prefix}.log");
    fs::create_dir_all(output.join("logs"))?;
    // Save the full failed record and worker log before the classic worker can
    // replace its ordinary log or overwrite evidence with the same profile name.
    artifact_cache::atomic_write(
        &output.join(&snapshot),
        &serde_json::to_vec_pretty(previous)?,
    )?;
    let old_log = output.join("logs").join(format!("{}.log", &key[..20]));
    let retained_log = if old_log.is_file() {
        fs::copy(&old_log, output.join(&log))?;
        Some(log)
    } else {
        None
    };
    let mut next = previous.clone();
    next["comparison_fallback"] = json!({
        "run_id":run["run_id"],"status":"running","reason_code":reason,
        "requested_capture_mode":"resident","actual_capture_mode":"classic",
        "workers":1,"original_attempt":{
            "status":previous["status"],"error":previous["error"],
            "elapsed_seconds":previous["comparing_elapsed_seconds"],
            "last_execution":previous["last_execution"],"comparison_batch":previous["comparison_batch"],
            "record_snapshot":snapshot,"worker_log":retained_log
        }
    });
    next["comparison_batch"] = json!({
        "run_id":run["run_id"],"number":previous["comparison_batch"]["number"],
        "workers":1,"requested_workers":run["options"]["comparison_workers"],
        "scheduling":"sequential fallback after batch"
    });
    next["capture_mode"] = json!("classic");
    next.as_object_mut().unwrap().remove("resident_capture");
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed_record() -> Value {
        json!({"capture_mode":"resident","complete":true,"status":"skipped",
            "error":"resident original/000001: {\"code\":\"TASKPANE_RESTARTED\"}",
            "results":[{"profile":"too-aggressive-cleaning","checks_complete":false}],
            "comparing_elapsed_seconds":12.5,"comparison_batch":{"workers":2,"number":3},
            "last_execution":{"run_id":"test-run"},"resident_capture":{"status":"failed"}})
    }

    fn adaptive_failure() -> Value {
        let mut record = failed_record();
        record["error"] = json!("resident Excel process exited unexpectedly: original");
        record["comparison_batch"]["adaptive"] = json!(true);
        record["comparison_batch"]["resident_memory_limit_mib"] = json!(2048);
        record["comparison_batch"]["configured_resident_memory_mib"] = json!(16384);
        record
    }

    #[test]
    fn adaptive_retry_requires_a_reduced_cap_and_completed_primary_failure() {
        let record = adaptive_failure();
        assert_eq!(recoverable_failure(&record), Some("ADAPTIVE_MEMORY_BUDGET"));
        for effective in [json!(16384), json!(32768), json!(0), Value::Null] {
            let mut unchanged = record.clone();
            unchanged["comparison_batch"]["resident_memory_limit_mib"] = effective;
            assert_eq!(recoverable_failure(&unchanged), None);
        }
        for (field, value) in [
            ("adaptive", json!(false)),
            ("configured_resident_memory_mib", Value::Null),
        ] {
            let mut unchanged = record.clone();
            unchanged["comparison_batch"][field] = value;
            assert_eq!(recoverable_failure(&unchanged), None);
        }
        for (field, value) in [
            ("complete", json!(false)),
            ("capture_mode", json!("classic")),
            ("status", json!("comparing")),
        ] {
            let mut unchanged = record.clone();
            unchanged[field] = value;
            assert_eq!(recoverable_failure(&unchanged), None);
        }
    }

    #[test]
    fn explicit_recovery_codes_take_priority_over_reduced_memory_cap() {
        for code in [
            "IMAGE_BUDGET",
            "TASKPANE_RESTARTED",
            "InvalidOperationInCellEditMode",
        ] {
            let mut record = adaptive_failure();
            record["error"] = json!(format!("resident capture failed: {code}"));
            assert_eq!(recoverable_failure(&record), Some(code));
        }
    }

    #[test]
    fn adaptive_retry_excludes_deterministic_auxiliary_and_historical_errors() {
        for message in [
            "Drawing boundary is unresolved on Sheet1",
            "drawing anchor exceeds capture boundary",
        ] {
            let mut record = adaptive_failure();
            record["error"] = json!(message);
            assert_eq!(recoverable_failure(&record), None);
            record["error"] = json!("One or more comparisons could not be completed.");
            record["results"][0]["failures"] = json!([message]);
            assert_eq!(recoverable_failure(&record), None);
        }
        let mut record = adaptive_failure();
        record["results"] = json!(
            prepared::PROFILES
                .iter()
                .map(|name| json!({
                    "profile":name,"checks_complete":true
                }))
                .collect::<Vec<_>>()
        );
        record["whitespace_diagnosis"] = json!({"status":"failed","error":"worker exited"});
        assert_eq!(recoverable_failure(&record), None);

        let mut record = adaptive_failure();
        record["error"] = Value::Null;
        record["whitespace_diagnosis"] = json!({"status":"failed","error":"worker exited"});
        record["comparison_fallback"] = json!({"original_attempt":{"error":"TASKPANE_RESTARTED"}});
        assert_eq!(recoverable_failure(&record), None);
        record["results"][0]["failures"] = json!(["workbook timed out"]);
        assert_eq!(recoverable_failure(&record), Some("ADAPTIVE_MEMORY_BUDGET"));
    }

    #[test]
    fn only_completed_recoverable_primary_resident_failures_retry() {
        let mut record = failed_record();
        assert_eq!(recoverable_failure(&record), Some("TASKPANE_RESTARTED"));
        record["error"] = json!("capture failed: IMAGE_BUDGET");
        assert_eq!(recoverable_failure(&record), Some("IMAGE_BUDGET"));
        record["error"] = json!(
            "Resident client error: {\"client_id\":\"original\",\"code\":\"InvalidOperationInCellEditMode\"}"
        );
        assert_eq!(
            recoverable_failure(&record),
            Some("InvalidOperationInCellEditMode")
        );
        for error in [
            "drawing anchor exceeds capture boundary",
            "workbook timed out",
            "IMAGE_BUDGET_OTHER",
            "InvalidOperationInCellEditModeOther",
        ] {
            record["error"] = json!(error);
            assert_eq!(recoverable_failure(&record), None);
        }
        record["error"] = json!("IMAGE_BUDGET");
        record["complete"] = json!(false);
        assert_eq!(recoverable_failure(&record), None);
        record["complete"] = json!(true);
        record["capture_mode"] = json!("classic");
        assert_eq!(recoverable_failure(&record), None);
    }

    #[test]
    fn supplemental_or_historical_failures_do_not_trigger_primary_retry() {
        let mut record = failed_record();
        record["results"] = json!(
            prepared::PROFILES
                .iter()
                .map(|name| json!({
            "profile":name,"checks_complete":true}))
                .collect::<Vec<_>>()
        );
        assert_eq!(recoverable_failure(&record), None);
        record["results"] = json!([]);
        record["error"] = json!("different primary failure");
        record["whitespace_diagnosis"] = json!({"status":"failed","error":"IMAGE_BUDGET"});
        record["comparison_fallback"] = json!({"original_attempt":{"error":"TASKPANE_RESTARTED"}});
        assert_eq!(recoverable_failure(&record), None);
    }

    #[test]
    fn retry_preserves_failed_attempt_and_log_with_actual_sequential_attribution() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "resident-fallback")?;
        fs::create_dir_all(root.join("logs"))?;
        let id = "source:42:workbook.xlsx";
        let key = artifact_cache::sha256(id.as_bytes());
        let log = root.join("logs").join(format!("{}.log", &key[..20]));
        fs::write(&log, b"original resident worker log")?;
        let previous = failed_record();
        let run =
            json!({"run_id":"test-run","options":{"resident_capture":true,"comparison_workers":2}});
        let next = prepare_retry(&root, id, &previous, &run, "TASKPANE_RESTARTED")?;
        assert_eq!(next["capture_mode"], "classic");
        assert_eq!(next["comparison_batch"]["workers"], 1);
        assert_eq!(next["comparison_batch"]["requested_workers"], 2);
        assert!(next["resident_capture"].is_null());
        let original = &next["comparison_fallback"]["original_attempt"];
        assert_eq!(original["elapsed_seconds"], 12.5);
        assert_eq!(original["comparison_batch"]["workers"], 2);
        let saved: Value = serde_json::from_slice(&fs::read(
            root.join(original["record_snapshot"].as_str().unwrap()),
        )?)?;
        assert_eq!(saved, previous);
        fs::write(&log, b"replacement classic log")?;
        assert_eq!(
            fs::read(root.join(original["worker_log"].as_str().unwrap()))?,
            b"original resident worker log"
        );
        assert_eq!(recoverable_failure(&next), None);
        assert_eq!(run["options"]["resident_capture"], true);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
