//! Admit groups directly from the user's live target, without predicting their RAM.
use super::super::control_room::{self, Controls};
use super::*;
#[path = "disk_space.rs"]
mod disk_space;
#[path = "manual_restarts.rs"]
mod restarts;
use std::collections::BTreeMap;

struct Active {
    job: Comparison,
    participant: batch_cleanup::Participant,
    finished: bool,
    error: Option<String>,
    pause_millis: u64,
    restart_requests: Vec<restarts::Request>,
}

/// Complete the live attempt, leaving earlier attempts of this workbook unchanged.
fn mark_finished(active: &mut [Active], position: usize, error: Option<String>) {
    if let Some(job) = active.iter_mut().find(|a| a.job.position == position && !a.finished) {
        job.finished = true;
        job.error = error;
    }
}

fn cancel_active(active: &[Active]) {
    for group in active {
        group.participant.cancel();
    }
}

fn excel_is_opening(launcher: &Value, now: u64) -> bool {
    launcher["sampled_unix_ms"]
        .as_u64()
        .is_some_and(|sample| now.saturating_sub(sample) < 5_000)
        && launcher["clients"]
            .as_object()
            .is_some_and(|clients| clients.values().any(|client| client["status"] == "opening"))
}

fn another_group_is_opening(opening: &[usize], index: usize) -> bool {
    opening.iter().any(|&peer| peer != index)
}

fn refresh_groups(queue: &mut Value, active: &[Active], records: &[Value], pending: usize) {
    for job in active {
        let id = control_room::group_id(&job.job.id);
        if let Some(group) = queue["groups"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|g| g["id"] == id)
        {
            group["active"] = json!(!job.finished);
            let record = &records[job.job.position];
            if let Some(attempt) = record["control_room_attempt"]["id"].as_str() {
                group["attempt_id"] = json!(attempt);
                group["started_unix_ms"] = record["control_room_attempt"]["started_unix_ms"].clone();
            }
            group["capture_mode"] = record["capture_mode"].clone();
            group["capture_deferred"] = record["capture_deferred"].clone();
            group["recalc_identity_changed"] = record["recalc_identity_changed"].clone();
            group["cleanup_pending"] = records[job.job.position]["cleanup_pending"].clone();
            if group["status"] == "cleanup_blocked" {
                group.as_object_mut().unwrap().remove("error");
            }
            group["status"] = if records[job.job.position]["cleanup_pending"]["state"] == "waiting"
            {
                group["error"] = records[job.job.position]["cleanup_pending"]["error"].clone();
                json!("cleanup_blocked")
            } else if let Some(error) = &job.error {
                group["error"] = json!(error);
                json!("failed")
            } else if job.participant.restart_requested() {
                json!(if job.finished {
                    "queued"
                } else {
                    "restart_waiting_cleanup"
                })
            } else if job.finished {
                records[job.job.position]["status"].clone()
            } else if job.participant.ready() {
                json!("waiting_cleanup")
            } else {
                json!("active")
            };
        }
    }
    queue["open_groups"] = json!(active.iter().filter(|a| !a.finished).count());
    queue["working_groups"] = json!(
        active
            .iter()
            .filter(|a| !a.finished && !a.participant.ready())
            .count()
    );
    queue["cleanup_groups"] = json!(
        active
            .iter()
            .filter(|a| !a.finished && a.participant.ready())
            .count()
    );
    queue["waiting_groups"] = json!(pending);
    queue["updated_unix_ms"] = json!(provenance::now_ms());
}

fn slots_to_start(target: usize, open: usize, pending: usize, draining: bool) -> usize {
    if draining {
        0
    } else {
        target.saturating_sub(open).min(pending)
    }
}

fn admission_target(target_groups: usize, aggressive_first: bool) -> usize {
    if target_groups == 0 {
        0
    } else {
        target_groups + usize::from(aggressive_first)
    }
}

fn update_phase(queue: &mut Value, target: usize) {
    queue["phase"] = json!(
        if target > 0 && queue["admission_guard"]["blocked"] == true {
            "waiting_disk"
        } else if target > 0 {
            "comparing"
        } else if queue["open_groups"].as_u64().unwrap_or(0) > 0 {
            "draining"
        } else {
            "paused"
        }
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compare(
    source: &Source,
    jobs: Vec<Comparison>,
    output: &Path,
    records: &mut Vec<Value>,
    corpora: &[Value],
    limits: &ArchiveArgs,
    run: &Value,
    log_level: LogLevel,
    mut initial_admission: Option<String>,
) -> Result<()> {
    let room = output.join("control-room");
    let producer = run["executable_sha256"]
        .as_str()
        .context("missing runner build identity")?;
    let compatible: Vec<String> =
        serde_json::from_value(run["options"]["reuse_prepared_from"].clone())?;
    let manifest = artifact_cache::sha256(&serde_json::to_vec_pretty(&run["selection"])?);
    let order = match run["options"]["order"].as_str() {
        Some("size-asc") => selection::ProcessingOrder::SizeAsc,
        Some("size-desc") => selection::ProcessingOrder::SizeDesc,
        _ => selection::ProcessingOrder::Priority,
    };
    let mut controls = Controls::read(&room)?;
    let mut queue =
        control_room::read_json(&room.join("queue.json")).unwrap_or_else(|| json!({"groups":[]}));
    let resuming_deferred = jobs.is_empty() && queue["phase"] == "awaiting_deferred_manual";
    queue["phase"] = json!("comparing");
    queue["running"] = json!(true);
    let groups = queue["groups"]
        .as_array_mut()
        .context("missing control room groups")?;
    let eligible = restarts::eligible(source, records);
    for group in groups.iter_mut() {
        let allowed = group["id"]
            .as_str()
            .is_some_and(|id| eligible.contains_key(id));
        if allowed || !resuming_deferred {
            group["restart_eligible"] = json!(allowed);
        }
        if allowed {
            group["run_id"] = run["run_id"].clone();
        }
    }
    for job in &jobs {
        let id = control_room::group_id(&job.id);
        let group = json!({"id":id,"workbook":records[job.position]["name"],"corpus":source.name,
            "status":"queued","active":false,"run_id":run["run_id"],"restart_eligible":true,"sequence":groups.len()+1});
        if let Some(previous) = groups.iter_mut().find(|g| g["id"] == id) {
            let sequence = previous["sequence"].clone();
            *previous = group;
            previous["sequence"] = sequence;
        } else {
            groups.push(group);
        }
    }
    control_room::write(&room.join("queue.json"), &queue)?;
    let mut pending: VecDeque<_> = jobs.into();
    let aggressive_first = run["options"]["aggressive_first"] == true;
    queue["prefetch_groups"] = json!(usize::from(aggressive_first));
    let mut wave = 0;
    let mut report_dirty = false;
    let mut last_report = Instant::now();
    let mut queued_restarts = BTreeMap::new();
    let mut disk_guard = disk_space::Guard::new();
    loop {
        restarts::poll(
            &room,
            &mut queue,
            &eligible,
            &[],
            &mut pending,
            records,
            None,
            &mut queued_restarts,
        )?;
        let Some(front) = pending.front().cloned() else {
            break;
        };
        if initial_admission.as_deref() != Some(front.id.as_str()) {
            control_room::wait_for_admission(output, "comparing")?;
        }
        if !disk_guard.allows_admission(&mut queue, provenance::now_ms()) {
            if let Ok(updated) = Controls::read(&room) {
                controls = updated;
            }
            update_phase(&mut queue, controls.target_groups);
            control_room::write(&room.join("queue.json"), &queue)?;
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        if !reusable_preparation(
            &records[front.position],
            &bundle_path(output, &front.id),
            producer,
            &source.fingerprint,
            &compatible,
            run["options"]["preparation_hash_profile"].as_str(),
        ) {
            // A completed historical comparison normally discarded its ZIP.
            // Rebuild it between waves; conversion must not disturb live peers.
            let provenance = source.selection.provenance(
                front.index,
                &source.id,
                &source.fingerprint,
                &manifest,
                order,
            );
            let storage = storage_guard::check(output, provenance["bytes"].as_u64());
            let storage_error = match &storage {
                Ok(space) if !space.blocked => None,
                Ok(space) => Some(format!(
                    "Preparation needs {} free bytes on the {}; {} bytes available.",
                    space.required_free_bytes, space.volume, space.free_bytes
                )),
                Err(error) => Some(format!(
                    "Cannot verify free disk space: {error:#}"
                )),
            };
            if let Some(error) = storage_error {
                queue["phase"] = json!("waiting_disk");
                queue["preparation_storage"] = match storage {
                    Ok(space) => serde_json::to_value(space)?,
                    Err(_) => Value::Null,
                };
                queue["preparation_storage_error"] = json!(error);
                control_room::write(&room.join("queue.json"), &queue)?;
                progress::tick(output);
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
            queue.as_object_mut().unwrap().remove("preparation_storage");
            queue.as_object_mut().unwrap().remove("preparation_storage_error");
            control_room::archive_record(&room, &front.id, &records[front.position])?;
            invalidate_preparation_claim(&mut records[front.position]);
            records[front.position]["prepared_by"] = json!(producer);
            records[front.position]["source_fingerprint"] = json!(source.fingerprint);
            records[front.position]["run_selection"] = provenance;
            let id = control_room::group_id(&front.id);
            if let Some(group) = queue["groups"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|g| g["id"] == id)
            {
                group["status"] = json!("preparing");
            }
            control_room::write(&room.join("queue.json"), &queue)?;
            execution::process(
                source,
                front.index,
                &front.id,
                front.position,
                JobMode::Prepare,
                output,
                records,
                corpora,
                limits,
                run,
                log_level,
            )?;
            if classify_preparation(&mut records[front.position]) {
                write_record(&record_path(output, &front.id), &records[front.position])?;
                progress::observe(output, &records[front.position]);
            }
            if records[front.position]["preparation_disposition"] != "needs_visual"
                || !bundle_path(output, &front.id).is_file()
            {
                pending.pop_front();
                restarts::finish(
                    &mut queued_restarts.remove(&front.id).unwrap_or_default(),
                    &records[front.position],
                )?;
                if let Some(group) = queue["groups"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|g| g["id"] == id)
                {
                    group["status"] = records[front.position]["status"].clone();
                }
                control_room::write(&room.join("queue.json"), &queue)?;
                continue;
            }
            // Preparation was already admitted. A subsequent target of zero
            // drains this same workbook through its comparison before pausing.
            initial_admission = Some(front.id.clone());
        }
        wave += 1;
        let mut cleanup = batch_cleanup::Batch::growing(output, limits.workbook_timeout_seconds)?;
        let mut active: Vec<Active> = Vec::new();
        let mut errors = Vec::new();
        std::thread::scope(|scope| {
            let (sender, receiver) = mpsc::channel();
            let mut handles = Vec::new();
            let mut last_snapshot = Instant::now() - Duration::from_secs(2);
            let mut last_pause_sample = Instant::now();
            loop {
                // A transient sharing violation must not reset a user-selected target.
                if let Ok(updated) = Controls::read(&room) {
                    controls = updated;
                }
                // Immediate stop closes admission and cancels active workers.
                // Orderly stop still finishes admitted groups and cleanup.
                if control_room::terminate_requested(&room) {
                    controls.target_groups = 0;
                    controls.paused = false;
                    cleanup.close_admission();
                    cancel_active(&active);
                } else if control_room::stop_requested(&room) {
                    controls.target_groups = 0;
                    controls.paused = false;
                    if initial_admission.is_none() {
                        cleanup.close_admission();
                    }
                }
                if controls.target_groups == 0 && !active.is_empty() {
                    cleanup.close_admission();
                }
                if last_snapshot.elapsed() >= Duration::from_secs(1) {
                    if let Err(error) = restarts::poll(
                        &room,
                        &mut queue,
                        &eligible,
                        &active,
                        &mut pending,
                        records,
                        Some(&mut cleanup),
                        &mut queued_restarts,
                    ) {
                        errors.push(format!("consume restart request: {error:#}"));
                        cleanup.close_admission();
                        cancel_active(&active);
                    }
                }
                // End an idle wave; completed groups otherwise release their own
                // cleanup gate and free admission slots independently.
                if !active.is_empty() && active.iter().all(|a| a.finished || a.participant.ready())
                {
                    cleanup.close_admission();
                }
                let count = slots_to_start(
                    if active.is_empty()
                        && pending.front().is_some_and(|job| {
                            initial_admission.as_deref() == Some(job.id.as_str())
                        })
                    {
                        1
                    } else {
                        admission_target(controls.target_groups, aggressive_first)
                    },
                    active.iter().filter(|group| !group.finished).count(),
                    pending.len(),
                    !cleanup.admission_open()
                        || !disk_guard.allows_admission(&mut queue, provenance::now_ms()),
                );
                for _ in 0..count {
                    let job = pending.pop_front().unwrap();
                    initial_admission = None;
                    if !reusable_preparation(
                        &records[job.position],
                        &bundle_path(output, &job.id),
                        producer,
                        &source.fingerprint,
                        &compatible,
                        run["options"]["preparation_hash_profile"].as_str(),
                    ) {
                        pending.push_front(job);
                        cleanup.close_admission();
                        break;
                    }
                    let participant = match cleanup.add_manual_participant() {
                        Ok(value) => value,
                        Err(error) => {
                            errors.push(format!("register manual comparison group: {error:#}"));
                            pending.push_front(job);
                            cleanup.close_admission();
                            cancel_active(&active);
                            break;
                        }
                    };
                    let mut record = records[job.position].clone();
                    let mut restart_requests = queued_restarts.remove(&job.id).unwrap_or_default();
                    if !restart_requests.is_empty() {
                        record["full_scan_deferred"] = json!(restarts::full_scan_requested(&restart_requests));
                    }
                    let attempt =
                        match restarts::begin_attempt(&room, &job, &mut record, &controls, run) {
                            Ok(attempt) => attempt,
                            Err(error) => {
                                errors.push(format!("start fresh comparison attempt: {error:#}"));
                                let _ = participant.mark_ready();
                                cleanup.close_admission();
                                cancel_active(&active);
                                pending.push_front(job);
                                break;
                            }
                        };
                    record["compared_by"] = json!(producer);
                    record["run_selection"] = source.selection.provenance(
                        job.index,
                        &source.id,
                        &source.fingerprint,
                        &manifest,
                        order,
                    );
                    record["comparison_batch"] = json!({"run_id":run["run_id"],"number":wave,
                        "scheduling":"manual","requested_workers":controls.target_groups});
                    let id = control_room::group_id(&job.id);
                    if let Some(group) = queue["groups"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|g| g["id"] == id)
                    {
                        group["status"] = json!("opening");
                        group["active"] = json!(true);
                        group["started_unix_ms"] = json!(provenance::now_ms());
                        group["attempt_id"] = json!(attempt);
                    }
                    for request in &mut restart_requests {
                        request.value["replacement_attempt_id"] = json!(attempt);
                        if let Err(error) = request.update(
                            "restarting",
                            "Replacement comparison admitted with current capture settings.",
                        ) {
                            errors.push(format!("publish restart admission: {error:#}"));
                        }
                    }
                    let thread_job = job.clone();
                    let thread_participant = participant.clone();
                    let sender = sender.clone();
                    let spawned = std::thread::Builder::new()
                        .name(format!("compare-{id}"))
                        .spawn_scoped(scope, move || {
                            let _ready_on_exit = ReadyOnExit(Some(thread_participant.clone()));
                            let position = thread_job.position;
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    execution::run_record(
                                        source,
                                        thread_job.index,
                                        &thread_job.id,
                                        JobMode::Compare,
                                        output,
                                        record,
                                        limits,
                                        run,
                                        log_level,
                                        Some(thread_participant),
                                        |value, _| {
                                            let _ = sender
                                                .send(Event::Checkpoint(position, value.clone()));
                                        },
                                    )
                                }))
                                .unwrap_or_else(|_| {
                                    Err(anyhow::anyhow!("manual comparison coordinator panicked"))
                                });
                            let _ = sender.send(Event::Finished(position, result));
                        });
                    match spawned {
                        Ok(handle) => {
                            handles.push(handle);
                            active.push(Active {
                                job,
                                participant,
                                finished: false,
                                error: None,
                                pause_millis: 0,
                                restart_requests,
                            });
                        }
                        Err(error) => {
                            errors.push(format!("start manual comparison coordinator: {error}"));
                            let _ = participant.mark_ready();
                            participant.cancel();
                            pending.push_front(job);
                            cleanup.close_admission();
                            cancel_active(&active);
                            break;
                        }
                    }
                }
                if !errors.is_empty() {
                    cleanup.close_admission();
                    cancel_active(&active);
                }
                if errors.is_empty() {
                    if let Err(error) = cleanup.release_if_ready() {
                        errors.push(format!("release manual group cleanup: {error:#}"));
                        cleanup.close_admission();
                        cancel_active(&active);
                    }
                }
                if last_snapshot.elapsed() >= Duration::from_secs(1) {
                    // Excel opening can stall Office.js in other instances. Credit
                    // only fresh evidence of an actual open, not a queued request.
                    let elapsed = last_pause_sample
                        .elapsed()
                        .as_millis()
                        .min(u64::MAX as u128) as u64;
                    last_pause_sample = Instant::now();
                    let now = provenance::now_ms();
                    let opening: Vec<_> = active
                        .iter()
                        .enumerate()
                        .filter_map(|(index, a)| {
                            (!a.finished
                                && control_room::read_json(
                                    &room
                                        .join("groups")
                                        .join(control_room::group_id(&a.job.id))
                                        .join("launcher.json"),
                                )
                                .is_some_and(|sample| excel_is_opening(&sample, now)))
                            .then_some(index)
                        })
                        .collect();
                    if !opening.is_empty() {
                        for (_, job) in active.iter_mut().enumerate().filter(|(index, a)| {
                            !a.finished && another_group_is_opening(&opening, *index)
                        }) {
                            job.pause_millis = job.pause_millis.saturating_add(elapsed);
                            if let Some(path) = &job.participant.pause_path {
                                if let Err(error) = artifact_cache::atomic_write(
                                    path,
                                    &serde_json::to_vec(
                                        &json!({"elapsed_millis":job.pause_millis}),
                                    )
                                    .unwrap(),
                                ) {
                                    errors.push(format!("publish opening pause: {error:#}"));
                                    cleanup.close_admission();
                                    break;
                                }
                            }
                        }
                    }
                    if !errors.is_empty() {
                        cancel_active(&active);
                    }
                    refresh_groups(&mut queue, &active, records, pending.len());
                    update_phase(&mut queue, controls.target_groups);
                    progress::tick(output);
                    if let Err(error) = control_room::write(&room.join("queue.json"), &queue) {
                        eprintln!("[study] Could not refresh live control state: {error:#}");
                    }
                    last_snapshot = Instant::now();
                }
                if !cleanup.admission_open() && active.iter().all(|a| a.finished) {
                    refresh_groups(&mut queue, &active, records, pending.len());
                    if let Err(error) = control_room::write(&room.join("queue.json"), &queue) {
                        errors.push(format!("publish completed wave: {error:#}"));
                    }
                    break;
                }
                if active.is_empty() && !errors.is_empty() {
                    break;
                }
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(Event::Checkpoint(position, record)) => {
                        records[position] = record;
                        report_dirty = true;
                    }
                    Ok(Event::Finished(position, result)) => {
                        let message = result.as_ref().err()
                            .map(|error| format!("comparison record {position}: {error:#}"));
                        mark_finished(&mut active, position, message.clone());
                        match result {
                            Ok(record) => {
                                records[position] = record;
                                report_dirty = true;
                            }
                            Err(_) => {
                                errors.push(message.expect("failed completion has an error"));
                                cleanup.close_admission();
                                cancel_active(&active);
                            }
                        }
                        last_snapshot = Instant::now() - Duration::from_secs(2);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // Durable workbook records are already saved. Expensive
                        // full JSON/HTML publication is derived work: batch it,
                        // and process completions/admit replacements first.
                        if report_dirty && last_report.elapsed() >= Duration::from_secs(30) {
                            publish_progress(output, records, corpora, "comparing");
                            report_dirty = false;
                            last_report = Instant::now();
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        errors.push("manual comparison workers disconnected".into());
                        cancel_active(&active);
                        break;
                    }
                }
            }
            for handle in handles {
                if handle.join().is_err() {
                    errors.push("manual comparison coordinator panicked".into());
                }
            }
        });
        control_room::check_stop(&room)?;
        ensure!(
            errors.is_empty(),
            "manual comparison failed: {}",
            errors.join("; ")
        );
        for job in &active {
            if job.participant.restart_requested() {
                continue;
            }
            if fallback::recoverable_failure(&records[job.job.position]).is_none()
                || !bundle_path(output, &job.job.id).is_file()
            {
                continue;
            }
            let id = control_room::group_id(&job.job.id);
            // A serial fallback is a new admission too. Do not label it active
            // while target zero or a requested stop prevents it from starting.
            control_room::wait_for_admission(output, "comparing")?;
            while !disk_guard.allows_admission(&mut queue, provenance::now_ms()) {
                control_room::check_stop(&room)?;
                if let Ok(updated) = Controls::read(&room) {
                    controls = updated;
                }
                refresh_groups(&mut queue, &active, records, pending.len());
                update_phase(&mut queue, controls.target_groups);
                control_room::write(&room.join("queue.json"), &queue)?;
                std::thread::sleep(Duration::from_secs(1));
            }
            if let Ok(updated) = Controls::read(&room) {
                controls = updated;
            }
            let result = fallback::after_batch(
                source,
                std::slice::from_ref(&job.job),
                output,
                records,
                corpora,
                limits,
                run,
                log_level,
            );
            refresh_groups(&mut queue, &active, records, pending.len());
            if let Ok(updated) = Controls::read(&room) {
                controls = updated;
            }
            update_phase(&mut queue, controls.target_groups);
            if let Err(error) = &result {
                let group = queue["groups"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|g| g["id"] == id)
                    .unwrap();
                group["status"] = json!("failed");
                group["error"] = json!(format!("{error:#}"));
            }
            control_room::write(&room.join("queue.json"), &queue)?;
            control_room::check_stop(&room)?;
            result?;
        }
        for job in &mut active {
            restarts::finish(&mut job.restart_requests, &records[job.job.position])?;
        }
    }
    Ok(())
}

/// Keep the control room available for explicit full-scan requests after the
/// normal queue has drained. No workbook or Excel process remains open here.
#[allow(clippy::too_many_arguments)]
pub(super) fn await_deferred_manual(
    sources: &[Source],
    output: &Path,
    records: &mut Vec<Value>,
    corpora: &[Value],
    limits: &ArchiveArgs,
    run: &Value,
    log_level: LogLevel,
) -> Result<()> {
    let room = output.join("control-room");
    let source_by_group: BTreeMap<String, usize> = sources.iter().enumerate()
        .flat_map(|(source_index, source)| restarts::eligible(source, records)
            .into_keys().map(move |group| (group, source_index)))
        .collect();
    fs::create_dir_all(room.join("commands"))?;
    let mut idle_published = false;

    loop {
        control_room::check_stop(&room)?;
        let deferred: BTreeMap<String, usize> = records.iter().enumerate()
            .filter(|(_, record)| record["status"] == "paused-for-later"
                && record["complete"] == true && has_deferred_full_scan(record))
            .filter_map(|(position, record)| {
                let group = control_room::group_id(record["identity"].as_str()?);
                source_by_group.contains_key(&group).then_some((group, position))
            })
            .collect();
        if deferred.is_empty() {
            return Ok(());
        }

        if !idle_published {
            let mut queue = control_room::read_json(&room.join("queue.json"))
                .context("missing control room queue while awaiting deferred full scans")?;
            queue["phase"] = json!("awaiting_deferred_manual");
            queue["running"] = json!(true);
            queue["updated_unix_ms"] = json!(provenance::now_ms());
            if let Some(groups) = queue["groups"].as_array_mut() {
                for (id, &position) in &deferred {
                    if groups.iter().any(|group| group["id"] == *id) {
                        continue;
                    }
                    let record = &records[position];
                    groups.push(json!({"id":id,"workbook":record["name"],
                        "corpus":record["corpus_name"],"attempt_id":record["control_room_attempt"]["id"],
                        "sequence":groups.len()+1,"active":false}));
                }
                for group in groups {
                    let Some(id) = group["id"].as_str() else { continue };
                    if let Some(&position) = deferred.get(id) {
                        let record = &records[position];
                        group["run_id"] = run["run_id"].clone();
                        group["restart_eligible"] = json!(true);
                        group["capture_deferred"] = record["capture_deferred"].clone();
                        group["status"] = record["status"].clone();
                        group["active"] = json!(false);
                    } else {
                        group["restart_eligible"] = json!(false);
                    }
                }
            }
            control_room::write(&room.join("queue.json"), &queue)?;
            progress::await_deferred(output);
            idle_published = true;
        }

        let mut requested_source = None;
        for entry in fs::read_dir(room.join("commands"))? {
            let path = entry?.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Some(command) = control_room::read_json(&path) else { continue };
            if command["status"] != "pending"
                || command["full_scan_deferred"] != true
                || command["run_id"] != run["run_id"]
            {
                continue;
            }
            let Some(group) = command["group_id"].as_str() else { continue };
            if deferred.contains_key(group) {
                requested_source = source_by_group.get(group).copied();
                break;
            }
        }
        if let Some(source_index) = requested_source {
            compare(&sources[source_index], Vec::new(), output, records, corpora,
                limits, run, log_level, None)?;
            publish_progress(output, records, corpora, "awaiting deferred full scan");
            idle_published = false;
            continue;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarted_attempt_completes_without_mutating_its_finished_predecessor() {
        let participant: batch_cleanup::Participant = serde_json::from_value(json!({
            "ready_path":"unused-ready", "release_path":"unused-release",
            "owner_pid":0, "timeout_seconds":1
        })).unwrap();
        let attempt = |finished| Active {
            job: Comparison { index: 0, id: "same-workbook".into(), position: 7 },
            participant: participant.clone(),
            finished,
            error: None,
            pause_millis: 0,
            restart_requests: Vec::new(),
        };
        for error in [None, Some("replacement failed".to_owned())] {
            let mut active = vec![attempt(true), attempt(false)];
            mark_finished(&mut active, 7, error.clone());
            assert!(active.iter().all(|job| job.finished));
            assert_eq!(active[0].error, None);
            assert_eq!(active[1].error, error);
        }
    }

    #[test]
    fn target_changes_act_without_waiting_for_ready_or_estimating_memory() {
        assert_eq!(slots_to_start(24, 1, 100, false), 23);
        assert_eq!(slots_to_start(2, 8, 100, false), 0);
        assert_eq!(slots_to_start(128, 8, 1000, false), 120);
        assert_eq!(slots_to_start(8, 2, 3, false), 3);
        assert_eq!(slots_to_start(24, 2, 100, true), 0);
        assert_eq!(slots_to_start(0, 0, 100, false), 0);
        assert_eq!(slots_to_start(0, 20, 100, false), 0);
    }

    #[test]
    fn aggressive_screening_admits_one_preparing_group_ahead() {
        assert_eq!(admission_target(2, true), 3);
        assert_eq!(slots_to_start(admission_target(2, true), 2, 100, false), 1);
        assert_eq!(admission_target(2, false), 2);
        assert_eq!(admission_target(0, true), 0);
    }

    #[test]
    fn disk_guard_does_not_overwrite_target_or_cancel_active_work() {
        let mut queue = json!({"open_groups":4,"admission_guard":{"blocked":true}});
        update_phase(&mut queue, 24);
        assert_eq!(queue["phase"], "waiting_disk");
        assert_eq!(queue["open_groups"], 4);
        assert_eq!(slots_to_start(24, 4, 100, true), 0);
        queue["admission_guard"]["blocked"] = json!(false);
        update_phase(&mut queue, 24);
        assert_eq!(queue["phase"], "comparing");
        assert_eq!(slots_to_start(24, 4, 100, false), 20);
        queue["admission_guard"]["blocked"] = json!(true);
        update_phase(&mut queue, 0);
        assert_eq!(queue["phase"], "draining");
    }

    #[test]
    fn only_actual_recent_excel_opens_pause_other_job_deadlines() {
        let mut sample =
            json!({"sampled_unix_ms":10_000,"clients":{"original":{"status":"waiting_open"}}});
        assert!(!excel_is_opening(&sample, 11_000));
        sample["clients"]["original"]["status"] = json!("opening");
        assert!(excel_is_opening(&sample, 11_000));
        assert!(!excel_is_opening(&sample, 16_000));
        sample["clients"]["original"]["status"] = json!("ready");
        assert!(!excel_is_opening(&sample, 11_000));
    }

    #[test]
    fn a_group_cannot_extend_its_own_opening_deadline() {
        assert!(!another_group_is_opening(&[0], 0));
        assert!(another_group_is_opening(&[0], 1));
        assert!(!another_group_is_opening(&[], 0));
        assert!(another_group_is_opening(&[1, 2], 0));
    }
}
