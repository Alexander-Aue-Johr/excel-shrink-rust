//! Persist scientific outcomes separately from compact display telemetry.
use super::telemetry::{Lifecycle, StepStatus};
use super::{Comparison, Report};
use crate::native_process::process_alive;
use anyhow::Result;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs};

impl Comparison<'_> {
    /// Persist the scientific result separately from compact display telemetry.
    pub(super) fn complete(&mut self) -> Result<()> {
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
            state.report = self.report.telemetry_summary();
        })
    }

    /// Kill the owned host job on failure and preserve partial evidence honestly.
    pub(super) fn record_failure(&mut self, error: &anyhow::Error) -> Result<()> {
        self.release_excel_and_wait_for_exit()?;
        self.report.status = "failed".into();
        self.report.check_performance = match self.report.check_performance {
            "pending" => "not_run",
            "running" => "failed_unconfirmed",
            outcome => outcome,
        };
        self.report.candidate_count = self.candidates.len();
        fs::write(
            self.args.output.join("partial-report.json"),
            serde_json::to_vec_pretty(&self.report)?,
        )?;
        let message = format!("{error:#}");
        fs::write(
            self.args.output.join("failure.json"),
            serde_json::to_vec_pretty(&json!({"status":"failed", "error":message}))?,
        )?;
        self.publish_failed_state(message)
    }

    /// Reconcile failed steps with the latest verified process observations.
    fn publish_failed_state(&self, message: String) -> Result<()> {
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
            state.report = self.report.telemetry_summary();
        })
    }
}

impl Report {
    /// Project the saved findings into the existing Observatory summary schema.
    fn telemetry_summary(&self) -> Value {
        json!({
            "status": self.status,
            "check_performance": self.check_performance,
            "check_performance_differences": self.check_performance_evidence.as_ref().and_then(|e| e.difference_count()),
            "limitations": self.limitations,
            "before_differences": self.findings.iter()
                .filter(|finding| finding.before_recalc_differs || finding.before_recalc_geometry_differs)
                .count(),
            "after_differences": self.findings.iter()
                .filter(|finding| finding.after_shrink_recalc_differs || finding.after_shrink_recalc_geometry_differs)
                .count(),
            "original_recalc_drift": self.findings.iter()
                .filter(|finding| finding.also_changed_by_original_recalc)
                .count(),
        })
    }
}

/// Scientific coverage limits apply independently of the display or process health.
pub(super) fn new_report() -> Report {
    Report {
        status: "running".into(),
        check_performance: "not_requested",
        check_performance_evidence: None,
        transitions: Vec::new(),
        height_checks: Vec::new(),
        comparisons: Vec::new(),
        candidate_count: 0,
        findings: Vec::new(),
        phases: Vec::new(),
        excel_pids: BTreeMap::new(),
        capture_counts: BTreeMap::new(),
        limitations: vec![
            "Only changed cell XML is selected; drawing, chart, style and worksheet geometry changes are not yet exhaustively discovered.".into(),
            "Excel may recalculate during open even when manual calculation is requested.".into(),
            "Opening/saving and rebuild-copy reopening are retained as separate XML controls. Exact content/context matches identify overlaps conservatively; absence of a match does not establish exclusive causality.".into(),
            "Worksheet cells are streamed into temporary fixed-size fingerprints without worksheet-byte or cell-count limits. Layout metadata and actual change evidence still require memory; this command is not a replacement for the corpus runner.".into(),
        ],
    }
}
