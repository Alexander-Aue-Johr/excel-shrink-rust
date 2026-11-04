//! Durable, bounded process telemetry. A process probe is not an add-in heartbeat
//! and neither signal is evidence that capture or recalculation made progress.

use super::Args;
use crate::native_process::process_alive;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    Discover,
    Open,
    Save,
    InspectSave,
    Optimize,
    InspectOptimization,
    MeasureHeights,
    CaptureBefore,
    Recalculate,
    CaptureAfter,
    Compare,
    Close,
}

const ALL_PHASES: [Phase; 12] = [
    Phase::Discover,
    Phase::Open,
    Phase::Save,
    Phase::InspectSave,
    Phase::Optimize,
    Phase::InspectOptimization,
    Phase::MeasureHeights,
    Phase::CaptureBefore,
    Phase::Recalculate,
    Phase::CaptureAfter,
    Phase::Compare,
    Phase::Close,
];

impl Phase {
    /// The same ordered contract drives execution telemetry and diagram columns.
    fn columns() -> Value {
        json!([
            {"id":"discover", "label":"Find changes", "description":"Compare original and Default cell XML; empty style changes become row-geometry candidates."},
            {"id":"open", "label":"Open Excel", "description":"Open only required states; byte-identical Default inputs need no separate Excel process."},
            {"id":"save", "label":"Save copies", "description":"Observe ordinary Excel save effects before an explicit rebuild."},
            {"id":"inspect_save", "label":"Inspect saved XML", "description":"Add candidate cells also changed by ordinary Excel save."},
            {"id":"optimize", "label":"Optimize all", "description":"Run native Check Performance on a separate saved Original copy; require positive completion."},
            {"id":"inspect_optimization", "label":"Inspect optimization", "description":"Add XML changes from Check Performance, separately from ordinary save effects."},
            {"id":"measure_heights", "label":"Check row heights", "description":"Office.js checks automatic-height spans; retained identical fixed heights are excluded."},
            {"id":"capture_before", "label":"Capture before", "description":"Capture the same candidate regions in all enabled open copies."},
            {"id":"recalculate", "label":"Full rebuild", "description":"Rebuild only when calculation dependencies require it; unsupported features keep the rebuild path."},
            {"id":"capture_after", "label":"Capture after", "description":"Capture the same regions after each workbook rebuild."},
            {"id":"compare", "label":"Compare evidence", "description":"Compare observed states and geometry; explicit identity and dependency proofs explain omitted states."},
            {"id":"close", "label":"Close Excel", "description":"Close owned copies and verify the exact Excel processes have exited."}
        ])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub(super) enum LaneId {
    O,
    SD,
    CP,
    OR,
    SDR,
    CPR,
}

impl LaneId {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::O => "O",
            Self::SD => "SD",
            Self::CP => "CP",
            Self::OR => "OR",
            Self::SDR => "SDR",
            Self::CPR => "CPR",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Lifecycle {
    NotStarted,
    Opening,
    Open,
    Closing,
    Closed,
    Failed,
    Unavailable,
    Skipped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StepStatus {
    Pending,
    Running,
    Done,
    Skipped,
    Failed,
    Unavailable,
}

#[derive(Clone, Serialize)]
pub(super) struct Lane {
    pub id: LaneId,
    pub label: &'static str,
    pub lifecycle: Lifecycle,
    pub pid: Option<u32>,
    pub process_started_at: Option<i64>,
    pub process_alive: Option<bool>,
    pub process_heartbeat_at: Option<i64>,
    pub process_heartbeat_count: u64,
    pub xml_started_at: Option<i64>,
    pub xml_finished_at: Option<i64>,
    pub closed_at: Option<i64>,
    pub steps: BTreeMap<Phase, StepStatus>,
    pub captures_done: usize,
    pub skipped_zero_area_regions: usize,
    pub captures_total: Option<usize>,
    pub decision_note: Option<String>,
    pub capture_selection_complete: bool,
    pub current_sheet: Option<String>,
    pub current_range: Option<String>,
}

#[derive(Serialize)]
pub(super) struct Snapshot {
    pub run_id: String,
    pub workbook: String,
    pub execution_strategy: &'static str,
    pub capture_policy: &'static str,
    pub report_directory: PathBuf,
    pub status: String,
    pub started_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
    pub revision: u64,
    /// Monotonic count of successfully persisted nonempty capture images.
    pub capture_success_sequence: u64,
    pub candidate_count: Option<usize>,
    pub lanes: Vec<Lane>,
    pub columns: Value,
    pub report: Value,
    pub error: Option<String>,
    pub publication_errors: Vec<String>,
    pub current_operation: Value,
    pub last_successful_operation: Value,
}

impl Snapshot {
    /// Apply a workbook-pair phase without treating Check Performance as executed.
    pub fn pair_step(&mut self, phase: Phase, status: StepStatus) {
        self.step(LaneId::O, phase, status);
        self.step(LaneId::SD, phase, status);
    }
    /// Resolve a fixed workbook lane without conflating it with an arbitrary PID.
    pub fn lane(&mut self, id: LaneId) -> &mut Lane {
        self.lanes
            .iter_mut()
            .find(|lane| lane.id == id)
            .expect("fixed comparison lane")
    }

    /// Record a real phase transition, including XML timestamps for afterglow.
    pub fn step(&mut self, id: LaneId, phase: Phase, status: StepStatus) {
        let lane = self.lane(id);
        lane.steps.insert(phase, status);
        if matches!(
            phase,
            Phase::Discover | Phase::InspectSave | Phase::InspectOptimization
        ) {
            if status == StepStatus::Running {
                lane.xml_started_at = Some(now());
                lane.xml_finished_at = None;
            } else if status == StepStatus::Done {
                lane.xml_finished_at = Some(now());
            }
        }
    }

    /// Refresh capture work after save discovers additional candidates.
    pub fn candidates(&mut self, count: usize) {
        self.candidate_count = Some(count);
        for lane in &mut self.lanes {
            if !matches!(lane.lifecycle, Lifecycle::Unavailable | Lifecycle::Skipped) {
                lane.captures_total = Some(count);
            }
        }
    }
}

struct Shared {
    snapshot: Snapshot,
    paths: Vec<PathBuf>,
    stop: bool,
    last_published: Instant,
}

/// One publisher serializes phase changes and an independent PID-liveness sampler.
pub(super) struct Telemetry {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    sampler: Option<thread::JoinHandle<()>>,
}

impl Telemetry {
    /// Read identities without emitting a new event or writing a snapshot.
    pub fn owned_processes(&self) -> Result<Vec<(u32, i64)>> {
        let state = self
            .shared
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Telemetry lock poisoned"))?;
        Ok(state
            .snapshot
            .lanes
            .iter()
            .filter_map(|lane| lane.pid.zip(lane.process_started_at))
            .collect())
    }
    /// Publish initial state before any XML scan or Excel opening can block.
    pub fn start(args: &Args) -> Result<Self> {
        let run_id = format!("{}-{}", now(), uuid::Uuid::new_v4());
        let mut paths = vec![args.output.join("telemetry.json")];
        if let Some(group) = &args.observatory_group_dir {
            anyhow::ensure!(group.is_dir(), "Observatory group directory does not exist");
            paths.push(group.join("targeted-comparison.json"));
            let registry = group
                .parent()
                .and_then(|groups| groups.parent())
                .context("Observatory group has no control-room root")?
                .join("targeted-comparisons");
            fs::create_dir_all(&registry)?;
            paths.push(registry.join(format!("{run_id}.json")));
        }
        let lanes = [
            (LaneId::O, "Original"),
            (LaneId::SD, "Shrink Default"),
            (LaneId::CP, "Check Performance"),
            (LaneId::OR, "Original (rebuilt copy)"),
            (LaneId::SDR, "Default (rebuilt copy)"),
            (LaneId::CPR, "Check Performance (rebuilt copy)"),
        ]
        .into_iter()
        .map(|(id, label)| Lane {
            id,
            label,
            lifecycle: if matches!(id, LaneId::CP | LaneId::CPR) && args.skip_check_performance {
                Lifecycle::Unavailable
            } else {
                Lifecycle::NotStarted
            },
            pid: None,
            process_started_at: None,
            process_alive: None,
            process_heartbeat_at: None,
            process_heartbeat_count: 0,
            xml_started_at: None,
            xml_finished_at: None,
            closed_at: None,
            steps: ALL_PHASES
                .into_iter()
                .map(|phase| {
                    (
                        phase,
                        if matches!(id, LaneId::CP | LaneId::CPR) && args.skip_check_performance {
                            StepStatus::Unavailable
                        } else if (id != LaneId::CP
                            && matches!(phase, Phase::Optimize | Phase::InspectOptimization))
                            || (matches!(id, LaneId::O | LaneId::SD | LaneId::CP)
                                && matches!(phase, Phase::Recalculate | Phase::CaptureAfter))
                            || (matches!(id, LaneId::OR | LaneId::SDR | LaneId::CPR)
                                && phase == Phase::CaptureBefore)
                        {
                            StepStatus::Skipped
                        } else {
                            StepStatus::Pending
                        },
                    )
                })
                .collect(),
            captures_done: 0,
            skipped_zero_area_regions: 0,
            captures_total: None,
            decision_note: None,
            capture_selection_complete: false,
            current_sheet: None,
            current_range: None,
        })
        .collect();
        let snapshot = Snapshot {
            run_id,
            execution_strategy: super::EXECUTION_STRATEGY,
            capture_policy: super::CAPTURE_POLICY,
            report_directory: args.output.clone(),
            workbook: args
                .workbook_label
                .clone()
                .or_else(|| std::env::var("EXCEL_SHRINK_TARGETED_WORKBOOK_LABEL").ok())
                .unwrap_or_else(|| {
                    args.original
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                }),
            status: "running".into(),
            started_at: now(),
            updated_at: now(),
            finished_at: None,
            revision: 0,
            capture_success_sequence: 0,
            candidate_count: None,
            lanes,
            columns: Phase::columns(),
            report: json!({}),
            error: None,
            publication_errors: Vec::new(),
            current_operation: Value::Null,
            last_successful_operation: Value::Null,
        };
        let shared = Arc::new((
            Mutex::new(Shared {
                snapshot,
                paths,
                stop: false,
                last_published: Instant::now(),
            }),
            Condvar::new(),
        ));
        {
            let mut state = shared
                .0
                .lock()
                .map_err(|_| anyhow::anyhow!("Telemetry lock poisoned"))?;
            write_snapshot(&mut state)?;
        }
        let monitor = shared.clone();
        let sampler = thread::spawn(move || sample_processes(monitor));
        Ok(Self {
            shared,
            sampler: Some(sampler),
        })
    }

    /// Change and persist one coherent snapshot; readers never receive partial JSON.
    pub fn update(&self, update: impl FnOnce(&mut Snapshot)) -> Result<()> {
        let mut state = self
            .shared
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Telemetry lock poisoned"))?;
        update(&mut state.snapshot);
        write_snapshot(&mut state)?;
        self.shared.1.notify_one();
        Ok(())
    }

    /// Coalesce hot capture counters; phase transitions still publish immediately.
    pub fn progress(&self, update: impl FnOnce(&mut Snapshot)) -> Result<()> {
        let mut state = self
            .shared
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Telemetry lock poisoned"))?;
        update(&mut state.snapshot);
        if state.last_published.elapsed() >= Duration::from_millis(250) {
            write_snapshot(&mut state)?;
        }
        Ok(())
    }
}

impl Drop for Telemetry {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.0.lock() {
            state.stop = true;
        }
        self.shared.1.notify_one();
        if let Some(sampler) = self.sampler.take() {
            let _ = sampler.join();
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Replace existing snapshots under the same lock that orders their revisions.
fn write_snapshot(state: &mut Shared) -> Result<()> {
    state.snapshot.updated_at = now();
    state.snapshot.revision += 1;
    state.snapshot.publication_errors.clear();
    let bytes = serde_json::to_vec_pretty(&state.snapshot)?;
    for path in state.paths.iter().skip(1) {
        // Concurrent manual rechecks may share the legacy group snapshot, but
        // must never rename or overwrite one another's temporary publication.
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let temporary = path.with_file_name(format!(
            ".{name}.{}.{}.tmp",
            state.snapshot.run_id, state.snapshot.revision
        ));
        let publication = fs::write(&temporary, &bytes).and_then(|()| fs::rename(&temporary, path));
        if let Err(error) = publication {
            // Dashboard readers may briefly omit FILE_SHARE_DELETE. Keep their
            // last complete snapshot and retry on the next update/heartbeat.
            // This optional mirror must not cancel captured research evidence.
            let _ = fs::remove_file(&temporary);
            state
                .snapshot
                .publication_errors
                .push(format!("Publish {}: {error}", path.display()));
        }
    }
    // Local evidence remains authoritative and includes mirror publication
    // failures. Its bounded atomic replacement already retries Windows locks.
    crate::regression::artifact_cache::atomic_write(
        &state.paths[0],
        &serde_json::to_vec_pretty(&state.snapshot)?,
    )?;
    state.last_published = Instant::now();
    Ok(())
}

/// Heartbeats here are actual PID-and-creation-time observations, not progress.
fn sample_processes(shared: Arc<(Mutex<Shared>, Condvar)>) {
    loop {
        let Ok(state) = shared.0.lock() else { return };
        let Ok((mut state, _)) = shared.1.wait_timeout(state, Duration::from_secs(2)) else {
            return;
        };
        if state.stop {
            return;
        }
        for lane in &mut state.snapshot.lanes {
            if lane.lifecycle == Lifecycle::Closed {
                continue;
            }
            let (Some(pid), Some(started)) = (lane.pid, lane.process_started_at) else {
                continue;
            };
            lane.process_alive = process_alive(i64::from(pid), started);
            if lane.process_alive == Some(true) {
                lane.process_heartbeat_at = Some(now());
                lane.process_heartbeat_count += 1;
            } else if lane.process_alive == Some(false) {
                lane.lifecycle = Lifecycle::Closed;
                lane.closed_at.get_or_insert(now());
            }
        }
        if let Err(error) = write_snapshot(&mut state) {
            state.snapshot.error = Some(format!("Telemetry publication failed: {error:#}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn locked_dashboard_does_not_abort_work_and_recovers_without_losing_progress() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(format!("locked-telemetry-{}", uuid::Uuid::new_v4()));
        let output = root.join("result");
        let group = root.join("control/groups/one");
        fs::create_dir_all(&output)?;
        fs::create_dir_all(&group)?;
        let args = Args {
            original: root.join("original.xlsx"),
            default: root.join("default.xlsx"),
            output: output.clone(),
            host_script: None,
            observatory_group_dir: Some(group.clone()),
            workbook_label: None,
            operation_timeout_seconds: 1,
            skip_check_performance: true,
            check_performance: false,
            serial_captures: false,
            prepare_artifacts_only: false,
        };
        let telemetry = Telemetry::start(&args)?;
        let mirror = group.join("targeted-comparison.json");
        let previous = fs::read(&mirror)?;
        let reader = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&mirror)?;
        telemetry.update(|state| state.lane(LaneId::O).captures_done = 7)?;
        assert_eq!(
            fs::read(&mirror)?,
            previous,
            "Keep the last complete dashboard snapshot"
        );
        let local: Value = serde_json::from_slice(&fs::read(output.join("telemetry.json"))?)?;
        assert_eq!(local["lanes"][0]["captures_done"], 7);
        assert_eq!(local["publication_errors"].as_array().unwrap().len(), 1);
        assert!(
            local["error"].is_null(),
            "A mirror lock is not a comparison failure"
        );
        {
            // A notified heartbeat may publish concurrently with this assertion.
            // Inspect cleanup between publications, under the writer's lock.
            let _publication = telemetry.shared.0.lock().unwrap();
            assert!(!fs::read_dir(&group)?.any(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "tmp")));
        }
        drop(reader);
        telemetry.update(|state| state.lane(LaneId::O).captures_done = 8)?;
        let recovered: Value = serde_json::from_slice(&fs::read(&mirror)?)?;
        assert_eq!(recovered["lanes"][0]["captures_done"], 8);
        assert!(recovered["publication_errors"]
            .as_array()
            .unwrap()
            .is_empty());
        drop(telemetry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn quick_xml_work_and_candidate_growth_are_persisted_without_fake_beats() {
        let root = std::env::current_dir()
            .unwrap()
            .join(".tmp")
            .join(format!("targeted-unit-{}", uuid::Uuid::new_v4()));
        let output = root.join("result");
        let group = root.join("control/groups/one");
        fs::create_dir_all(&output).unwrap();
        fs::create_dir_all(&group).unwrap();
        let args = Args {
            original: root.join("original.xlsx"),
            default: root.join("default.xlsx"),
            output: output.clone(),
            host_script: None,
            observatory_group_dir: Some(group.clone()),
            workbook_label: None,
            operation_timeout_seconds: 1,
            skip_check_performance: true,
            check_performance: false,
            serial_captures: false,
            prepare_artifacts_only: false,
        };
        let telemetry = Telemetry::start(&args).unwrap();
        telemetry
            .update(|state| {
                state.pair_step(Phase::Discover, StepStatus::Running);
                state.pair_step(Phase::Discover, StepStatus::Done);
                state.candidates(3);
                state.lane(LaneId::O).captures_done = 1;
                state.candidates(5);
            })
            .unwrap();
        drop(telemetry);
        let snapshot: Value =
            serde_json::from_slice(&fs::read(output.join("telemetry.json")).unwrap()).unwrap();
        let lane = &snapshot["lanes"][0];
        assert!(
            lane["xml_finished_at"].as_i64().unwrap() >= lane["xml_started_at"].as_i64().unwrap()
        );
        assert_eq!(lane["captures_done"], 1);
        assert_eq!(lane["captures_total"], 5);
        assert!(lane["process_heartbeat_at"].is_null());
        assert_eq!(snapshot["lanes"][2]["steps"]["open"], "unavailable");
        assert_eq!(
            fs::read(group.join("targeted-comparison.json")).unwrap(),
            fs::read(output.join("telemetry.json")).unwrap()
        );
        assert!(root
            .join("control/targeted-comparisons")
            .read_dir()
            .unwrap()
            .next()
            .is_some());
    }
}
