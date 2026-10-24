//! Fixed or growing comparison waves; only the coordinator writes shared reports.
mod adaptive;
mod live;
mod manual;

use super::*;
use std::collections::VecDeque;
use std::sync::mpsc;

#[derive(Clone)]
pub(super) struct Comparison {
    pub index: usize,
    pub id: String,
    pub position: usize,
}

const GIB: u64 = 1024 * 1024 * 1024;
const SYSTEM_RESERVE: u64 = 4 * GIB;

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
    if run["options"]["control_room"] == true {
        return manual::compare(
            source, jobs, output, records, corpora, limits, run, log_level, None,
        );
    }
    if run["options"]["adaptive_concurrency"] == true {
        return live::compare(
            source, jobs, requested, output, records, corpora, limits, run, log_level,
        );
    }
    let memory_budget = MemoryBudget::configured(run["options"]["resident_capture"] == true)?;
    let mut pending: VecDeque<_> = jobs.into();
    let mut batch_number = 0;
    while !pending.is_empty() {
        batch_number += 1;
        let available = if requested > 1 {
            match available_memory() {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    eprintln!(
                        "[study] Cannot read available RAM; using one comparison worker: {error}"
                    );
                    None
                }
            }
        } else {
            None
        };
        let count = admitted_workers(requested, available, memory_budget).min(pending.len());
        if requested > 1 {
            eprintln!(
                "[study] {}: comparison batch {batch_number}, {count}/{requested} workers, available RAM {} GiB",
                source.name,
                available
                    .map(|bytes| format!("{:.1}", bytes as f64 / GIB as f64))
                    .unwrap_or_else(|| "unknown".into())
            );
        }
        let cleanup = (count > 1 && run["options"]["resident_capture"] == true)
            .then(|| batch_cleanup::Batch::new(output, count, limits.workbook_timeout_seconds))
            .transpose()?;
        let batch: Vec<_> = pending
            .drain(..count)
            .enumerate()
            .map(|(slot, job)| {
                let mut record = records[job.position].clone();
                record["comparison_batch"] = json!({
                    "run_id":run["run_id"],"number":batch_number,"workers":count,
                    "requested_workers":requested,"available_physical_bytes":available,
                    "workbook_reserve_bytes":memory_budget.total(),"system_reserve_bytes":SYSTEM_RESERVE,
                    "rust_worker_reserve_bytes":memory_budget.worker,
                    "excel_session_reserve_bytes":memory_budget.excel,
                });
                (
                    job.position,
                    (
                        job,
                        record,
                        cleanup.as_ref().map(|batch| batch.participant(slot)),
                    ),
                )
            })
            .collect();
        let attempted: Vec<_> = batch.iter().map(|(_, (job, _, _))| job.clone()).collect();
        run_batch(
            batch,
            |(job, initial, barrier), checkpoint| {
                execution::run_record(
                    source,
                    job.index,
                    &job.id,
                    JobMode::Compare,
                    output,
                    initial,
                    limits,
                    run,
                    log_level,
                    barrier,
                    |record, _phase| checkpoint(record.clone()),
                )
            },
            |position, record| {
                if records[position] != record {
                    records[position] = record;
                    publish_progress(output, records, corpora, "comparing");
                }
            },
            cleanup.as_ref(),
        )?;
        // Every worker and its owned Excel processes have finished before any
        // replacement starts. Opening a new workbook cannot stall a previous batch.
        fallback::after_batch(
            source, &attempted, output, records, corpora, limits, run, log_level,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_preparing_workbook(
    source: &Source,
    job: Comparison,
    output: &Path,
    records: &mut Vec<Value>,
    corpora: &[Value],
    limits: &ArchiveArgs,
    run: &Value,
    log_level: LogLevel,
) -> Result<()> {
    let id = job.id.clone();
    manual::compare(
        source,
        vec![job],
        output,
        records,
        corpora,
        limits,
        run,
        log_level,
        Some(id),
    )
}

enum Event {
    Checkpoint(usize, Value),
    Finished(usize, Result<Value>),
}

fn run_batch<T: Send>(
    jobs: Vec<(usize, T)>,
    execute: impl Fn(T, &dyn Fn(Value)) -> Result<Value> + Sync,
    mut checkpoint: impl FnMut(usize, Value),
    cleanup: Option<&batch_cleanup::Batch>,
) -> Result<()> {
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        let mut handles = Vec::new();
        for (slot, (position, job)) in jobs.into_iter().enumerate() {
            let barrier = cleanup.map(|batch| batch.participant(slot));
            let sender = sender.clone();
            let execute = &execute;
            handles.push((
                position,
                scope.spawn(move || {
                    // Includes infrastructure failures and panics before launch.
                    let _ready_on_exit = ReadyOnExit(barrier);
                    let result = execute(job, &|record| {
                        let _ = sender.send(Event::Checkpoint(position, record));
                    });
                    let _ = sender.send(Event::Finished(position, result));
                }),
            ));
        }
        drop(sender);
        let mut errors = Vec::new();
        loop {
            if let Some(cleanup) = cleanup {
                if let Err(error) = cleanup.release_if_ready() {
                    if errors.is_empty() {
                        errors.push(format!("release batch cleanup: {error:#}"));
                    }
                }
            }
            let event = match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => event,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            match event {
                Event::Checkpoint(position, record) | Event::Finished(position, Ok(record)) => {
                    checkpoint(position, record);
                }
                Event::Finished(position, Err(error)) => {
                    errors.push(format!("record {position}: {error:#}"));
                }
            }
        }
        for (position, handle) in handles {
            if handle.join().is_err() {
                errors.push(format!(
                    "record {position}: comparison coordinator panicked"
                ));
            }
        }
        // Infrastructure failures stop the next batch only after all peers have
        // saved their results. Normal workbook failures are durable skipped records.
        ensure!(
            errors.is_empty(),
            "comparison batch failed: {}",
            errors.join("; ")
        );
        Ok(())
    })
}

struct ReadyOnExit(Option<batch_cleanup::Participant>);
impl Drop for ReadyOnExit {
    fn drop(&mut self) {
        if let Some(barrier) = &self.0 {
            if let Err(error) = barrier.mark_ready() {
                eprintln!("Could not mark finished batch worker: {error:#}");
            }
        }
    }
}

#[derive(Clone, Copy)]
struct MemoryBudget {
    worker: u64,
    excel: u64,
}

impl MemoryBudget {
    fn configured(resident: bool) -> Result<Self> {
        let worker =
            worker::memory_limit_bytes().context("configure comparison worker RAM reserve")?;
        let automation = configured_excel_limit("EXCEL_SHRINK_EXCEL_MEMORY_MIB", 4096, 256)?;
        let excel = if resident {
            // COM validation runs before resident capture. Reserve the larger
            // phase alongside Rust, whose separate job can allocate concurrently.
            configured_excel_limit("EXCEL_SHRINK_RESIDENT_MEMORY_MIB", 16384, 1024)?.max(automation)
        } else {
            automation
        };
        Ok(Self { worker, excel })
    }

    fn total(self) -> u64 {
        self.worker.saturating_add(self.excel)
    }
}

fn configured_excel_limit(name: &str, default_mib: u64, minimum_mib: u64) -> Result<u64> {
    let mib = match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .with_context(|| format!("invalid {name}"))?,
        Err(std::env::VarError::NotPresent) => default_mib,
        Err(error) => return Err(error.into()),
    };
    ensure!(
        (minimum_mib..=65536).contains(&mib),
        "{name} must be between {minimum_mib} and 65536 MiB"
    );
    Ok(mib * 1_048_576)
}

fn admitted_workers(requested: usize, available: Option<u64>, budget: MemoryBudget) -> usize {
    let affordable = available
        .map(|bytes| bytes.saturating_sub(SYSTEM_RESERVE) / budget.total())
        .unwrap_or(1);
    usize::try_from(affordable.max(1))
        .unwrap_or(usize::MAX)
        .min(requested)
}

fn configured_reserves() -> Result<(Option<u64>, u64)> {
    let system = std::env::var_os("EXCEL_SHRINK_SYSTEM_RESERVE_MIB")
        .map(|_| configured_excel_limit("EXCEL_SHRINK_SYSTEM_RESERVE_MIB", 4096, 1024))
        .transpose()?;
    let helpers = configured_excel_limit("EXCEL_SHRINK_HELPER_RESERVE_MIB", 1024, 256)?;
    Ok((system, helpers))
}

fn wait_for_adaptive_admission(
    controller: &adaptive::Controller,
    remaining: usize,
    mut on_wait: impl FnMut(),
) -> Result<(adaptive::Memory, adaptive::Plan)> {
    loop {
        let memory =
            memory_status().context("read physical RAM and commit before adaptive admission")?;
        match controller.plan(memory, remaining) {
            Ok(plan) => return Ok((memory, plan)),
            Err(error) => {
                let minimum = controller.plan(
                    adaptive::Memory {
                        physical: u64::MAX,
                        commit: u64::MAX,
                        total_physical: memory.total_physical,
                    },
                    1,
                )?;
                ensure!(
                    memory.total_physical >= minimum.group_reserve + minimum.system_reserve,
                    "configured memory limits need at least {:.1} GiB for one group, but this machine has {:.1} GiB; lower EXCEL_SHRINK_WORKER_MEMORY_MIB, EXCEL_SHRINK_RESIDENT_MEMORY_MIB and/or EXCEL_SHRINK_EXCEL_MEMORY_MIB",
                    (minimum.group_reserve + minimum.system_reserve) as f64 / GIB as f64,
                    memory.total_physical as f64 / GIB as f64
                );
                eprintln!("[study] Waiting for memory before the next group: {error:#}");
                on_wait();
                // No workbook or Excel process is held while this loop waits.
                std::thread::sleep(Duration::from_secs(30));
            }
        }
    }
}

// A fallback restores the configured paired-capture limits. Its admission must
// therefore be checked independently from the smaller adaptive resident cap.
pub(super) fn wait_for_classic_retry_memory(run: &Value) -> Result<()> {
    if run["options"]["adaptive_concurrency"] != true {
        return Ok(());
    }
    let budget = MemoryBudget::configured(false)?;
    let (system_reserve, helper_reserve) = configured_reserves()?;
    let controller = adaptive::Controller::new(
        1,
        adaptive::Config {
            worker: budget.worker,
            resident: budget.excel.max(GIB),
            automation: budget.excel,
            system_reserve,
            helper_reserve,
        },
    );
    wait_for_adaptive_admission(&controller, 1, || {})?;
    Ok(())
}

fn available_memory() -> std::io::Result<u64> {
    Ok(memory_status()?.physical)
}

#[cfg(windows)]
fn memory_status() -> std::io::Result<adaptive::Memory> {
    #[repr(C)]
    #[derive(Default)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total_physical: u64,
        available_physical: u64,
        total_page_file: u64,
        available_page_file: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    }
    let mut status = MemoryStatus {
        length: std::mem::size_of::<MemoryStatus>() as u32,
        ..Default::default()
    };
    // SAFETY: the writable buffer has the documented MEMORYSTATUSEX layout and
    // its initialized length matches the allocation for the duration of the call.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(adaptive::Memory {
        physical: status.available_physical,
        commit: status.available_page_file,
        total_physical: status.total_physical,
    })
}

#[cfg(not(windows))]
fn memory_status() -> std::io::Result<adaptive::Memory> {
    // WSL's memory counters cannot establish how much memory Windows Excel can use.
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "native Windows RAM counters required",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};

    #[test]
    fn batch_admission_keeps_memory_for_windows_and_each_workbook() {
        let budget = MemoryBudget {
            worker: 8 * GIB,
            excel: 16 * GIB,
        };
        assert_eq!(budget.total(), 24 * GIB);
        assert_eq!(admitted_workers(3, Some(76 * GIB), budget), 3);
        assert_eq!(admitted_workers(3, Some(76 * GIB - 1), budget), 2);
        assert_eq!(admitted_workers(3, Some(65 * GIB), budget), 2);
        assert_eq!(admitted_workers(3, Some(52 * GIB - 1), budget), 1);
        assert_eq!(admitted_workers(2, Some(100 * GIB), budget), 2);
        assert_eq!(admitted_workers(3, None, budget), 1);
        assert_eq!(admitted_workers(3, Some(0), budget), 1);
        assert_eq!(admitted_workers(24, Some(1024 * GIB), budget), 24);
        assert_eq!(admitted_workers(256, Some(76 * GIB), budget), 3);
        assert_eq!(admitted_workers(usize::MAX, Some(76 * GIB), budget), 3);
        let defaults = MemoryBudget {
            worker: 4 * GIB,
            excel: 16 * GIB,
        };
        assert_eq!(admitted_workers(3, Some(64 * GIB), defaults), 3);
        assert_eq!(admitted_workers(3, Some(64 * GIB - 1), defaults), 2);
    }

    #[test]
    fn one_failure_keeps_peer_checkpoints_and_waits_for_the_batch() {
        let barrier = Barrier::new(2);
        let coordinator = std::thread::current().id();
        let mut records = [Value::Null, Value::Null];
        let outcome = run_batch(
            vec![(0, false), (1, true)],
            |succeeds, checkpoint| {
                barrier.wait();
                checkpoint(json!({"complete":false}));
                if succeeds {
                    Ok(json!({"complete":true}))
                } else {
                    anyhow::bail!("simulated launch failure")
                }
            },
            |position, record| {
                assert_eq!(std::thread::current().id(), coordinator);
                records[position] = record;
            },
            None,
        );
        assert!(format!("{:#}", outcome.unwrap_err()).contains("simulated launch failure"));
        assert_eq!(records[0]["complete"], false);
        assert_eq!(records[1]["complete"], true);
    }

    #[test]
    fn later_batch_starts_only_after_every_previous_worker_finishes() -> Result<()> {
        let events = Arc::new(Mutex::new(Vec::new()));
        for batch in 0..2 {
            let barrier = Barrier::new(2);
            run_batch(
                vec![(0, 0), (1, 1)],
                |worker, checkpoint| {
                    events.lock().unwrap().push((batch, worker, "start"));
                    barrier.wait();
                    checkpoint(json!({"worker":worker}));
                    events.lock().unwrap().push((batch, worker, "end"));
                    Ok(json!({"complete":true}))
                },
                |_, _| {},
                None,
            )?;
        }
        let events = events.lock().unwrap();
        assert!(events[..4].iter().all(|event| event.0 == 0));
        assert!(events[4..].iter().all(|event| event.0 == 1));
        assert_eq!(events.len(), 8);
        Ok(())
    }

    #[test]
    fn early_failed_peer_releases_ready_worker_after_its_checkpoint() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "batch-failed-peer")?;
        fs::create_dir(&root)?;
        let cleanup = batch_cleanup::Batch::new(&root, 2, 1)?;
        let rendezvous = Barrier::new(2);
        let mut records = [Value::Null, Value::Null];
        let result = run_batch(
            vec![(0, true), (1, false)],
            |succeeds, checkpoint| {
                if succeeds {
                    checkpoint(json!({"complete":true}));
                    let participant = cleanup.participant(0);
                    participant.mark_ready()?;
                    assert!(!participant.released());
                    rendezvous.wait();
                    participant.wait(None, false)?;
                    assert!(cleanup.participant(1).ready());
                    Ok(json!({"complete":true,"cleanup_finished":true}))
                } else {
                    rendezvous.wait();
                    anyhow::bail!("worker failed before launching Excel")
                }
            },
            |position, record| records[position] = record,
            Some(&cleanup),
        );
        assert!(format!("{:#}", result.unwrap_err()).contains("failed before launching"));
        assert_eq!(records[0]["cleanup_finished"], true);
        assert!(cleanup.participant(1).ready());
        drop(cleanup);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
