//! Prepare independent workbooks concurrently; one coordinator owns shared state.
use super::*;
use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc;

struct Pending<'a> {
    source: &'a Source,
    index: usize,
    id: String,
    name: String,
    position: usize,
    provenance: Value,
    input_bytes: Option<u64>,
}

enum Event {
    Checkpoint(usize, Value),
    Finished(usize, Result<Value>),
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_all(
    sources: &[Source],
    selected_by_source: &[Vec<(usize, String)>],
    args: &StudyArgs,
    manifest_hash: &str,
    producer: &str,
    output: &Path,
    records: &mut Vec<Value>,
    corpora: &[Value],
    limits: &ArchiveArgs,
    run: &Value,
    log_level: LogLevel,
    limited: bool,
) -> Result<()> {
    let room = output.join("control-room");

    // Record positions and compatibility decisions must be stable before any
    // worker starts. A later resume then sees every accepted build claim.
    for (source, selected) in sources.iter().zip(selected_by_source) {
        for (index, name) in selected {
            let id = identity(source, *index, name);
            let position = ensure_record(records, source, *index, name);
            if !args.control_room
                && records[position]["complete"] == true
                && records[position]["status"] == "skipped"
            {
                control_room::archive_record(&room, &id, &records[position])?;
            }
            if !args.reuse_prepared_from.is_empty()
                && records[position]["prepared_by"] != producer
                && records[position]["preparation_accepted_by"] != producer
                && reusable_preparation(
                    &records[position],
                    &bundle_path(output, &id),
                    producer,
                    &source.fingerprint,
                    &args.reuse_prepared_from,
                    args.preparation_hash_profile.as_deref(),
                )
            {
                records[position]["preparation_accepted_by"] = json!(producer);
                write_record(&record_path(output, &id), &records[position])?;
            }
        }
    }

    let mut pending = VecDeque::new();
    for (source, selected) in sources.iter().zip(selected_by_source) {
        let mut reused = 0;
        for (index, name) in selected {
            let id = identity(source, *index, name);
            let position = ensure_record(records, source, *index, name);
            if reusable_preparation(
                &records[position],
                &bundle_path(output, &id),
                producer,
                &source.fingerprint,
                &args.reuse_prepared_from,
                args.preparation_hash_profile.as_deref(),
            ) {
                reused += 1;
                if classify_preparation(&mut records[position]) {
                    write_record(&record_path(output, &id), &records[position])?;
                }
                progress::observe(output, &records[position]);
                continue;
            }
            let provenance = source.selection.provenance(
                *index,
                &source.id,
                &source.fingerprint,
                manifest_hash,
                args.order,
            );
            pending.push_back(Pending {
                source,
                index: *index,
                id,
                name: name.clone(),
                position,
                input_bytes: provenance["bytes"].as_u64(),
                provenance,
            });
        }
        if reused > 0 {
            eprintln!(
                "[study] {}: reused {reused} prepared workbooks",
                source.name
            );
        }
    }
    if pending.is_empty() {
        if args.control_room {
            control_room::check_stop(&room)?;
        }
        return Ok(());
    }

    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        let mut active = BTreeMap::<usize, u64>::new();
        let mut handles = BTreeMap::new();
        let mut halt: Option<anyhow::Error> = None;
        let mut disk_wait: Option<(usize, String, Option<storage_guard::GuardState>)> = None;

        while !pending.is_empty() || !active.is_empty() {
            while halt.is_none() && disk_wait.is_none() && !pending.is_empty() {
                let target = if args.control_room {
                    match control_room::Controls::read(&room) {
                        Ok(controls) => controls.prepare_processes,
                        Err(error) => {
                            halt = Some(error.context("read preparation controls"));
                            break;
                        }
                    }
                } else {
                    1
                };
                if active.len() >= target {
                    break;
                }
                if args.control_room {
                    if let Err(error) = control_room::check_stop(&room) {
                        halt = Some(error);
                        break;
                    }
                }
                let next = pending.front().expect("pending job exists");
                let active_bytes = active.values().copied().fold(0u64, u64::saturating_add);
                let storage =
                    storage_guard::check_with_active(output, next.input_bytes, active_bytes);
                let reason = match &storage {
                    Ok(space) if !space.blocked => None,
                    Ok(space) => Some(format!(
                        "Preparation paused before {}: {} bytes free on the study output volume; {} bytes required. Free space and rerun to resume.",
                        next.name, space.free_bytes, space.required_free_bytes
                    )),
                    Err(error) => Some(format!(
                        "Preparation paused before {}: free space on the study output volume could not be verified: {error:#}. Rerun after checking the volume.",
                        next.name
                    )),
                };
                if let Some(reason) = reason {
                    disk_wait = Some((next.position, reason, storage.ok()));
                    break;
                }

                let next = pending.pop_front().expect("pending job exists");
                let position = next.position;
                invalidate_preparation_claim(&mut records[position]);
                records[position]["run_selection"] = next.provenance;
                records[position]
                    .as_object_mut()
                    .unwrap()
                    .remove("preparation_waiting_reason");
                records[position]
                    .as_object_mut()
                    .unwrap()
                    .remove("preparation_storage");
                records[position]["prepared_by"] = json!(producer);
                records[position]["source_fingerprint"] = json!(next.source.fingerprint);
                if args.control_room {
                    if let Err(error) = control_room::begin_preparation(
                        output,
                        next.source,
                        &next.id,
                        &mut records[position],
                        run,
                        active.len() + 1,
                    ) {
                        halt = Some(error.context("admit preparation in control room"));
                        break;
                    }
                }
                let initial = records[position].clone();
                let sender_for_job = sender.clone();
                active.insert(position, next.input_bytes.unwrap_or(0));
                handles.insert(
                    position,
                    scope.spawn(move || {
                        let result = execution::run_record(
                            next.source,
                            next.index,
                            &next.id,
                            JobMode::Prepare,
                            output,
                            initial,
                            limits,
                            run,
                            log_level,
                            None,
                            |record, _phase| {
                                let _ = sender_for_job
                                    .send(Event::Checkpoint(position, record.clone()));
                            },
                        );
                        let _ = sender_for_job.send(Event::Finished(position, result));
                    }),
                );
            }

            if active.is_empty() {
                break;
            }
            match receiver.recv_timeout(Duration::from_millis(250)) {
                Ok(Event::Checkpoint(position, record)) => {
                    records[position] = record;
                    if report_refresh_due(output) {
                        publish_progress(output, records, corpora, "shrinking");
                    }
                }
                Ok(Event::Finished(position, result)) => {
                    active.remove(&position);
                    // Cleanup may have released enough disk to admit the next
                    // workbook while other preparations are still running.
                    disk_wait = None;
                    if let Some(handle) = handles.remove(&position) {
                        if handle.join().is_err() && halt.is_none() {
                            halt = Some(anyhow::anyhow!("preparation coordinator panicked"));
                        }
                    }
                    match result {
                        Ok(record) => {
                            records[position] = record;
                            let id = records[position]["identity"].as_str().unwrap().to_owned();
                            if classify_preparation(&mut records[position]) {
                                if let Err(error) =
                                    write_record(&record_path(output, &id), &records[position])
                                {
                                    halt.get_or_insert(
                                        error.context("save preparation disposition"),
                                    );
                                }
                            }
                            progress::observe(output, &records[position]);
                        }
                        Err(error) => {
                            let id = records[position]["identity"].as_str().unwrap();
                            if let Ok(bytes) = fs::read(record_path(output, id)) {
                                if let Ok(record) = serde_json::from_slice(&bytes) {
                                    records[position] = record;
                                }
                            }
                            if halt.is_none() {
                                halt = Some(error.context("prepare workbook"));
                            }
                        }
                    }
                    if args.control_room {
                        let id = records[position]["identity"].as_str().unwrap();
                        if let Err(error) = control_room::finish_preparation(
                            output,
                            id,
                            &records[position],
                            active.len(),
                        ) {
                            halt.get_or_insert(error.context("finish preparation in control room"));
                        }
                    }
                    if report_refresh_due(output) {
                        publish_progress(output, records, corpora, "shrinking");
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // A worker panic sends no Finished event. Detect it without
                    // waiting indefinitely for a channel sender that vanished.
                    let finished_without_event: Vec<_> = handles
                        .iter()
                        .filter(|(position, handle)| {
                            active.contains_key(position) && handle.is_finished()
                        })
                        .map(|(position, _)| *position)
                        .collect();
                    for position in finished_without_event {
                        let handle = handles.remove(&position).unwrap();
                        if handle.join().is_err() {
                            active.remove(&position);
                            disk_wait = None;
                            halt.get_or_insert_with(|| {
                                anyhow::anyhow!(
                                    "preparation coordinator panicked for record {position}"
                                )
                            });
                        }
                    }
                    progress::tick(output);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    halt.get_or_insert_with(|| {
                        anyhow::anyhow!("preparation workers ended without final records")
                    });
                    break;
                }
            }
        }
        drop(sender);
        for (position, handle) in handles {
            if handle.join().is_err() && halt.is_none() {
                halt = Some(anyhow::anyhow!(
                    "preparation coordinator panicked for record {position}"
                ));
            }
        }
        if args.control_room && (halt.is_none() || control_room::terminate_requested(&room)) {
            control_room::check_stop(&room)?;
        }
        if let Some((position, reason, storage)) = disk_wait {
            let id = records[position]["identity"].as_str().unwrap().to_owned();
            records[position]["preparation_waiting_reason"] = json!(reason);
            records[position]["preparation_storage"] = match storage {
                Some(space) => serde_json::to_value(space)?,
                None => Value::Null,
            };
            write_record(&record_path(output, &id), &records[position])?;
            progress::observe(output, &records[position]);
            progress::wait_for_disk(output);
            if let Some(mut queue) = control_room::read_json(&room.join("queue.json")) {
                queue["phase"] = json!("waiting_disk");
                queue["preparation_storage_error"] = json!(reason);
                queue["preparation_storage"] = records[position]["preparation_storage"].clone();
                control_room::write(&room.join("queue.json"), &queue)?;
            }
            publish(output, records, corpora, false, limited, "waiting for disk")?;
            anyhow::bail!(reason);
        }
        if let Some(error) = halt {
            publish_progress(output, records, corpora, "shrinking");
            return Err(error);
        }
        publish_progress(output, records, corpora, "prepared");
        Ok(())
    })
}
