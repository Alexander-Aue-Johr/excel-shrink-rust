use super::*;
use anyhow::bail;

#[allow(clippy::too_many_arguments)]
pub(super) fn process(
    source: &Source,
    index: usize,
    id: &str,
    position: usize,
    mode: JobMode,
    output: &Path,
    records: &mut Vec<Value>,
    corpora: &[Value],
    limits: &ArchiveArgs,
    run: &Value,
    log_level: LogLevel,
) -> Result<()> {
    if run["options"]["control_room"] == true {
        if mode == JobMode::Prepare {
            // The group target controls visual comparisons. Preparation must
            // finish for the whole selection before that target is admitted.
            control_room::check_stop(&output.join("control-room"))?;
        } else {
            control_room::wait_for_admission(output, "comparing")?;
        }
        control_room::begin_serial(output, source, id, &mut records[position], mode, run)?;
    }
    let initial = records[position].clone();
    let result = run_record(
        source,
        index,
        id,
        mode,
        output,
        initial,
        limits,
        run,
        log_level,
        None,
        |checkpoint, _phase| {
            records[position] = checkpoint.clone();
            // Keep the worker heartbeat and exit polling independent of the
            // full corpus report, including during preparation and diagnosis.
        },
    )?;
    records[position] = result;
    if run["options"]["control_room"] == true {
        control_room::finish_serial(output, id, &records[position])?;
    }
    if report_refresh_due(output) {
        let phase = match mode {
            JobMode::Prepare => "shrinking",
            JobMode::Diagnose => "diagnosing whitespace",
            _ => "comparing",
        };
        publish_progress(output, records, corpora, phase);
    }
    Ok(())
}

// This worker owns only its workbook record and evidence directory. The caller
// publishes shared reports, either directly or through the batch event channel.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_record(
    source: &Source,
    index: usize,
    id: &str,
    mode: JobMode,
    output: &Path,
    mut state: Value,
    limits: &ArchiveArgs,
    run: &Value,
    log_level: LogLevel,
    cleanup_barrier: Option<batch_cleanup::Participant>,
    mut on_progress: impl FnMut(&Value, &str),
) -> Result<Value> {
    let mut active_progress = progress::start_job(output, id, mode, &state);
    let key = artifact_cache::sha256(id.as_bytes());
    let work = output.join(".work").join(&key[..20]);
    let record = record_path(output, id);
    if let Err(error) = wait_for_owned_cleanup(
        &work,
        &mut state,
        cleanup_barrier.as_ref(),
        &mut on_progress,
    ) {
        write_record(&record, &state)?;
        return Err(error);
    }
    if work.exists() {
        fs::remove_dir_all(&work)?;
    }
    fs::create_dir_all(&work)?;
    let diagnosis_only = mode == JobMode::Diagnose;
    if !diagnosis_only {
        state["complete"] = json!(false);
    }
    if mode == JobMode::Prepare && run["options"]["preparation_stage"] != "hash_sweep" {
        state["prepared"] = json!(false);
    }
    if !diagnosis_only {
        state["error"] = Value::Null;
    }
    if !diagnosis_only {
        state["status"] = json!(if mode == JobMode::Prepare {
            "shrinking"
        } else {
            "comparing"
        });
    }
    state["last_execution"] = json!({"run_id":run["run_id"],"machine_id":run["machine_id"],
        "machine_manifest":run["machine_manifest"],"mode":mode,"started_unix_ms":provenance::now_ms()});
    if mode == JobMode::Compare {
        state["comparison_pass"] = run["options"]["comparison_pass"].clone();
        prepared::clear_startup_measurements(&mut state);
        state["capture_mode"] = json!(if run["options"]["resident_capture"] == true {
            "resident"
        } else {
            "classic"
        });
        state["last_execution"]["capture_mode"] = state["capture_mode"].clone();
        // The current attempt must not display metrics from an earlier session.
        state.as_object_mut().unwrap().remove("resident_capture");
        state.as_object_mut().unwrap().remove("worker_memory");
        state.as_object_mut().unwrap().remove("package_identity");
    }
    write_record(&record, &state)?;
    let job = MemberJob {
        archive: source.path.clone(),
        index,
        identity: id.into(),
        name: state["name"].as_str().unwrap().into(),
        work: work.clone(),
        record: record.clone(),
        report_root: output.into(),
        max_workbook_bytes: limits.max_workbook_bytes,
        mode,
        bundle: Some(bundle_path(output, id)),
        resident_capture: run["options"]["resident_capture"] == true,
        hide_excel: state["control_room_attempt"]["settings"]["hide_excel"]
            .as_bool()
            .unwrap_or(run["options"]["hide_excel"] == true),
        // Coordinator observations also belong to classic fallback attempts.
        // Resident-only client controls are filtered separately by worker::launch.
        control_room: (run["options"]["control_room"] == true)
            .then(|| output.join("control-room")),
        cleanup_barrier,
        run_manifest: Some(
            json!({"run_id":run["run_id"],"machine_id":run["machine_id"],
            "machine_manifest":run["machine_manifest"],
            "preparation_hash_profile":run["options"]["preparation_hash_profile"],
            "preparation_stage":run["options"]["preparation_stage"]}),
        ),
    };
    let path = work.join("job.json");
    fs::write(&path, serde_json::to_vec_pretty(&job)?)?;
    let phase = if diagnosis_only {
        "diagnosing whitespace"
    } else if mode == JobMode::Prepare && run["options"]["preparation_stage"] == "hash_sweep" {
        "hash sweeping"
    } else if mode == JobMode::Prepare {
        "shrinking"
    } else {
        "comparing"
    };
    eprintln!("[study] {} · {phase} · {}", source.name, job.name);
    on_progress(&state, phase);
    progress::observe(output, &state);
    let started = Instant::now();
    let outcome = worker::launch(&job, &path, limits, log_level, || {
        progress::tick(output);
        let current: Value = serde_json::from_slice(&fs::read(&record)?)?;
        if current != state {
            state = current;
            progress::observe(output, &state);
            on_progress(&state, phase);
        }
        Ok(())
    });
    let mut result: Value = serde_json::from_slice(&fs::read(&record)?)?;
    let immediate_stop = job
        .control_room
        .as_ref()
        .is_some_and(|root| control_room::terminate_requested(root));
    if immediate_stop
        || job
            .cleanup_barrier
            .as_ref()
            .is_some_and(|barrier| barrier.cancelled() || barrier.restart_requested())
    {
        let restarting = job
            .cleanup_barrier
            .as_ref()
            .is_some_and(|barrier| barrier.restart_requested());
        if restarting {
            // Replacement removes .work. Preserve whatever evidence and logs
            // were already published, without treating the attempt as complete.
            let _ = retain_worker_memory(&mut result, &work);
            retain_visual_evidence(&mut result, &work, output, &key[..20], diagnosis_only);
            relative_images(&mut result, output);
            let attempt = result["control_room_attempt"]["id"]
                .as_str()
                .unwrap_or("legacy");
            let log_key = format!(
                "{}-{}",
                &key[..20],
                &artifact_cache::sha256(attempt.as_bytes())[..12]
            );
            let _ = retain_capture_logs(&work, output, &log_key);
            let _ = retain_resident_diagnostics(&work, output, &log_key, true);
        }
        mark_cancelled(&mut result, job.bundle.as_deref());
        if restarting {
            result["status"] = json!("restart_pending");
            result["last_execution"]["restart_requested"] = json!(true);
            result["error"] =
                json!("User requested a fresh comparison; previous evidence retained.");
        }
        if let Err(error) = &outcome {
            result["last_execution"]["interruption_detail"] = json!(format!("{error:#}"));
        }
        // Keep the prepared ZIP and work directory. A subsequent resume can
        // validate/reuse the ZIP and retry this unfinished comparison normally.
        write_record(&record, &result)?;
        progress::observe(output, &result);
        on_progress(&result, "interrupted");
        active_progress.finish(&result);
        return Ok(result);
    }
    if let Err(error) = outcome {
        if diagnosis_only {
            result["whitespace_diagnosis"]["version"] = json!(whitespace::VERSION);
            result["whitespace_diagnosis"]["status"] = json!("failed");
            result["whitespace_diagnosis"]["error"] = json!(format!("{error:#}"));
        } else {
            result["status"] = json!("skipped");
            result["error"] = json!(format!("{error:#}"));
        }
    }
    if let Err(error) = wait_for_owned_cleanup(
        &work,
        &mut result,
        job.cleanup_barrier.as_ref(),
        &mut on_progress,
    ) {
        write_record(&record, &result)?;
        return Err(error);
    }
    if let Err(error) = retain_worker_memory(&mut result, &work) {
        eprintln!("Could not preserve worker memory measurement: {error:#}");
    }
    retain_visual_evidence(&mut result, &work, output, &key[..20], diagnosis_only);
    relative_images(&mut result, output);
    let failed = result["status"] == "skipped"
        || result["hash_sweep_error"].is_string()
        || result["whitespace_diagnosis"]["status"] == "failed";
    if failed {
        if let Err(error) = retain_capture_logs(&work, output, &key[..20]) {
            eprintln!("Could not preserve capture diagnostics: {error:#}");
        }
    }
    if let Err(error) = retain_resident_diagnostics(&work, output, &key[..20], failed) {
        eprintln!("Could not preserve resident capture diagnostics: {error:#}");
    }
    result[format!("{phase}_elapsed_seconds")] = json!(started.elapsed().as_secs_f64());
    result["complete"] =
        json!(diagnosis_only || mode == JobMode::Compare || result["status"] == "skipped");
    if mode == JobMode::Compare {
        strategy::finish(&mut result);
    }
    cleanup_work(&work, &bundle_path(output, id), &mut result)?;
    write_record(&record, &result)?;
    progress::observe(output, &result);
    on_progress(&result, phase);
    active_progress.finish(&result);
    Ok(result)
}

// An uncertain owner reserves this slot during bounded retries. If it cannot be
// verified, stop with its journal intact rather than wait forever or admit orphans.
fn wait_for_owned_cleanup(
    work: &Path,
    record: &mut Value,
    participant: Option<&batch_cleanup::Participant>,
    on_progress: &mut impl FnMut(&Value, &str),
) -> Result<()> {
    let result = retry_owned_cleanup(
        || worker::cleanup_windows(work),
        |observation| {
            record["cleanup_pending"] = observation;
            on_progress(record, "cleanup");
        },
        || {
            let Some(participant) = participant else {
                bail!("Cannot confirm owned process cleanup; retaining work directory");
            };
            // Poll cancellation without retrying the OS access check in a loop.
            for _ in 0..120 {
                ensure!(
                    !participant.cancelled(),
                    "Cleanup cancelled; retaining work directory"
                );
                std::thread::sleep(Duration::from_millis(250));
            }
            Ok(())
        },
    );
    if result.is_ok() && record["cleanup_pending"]["state"] == "waiting" {
        // A resumed attempt may succeed on its first cleanup check. Its old
        // waiting observation must not keep the new attempt visually blocked.
        record["cleanup_pending"]["state"] = json!("resolved");
        record["cleanup_pending"]["last_attempt_unix_ms"] = json!(provenance::now_ms());
        on_progress(record, "cleanup");
    }
    result
}

fn retry_owned_cleanup(
    mut cleanup: impl FnMut() -> Result<()>,
    mut observe: impl FnMut(Value),
    mut wait: impl FnMut() -> Result<()>,
) -> Result<()> {
    let since = provenance::now_ms();
    let mut attempts = 0u64;
    loop {
        match cleanup() {
            Ok(()) => {
                if attempts > 0 {
                    observe(json!({"state":"resolved", "attempts":attempts + 1,
                        "since_unix_ms":since,"last_attempt_unix_ms":provenance::now_ms()}));
                }
                return Ok(());
            }
            Err(error) => {
                attempts += 1;
                let exhausted = attempts >= 3;
                observe(json!({"state":"waiting", "error":format!("{error:#}"),
                    "attempts":attempts,"since_unix_ms":since,
                    "retry_exhausted":exhausted,
                    "last_attempt_unix_ms":provenance::now_ms()}));
                if exhausted {
                    return Err(error).context(
                        "Owned cleanup could not be confirmed after 3 attempts; stopping the run and retaining its process journal"
                    );
                }
                wait().with_context(|| format!("owned cleanup remains unconfirmed: {error:#}"))?;
            }
        }
    }
}

fn cleanup_work(work: &Path, bundle: &Path, result: &mut Value) -> Result<()> {
    fs::remove_dir_all(work)?;
    // Preparation is reusable research data. Comparisons discard only their
    // working copies, including after a successful visual check.
    result["prepared"] = json!(result["prepared"] == true && bundle.is_file());
    Ok(())
}

fn mark_cancelled(result: &mut Value, bundle: Option<&Path>) {
    result["complete"] = json!(false);
    result["status"] = json!("interrupted");
    result["error"] = json!("Coordinator cancelled this attempt; retained for resume.");
    result["prepared"] = json!(bundle.is_some_and(Path::is_file));
    result["last_execution"]["cancelled"] = json!(true);
}

fn retain_worker_memory(record: &mut Value, work: &Path) -> Result<()> {
    let path = work.join("worker-memory.json");
    if path.is_file() {
        record["worker_memory"] = serde_json::from_slice(&fs::read(path)?)?;
    }
    Ok(())
}

fn retain_visual_evidence(
    result: &mut Value,
    work: &Path,
    output: &Path,
    key: &str,
    diagnosis_only: bool,
) {
    let mut evidence = output.join("evidence").join(key);
    if let Some(attempt) = result["control_room_attempt"]["id"].as_str() {
        // Hash untrusted/imported record fields before using them as a path.
        evidence = evidence.join(format!(
            "attempt-{}",
            &artifact_cache::sha256(attempt.as_bytes())[..20]
        ));
    }
    if result["capture_mode"] == "classic" && result["comparison_fallback"]["status"] == "running" {
        if let Some(run_id) = result["comparison_fallback"]["run_id"].as_str() {
            let attempt = artifact_cache::sha256(run_id.as_bytes());
            evidence = evidence.join("classic-fallback").join(&attempt[..16]);
        }
    }
    // Reused primary evidence already lives under output, outside this worker's
    // temporary directory. Retain only fresh profiles, including freshly retried
    // aggressive comparisons; leave the preserved profile and its diagnosis intact.
    let secondary = super::strategy::pass(result) == super::strategy::SECONDARY;
    let mut reused_profiles = Vec::new();
    if secondary {
        for (index, profile) in result["results"].as_array_mut().into_iter().flatten().enumerate() {
            if profile["profile"] == super::strategy::AGGRESSIVE
                && profile["comparison_reused_from_pass"] == super::strategy::PRIMARY
                && (profile["checks_complete"] == true || profile["package_checks_complete"] == true)
            {
                reused_profiles.push((index, profile.take()));
            }
        }
    }
    let mut visual_diagnoses = Vec::new();
    for profile in result["results"].as_array_mut().into_iter().flatten() {
        if profile["visual_diagnosis"].is_object() {
            visual_diagnoses.push((profile["profile"].as_str().unwrap_or("unknown").to_owned(), profile["visual_diagnosis"].take()));
        }
    }
    let mut diagnosis = result["whitespace_diagnosis"].take();
    if !diagnosis_only {
        if let Err(error) = worker::retain_evidence(result, work, &evidence.join("comparison")) {
            result["status"] = json!("skipped");
            result["error"] = json!(format!("preserve image evidence: {error:#}"));
            clear_images(result);
        }
    }
    // Supplemental evidence failure must not erase a completed primary check.
    if let Err(error) =
        worker::retain_evidence(&mut diagnosis, work, &evidence.join("whitespace-v1"))
    {
        diagnosis["status"] = json!("failed");
        diagnosis["error"] = json!(format!("preserve whitespace evidence: {error:#}"));
        clear_images(&mut diagnosis);
    }
    result["whitespace_diagnosis"] = diagnosis;
    for (name, mut diagnosis) in visual_diagnoses {
        let directory = evidence.join("visual-diagnosis").join(&artifact_cache::sha256(name.as_bytes())[..16]);
        if let Err(error) = worker::retain_evidence(&mut diagnosis, work, &directory) {
            diagnosis["status"] = json!("unresolved");
            diagnosis["category"] = json!("unresolved");
            diagnosis["error"] = json!(format!("preserve diagnostic evidence: {error:#}"));
            clear_images(&mut diagnosis);
        }
        if let Some(profile) = result["results"].as_array_mut().into_iter().flatten().find(|profile| profile["profile"] == name) {
            profile["visual_diagnosis"] = diagnosis;
        }
    }
    for (index, profile) in reused_profiles {
        result["results"][index] = profile;
    }
}

fn retain_capture_logs(work: &Path, output: &Path, key: &str) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    const MAX_LOG_BYTES: u64 = 1024 * 1024;
    let logs = output.join("logs");
    fs::create_dir_all(&logs)?;
    for profile in prepared::PROFILES {
        let mut captures = vec![(
            profile.to_owned(),
            work.join("comparisons").join(profile).join("visual/images"),
        )];
        for phase in [
            "without-whitespace-deletion",
            "reproduce-original-comparison",
        ] {
            captures.push((
                format!("{profile}-{phase}"),
                work.join("whitespace")
                    .join(profile)
                    .join(phase)
                    .join("images"),
            ));
        }
        for (label, capture) in captures {
            for name in ["host.stdout.log", "host.stderr.log"] {
                let path = capture.join(name);
                if !path.is_file() {
                    continue;
                }
                let mut source = fs::File::open(path)?;
                let length = source.metadata()?.len();
                source.seek(SeekFrom::Start(length.saturating_sub(MAX_LOG_BYTES)))?;
                let mut tail = Vec::new();
                source.take(MAX_LOG_BYTES).read_to_end(&mut tail)?;
                fs::write(logs.join(format!("{key}-{label}-{name}")), tail)?;
            }
        }
    }
    Ok(())
}

// Keep enough timing and heartbeat evidence to inspect whether opening another
// workbook blocked a resident client. Explicit filenames exclude staged data,
// client descriptors, command payloads and captured PNGs.
fn retain_resident_diagnostics(work: &Path, output: &Path, key: &str, failed: bool) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let session = work.join("resident");
    if !session.is_dir() {
        return Ok(());
    }
    let logs = output.join("logs");
    fs::create_dir_all(&logs)?;
    let mut names = vec![
        "metrics.json",
        "launcher-events.jsonl",
        "host-events.jsonl",
        "host.stdout.log",
    ];
    if failed {
        names.extend([
            "launcher.stdout.log",
            "launcher.stderr.log",
            "launcher-result.json",
            "host.stderr.log",
            "host-build.stdout.log",
            "host-build.stderr.log",
        ]);
    }
    for name in names {
        let path = session.join(name);
        if !path.is_file() {
            continue;
        }
        let limit = if !failed && name == "host.stdout.log" {
            256 * 1024
        } else {
            2 * 1024 * 1024
        };
        let mut source = fs::File::open(path)?;
        let offset = source.metadata()?.len().saturating_sub(limit);
        source.seek(SeekFrom::Start(offset))?;
        let mut tail = Vec::new();
        source.take(limit).read_to_end(&mut tail)?;
        if offset > 0 && name.ends_with(".jsonl") {
            // A retained event file remains parseable line by line.
            if let Some(newline) = tail.iter().position(|&byte| byte == b'\n') {
                tail.drain(..=newline);
            } else {
                tail.clear();
            }
        }
        let suffix = if offset > 0 && name.ends_with(".json") {
            ".tail.log"
        } else {
            ""
        };
        fs::write(logs.join(format!("{key}-resident-{name}{suffix}")), tail)?;
    }
    Ok(())
}

fn relative_images(value: &mut Value, root: &Path) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key.ends_with("_image") {
                    if let Some(path) = value.as_str() {
                        if let Ok(relative) = Path::new(path).strip_prefix(root) {
                            *value = json!(relative.to_string_lossy().replace('\\', "/"));
                        }
                    }
                } else {
                    relative_images(value, root);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                relative_images(item, root);
            }
        }
        _ => {}
    }
}

fn clear_images(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| !key.ends_with("_image"));
            for value in map.values_mut() {
                clear_images(value);
            }
        }
        Value::Array(items) => {
            for value in items {
                clear_images(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permanently_unverified_cleanup_stops_after_three_attempts() {
        let mut attempts = 0;
        let mut waits = 0;
        let mut observations = Vec::new();
        let error = retry_owned_cleanup(
            || {
                attempts += 1;
                bail!("owner identity query denied")
            },
            |value| observations.push(value),
            || { waits += 1; Ok(()) },
        ).unwrap_err();
        assert_eq!(attempts, 3);
        assert_eq!(waits, 2);
        assert_eq!(observations.last().unwrap()["retry_exhausted"], true);
        assert!(format!("{error:#}").contains("stopping the run"));
        assert!(format!("{error:#}").contains("owner identity query denied"));
    }

    #[test]
    fn uncertain_cleanup_keeps_its_slot_until_identity_can_be_verified() -> Result<()> {
        let mut attempts = 0;
        let mut waits = 0;
        let mut observations = Vec::new();
        retry_owned_cleanup(
            || {
                attempts += 1;
                ensure!(attempts > 2, "Win32 5: cannot verify owner");
                Ok(())
            },
            |value| observations.push(value),
            || {
                waits += 1;
                Ok(())
            },
        )?;
        assert_eq!(attempts, 3);
        assert_eq!(waits, 2);
        assert_eq!(
            observations
                .iter()
                .map(|v| v["state"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["waiting", "waiting", "resolved"]
        );
        assert!(
            observations[0]["error"]
                .as_str()
                .unwrap()
                .contains("Win32 5")
        );
        assert_eq!(observations[2]["attempts"], 3);
        Ok(())
    }

    #[test]
    fn cancelled_cleanup_does_not_report_resolution() {
        let mut observations = Vec::new();
        let result = retry_owned_cleanup(
            || bail!("unverified owner"),
            |value| observations.push(value),
            || bail!("cancelled"),
        );
        assert!(result.is_err());
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0]["state"], "waiting");
    }

    #[test]
    fn resume_clears_stale_wait_when_the_owner_is_already_gone() -> Result<()> {
        let root = super::super::tests::test_directory("cleanup-resume")?;
        let mut record = json!({"cleanup_pending":{"state":"waiting","error":"prior denial"}});
        let mut observations = Vec::new();
        wait_for_owned_cleanup(&root, &mut record, None, &mut |value, _| {
            observations.push(value.clone())
        })?;
        assert_eq!(record["cleanup_pending"]["state"], "resolved");
        assert_eq!(observations.len(), 1);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn work_cleanup_retains_bundles_without_promoting_failed_preparation() -> Result<()> {
        let root = super::super::tests::test_directory("keep-preparation")?;
        let work = root.join("work");
        fs::create_dir_all(&work)?;
        fs::write(work.join("original.xlsx"), b"working copy")?;
        let bundle = root.join("prepared.zip");
        fs::write(&bundle, b"reusable preparation")?;
        let mut record = json!({"complete":true,"status":"unchanged","prepared":true});
        cleanup_work(&work, &bundle, &mut record)?;
        assert!(!work.exists());
        assert_eq!(fs::read(&bundle)?, b"reusable preparation");
        assert_eq!(record["prepared"], true);
        assert_eq!(record["complete"], true);
        // A failed new preparation must not claim the previous bundle as its own.
        fs::create_dir_all(&work)?;
        record["prepared"] = json!(false);
        record["status"] = json!("skipped");
        cleanup_work(&work, &bundle, &mut record)?;
        assert_eq!(record["prepared"], false);
        assert_eq!(fs::read(&bundle)?, b"reusable preparation");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn cancelled_attempt_remains_incomplete_and_keeps_its_prepared_bundle() -> Result<()> {
        let root = super::super::tests::test_directory("cancel-record")?;
        let bundle = root.join("prepared.zip");
        fs::write(&bundle, b"retained ZIP fixture")?;
        let mut record = json!({"complete":true,"status":"skipped","prepared":true,"last_execution":{},"results":[{"checks_complete":true}]});
        mark_cancelled(&mut record, Some(&bundle));
        assert_eq!(record["complete"], false);
        assert_eq!(record["status"], "interrupted");
        assert_eq!(record["prepared"], true);
        assert_eq!(record["last_execution"]["cancelled"], true);
        assert_eq!(record["results"][0]["checks_complete"], true);
        assert_eq!(fs::read(&bundle)?, b"retained ZIP fixture");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn worker_memory_measurement_survives_work_directory_cleanup() -> Result<()> {
        let work = super::super::tests::test_directory("worker-memory-record")?;
        let measured = json!({"version":1,"metric":"private committed","current_bytes":1024,
            "peak_bytes":2048,"limit_bytes":4096,"final_sample":true});
        fs::write(
            work.join("worker-memory.json"),
            serde_json::to_vec(&measured)?,
        )?;
        let mut record = json!({});
        retain_worker_memory(&mut record, &work)?;
        fs::remove_dir_all(&work)?;
        assert_eq!(record["worker_memory"], measured);
        Ok(())
    }

    #[test]
    fn classic_fallback_keeps_images_from_the_failed_resident_attempt() -> Result<()> {
        let root = super::super::tests::test_directory("fallback-evidence")?;
        let work = root.join("work");
        fs::create_dir(&work)?;
        let image = work.join("original.png");
        fs::write(&image, b"resident evidence")?;
        let mut original = json!({"capture_mode":"resident","results":[{"original_image":image}]});
        retain_visual_evidence(&mut original, &work, &root, "example", false);
        let first = PathBuf::from(original["results"][0]["original_image"].as_str().unwrap());
        fs::write(&image, b"classic evidence")?;
        let mut retry = json!({"capture_mode":"classic","results":[{"original_image":image}],
            "comparison_fallback":{"status":"running","run_id":"test-run"}});
        retain_visual_evidence(&mut retry, &work, &root, "example", false);
        let second = PathBuf::from(retry["results"][0]["original_image"].as_str().unwrap());
        assert_ne!(first, second);
        assert_eq!(fs::read(first)?, b"resident evidence");
        assert_eq!(fs::read(second)?, b"classic evidence");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn priority_secondary_preserves_primary_evidence_and_retains_new_profiles() -> Result<()> {
        let root = super::super::tests::test_directory("priority-secondary-evidence")?;
        let work = root.join("work");
        fs::create_dir(&work)?;
        let primary_image = work.join("aggressive.png");
        let primary_diagnosis = work.join("aggressive-diagnosis.png");
        fs::write(&primary_image, b"primary comparison")?;
        fs::write(&primary_diagnosis, b"primary diagnosis")?;
        let mut primary = json!({"status":"differences","comparison_pass":super::super::strategy::PRIMARY,
            "results":[{"profile":super::super::strategy::AGGRESSIVE,"checks_complete":true,
                "sheets":[{"original_image":primary_image}],
                "visual_diagnosis":{"status":"classified","category":"row_height",
                    "repair_image":primary_diagnosis}}]});
        retain_visual_evidence(&mut primary, &work, &root, "example", false);
        relative_images(&mut primary, &root);
        let mut reused = primary["results"][0].clone();
        reused["comparison_reused_from_pass"] = json!(super::super::strategy::PRIMARY);
        let image = work.join("default.png");
        let diagnosis = work.join("default-diagnosis.png");
        fs::write(&image, b"secondary comparison")?;
        fs::write(&diagnosis, b"secondary diagnosis")?;
        let mut record = json!({"status":"differences","complete":true,
            "comparison_pass":super::super::strategy::SECONDARY,
            "results":[{"profile":"default","checks_complete":true,"shrunk_image":image,
                "visual_diagnosis":{"status":"classified","repair_image":diagnosis}},reused]});
        retain_visual_evidence(&mut record, &work, &root, "example", false);
        relative_images(&mut record, &root);
        fs::remove_dir_all(&work)?;
        assert_eq!(record["status"], "differences");
        assert_eq!(record["complete"], true);
        assert_eq!(record["results"][1], reused);
        for (path, bytes) in [
            (&record["results"][1]["sheets"][0]["original_image"], b"primary comparison".as_slice()),
            (&record["results"][1]["visual_diagnosis"]["repair_image"], b"primary diagnosis".as_slice()),
            (&record["results"][0]["shrunk_image"], b"secondary comparison".as_slice()),
            (&record["results"][0]["visual_diagnosis"]["repair_image"], b"secondary diagnosis".as_slice()),
        ] {
            assert_eq!(fs::read(root.join(path.as_str().unwrap()))?, bytes);
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn priority_secondary_failure_keeps_primary_evidence_for_retry() -> Result<()> {
        let root = super::super::tests::test_directory("priority-secondary-evidence-failure")?;
        let work = root.join("work");
        fs::create_dir(&work)?;
        let reused = json!({"profile":super::super::strategy::AGGRESSIVE,"checks_complete":true,
            "comparison_reused_from_pass":super::super::strategy::PRIMARY,
            "original_image":"evidence/previous.png",
            "visual_diagnosis":{"status":"classified","repair_image":"evidence/repair.png"}});
        let mut record = json!({"status":"differences","comparison_pass":super::super::strategy::SECONDARY,
            "results":[reused,{"profile":"default","shrunk_image":work.join("missing.png")}]});
        retain_visual_evidence(&mut record, &work, &root, "example", false);
        assert_eq!(record["status"], "skipped");
        assert!(record["error"].as_str().unwrap().contains("preserve image evidence"));
        assert_eq!(record["results"][0], reused);
        assert!(record["results"][1].get("shrunk_image").is_none());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn priority_secondary_retains_fresh_aggressive_retry_evidence() -> Result<()> {
        let root = super::super::tests::test_directory("priority-secondary-aggressive-retry")?;
        let work = root.join("work");
        fs::create_dir(&work)?;
        let image = work.join("aggressive.png");
        fs::write(&image, b"fresh aggressive retry")?;
        let mut record = json!({"status":"differences","comparison_pass":super::super::strategy::SECONDARY,
            "priority_validation":{"primary":{"status":"failed"}},
            "results":[{"profile":super::super::strategy::AGGRESSIVE,"checks_complete":true,"shrunk_image":image}]});
        retain_visual_evidence(&mut record, &work, &root, "example", false);
        relative_images(&mut record, &root);
        fs::remove_dir_all(&work)?;
        assert_eq!(record["status"], "differences");
        let retained = root.join(record["results"][0]["shrunk_image"].as_str().unwrap());
        assert_eq!(fs::read(retained)?, b"fresh aggressive retry");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn failed_supplemental_evidence_preserves_completed_primary_results() -> Result<()> {
        let root = super::super::tests::test_directory("whitespace-evidence-failure")?;
        let primary = json!([{"sheets":[{"original_image":"evidence/old-original.png", "shrunk_image":"evidence/old-shrunk.png"}]}]);
        let mut record = json!({"complete":true,"status":"differences","results":primary,
            "whitespace_diagnosis":{"status":"complete","control":{"original_image":root.join("missing.png")}}});
        retain_visual_evidence(&mut record, &root, &root, "example", true);
        assert_eq!(record["results"], primary);
        assert_eq!(record["status"], "differences");
        assert_eq!(record["complete"], true);
        assert_eq!(record["whitespace_diagnosis"]["status"], "failed");
        assert!(
            record["whitespace_diagnosis"]["control"]
                .get("original_image")
                .is_none()
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn keeps_bounded_failure_logs_without_workbooks_or_images() -> Result<()> {
        let root = super::super::tests::test_directory("failure-logs")?;
        let work = root.join("work");
        let capture = work.join("comparisons/default/visual/images");
        fs::create_dir_all(&capture)?;
        let mut log = vec![b'a'; 2 * 1024 * 1024];
        log.extend_from_slice(b"last failure");
        fs::write(capture.join("host.stdout.log"), log)?;
        fs::write(capture.join("original.xlsx"), b"workbook")?;
        fs::write(capture.join("equal.png"), b"image")?;
        retain_capture_logs(&work, &root, "example")?;
        fs::remove_dir_all(&work)?;
        let kept = fs::read(root.join("logs/example-default-host.stdout.log"))?;
        assert_eq!(kept.len(), 1024 * 1024);
        assert!(kept.ends_with(b"last failure"));
        assert_eq!(fs::read_dir(root.join("logs"))?.count(), 1);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn resident_diagnostics_keep_bounded_timing_evidence_without_staged_data() -> Result<()> {
        let root = super::super::tests::test_directory("resident-diagnostics")?;
        let work = root.join("work");
        let session = work.join("resident");
        fs::create_dir_all(session.join("images"))?;
        fs::create_dir_all(session.join("clients"))?;
        let metrics = b"{\"status\":\"complete\",\"opening_seconds\":3.5}";
        fs::write(session.join("metrics.json"), metrics)?;
        let event = b"{\"event\":\"heartbeat\"}\n";
        fs::write(session.join("launcher-events.jsonl"), event.repeat(120_000))?;
        fs::write(session.join("host.stdout.log"), vec![b'h'; 3 * 1024 * 1024])?;
        fs::write(session.join("host.stderr.log"), b"capture failed")?;
        fs::write(session.join("launcher.stderr.log"), b"launcher failed")?;
        fs::write(session.join("original.xlsx"), b"staged workbook")?;
        fs::write(session.join("images/tile.png"), b"image")?;
        fs::write(session.join("clients/original.json"), b"private descriptor")?;

        let success = root.join("success");
        retain_resident_diagnostics(&work, &success, "example", false)?;
        let logs = success.join("logs");
        assert_eq!(
            fs::read(logs.join("example-resident-metrics.json"))?,
            metrics
        );
        assert_eq!(
            fs::metadata(logs.join("example-resident-host.stdout.log"))?.len(),
            256 * 1024
        );
        let events = fs::read_to_string(logs.join("example-resident-launcher-events.jsonl"))?;
        assert!(events.len() <= 2 * 1024 * 1024);
        assert!(!events.is_empty());
        for line in events.lines() {
            assert_eq!(serde_json::from_str::<Value>(line)?["event"], "heartbeat");
        }
        assert_eq!(fs::read_dir(&logs)?.count(), 3);

        let failure = root.join("failure");
        retain_resident_diagnostics(&work, &failure, "example", true)?;
        let logs = failure.join("logs");
        assert_eq!(
            fs::metadata(logs.join("example-resident-host.stdout.log"))?.len(),
            2 * 1024 * 1024
        );
        assert_eq!(
            fs::read(logs.join("example-resident-host.stderr.log"))?,
            b"capture failed"
        );
        assert_eq!(
            fs::read(logs.join("example-resident-launcher.stderr.log"))?,
            b"launcher failed"
        );
        assert_eq!(fs::read_dir(logs)?.count(), 5);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
