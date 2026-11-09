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
            fs::write(
                self.args.output.join("operations.json"),
                serde_json::to_vec_pretty(&host.operation_measurements)?,
            )?;
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
            "chart_capture_mode":self.chart_capture_mode,
            "check_performance": self.check_performance,
            "check_performance_cache": self.check_performance_cache,
            "check_performance_differences": count(LaneId::OR, LaneId::CPR),
            "limitations": self.limitations,
            "cf_excluded_decisions": self.conditional_formatting.iter().flat_map(|check| &check.decisions).filter(|decision| decision.outcome == "excluded").count(),
            "cf_partial_decisions": self.conditional_formatting.iter().flat_map(|check| &check.decisions).filter(|decision| decision.outcome == "partial").count(),
            "cf_capture_decisions": self.conditional_formatting.iter().flat_map(|check| &check.decisions).filter(|decision| decision.outcome == "capture").count(),
            "chart_comparisons": self.comparisons.iter().flat_map(|pair| &pair.charts).filter(|chart| chart.pixels_compared).count(),
            "chart_dependency_exclusions": self.comparisons.iter().flat_map(|pair| &pair.charts).filter(|chart| chart.dependencies_unchanged).count(),
            "chart_differences": self.comparisons.iter().flat_map(|pair| &pair.charts)
                .filter(|chart| chart.presence_differs || chart.pixels_differ || chart.geometry_differs).count(),
            "before_differences": count(LaneId::O, LaneId::SD).or_else(|| self.input_identity.identical().then_some(0)),
            "after_differences": count(LaneId::OR, LaneId::SDR),
            "original_recalc_drift": count(LaneId::O, LaneId::OR),
            "xml_audits": self.transitions.iter().filter_map(|transition| {
                transition.timings_ms.as_ref().map(|timings| json!({
                    "label":transition.label,
                    "change_count":transition.change_count,
                    "timings_ms":timings,
                }))
            }).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod audit_timing_tests {
    use super::*;
    use crate::regression::targeted_comparison::xml_changes::{AuditTimings, Transition};

    #[test]
    fn audit_summary_exposes_measured_durations_without_paths_or_invented_history() {
        let mut report = new_report();
        report.transitions.push(Transition {
            label: "saved original".into(),
            before_file: "private-before.xlsx".into(),
            after_file: "private-after.xlsx".into(),
            changes_file: "private-audit.jsonl.gz".into(),
            change_count: 12,
            skipped_outside_content_cell_changes: BTreeMap::new(),
            timings_ms: None,
        });
        assert_eq!(report.telemetry_summary()["xml_audits"], json!([]));
        report.transitions[0].timings_ms = Some(AuditTimings {
            read_snapshots: 10,
            differences: 20,
            attribution: 30,
            write_evidence: 40,
            total: 110,
        });
        let summary = report.telemetry_summary();
        assert_eq!(summary["xml_audits"][0]["change_count"], 12);
        assert_eq!(summary["xml_audits"][0]["timings_ms"]["write_evidence"], 40);
        assert_eq!(summary["xml_audits"][0]["timings_ms"]["total"], 110);
        assert!(!summary.to_string().contains("private-"));
    }
}

/// Scientific coverage limits apply independently of the display or process health.
pub(super) fn new_report() -> Report {
    Report {
        workbook_bytes: BTreeMap::new(),
        capture_policy: super::CAPTURE_POLICY,
        execution_strategy: super::EXECUTION_STRATEGY,
        capture_partition: "visible-geometry-v2",
        chart_capture_mode: Default::default(),
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
        chart_dependencies: Vec::new(),
        native_drawing_bounds: Vec::new(),
        comparisons: Vec::new(),
        formula_diagnostics: Value::Null,
        candidate_count: 0,
        content_coverage: Vec::new(),
        findings: Vec::new(),
        phases: Vec::new(),
        excel_pids: BTreeMap::new(),
        capture_counts: BTreeMap::new(),
        limitations: vec![
            "Gridlines are disabled consistently. Policy v10 compares only common pixels when measured capture/content origins match and content cores are contained. capture_extent_ignored records omitted right/bottom extent and retains both raw images and geometries. Changed overlapping pixels remain differences. Shifted origins, missing measurements and charts retain conservative verdicts. blank_margin_ignored records the stronger exact white-margin proof.".into(),
            "Row-height checks cover rows above and within the unpadded union of content envelopes, including potentially visible fill/border/conditional-formatting extents. Context margin rows below content are excluded from height dependencies; captures retain their surrounding context. Charts retain independent full-chart observations.".into(),
            "Strategy paired-inplace-rebuild-v1 compares rebuilt Original against rebuilt Default and optional rebuilt Check Performance. Each reuses its existing Excel process. Pre-rebuild captures are omitted for throughput; XML save/optimization/rebuild controls are retained. Volatility and exclusive causal attribution require follow-up.".into(),
            "Cell capture regions are intersected with the union of content-bearing cells and potentially visible fill/border/conditional-formatting extents across raw, saved and rebuilt inputs, plus one row/column of margin. Exterior regions outside that envelope are intentionally skipped, not visually confirmed identical; content_coverage records selection counts and bounds. Chart exports are independent of this policy.".into(),
            "Capture partition visible-geometry-v2 merges adjacent internal grid tiles without changing their union. It measures every live state and splits only for image budgets (30,000 pixels per edge, 32 million pixels total), estimating conservatively at two pixels per point plus a rounding margin. Actual exports are checked too. Zero-area ranges retain geometry and explicit non-rendering observations without images or pixel equality claims. Later states can repartition and backfill earlier retained workbooks.".into(),
            "CF decisions concern rule contributions within the recorded union of comparison extents. Literal cellIs exclusions require identical cell-XML fingerprints and visual context. Formula inputs remain captures; unknown or changed rules retain region captures. This is not a proof of whole-workbook visual equality.".into(),
            "Chart mode deterministic excludes only embedded charts with complete unchanged package, XML/cache, source-cell/style/layout/CF and live-geometry dependencies. Exclusions are not pixel comparisons. Unknown references, external or unsupported parts, ambiguous names and chart sheets retain full-chart exports. Mode conservative exports every chart. Other drawings and all worksheet geometry effects are not exhaustively discovered.".into(),
            "Charts with measured zero width or height retain geometry and an explicit zero-area-chart observation without a pixel verdict. A transition between visible and zero-area states is a rendering/geometry difference.".into(),
            "Disposable targeted copies are staged with calcMode=manual, fullCalcOnLoad=0, forceFullCalc=0 and calcOnSave=0 before Excel opens them. Cached formula values are retained until each resident workbook is explicitly rebuilt. Excel may still autonomously refresh content; observed save/rebuild transitions remain audited. Original inputs and prepared bundles are unchanged. This suppresses unintended open/save recalculation and changes the opening baseline relative to capture policy v4.".into(),
            "Opening/saving and the in-place SD rebuild are retained as separate XML controls. Exact content/context matches identify overlaps conservatively; absence of a match does not establish exclusive causality.".into(),
            "Worksheet cells are streamed into temporary fixed-size fingerprints without worksheet-byte or cell-count limits. Exterior empty-cell changes are counted before candidate allocation. Layout metadata and retained change evidence still require memory.".into(),
            "When content coverage marks background_tail_sampling, supported borderless solid-fill bands with known sheet-wide gridline settings are represented through their finite transitions and tail samples. Pixels beyond that envelope are not captured; this is not exhaustive whole-grid pixel coverage.".into(),
        ],
    }
}
