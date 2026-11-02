//! Targeted comparison workflow; the called phases own XML, Excel, and evidence details.
mod candidates;
mod content_bounds;
mod cell_stream;
mod charts;
mod conditional_formatting;
mod check_performance;
mod evidence;
mod image_comparison;
mod change_evidence;
mod heights;
mod host;
mod reporting;
mod staged;
mod telemetry;
mod workbooks;
mod xml_changes;
pub(super) mod prefix_inspection;

use anyhow::{ensure, Result};
use candidates::{discover_candidates, find_changes_caused_by_save, SaveChanges};
use clap::Parser;
use evidence::{compare_evidence, Evidence};
use host::ExcelHost;
use reporting::new_report;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use telemetry::{LaneId, Phase, StepStatus, Telemetry};

pub(super) const CAPTURE_POLICY: &str = "targeted-content-grid-v2";
pub(super) const CAPTURE_ROWS: u32 = 128;
pub(super) const CAPTURE_COLUMNS: u32 = 16;

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
    /// Omit the native Check Performance lane (otherwise requires a private desktop).
    #[arg(long)]
    skip_check_performance: bool,
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
    capture_policy: &'static str,
    image_comparison_statistics: image_comparison::Statistics,
    status: String,
    check_performance: &'static str,
    check_performance_evidence: Option<check_performance::PerformanceEvidence>,
    transitions: Vec<xml_changes::Transition>,
    height_checks: Vec<heights::HeightCheck>,
    conditional_formatting: Vec<conditional_formatting::Check>,
    comparisons: Vec<staged::VisualComparison>,
    candidate_count: usize,
    content_coverage: Vec<content_bounds::Coverage>,
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
    performance: check_performance::PerformanceState,
    image_comparisons: image_comparison::ImageComparisons,
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
    if let Err(error) = comparison.execute_staged_comparison() {
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
        if !args.skip_check_performance {
            ensure!(std::env::var("EXCEL_SHRINK_EXPECTED_DESKTOP")
                .is_ok_and(|desktop| !desktop.is_empty() && !desktop.eq_ignore_ascii_case("Default")),
                "Check Performance requires the private-desktop launcher; use --skip-check-performance to omit it");
        }
        let paths = WorkingPaths::prepare(args)?;
        let telemetry = Telemetry::start(args)?;
        Ok(Self {
            args,
            paths,
            report: {
                let mut report = new_report();
                if !args.skip_check_performance {
                    report.check_performance = "pending";
                }
                report
            },
            telemetry,
            candidates: BTreeSet::new(),
            original_save_changes: BTreeSet::new(),
            captures: Captures::default(),
            excel: None,
            performance: check_performance::PerformanceState::default(),
            image_comparisons: image_comparison::ImageComparisons::default(),
        })
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
        self.start_excel_host()?;
        for id in [LaneId::O, LaneId::SD] {
            self.open_workbook(id)?;
        }
        self.report.phases.push("original and shrink opened".into());
        Ok(())
    }

    /// Separate ordinary save effects from the later explicit rebuild controls.
    fn inspect_save_effects(&mut self) -> Result<()> {
        let before_save = self.paths.preserve_before_save()?;
        self.save_workbooks()?;
        self.telemetry
            .update(|state| state.pair_step(Phase::InspectSave, StepStatus::Running))?;

        let save_changes = find_changes_caused_by_save(&before_save, &self.paths)?;
        self.include_save_changes(save_changes)?;

        self.report
            .phases
            .push("Excel save effects inspected".into());
        self.telemetry.update(|state| {
            state.candidates(self.candidates.len());
            state.pair_step(Phase::InspectSave, StepStatus::Done);
        })
    }

    /// Keep Original's save drift separate while extending the capture selection.
    fn include_save_changes(&mut self, changes: SaveChanges) -> Result<()> {
        self.original_save_changes = changes.original;
        self.candidates
            .extend(self.original_save_changes.iter().cloned());
        self.candidates.extend(changes.default);
        Ok(())
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
            &mut self.image_comparisons,
        )?;
        self.report.candidate_count = self.candidates.len();
        self.report
            .phases
            .push("images and rendered geometry compared".into());
        self.telemetry
            .update(|state| state.pair_step(Phase::Compare, StepStatus::Done))
    }
}
