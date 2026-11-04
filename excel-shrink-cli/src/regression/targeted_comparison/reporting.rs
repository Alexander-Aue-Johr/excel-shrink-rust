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
        self.record_workbook_sizes()?;
        self.report.status = "completed_with_limitations".into();
        self.report.image_comparison_statistics = self.image_comparisons.statistics();
        self.report.xml_snapshot_statistics = self.snapshots.statistics();
        self.write_completion()
    }

    /// Retain file sizes before the archive worker removes disposable Excel states.
    fn record_workbook_sizes(&mut self) -> Result<()> {
        for (label, path) in [
            ("input_original", &self.args.original),
            ("input_default", &self.args.default),
        ] {
            self.report
                .workbook_bytes
                .insert(label.into(), fs::metadata(path)?.len());
        }
        for entry in fs::read_dir(&self.args.output)? {
            let entry = entry?;
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "xlsx")
                && entry.file_type()?.is_file()
            {
                self.report.workbook_bytes.insert(
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.metadata()?.len(),
                );
            }
        }
        Ok(())
    }

    /// Retain explicit limitations and report only comparisons actually performed.
    fn write_completion(&self) -> Result<()> {
        fs::write(
            self.args.output.join("report.json"),
            serde_json::to_vec(&self.report)?,
        )?;
        self.telemetry.update(|state| {
            state.status = self.report.status.clone();
            state.finished_at = Some(chrono::Utc::now().timestamp_millis());
            state.report = self.report.telemetry_summary();
        })
    }

    /// Kill the owned host job on failure and preserve partial evidence honestly.
    pub(super) fn record_failure(&mut self, error: &anyhow::Error) -> Result<()> {
        if let Some(host) = &self.excel {
            fs::write(self.args.output.join("operations.json"), serde_json::to_vec_pretty(&host.operation_measurements)?)?;
        }
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
            serde_json::to_vec(&self.report)?,
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
                } else if !matches!(lane.lifecycle, Lifecycle::Unavailable | Lifecycle::Skipped) {
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
    pub(super) fn telemetry_summary(&self) -> Value {
        use super::telemetry::LaneId;
        let count = |left, right| {
            self.comparisons
                .iter()
                .find_map(|pair| pair.region_differences(left, right))
        };
        json!({
            "status": self.status,
            "input_identity":self.input_identity,
            "state_decisions":self.state_decisions,
            "check_performance": self.check_performance,
            "check_performance_cache": self.check_performance_cache,
            "check_performance_differences": count(LaneId::O, LaneId::CP),
            "limitations": self.limitations,
            "cf_excluded_decisions": self.conditional_formatting.iter().flat_map(|check| &check.decisions).filter(|decision| decision.outcome == "excluded").count(),
            "cf_partial_decisions": self.conditional_formatting.iter().flat_map(|check| &check.decisions).filter(|decision| decision.outcome == "partial").count(),
            "cf_capture_decisions": self.conditional_formatting.iter().flat_map(|check| &check.decisions).filter(|decision| decision.outcome == "capture").count(),
            "chart_comparisons": self.comparisons.iter().map(|pair| pair.charts.len()).sum::<usize>(),
            "chart_differences": self.comparisons.iter().flat_map(|pair| &pair.charts)
                .filter(|chart| chart.presence_differs || chart.pixels_differ || chart.geometry_differs).count(),
            "before_differences": count(LaneId::O, LaneId::SD).or_else(|| self.input_identity.identical().then_some(0)),
            "after_differences": count(LaneId::O, LaneId::SDR),
            "original_recalc_drift": count(LaneId::O, LaneId::OR),
        })
    }
}

/// Scientific coverage limits apply independently of the display or process health.
pub(super) fn new_report() -> Report {
    Report {
        workbook_bytes: BTreeMap::new(),
        capture_policy: super::CAPTURE_POLICY,
        execution_strategy: "default-inplace-rebuild-v1",
        capture_partition: "visible-geometry-v1",
        input_identity: Default::default(),
        state_decisions: BTreeMap::new(),
        rebuild_checks: BTreeMap::new(),
        timings_ms: BTreeMap::new(),
        image_comparison_statistics: Default::default(),
        xml_snapshot_statistics: Default::default(),
        status: "running".into(),
        check_performance: "not_requested",
        check_performance_cache: Value::Null,
        control_artifacts_cache: Value::Null,
        check_performance_evidence: None,
        transitions: Vec::new(),
        height_checks: Vec::new(),
        conditional_formatting: Vec::new(),
        comparisons: Vec::new(),
        candidate_count: 0,
        content_coverage: Vec::new(),
        findings: Vec::new(),
        phases: Vec::new(),
        excel_pids: BTreeMap::new(),
        capture_counts: BTreeMap::new(),
        limitations: vec![
            "Gridlines are disabled consistently. A trailing blank margin geometry change is ignored only when both captures have the same measured content core and all top-left-aligned nonblank pixels match. Raw geometry and images remain in the report; blank_margin_ignored records this decision. Explicit borders, overflow ink and changes to core geometry remain differences.".into(),
            "Row-height checks cover rows above and within the union of content envelopes, including one margin row. Formatting-only tails below that envelope are excluded, not certified equal; charts retain independent full-chart observations.".into(),
            "Strategy default-inplace-rebuild-v1 compares Original with SD before and after rebuilding the same SD instance. Original and optional Check Performance are not explicitly rebuilt. The SD pre-rebuild comparison retains its phase-specific region selection; regions first discovered after SD rebuild have no SD-before image or volatility/causal classification.".into(),
            "Cell capture regions are intersected with the union of content-bearing cell bounds across raw, saved and rebuilt inputs, plus one row/column of margin. Formatting-only exterior regions are intentionally skipped, not visually confirmed identical; content_coverage records selection counts and bounds. Chart exports are independent of this policy.".into(),
            "Capture partition visible-geometry-v1 merges adjacent internal grid tiles without changing their union. It measures every live state and splits only for image budgets (30,000 pixels per edge, 32 million pixels total), estimating conservatively at two pixels per point plus a rounding margin. Actual exports are checked too. Zero-area ranges retain geometry and explicit non-rendering observations without images or pixel equality claims. Later states can repartition and backfill earlier retained workbooks.".into(),
            "CF decisions concern rule contributions within the recorded union of comparison extents. Literal cellIs exclusions require identical cell-XML fingerprints and visual context. Formula inputs remain captures; unknown or changed rules retain region captures. This is not a proof of whole-workbook visual equality.".into(),
            "Embedded charts and standalone chart sheets are compared as whole-chart exports in every retained state. Chart-sheet exports use a consistent viewport in the existing owned Excel process. Other drawings, style and worksheet geometry changes are not yet exhaustively discovered.".into(),
            "Charts with measured zero width or height retain geometry and an explicit zero-area-chart observation without a pixel verdict. A transition between visible and zero-area states is a rendering/geometry difference.".into(),
            "Excel may recalculate during open even when manual calculation is requested.".into(),
            "Opening/saving and the in-place SD rebuild are retained as separate XML controls. Exact content/context matches identify overlaps conservatively; absence of a match does not establish exclusive causality.".into(),
            "Worksheet cells are streamed into temporary fixed-size fingerprints without worksheet-byte or cell-count limits. Exterior empty-cell changes are counted before candidate allocation. Layout metadata and retained change evidence still require memory.".into(),
        ],
    }
}
