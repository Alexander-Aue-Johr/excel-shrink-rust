//! Grow a comparison wave after each new group has opened its primary clients.
//! Cleanup begins only once admission closes; a draining wave never refills.
use super::*;

const FRESH_MILLIS: u64 = 5_000;

struct Active {
    job: Comparison,
    participant: batch_cleanup::Participant,
    plan: adaptive::Plan,
    started_ms: u64,
    started: Instant,
    pause_millis: u64,
    finished: bool,
    primary_ready: bool,
    normalized_peak: Option<u64>,
}

#[derive(Clone, Copy, Default, Debug)]
struct Sample {
    ready: bool,
    failed: bool,
    normalized_peak: Option<u64>,
    resident_current: u64,
    resident_private_working_set: u64,
    worker_current: u64,
}

fn fresh(value: &Value, started_ms: u64, now_ms: u64) -> bool {
    value["sampled_unix_ms"].as_u64().is_some_and(|sampled| {
        sampled >= started_ms && sampled <= now_ms && now_ms - sampled <= FRESH_MILLIS
    })
}

fn sample_values(resident: &Value, worker: &Value, started_ms: u64, now_ms: u64) -> Sample {
    let mut sample = Sample {
        failed: matches!(resident["status"].as_str(), Some("failed" | "cancelled")),
        ..Default::default()
    };
    if fresh(worker, started_ms, now_ms) && worker["final_sample"] != true {
        sample.worker_current = worker["current_bytes"].as_u64().unwrap_or(0);
    }
    if !fresh(resident, started_ms, now_ms) || resident["status"] != "running" {
        return sample;
    }
    sample.resident_current = resident["current_job_bytes"].as_u64().unwrap_or(0);
    // Optional on hosts that can provide private resident pages. Shared working
    // sets and private commit are not substitutes for this physical measurement.
    sample.resident_private_working_set = resident["current_job_private_working_set_bytes"]
        .as_u64()
        .unwrap_or(0)
        .min(sample.resident_current);
    let expected = resident["expected_primary_clients"].as_u64().unwrap_or(0);
    let connected = resident["clients"].as_object().is_some_and(|clients| {
        clients.len() >= expected as usize
            && clients.values().all(|client| {
                client["startup_seconds"]
                    .as_f64()
                    .is_some_and(|seconds| seconds.is_finite() && seconds >= 0.0)
                    && client["closed"] != true
            })
    });
    let peak = resident["peak_job_bytes"].as_u64().unwrap_or(0);
    sample.ready = resident["primary_clients_ready"] == true
        && (1..=4).contains(&expected)
        && connected
        && sample.resident_current > 0
        && sample.worker_current > 0
        && peak >= sample.resident_current;
    if sample.ready {
        sample.normalized_peak = adaptive::normalized_peak(peak, expected as usize);
    }
    sample
}

fn read_sample(output: &Path, active: &Active, now_ms: u64) -> Sample {
    let key = artifact_cache::sha256(active.job.id.as_bytes());
    let work = output.join(".work").join(&key[..20]);
    let read = |path| {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or(Value::Null)
    };
    sample_values(
        &read(work.join("resident/metrics.json")),
        &read(work.join("worker-memory.json")),
        active.started_ms,
        now_ms,
    )
}

fn reserved_memory(active: &[Active], samples: &[Sample], worker_limit: u64) -> (u64, u64) {
    active
        .iter()
        .zip(samples)
        .filter(|(job, _)| !job.finished)
        .fold((0u64, 0u64), |(physical, commit), (job, sample)| {
            // The resident cap also covers later whitespace clients. COM precedes
            // resident capture, so the group's Excel reserve is their larger cap.
            let excel_limit = job
                .plan
                .group_reserve
                .saturating_sub(worker_limit)
                .saturating_sub(job.plan.helper_reserve);
            let unused = worker_limit
                .saturating_sub(sample.worker_current.min(worker_limit))
                .saturating_add(
                    excel_limit.saturating_sub(sample.resident_current.min(excel_limit)),
                )
                .saturating_add(job.plan.helper_reserve);
            // Credit only measured private resident pages. Rust and helper RAM
            // remain fully reserved; unavailable telemetry gets no credit.
            (
                physical.saturating_add(
                    job.plan
                        .group_reserve
                        .saturating_sub(sample.resident_private_working_set.min(excel_limit)),
                ),
                commit.saturating_add(unused),
            )
        })
}

#[derive(Debug, PartialEq, Eq)]
enum Admission {
    Wait,
    Add,
    Drain(&'static str),
}

fn admission(
    sample: Sample,
    any_group_done: bool,
    count: usize,
    requested: usize,
    remaining: usize,
    affordable: bool,
) -> Admission {
    if any_group_done || sample.failed {
        Admission::Drain("a group finished or failed")
    } else if count >= requested || remaining == 0 {
        Admission::Drain("requested group limit or end of queue")
    } else if !sample.ready {
        Admission::Wait
    } else if !affordable {
        Admission::Drain("physical RAM or commit reserve")
    } else {
        Admission::Add
    }
}

fn any_group_done(active: &[Active]) -> bool {
    active
        .iter()
        .any(|job| job.finished || job.participant.ready())
}

fn opening_pause_recipients(active: &[Active], newest: usize) -> impl Iterator<Item = usize> + '_ {
    // Ready peers still own Excel while waiting for cleanup, and therefore
    // share the wait caused by the already-admitted newest group's startup.
    active[..newest]
        .iter()
        .enumerate()
        .filter_map(|(index, job)| (!job.finished).then_some(index))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compare(
    source: &Source,
    jobs: Vec<Comparison>,
    requested: usize,
    output: &Path,
    records: &mut Vec<Value>,
    corpora: &[Value],
    limits: &ArchiveArgs,
    run: &Value,
    log_level: LogLevel,
) -> Result<()> {
    ensure!(
        requested > 0 && run["options"]["resident_capture"] == true,
        "adaptive comparison requires resident capture and a positive group limit"
    );
    let budget = MemoryBudget::configured(true)?;
    let (system_reserve, helper_reserve) = configured_reserves()?;
    let configured_resident =
        configured_excel_limit("EXCEL_SHRINK_RESIDENT_MEMORY_MIB", 16384, 1024)?;
    let mut controller = adaptive::Controller::new(
        requested,
        adaptive::Config {
            worker: budget.worker,
            resident: configured_resident,
            automation: configured_excel_limit("EXCEL_SHRINK_EXCEL_MEMORY_MIB", 4096, 256)?,
            system_reserve,
            helper_reserve,
        },
    );
    let mut pending: VecDeque<_> = jobs.into();
    let mut wave_number = 0;
    while !pending.is_empty() {
        wave_number += 1;
        let initial = wait_for_adaptive_admission(&controller, 1, || {
            publish_progress(output, records, corpora, "waiting for memory");
        })?;
        let mut cleanup = batch_cleanup::Batch::growing(output, limits.workbook_timeout_seconds)?;
        let mut active = Vec::<Active>::new();
        let mut next = Some(initial);
        let mut admission_reserves = (0, 0);
        let mut errors = Vec::new();
        std::thread::scope(|scope| {
            let (sender, receiver) = mpsc::channel();
            let mut handles = Vec::new();
            let mut last_tick = Instant::now();
            loop {
                // A peer may finish between admission and the next launch. Do
                // not extend its cleanup wait by opening another group.
                if next.is_some() && any_group_done(&active) {
                    next = None;
                    cleanup.close_admission();
                }
                if let Some((memory, plan)) = next.take() {
                    let job = pending
                        .pop_front()
                        .expect("admission requires a pending job");
                    let participant = match cleanup.add_participant() {
                        Ok(participant) => participant,
                        Err(error) => {
                            pending.push_front(job);
                            errors.push(format!("register comparison group: {error:#}"));
                            cleanup.close_admission();
                            continue;
                        }
                    };
                    let mut initial = records[job.position].clone();
                    initial["comparison_batch"] = json!({
                        "run_id":run["run_id"],"number":wave_number,"workers":active.len()+1,
                        "admission_slot":active.len()+1,
                        "requested_workers":requested,"adaptive":true,"scheduling":"live admission after primary clients are ready; drain together",
                        "resident_memory_estimate_mib":plan.resident_limit / 1_048_576,
                        "configured_resident_memory_estimate_mib":configured_resident / 1_048_576,
                        "available_physical_bytes":memory.physical,"available_commit_bytes":memory.commit,
                        "total_physical_bytes":memory.total_physical,"workbook_reserve_bytes":plan.group_reserve,
                        "system_reserve_bytes":plan.system_reserve,"helper_reserve_bytes":plan.helper_reserve,
                        "rust_worker_reserve_bytes":budget.worker,"excel_session_reserve_bytes":plan.group_reserve-budget.worker-plan.helper_reserve,
                        "reference_peak_bytes":plan.reference_peak_bytes,"calibration_samples":plan.calibration_samples,
                        "estimate_rule":"maximum live and recent completed resident peaks, normalized to at least 6 clients; 1.5x plus 1 GiB rounded up; configured estimate retained",
                        "physical_reserve_rule":"active estimates minus fresh private resident job pages when available; Rust and helpers fully reserved"
                    });
                    initial["comparison_batch"]["admission_reason"] = json!(if active.is_empty() {
                        "first group"
                    } else {
                        "previous group primary clients ready; RAM and commit sufficient"
                    });
                    initial["comparison_batch"]["prior_groups_physical_reserve_bytes"] =
                        json!(admission_reserves.0);
                    initial["comparison_batch"]["prior_groups_commit_reserve_bytes"] =
                        json!(admission_reserves.1);
                    eprintln!("[study] {}: live wave {wave_number}, admitting group {}/{requested}; RAM {:.1} GiB, commit {:.1} GiB, Excel cap {:.1} GiB",
                        source.name, active.len()+1, memory.physical as f64/GIB as f64, memory.commit as f64/GIB as f64, plan.resident_limit as f64/GIB as f64);
                    let position = job.position;
                    let sender = sender.clone();
                    let worker_job = job.clone();
                    let barrier = participant.clone();
                    active.push(Active {
                        job,
                        participant,
                        plan,
                        started_ms: provenance::now_ms(),
                        started: Instant::now(),
                        pause_millis: 0,
                        finished: false,
                        primary_ready: false,
                        normalized_peak: None,
                    });
                    handles.push(scope.spawn(move || {
                        let ready_on_exit = ReadyOnExit(Some(barrier.clone()));
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            execution::run_record(
                                source,
                                worker_job.index,
                                &worker_job.id,
                                JobMode::Compare,
                                output,
                                initial,
                                limits,
                                run,
                                log_level,
                                Some(barrier),
                                |record, _| {
                                    let _ =
                                        sender.send(Event::Checkpoint(position, record.clone()));
                                },
                            )
                        }))
                        .unwrap_or_else(|_| {
                            Err(anyhow::anyhow!("comparison coordinator panicked"))
                        });
                        drop(ready_on_exit);
                        let _ = sender.send(Event::Finished(position, result));
                    }));
                    last_tick = Instant::now();
                }

                let now = Instant::now();
                let elapsed_ms = now
                    .duration_since(last_tick)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64;
                last_tick = now;
                let now_ms = provenance::now_ms();
                let samples: Vec<_> = active
                    .iter()
                    .map(|job| read_sample(output, job, now_ms))
                    .collect();
                for (job, sample) in active.iter_mut().zip(&samples) {
                    job.primary_ready |= sample.ready;
                    job.normalized_peak = job.normalized_peak.max(sample.normalized_peak);
                }
                let newest = active.len().checked_sub(1);
                if let Some(index) = newest {
                    let latest = &active[index];
                    let warming = !latest.finished
                        && !latest.participant.ready()
                        && !latest.primary_ready
                        && !samples[index].failed
                        && latest.started.elapsed()
                            < Duration::from_secs(limits.workbook_timeout_seconds);
                    if warming {
                        let recipients: Vec<_> = opening_pause_recipients(&active, index).collect();
                        for older_index in recipients {
                            let older = &mut active[older_index];
                            older.pause_millis = older.pause_millis.saturating_add(elapsed_ms);
                            if let Some(path) = &older.participant.pause_path {
                                if let Err(error) = artifact_cache::atomic_write(
                                    path,
                                    &serde_json::to_vec(
                                        &json!({"elapsed_millis":older.pause_millis}),
                                    )
                                    .unwrap(),
                                ) {
                                    if errors.is_empty() {
                                        errors.push(format!("publish opening pause: {error:#}"));
                                    }
                                    cleanup.close_admission();
                                }
                            }
                        }
                    }
                    if cleanup.admission_open() {
                        let (physical, commit) = reserved_memory(&active, &samples, budget.worker);
                        let live_peak = active.iter().filter_map(|job| job.normalized_peak).max();
                        let planned = memory_status()
                            .context("read memory for live group admission")
                            .and_then(|memory| {
                                controller
                                    .plan_next(memory, physical, commit, live_peak)
                                    .map(|plan| (memory, plan))
                            });
                        match admission(
                            samples[index],
                            any_group_done(&active),
                            active.len(),
                            requested,
                            pending.len(),
                            planned.is_ok(),
                        ) {
                            Admission::Wait => {}
                            Admission::Add => {
                                admission_reserves = (physical, commit);
                                next = planned.ok();
                            }
                            Admission::Drain(reason) => {
                                eprintln!("[study] Live wave {wave_number}: stop adding groups ({reason}); {} group(s) will drain together", active.len());
                                if reason == "physical RAM or commit reserve" {
                                    if let Err(error) = planned {
                                        eprintln!("[study] Existing group headroom: physical {:.1} GiB, commit {:.1} GiB; {error:#}", physical as f64/GIB as f64, commit as f64/GIB as f64);
                                    }
                                }
                                cleanup.close_admission();
                            }
                        }
                    }
                } else {
                    cleanup.close_admission();
                }
                if let Err(error) = cleanup.release_if_ready() {
                    if errors.is_empty() {
                        errors.push(format!("release comparison wave: {error:#}"));
                    }
                    cleanup.close_admission();
                }
                if !cleanup.admission_open() && active.iter().all(|job| job.finished) {
                    break;
                }
                if next.is_some() {
                    continue;
                }
                match receiver.recv_timeout(Duration::from_millis(250)) {
                    Ok(Event::Checkpoint(position, record)) => {
                        if records[position] != record {
                            records[position] = record;
                            publish_progress(output, records, corpora, "comparing");
                        }
                    }
                    Ok(Event::Finished(position, result)) => {
                        if let Some(job) =
                            active.iter_mut().find(|job| job.job.position == position)
                        {
                            job.finished = true;
                        }
                        match result {
                            Ok(record) => {
                                records[position] = record;
                                publish_progress(output, records, corpora, "comparing");
                            }
                            Err(error) => {
                                errors.push(format!("record {position}: {error:#}"));
                                cleanup.close_admission();
                            }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        cleanup.close_admission();
                    }
                }
            }
            for handle in handles {
                if handle.join().is_err() {
                    errors.push("comparison thread panicked after reporting its result".into());
                }
            }
        });
        let admitted = active.len();
        for job in &active {
            let record = &mut records[job.job.position];
            record["comparison_batch"]["admitted_groups"] = json!(admitted);
            record["comparison_batch"]["workers"] = json!(admitted);
            record["comparison_batch"]["opening_pause_millis"] = json!(job.pause_millis);
            write_record(&record_path(output, &job.job.id), record)?;
        }
        ensure!(
            errors.is_empty(),
            "live comparison wave failed: {}",
            errors.join("; ")
        );
        let attempted: Vec<_> = active.into_iter().map(|job| job.job).collect();
        controller.observe(
            &attempted
                .iter()
                .map(|job| records[job.position].clone())
                .collect::<Vec<_>>(),
        );
        fallback::after_batch(
            source, &attempted, output, records, corpora, limits, run, log_level,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resident() -> Value {
        json!({"status":"running","sampled_unix_ms":10_000,"primary_clients_ready":true,
            "expected_primary_clients":4,"current_job_bytes":2*GIB,"peak_job_bytes":3*GIB,
            "clients":(0..4).map(|i| (i.to_string(), json!({"startup_seconds":1.0,"closed":false}))).collect::<serde_json::Map<_,_>>()})
    }

    fn worker() -> Value {
        json!({"sampled_unix_ms":10_000,"current_bytes":GIB,"final_sample":false})
    }

    #[test]
    fn earlier_ready_peer_stops_growth_but_keeps_pause_credit_during_newest_startup() -> Result<()>
    {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "live-ready-peer")?;
        fs::create_dir(&root)?;
        let mut cleanup = batch_cleanup::Batch::growing(&root, 30)?;
        let controller = adaptive::Controller::new(
            24,
            adaptive::Config {
                worker: 8 * GIB,
                resident: 32 * GIB,
                automation: 4 * GIB,
                system_reserve: Some(8 * GIB),
                helper_reserve: GIB,
            },
        );
        let plan = controller.plan(
            adaptive::Memory {
                physical: 128 * GIB,
                commit: 128 * GIB,
                total_physical: 128 * GIB,
            },
            1,
        )?;
        let mut active = (0..2)
            .map(|position| {
                Ok(Active {
                    job: Comparison {
                        index: position,
                        id: format!("group-{position}"),
                        position,
                    },
                    participant: cleanup.add_participant()?,
                    plan,
                    started_ms: 9_000,
                    started: Instant::now(),
                    pause_millis: 0,
                    finished: false,
                    primary_ready: position == 0,
                    normalized_peak: None,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        assert!(!any_group_done(&active));
        active[0].participant.mark_ready()?;
        assert!(any_group_done(&active));
        let warming = Sample::default();
        assert_eq!(
            admission(warming, any_group_done(&active), 2, 24, 10, true),
            Admission::Drain("a group finished or failed")
        );
        assert_eq!(
            opening_pause_recipients(&active, 1).collect::<Vec<_>>(),
            vec![0]
        );
        let ready = sample_values(&resident(), &worker(), 9_000, 10_000);
        // Even after the newest clients connect, an earlier completed peer
        // prevents a third group from extending the cleanup lifetime.
        assert_eq!(
            admission(ready, any_group_done(&active), 2, 24, 10, true),
            Admission::Drain("a group finished or failed")
        );
        active[0].finished = true;
        assert_eq!(opening_pause_recipients(&active, 1).count(), 0);
        drop(cleanup);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn complete_primary_startup_admits_second_group_before_first_comparison_finishes() {
        let sample = sample_values(&resident(), &worker(), 9_000, 10_000);
        assert!(sample.ready);
        assert_eq!(sample.normalized_peak, Some(9 * GIB / 2));
        // No completed workbook or visual response is needed. The same policy
        // runs before receiving a Finished event in the real coordinator.
        assert_eq!(admission(sample, false, 1, 24, 10, true), Admission::Add);
        assert_eq!(
            admission(sample, true, 1, 24, 10, true),
            Admission::Drain("a group finished or failed")
        );
        assert_eq!(
            admission(sample, false, 24, 24, 10, true),
            Admission::Drain("requested group limit or end of queue")
        );
        assert_eq!(admission(sample, false, 23, 24, 10, true), Admission::Add);
        assert_eq!(
            admission(sample, false, 17, usize::MAX, 10, true),
            Admission::Add
        );
        assert_eq!(
            admission(sample, false, 1, 24, 10, false),
            Admission::Drain("physical RAM or commit reserve")
        );
    }

    #[test]
    fn partial_missing_stale_failed_and_package_only_metrics_cannot_open_another_group() {
        let mut invalid = vec![Value::Null, json!({"status":"package-audit"})];
        for (key, value) in [
            ("status", json!("starting")),
            ("status", json!("failed")),
            ("primary_clients_ready", json!(false)),
            ("expected_primary_clients", json!(0)),
            ("expected_primary_clients", json!(5)),
            ("current_job_bytes", Value::Null),
            ("current_job_bytes", json!(0)),
            ("peak_job_bytes", json!(GIB)),
            ("sampled_unix_ms", json!(4_999)),
            ("sampled_unix_ms", json!(10_001)),
            ("clients", json!({})),
        ] {
            let mut value_with_error = resident();
            value_with_error[key] = value;
            invalid.push(value_with_error);
        }
        let mut opening = resident();
        opening["clients"]["3"]["startup_seconds"] = Value::Null;
        invalid.push(opening);
        let mut closed = resident();
        closed["clients"]["3"]["closed"] = json!(true);
        invalid.push(closed);
        for value in invalid {
            let sample = sample_values(&value, &worker(), 9_000, 10_000);
            assert!(!sample.ready, "unexpectedly ready: {value}");
            assert_ne!(admission(sample, false, 1, 24, 10, true), Admission::Add);
        }
        assert!(!sample_values(&resident(), &Value::Null, 9_000, 10_000).ready);
        assert!(!sample_values(&resident(), &worker(), 10_001, 10_001).ready);
    }

    #[test]
    fn active_enforceable_budgets_are_reserved_and_only_fresh_current_usage_is_credited(
    ) -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "live-reserves")?;
        fs::create_dir(&root)?;
        let mut cleanup = batch_cleanup::Batch::growing(&root, 30)?;
        let participant = cleanup.add_participant()?;
        let controller = adaptive::Controller::new(
            24,
            adaptive::Config {
                worker: 8 * GIB,
                resident: 32 * GIB,
                automation: 4 * GIB,
                system_reserve: Some(8 * GIB),
                helper_reserve: GIB,
            },
        );
        let memory = adaptive::Memory {
            physical: 100 * GIB,
            commit: 100 * GIB,
            total_physical: 128 * GIB,
        };
        let first = controller.plan(memory, 1)?;
        let active = vec![Active {
            job: Comparison {
                index: 0,
                id: "first".into(),
                position: 0,
            },
            participant,
            plan: first,
            started_ms: 9_000,
            started: Instant::now(),
            pause_millis: 0,
            finished: false,
            primary_ready: true,
            normalized_peak: Some(9 * GIB / 2),
        }];
        let sample = sample_values(&resident(), &worker(), 9_000, 10_000);
        let (physical, commit) = reserved_memory(&active, &[sample], 8 * GIB);
        assert_eq!(physical, 41 * GIB); // Do not mistake private commit for RAM.
        assert_eq!(commit, 38 * GIB); // 41 minus current Rust1 and owned-job2.
        let next = controller.plan_next(memory, physical, commit, sample.normalized_peak)?;
        assert_eq!(next.resident_limit, 8 * GIB); // 4.5*1.5+1, rounded up.
        assert_eq!(next.group_reserve, 17 * GIB);
        let physical_edge = adaptive::Memory {
            physical: 66 * GIB,
            ..memory
        };
        assert!(controller
            .plan_next(physical_edge, physical, commit, sample.normalized_peak)
            .is_ok());
        assert!(controller
            .plan_next(
                adaptive::Memory {
                    physical: 66 * GIB - 1,
                    ..memory
                },
                physical,
                commit,
                sample.normalized_peak
            )
            .is_err());
        assert!(controller
            .plan_next(
                adaptive::Memory {
                    commit: 63 * GIB - 1,
                    ..memory
                },
                physical,
                commit,
                sample.normalized_peak
            )
            .is_err());
        let stale = sample_values(&resident(), &worker(), 9_000, 20_000);
        assert_eq!(
            reserved_memory(&active, &[stale], 8 * GIB),
            (41 * GIB, 41 * GIB)
        );
        assert_eq!(active[0].plan.resident_limit, 32 * GIB); // Older cap is never lowered.
        let mut measured_physical = resident();
        measured_physical["current_job_private_working_set_bytes"] = json!(GIB);
        let measured = sample_values(&measured_physical, &worker(), 9_000, 10_000);
        assert_eq!(
            reserved_memory(&active, &[measured], 8 * GIB),
            (40 * GIB, 38 * GIB)
        );
        // A racy/malformed physical count cannot exceed the measured commit or
        // reduce future headroom for the independent Rust worker.
        measured_physical["current_job_private_working_set_bytes"] = json!(100 * GIB);
        let measured = sample_values(&measured_physical, &worker(), 9_000, 10_000);
        assert_eq!(
            reserved_memory(&active, &[measured], 8 * GIB),
            (39 * GIB, 38 * GIB)
        );
        assert_eq!(
            reserved_memory(
                &active,
                &[sample_values(&measured_physical, &worker(), 9_000, 20_000)],
                8 * GIB
            ),
            (41 * GIB, 41 * GIB)
        );
        assert!(
            controller
                .plan_next(memory, physical, commit, Some(20 * GIB))?
                .resident_limit
                > next.resident_limit
        );
        drop(cleanup);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
