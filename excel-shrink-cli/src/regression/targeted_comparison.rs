//! Targeted comparison workflow; the called phases own XML, Excel, and evidence details.
mod candidates;
mod evidence;
mod host;
mod telemetry;

use crate::native_process::process_alive;
use anyhow::{ensure, Context, Result};
use candidates::discover_candidates;
use clap::Parser;
use evidence::{capture_candidates, compare_evidence, Evidence};
use host::ExcelHost;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    time::Duration,
};
use telemetry::{LaneId, Lifecycle, Phase, StepStatus, Telemetry};
const MAX_CANDIDATES: usize = 64;

#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// Untouched source XLSX; the command works on a copy.
    #[arg(long)]
    original: PathBuf,
    /// XLSX produced by the default shrink profile; the command works on a copy.
    #[arg(long)]
    default: PathBuf,
    /// New directory for the report, screenshots and working copies.
    #[arg(long)]
    output: PathBuf,
    /// PowerShell Excel host script (defaults to the repository copy).
    #[arg(long)]
    host_script: Option<PathBuf>,
    /// Existing Observatory group directory to receive live telemetry.
    #[arg(long)]
    observatory_group_dir: Option<PathBuf>,
    /// Human-readable workbook name in comparison cards (working copies may have generic names).
    #[arg(long)]
    workbook_label: Option<String>,
    /// Maximum duration of one Excel command; closing is additionally capped at 15 seconds.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..))]
    operation_timeout_seconds: u64,
}

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Serialize)]
struct Candidate {
    sheet: String,
    cell: String,
}

#[derive(Serialize)]
struct Finding {
    sheet: String,
    cell: String,
    range: String,
    before_recalc_differs: bool,
    after_shrink_recalc_differs: bool,
    also_changed_by_original_recalc: bool,
    before_recalc_geometry_differs: bool,
    after_shrink_recalc_geometry_differs: bool,
    original_save_xml_changed: bool,
    original_before_png: PathBuf,
    shrink_before_png: PathBuf,
    shrink_recalc_png: PathBuf,
    original_recalc_png: PathBuf,
    original_open_geometry: Value,
    shrink_open_geometry: Value,
    shrink_recalc_geometry: Value,
    original_recalc_geometry: Value,
}

#[derive(Serialize)]
struct Report {
    status: String,
    check_performance: &'static str,
    candidate_count: usize,
    findings: Vec<Finding>,
    limitations: Vec<String>,
    phases: Vec<String>,
    excel_pids: BTreeMap<String, u32>,
    capture_counts: BTreeMap<String, usize>,
}

struct WorkingPaths {
    original: PathBuf,
    default: PathBuf,
}

#[derive(Default)]
struct Captures {
    original_before: BTreeMap<Candidate, Evidence>,
    shrink_before: BTreeMap<Candidate, Evidence>,
    shrink_after: BTreeMap<Candidate, Evidence>,
    original_after: BTreeMap<Candidate, Evidence>,
}

struct Comparison<'a> {
    args: &'a Args,
    paths: WorkingPaths,
    report: Report,
    telemetry: Telemetry,
    candidates: BTreeSet<Candidate>,
    original_save_changes: BTreeSet<Candidate>,
    captures: Captures,
    excel: Option<ExcelHost>,
}

/// Run the same workflow from the CLI or an explicitly selected Observatory group.
/// Existing inputs and output directories are never overwritten. Failures retain
/// partial evidence and report the outcome of owned-process cleanup.
///
/// # Errors
///
/// Rejects invalid inputs, exceeded discovery bounds, failed Excel commands,
/// missing partner evidence, and publication errors. Cleanup failure stays
/// explicit instead of certifying an unconfirmed process exit or comparison.
pub(crate) fn run(args: Args) -> Result<()> {
    let mut comparison = Comparison::prepare(&args)?;
    if let Err(error) = comparison.execute() {
        if let Err(report_error) = comparison.record_failure(&error) {
            return Err(error.context(format!("Failure reporting also failed: {report_error:#}")));
        }
        return Err(error);
    }
    Ok(())
}

impl<'a> Comparison<'a> {
    /// Establish isolated working copies and telemetry before expensive operations.
    fn prepare(args: &'a Args) -> Result<Self> {
        ensure!(
            !args.output.exists(),
            "Output directory already exists: {}",
            args.output.display()
        );
        ensure!(
            args.original.is_file() && args.default.is_file(),
            "Both XLSX inputs must exist"
        );
        fs::create_dir_all(&args.output)?;
        let paths = WorkingPaths {
            original: args.output.join("original.xlsx"),
            default: args.output.join("default.xlsx"),
        };
        fs::copy(&args.original, &paths.original).context("Copy original")?;
        fs::copy(&args.default, &paths.default).context("Copy shrink result")?;
        let telemetry = Telemetry::start(args)?;
        Ok(Self {
            args,
            paths,
            report: new_report(),
            telemetry,
            candidates: BTreeSet::new(),
            original_save_changes: BTreeSet::new(),
            captures: Captures::default(),
            excel: None,
        })
    }

    /// Each call is one domain phase; transport, persistence, and pixel work stay below it.
    fn execute(&mut self) -> Result<()> {
        self.inspect_initial_xml()?;
        if self.candidates.is_empty() {
            return self.complete_without_captures();
        }
        self.open_workbooks()?;
        self.inspect_save_effects()?;
        self.capture_initial_states()?;
        self.rebuild_default_and_capture()?;
        self.rebuild_original_and_capture()?;
        self.compare_captured_evidence()?;
        self.close_workbooks()?;
        self.complete()
    }

    /// Discover bounded candidates without interpreting absent screenshots as equality.
    fn inspect_initial_xml(&mut self) -> Result<()> {
        self.telemetry
            .update(|state| state.pair_step(Phase::Discover, StepStatus::Running))?;
        self.candidates = discover_candidates(&self.paths.original, &self.paths.default)?;
        self.report.phases.push("initial XML compared".into());
        self.telemetry.update(|state| {
            state.candidates(self.candidates.len());
            state.pair_step(Phase::Discover, StepStatus::Done);
        })
    }

    /// Open the two copies sequentially and preserve their independent lifecycles.
    fn open_workbooks(&mut self) -> Result<()> {
        self.excel = Some(ExcelHost::start(
            self.args.host_script.as_deref(),
            Duration::from_secs(self.args.operation_timeout_seconds),
        )?);
        for (id, path) in [
            (LaneId::O, &self.paths.original),
            (LaneId::SD, &self.paths.default),
        ] {
            self.telemetry.update(|state| {
                state.step(id, Phase::Open, StepStatus::Running);
                state.lane(id).lifecycle = Lifecycle::Opening;
            })?;
            let pid = self
                .excel
                .as_mut()
                .context("Excel host not started")?
                .open_workbook(id, path, &self.telemetry)?;
            self.report.excel_pids.insert(id.key().into(), pid);
            self.telemetry.update(|state| {
                state.lane(id).lifecycle = Lifecycle::Open;
                state.step(id, Phase::Open, StepStatus::Done);
            })?;
        }
        self.report.phases.push("original and shrink opened".into());
        Ok(())
    }

    /// Separate ordinary save effects from the later explicit rebuild controls.
    fn inspect_save_effects(&mut self) -> Result<()> {
        let before_original = self.paths.original.with_extension("before-save.xlsx");
        let before_default = self.paths.default.with_extension("before-save.xlsx");
        fs::copy(&self.paths.original, &before_original)?;
        fs::copy(&self.paths.default, &before_default)?;
        for id in [LaneId::O, LaneId::SD] {
            self.telemetry
                .update(|state| state.step(id, Phase::Save, StepStatus::Running))?;
            self.excel
                .as_mut()
                .context("Excel host not started")?
                .save_workbook(id, &self.telemetry)?;
            self.telemetry
                .update(|state| state.step(id, Phase::Save, StepStatus::Done))?;
        }
        self.telemetry
            .update(|state| state.pair_step(Phase::InspectSave, StepStatus::Running))?;
        self.original_save_changes = discover_candidates(&before_original, &self.paths.original)?;
        self.candidates
            .extend(self.original_save_changes.iter().cloned());
        self.candidates
            .extend(discover_candidates(&before_default, &self.paths.default)?);
        ensure!(
            self.candidates.len() <= MAX_CANDIDATES,
            "More than {MAX_CANDIDATES} changed cells; aborting bounded comparison"
        );
        self.report
            .phases
            .push("Excel save effects inspected".into());
        self.telemetry.update(|state| {
            state.candidates(self.candidates.len());
            state.pair_step(Phase::InspectSave, StepStatus::Done);
        })
    }

    /// Retain both pre-rebuild states for the direct shrink comparison.
    fn capture_initial_states(&mut self) -> Result<()> {
        self.captures.original_before =
            self.capture(LaneId::O, Phase::CaptureBefore, "original-before")?;
        self.captures.shrink_before =
            self.capture(LaneId::SD, Phase::CaptureBefore, "shrink-before")?;
        self.report.phases.push("initial regions captured".into());
        Ok(())
    }

    /// Rebuild Default while retaining the original-before control.
    fn rebuild_default_and_capture(&mut self) -> Result<()> {
        self.rebuild(LaneId::SD)?;
        self.captures.shrink_after =
            self.capture(LaneId::SD, Phase::CaptureAfter, "shrink-recalc")?;
        self.report
            .phases
            .push("shrink rebuilt and captured".into());
        Ok(())
    }

    /// Rebuild original separately so ordinary calculation drift can be labelled.
    fn rebuild_original_and_capture(&mut self) -> Result<()> {
        self.rebuild(LaneId::O)?;
        self.captures.original_after =
            self.capture(LaneId::O, Phase::CaptureAfter, "original-recalc")?;
        self.report
            .phases
            .push("original rebuilt and captured".into());
        Ok(())
    }

    /// Publish one rebuild's actual start and successful completion.
    fn rebuild(&mut self, id: LaneId) -> Result<()> {
        self.telemetry
            .update(|state| state.step(id, Phase::Recalculate, StepStatus::Running))?;
        self.excel
            .as_mut()
            .context("Excel host not started")?
            .recalculate_workbook(id, &self.telemetry)?;
        self.telemetry
            .update(|state| state.step(id, Phase::Recalculate, StepStatus::Done))
    }

    /// Capture one lane/state and advance counters only after evidence was written.
    fn capture(
        &mut self,
        id: LaneId,
        phase: Phase,
        directory: &str,
    ) -> Result<BTreeMap<Candidate, Evidence>> {
        self.telemetry
            .update(|state| state.step(id, phase, StepStatus::Running))?;
        let images = capture_candidates(
            self.excel.as_mut().context("Excel host not started")?,
            id,
            &self.candidates,
            &self.args.output.join(directory),
            &self.telemetry,
        )?;
        *self
            .report
            .capture_counts
            .entry(id.key().into())
            .or_default() += images.len();
        self.telemetry
            .update(|state| state.step(id, phase, StepStatus::Done))?;
        Ok(images)
    }

    /// Compare all four required evidence sets; missing evidence is an error.
    fn compare_captured_evidence(&mut self) -> Result<()> {
        self.telemetry
            .update(|state| state.pair_step(Phase::Compare, StepStatus::Running))?;
        self.report.findings = compare_evidence(
            &self.candidates,
            &self.captures.original_before,
            &self.captures.shrink_before,
            &self.captures.shrink_after,
            &self.captures.original_after,
            &self.original_save_changes,
        )?;
        self.report.candidate_count = self.candidates.len();
        self.report
            .phases
            .push("images and rendered geometry compared".into());
        self.telemetry
            .update(|state| state.pair_step(Phase::Compare, StepStatus::Done))
    }

    /// Confirm each owned Excel exit before classifying the workbook as finished.
    fn close_workbooks(&mut self) -> Result<()> {
        for id in [LaneId::SD, LaneId::O] {
            self.telemetry.update(|state| {
                state.lane(id).lifecycle = Lifecycle::Closing;
                state.step(id, Phase::Close, StepStatus::Running);
            })?;
            self.excel
                .as_mut()
                .context("Excel host not started")?
                .close_workbook(id, &self.telemetry)?;
            self.telemetry.update(|state| {
                let lane = state.lane(id);
                lane.lifecycle = Lifecycle::Closed;
                lane.process_alive = Some(false);
                lane.closed_at = Some(chrono::Utc::now().timestamp_millis());
                state.step(id, Phase::Close, StepStatus::Done);
            })?;
        }
        self.excel.take();
        Ok(())
    }

    /// Keep no-candidate selection distinct from a successful whole-workbook proof.
    fn complete_without_captures(&mut self) -> Result<()> {
        self.report.status = "no_cell_candidates_with_limitations".into();
        self.telemetry.update(|state| {
            for id in [LaneId::O, LaneId::SD] {
                for phase in [
                    Phase::Open,
                    Phase::Save,
                    Phase::InspectSave,
                    Phase::CaptureBefore,
                    Phase::Recalculate,
                    Phase::CaptureAfter,
                    Phase::Compare,
                    Phase::Close,
                ] {
                    state.step(id, phase, StepStatus::Skipped);
                }
            }
        })?;
        self.write_completion()
    }

    /// Persist the scientific result separately from compact display telemetry.
    fn complete(&mut self) -> Result<()> {
        self.report.status = "completed_with_limitations".into();
        self.write_completion()
    }

    /// Retain explicit limitations and report only comparisons actually performed.
    fn write_completion(&self) -> Result<()> {
        fs::write(
            self.args.output.join("report.json"),
            serde_json::to_vec_pretty(&self.report)?,
        )?;
        self.telemetry.update(|state| {
            state.status = self.report.status.clone();
            state.finished_at = Some(chrono::Utc::now().timestamp_millis());
            state.report = json!({"status":self.report.status, "limitations":self.report.limitations,
                "before_differences":self.report.findings.iter().filter(|f| f.before_recalc_differs || f.before_recalc_geometry_differs).count(),
                "after_differences":self.report.findings.iter().filter(|f| f.after_shrink_recalc_differs || f.after_shrink_recalc_geometry_differs).count(),
                "original_recalc_drift":self.report.findings.iter().filter(|f| f.also_changed_by_original_recalc).count()});
        })
    }

    /// Kill the owned host job on failure and preserve partial evidence honestly.
    fn record_failure(&mut self, error: &anyhow::Error) -> Result<()> {
        self.excel.take();
        let identities = self.telemetry.owned_processes()?;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while identities
            .iter()
            .any(|(pid, started)| process_alive(i64::from(*pid), *started) != Some(false))
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        self.report.status = "failed".into();
        let message = format!("{error:#}");
        fs::write(
            self.args.output.join("failure.json"),
            serde_json::to_vec_pretty(&json!({"status":"failed", "error":message}))?,
        )?;
        self.telemetry.update(|state| {
            let mut unconfirmed = false;
            for lane in &mut state.lanes {
                if lane.xml_started_at.is_some() && lane.xml_finished_at.is_none() {
                    lane.xml_finished_at = Some(chrono::Utc::now().timestamp_millis());
                }
                for step in lane.steps.values_mut() {
                    if *step == StepStatus::Running {
                        *step = StepStatus::Failed;
                    }
                }
                if let (Some(pid), Some(started)) = (lane.pid, lane.process_started_at) {
                    lane.process_alive = process_alive(i64::from(pid), started);
                    if lane.process_alive == Some(false) {
                        lane.lifecycle = Lifecycle::Closed;
                        lane.closed_at
                            .get_or_insert(chrono::Utc::now().timestamp_millis());
                    } else {
                        unconfirmed = true;
                        lane.lifecycle = Lifecycle::Failed;
                    }
                } else if lane.lifecycle != Lifecycle::Unavailable {
                    lane.lifecycle = Lifecycle::NotStarted;
                }
            }
            state.status = if unconfirmed {
                "cleanup_unconfirmed"
            } else {
                "failed"
            }
            .into();
            state.finished_at = (!unconfirmed).then(|| chrono::Utc::now().timestamp_millis());
            state.error = Some(message);
        })
    }
}

/// Scientific coverage limits apply independently of the display or process health.
fn new_report() -> Report {
    Report { status: "running".into(), check_performance: "unavailable_not_tested", candidate_count: 0,
        findings: Vec::new(), phases: Vec::new(), excel_pids: BTreeMap::new(), capture_counts: BTreeMap::new(),
        limitations: vec![
            "Only changed cell XML is selected; drawing, chart, style and worksheet geometry changes are not yet exhaustively discovered.".into(),
            "At most 64 distinct candidate regions; larger cases fail explicitly.".into(),
            "Excel may recalculate during open even when manual calculation is requested.".into(),
            "Save-only XML changes are included; changes introduced only by rebuilding and saved-versus-reopened rendering are not checked.".into(),
            "Check Performance is unavailable in this comparison workflow.".into(),
            "The bounded cell index rejects worksheet XML over 64 MiB or 250,000 cells; this command is not a replacement for the corpus runner.".into(),
        ] }
}
