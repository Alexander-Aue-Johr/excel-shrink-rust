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
    if run["options"]["resident_capture"] != true || run["options"]["targeted_comparison"] == true {
        return Ok(());
    }
    let mut classic = run.clone();
    classic["options"]["resident_capture"] = json!(false);
    classic["options"]["comparison_workers"] = json!(1);
    let observed = run["options"]["control_room"] == true;
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
        if observed {
            control_room::wait_for_admission(output, "comparing")?;
        }
        scheduler::wait_for_classic_retry_memory(run)?;
        let mut initial = prepare_retry(output, &job.id, previous, run, reason)?;
        if observed {
            // Retire the resident attempt before the serial worker starts. Its
            // closed clients and old cleanup event must not describe this retry.
            control_room::begin_serial(output, source, &job.id, &mut initial, JobMode::Compare, run)?;
        }
        eprintln!(
            "[study] Retrying {} sequentially with classic capture after {reason}",
            job.id
        );
        let result = execution::run_record(
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
            |checkpoint, _phase| {
                records[job.position] = checkpoint.clone();
                // The worker watchdog invokes this callback. A whole-corpus
                // report must never delay its heartbeat or process polling.
            },
        );
        let mut result = match result {
            Ok(result) => result,
            Err(error) => {
                if observed {
                    let mut failed = records[job.position].clone();
                    failed["status"] = json!("failed");
                    control_room::finish_serial(output, &job.id, &failed)?;
                }
                return Err(error);
            }
        };
        finish_retry(&mut result);
        write_record(&record_path(output, &job.id), &result)?;
        records[job.position] = result;
        if observed {
            control_room::finish_serial(output, &job.id, &records[job.position])?;
        }
        // Durable results and queue completion precede this optional derived
        // report. Share its file timestamp across one-job fallback batches.
        if report_refresh_due(output) {
            publish_progress(output, records, corpora, "comparing");
        }
    }
    Ok(())
}

fn finish_retry(result: &mut Value) {
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
    // The resident guard measures a temporary system condition, not workbook
    // validity. Retry once with fewer live Excel instances after peers drain.
    if messages.iter().any(|message| message.contains("Resident RAM guard:")) {
        return Some("SYSTEM_MEMORY_PRESSURE");
    }
    for message in &messages {
        for token in message.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
            match token {
                "IMAGE_BUDGET" => return Some("IMAGE_BUDGET"),
                "TASKPANE_RESTARTED" => return Some("TASKPANE_RESTARTED"),
                "TASKPANE_CONTEXT_CHANGED" => return Some("TASKPANE_CONTEXT_CHANGED"),
                "InvalidOperationInCellEditMode" => return Some("InvalidOperationInCellEditMode"),
                _ => {}
            }
        }
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

    #[test]
    fn memory_pressure_retries_once_after_the_resident_batch() {
        let mut record = failed_record();
        record["error"] = json!(concat!(
            "resident Excel launcher failed: Resident RAM guard: ",
            "less than 1024 MiB physical RAM or commit headroom remains."
        ));
        assert_eq!(recoverable_failure(&record), Some("SYSTEM_MEMORY_PRESSURE"));

        record["capture_mode"] = json!("classic");
        assert_eq!(recoverable_failure(&record), None, "serial retries must not recurse");
        record["capture_mode"] = json!("resident");
        record["complete"] = json!(false);
        assert_eq!(recoverable_failure(&record), None, "active work must not be retried");

        record["complete"] = json!(true);
        record["results"] = json!(
            prepared::PROFILES.iter().map(|name| {
                json!({"profile": name, "checks_complete": true})
            }).collect::<Vec<_>>()
        );
        assert_eq!(recoverable_failure(&record), None, "completed comparisons must remain complete");
    }

    #[test]
    fn final_fallback_outcome_and_elapsed_time_are_ready_without_a_report_write() {
        for (status, expected) in [("skipped", "failed"), ("unchanged", "complete"), ("differences", "complete")] {
            let mut result = json!({"status":status,"comparing_elapsed_seconds":7.5,
                "comparison_fallback":{"status":"running","original_attempt":{"elapsed_seconds":12.5}}});
            finish_retry(&mut result);
            assert_eq!(result["comparison_fallback"]["status"], expected);
            assert_eq!(result["comparison_fallback"]["total_elapsed_seconds"], 20.0);
        }
    }

    #[test]
    fn sequential_fallback_batches_share_the_report_refresh_interval() -> Result<()> {
        let output = super::super::tests::test_directory("fallback-report-cadence")?;
        assert!(report_refresh_due(&output));
        let path = output.join("diagnosis.json");
        fs::write(&path, b"{}")?;
        assert!(!report_refresh_due(&output));
        fs::File::options().write(true).open(&path)?.set_modified(
            std::time::SystemTime::now() - Duration::from_secs(31))?;
        assert!(report_refresh_due(&output));
        fs::remove_dir_all(output)?;
        Ok(())
    }

    fn failed_record() -> Value {
        json!({"capture_mode":"resident","complete":true,"status":"skipped",
            "error":"resident original/000001: {\"code\":\"TASKPANE_RESTARTED\"}",
            "results":[{"profile":"too-aggressive-cleaning","checks_complete":false}],
            "comparing_elapsed_seconds":12.5,"comparison_batch":{"workers":2,"number":3},
            "last_execution":{"run_id":"test-run"},"resident_capture":{"status":"failed"}})
    }

    #[test]
    fn explicit_recovery_codes_are_recognized_without_a_memory_cap() {
        for code in [
            "IMAGE_BUDGET",
            "TASKPANE_RESTARTED",
            "TASKPANE_CONTEXT_CHANGED",
            "InvalidOperationInCellEditMode",
        ] {
            let mut record = failed_record();
            record["error"] = json!(format!("resident capture failed: {code}"));
            assert_eq!(recoverable_failure(&record), Some(code));
        }
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
            "TASKPANE_CONTEXT_CHANGED_OTHER",
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

    #[test]
    fn changed_taskpane_context_retries_without_relabelling_the_original_failure() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "context-fallback")?;
        let mut previous = failed_record();
        previous["error"] = json!("resident original/000001: {\"code\":\"TASKPANE_CONTEXT_CHANGED\",\"message\":\"A different taskpane document connected after work started; this does not establish a crash.\"}");
        previous["results"][0]["native_startup"] = json!({"method":"launch-to-addin-ready-v1","original":{"seconds":3.0}});
        let reason = recoverable_failure(&previous).expect("new context errors must remain recoverable");
        assert_eq!(reason,"TASKPANE_CONTEXT_CHANGED");
        let next = prepare_retry(&root,"source:42:context.xlsx",&previous,
            &json!({"run_id":"test-run","options":{"comparison_workers":4}}),reason)?;
        assert_eq!(next["comparison_fallback"]["reason_code"],reason);
        assert_eq!(next["comparison_fallback"]["original_attempt"]["error"],previous["error"]);
        let path = next["comparison_fallback"]["original_attempt"]["record_snapshot"].as_str().unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(root.join(path))?)?;
        assert_eq!(saved,previous);
        assert!(recoverable_failure(&next).is_none(),"classic retries must not recurse");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
