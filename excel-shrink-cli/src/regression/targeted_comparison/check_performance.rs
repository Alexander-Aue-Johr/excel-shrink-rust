//! Native optimization is a separate lane, with its own save baseline and evidence.
use super::evidence::{compare_performance, Evidence, PerformanceFinding};
use super::telemetry::{LaneId, Phase, StepStatus};
use super::{discover_candidates, Candidate, Comparison, MAX_CANDIDATES};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

#[derive(Default)]
pub(super) struct PerformanceState {
    pub(super) path: Option<PathBuf>,
    save_changes: BTreeSet<Candidate>,
    optimization_changes: BTreeSet<Candidate>,
    before: BTreeMap<Candidate, Evidence>,
    after: BTreeMap<Candidate, Evidence>,
}

#[derive(Serialize)]
pub(super) struct PerformanceEvidence {
    native_result: Value,
    before_optimization: PathBuf,
    after_optimization: PathBuf,
    ordinary_save_changes: BTreeSet<Candidate>,
    optimization_changes: BTreeSet<Candidate>,
    findings: Option<Vec<PerformanceFinding>>,
}

impl PerformanceEvidence {
    pub(super) fn difference_count(&self) -> Option<usize> {
        self.findings
            .as_ref()
            .map(|findings| findings.iter().filter(|finding| finding.differs()).count())
    }
}

impl Comparison<'_> {
    /// Establish a saved baseline before invoking the native optimization action.
    pub(super) fn optimize_performance_copy(&mut self) -> Result<()> {
        if self.args.skip_check_performance {
            return Ok(());
        }
        let path = self.args.output.join("check-performance.xlsx");
        fs::copy(&self.paths.original, &path)
            .context("Copy saved Original for Check Performance")?;
        self.performance.path = Some(path.clone());
        self.open_workbook(LaneId::CP)?;
        self.save_performance_baseline(&path)?;
        self.run_performance_optimization(&path)?;
        self.include_performance_changes()
    }

    /// Attribute opening and ordinary saving separately from Optimize all.
    fn save_performance_baseline(&mut self, path: &std::path::Path) -> Result<()> {
        self.telemetry
            .update(|state| state.step(LaneId::CP, Phase::Save, StepStatus::Running))?;
        self.excel
            .as_mut()
            .context("Excel host not started")?
            .save_workbook(LaneId::CP, &self.telemetry)?;
        self.telemetry.update(|state| {
            state.step(LaneId::CP, Phase::Save, StepStatus::Done);
            state.step(LaneId::CP, Phase::InspectSave, StepStatus::Running);
        })?;
        self.performance.save_changes = discover_candidates(&self.paths.original, path)?;
        self.telemetry
            .update(|state| state.step(LaneId::CP, Phase::InspectSave, StepStatus::Done))
    }

    /// Retain before/after files and native confirmation before discovering optimization changes.
    fn run_performance_optimization(&mut self, path: &std::path::Path) -> Result<()> {
        let before = self
            .args
            .output
            .join("check-performance.before-optimization.xlsx");
        let after = self
            .args
            .output
            .join("check-performance.after-optimization.xlsx");
        fs::copy(path, &before)?;
        self.report.check_performance = "running";
        self.telemetry
            .update(|state| state.step(LaneId::CP, Phase::Optimize, StepStatus::Running))?;
        let native_result = self
            .excel
            .as_mut()
            .context("Excel host not started")?
            .optimize_workbook(&self.telemetry)?;
        self.excel
            .as_mut()
            .context("Excel host not started")?
            .save_workbook(LaneId::CP, &self.telemetry)?;
        fs::copy(path, &after)?;
        self.report.check_performance = if native_result["status"] == "optimization_confirmed" {
            "optimization_confirmed"
        } else {
            "no_optimization_needed"
        };
        // Preserve confirmation even if bounded XML discovery subsequently fails.
        self.report.check_performance_evidence = Some(PerformanceEvidence {
            native_result,
            before_optimization: before.clone(),
            after_optimization: after.clone(),
            ordinary_save_changes: self.performance.save_changes.clone(),
            optimization_changes: BTreeSet::new(),
            findings: None,
        });
        self.telemetry.update(|state| {
            state.step(LaneId::CP, Phase::Optimize, StepStatus::Done);
            state.step(LaneId::CP, Phase::InspectOptimization, StepStatus::Running);
        })?;
        self.performance.optimization_changes = discover_candidates(&before, &after)?;
        self.telemetry
            .update(|state| state.step(LaneId::CP, Phase::InspectOptimization, StepStatus::Done))
    }

    /// All lanes capture the union, so changes found only by Check Performance have controls too.
    fn include_performance_changes(&mut self) -> Result<()> {
        self.candidates
            .extend(self.performance.save_changes.iter().cloned());
        self.candidates
            .extend(self.performance.optimization_changes.iter().cloned());
        ensure!(
            self.candidates.len() <= MAX_CANDIDATES,
            "More than {MAX_CANDIDATES} changed cells; aborting bounded comparison"
        );
        self.report
            .check_performance_evidence
            .as_mut()
            .context("Missing optimization evidence")?
            .optimization_changes = self.performance.optimization_changes.clone();
        self.report
            .phases
            .push("Check Performance optimized and saved XML inspected".into());
        self.telemetry
            .update(|state| state.candidates(self.candidates.len()))
    }

    /// Capture the optimized copy before its explicit rebuild.
    pub(super) fn capture_performance_before(&mut self) -> Result<()> {
        if !self.args.skip_check_performance {
            self.performance.before =
                self.capture(LaneId::CP, Phase::CaptureBefore, "performance-before")?;
        }
        Ok(())
    }

    /// Rebuild this lane independently from Default and the original drift control.
    pub(super) fn rebuild_performance_and_capture(&mut self) -> Result<()> {
        if !self.args.skip_check_performance {
            self.rebuild(LaneId::CP)?;
            self.performance.after =
                self.capture(LaneId::CP, Phase::CaptureAfter, "performance-recalc")?;
        }
        Ok(())
    }

    /// Missing CP images remain an error, never an implicit identical result.
    pub(super) fn compare_performance_evidence(&mut self) -> Result<()> {
        if self.args.skip_check_performance {
            return Ok(());
        }
        self.telemetry
            .update(|state| state.step(LaneId::CP, Phase::Compare, StepStatus::Running))?;
        let findings = compare_performance(
            &self.candidates,
            &self.captures.original_before,
            &self.performance.before,
            &self.performance.after,
            &self.captures.original_after,
            &self.performance.save_changes,
            &self.performance.optimization_changes,
        )?;
        self.report
            .check_performance_evidence
            .as_mut()
            .context("Missing optimization evidence")?
            .findings = Some(findings);
        self.telemetry
            .update(|state| state.step(LaneId::CP, Phase::Compare, StepStatus::Done))
    }
}
