//! Preserve live pre-rebuild states until every late candidate has matching evidence.
use super::{
    evidence::{capture_candidate, images_differ, Evidence},
    heights::{self, HeightCheck, HeightRun},
    telemetry::{LaneId, Lifecycle, Phase, StepStatus},
    xml_changes::{cell_candidates, Snapshot, Transition},
    Candidate, Comparison, MAX_CANDIDATES,
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

struct WorkbookState {
    path: PathBuf,
    xml: Snapshot,
    captures: BTreeMap<Candidate, Evidence>,
    heights: BTreeMap<(String, Vec<(u32, u32)>), Vec<HeightRun>>,
}

#[derive(Serialize)]
pub(super) struct VisualFinding {
    sheet: String,
    cell: String,
    pixels_differ: bool,
    geometry_differs: bool,
    left_png: PathBuf,
    right_png: PathBuf,
}

#[derive(Serialize)]
pub(super) struct VisualComparison {
    pub label: String,
    left: LaneId,
    right: LaneId,
    findings: Vec<VisualFinding>,
}

struct Workflow<'a, 'b> {
    comparison: &'a mut Comparison<'b>,
    states: BTreeMap<LaneId, WorkbookState>,
    inspected: BTreeSet<(LaneId, LaneId)>,
}

impl Comparison<'_> {
    /// Complete direct and variant-rebuild comparisons before starting Original's rebuild copy.
    pub(super) fn execute_staged_comparison(&mut self) -> Result<()> {
        Workflow {
            comparison: self,
            states: BTreeMap::new(),
            inspected: BTreeSet::new(),
        }
        .execute()
    }
}

impl Workflow<'_, '_> {
    /// Each call advances one observable domain phase; retained states make late discovery safe.
    fn execute(&mut self) -> Result<()> {
        self.open_and_save_controls()?;
        self.optimize_and_register_states()?;
        self.inspect_available_pairs()?;
        self.capture_missing_regions()?;
        self.publish_available_comparisons("before rebuild")?;
        self.rebuild_variant_copies()?;
        self.publish_available_comparisons("variants rebuilt; Original still unchanged")?;
        self.rebuild_copy(LaneId::O, LaneId::OR)?;
        self.publish_available_comparisons("Original rebuilt last")?;
        self.project_legacy_findings()?;
        self.annotate_overlapping_changes();
        self.close_states()?;
        self.comparison.complete()
    }

    fn open_and_save_controls(&mut self) -> Result<()> {
        self.comparison.inspect_initial_xml()?;
        self.comparison.open_workbooks()?;
        self.comparison.inspect_save_effects()?;
        let raw_original = self.comparison.args.original.clone();
        let raw_default = self.comparison.args.default.clone();
        let original = self.comparison.paths.original.clone();
        let default = self.comparison.paths.default.clone();
        self.audit(
            "default input versus original input",
            &raw_original,
            &raw_default,
        )?;
        self.audit("original opening/saving control", &raw_original, &original)?;
        self.audit("default opening/saving control", &raw_default, &default)?;
        self.register(LaneId::O, original)?;
        self.register(LaneId::SD, default)
    }

    fn optimize_and_register_states(&mut self) -> Result<()> {
        self.comparison.optimize_performance_copy()?;
        if !self.comparison.args.skip_check_performance {
            let output = &self.comparison.args.output;
            let original = self.comparison.paths.original.clone();
            let before = output.join("check-performance.before-optimization.xlsx");
            let after = output.join("check-performance.after-optimization.xlsx");
            self.audit(
                "Check Performance copy opening/saving control",
                &original,
                &before,
            )?;
            self.audit("Check Performance optimization", &before, &after)?;
            self.register(
                LaneId::CP,
                self.comparison
                    .performance
                    .path
                    .clone()
                    .context("Missing CP path")?,
            )?;
        }
        Ok(())
    }

    fn register(&mut self, id: LaneId, path: PathBuf) -> Result<()> {
        self.comparison
            .telemetry
            .update(|state| state.step(id, Phase::Discover, StepStatus::Running))?;
        let xml = Snapshot::read(&path)?;
        self.comparison
            .telemetry
            .update(|state| state.step(id, Phase::Discover, StepStatus::Done))?;
        self.states.insert(
            id,
            WorkbookState {
                path,
                xml,
                captures: BTreeMap::new(),
                heights: BTreeMap::new(),
            },
        );
        Ok(())
    }

    fn audit(&mut self, label: &str, before: &Path, after: &Path) -> Result<()> {
        let a = Snapshot::read(before)?;
        let b = Snapshot::read(after)?;
        let changes = a.changes(&b);
        self.comparison
            .candidates
            .extend(cell_candidates(&a, &b, &changes));
        self.check_candidate_limit()?;
        self.comparison.report.transitions.push(Transition {
            label: label.into(),
            before_file: before.into(),
            after_file: after.into(),
            changes,
        });
        Ok(())
    }

    fn check_candidate_limit(&mut self) -> Result<()> {
        ensure!(
            self.comparison.candidates.len() <= MAX_CANDIDATES,
            "More than {MAX_CANDIDATES} candidate regions; no complete targeted result established"
        );
        self.comparison.report.candidate_count = self.comparison.candidates.len();
        self.comparison
            .telemetry
            .update(|state| state.candidates(self.comparison.candidates.len()))
    }

    fn rebuild_variant_copies(&mut self) -> Result<()> {
        self.rebuild_copy(LaneId::SD, LaneId::SDR)?;
        if !self.comparison.args.skip_check_performance {
            self.rebuild_copy(LaneId::CP, LaneId::CPR)?;
        }
        Ok(())
    }

    /// Rebuild an independent copy so every original pre-state remains available for late captures.
    fn rebuild_copy(&mut self, parent: LaneId, id: LaneId) -> Result<()> {
        let source = self.states[&parent].path.clone();
        let path = self
            .comparison
            .args
            .output
            .join(format!("{}-rebuilt.xlsx", id.key()));
        fs::copy(&source, &path)?;
        self.comparison.telemetry.update(|state| {
            state.lane(id).lifecycle = Lifecycle::Opening;
            state.step(id, Phase::Open, StepStatus::Running);
        })?;
        let pid = self
            .comparison
            .excel
            .as_mut()
            .context("Excel host missing")?
            .open_workbook(id, &path, &self.comparison.telemetry)?;
        self.comparison
            .report
            .excel_pids
            .insert(id.key().into(), pid);
        self.comparison.telemetry.update(|state| {
            state.lane(id).lifecycle = Lifecycle::Open;
            state.step(id, Phase::Open, StepStatus::Done);
            state.step(id, Phase::Save, StepStatus::Running);
        })?;
        self.comparison
            .excel
            .as_mut()
            .unwrap()
            .save_workbook(id, &self.comparison.telemetry)?;
        let before = path.with_extension("before-rebuild.xlsx");
        fs::copy(&path, &before)?;
        self.comparison
            .telemetry
            .update(|state| state.step(id, Phase::Save, StepStatus::Done))?;
        self.audit(
            &format!("{} rebuild-copy opening/saving control", parent.key()),
            &source,
            &before,
        )?;
        self.comparison
            .telemetry
            .update(|state| state.step(id, Phase::InspectSave, StepStatus::Done))?;
        self.comparison.rebuild(id)?;
        self.audit(
            &format!("{} explicit rebuild and save", parent.key()),
            &before,
            &path,
        )?;
        self.register(id, path)?;
        self.inspect_available_pairs()?;
        self.capture_missing_regions()?;
        self.comparison.report.phases.push(format!(
            "{} rebuilt on retained copy; late candidates captured in all available states",
            parent.key()
        ));
        Ok(())
    }

    fn inspect_available_pairs(&mut self) -> Result<()> {
        let pairs = self.pairs();
        for (_, left, right) in pairs {
            if !self.inspected.insert((left, right)) {
                continue;
            }
            let a = self.states[&left].xml.clone();
            let b = self.states[&right].xml.clone();
            ensure!(
                a.sheets.keys().eq(b.sheets.keys()),
                "Sheet set changed; targeted coverage is incomplete"
            );
            let changes = a.changes(&b);
            self.comparison
                .candidates
                .extend(cell_candidates(&a, &b, &changes));
            self.check_candidate_limit()?;
            self.comparison.telemetry.update(|state| {
                state.step(left, Phase::MeasureHeights, StepStatus::Running);
                state.step(right, Phase::MeasureHeights, StepStatus::Running);
            })?;
            for sheet in a.sheets.keys() {
                let (spans, fixed) = heights::selection(&a, &b, sheet)?;
                if spans.is_empty() && fixed == 0 {
                    continue;
                }
                let check = HeightCheck {
                    left: left.key().into(),
                    right: right.key().into(),
                    sheet: sheet.clone(),
                    checked_spans: spans.clone(),
                    unchanged_fixed_rows: fixed,
                    differences: Vec::new(),
                    complete: false,
                };
                let index = self.comparison.report.height_checks.len();
                self.comparison.report.height_checks.push(check);
                let before = self.read_heights(left, sheet, &spans)?;
                let after = self.read_heights(right, sheet, &spans)?;
                let selected =
                    self.comparison.report.height_checks[index].compare(&before, &after)?;
                self.comparison.candidates.extend(selected);
                self.check_candidate_limit()?;
            }
            self.comparison.telemetry.update(|state| {
                state.step(left, Phase::MeasureHeights, StepStatus::Done);
                state.step(right, Phase::MeasureHeights, StepStatus::Done);
            })?;
        }
        Ok(())
    }

    fn read_heights(
        &mut self,
        id: LaneId,
        sheet: &str,
        spans: &[(u32, u32)],
    ) -> Result<Vec<HeightRun>> {
        if spans.is_empty() {
            return Ok(Vec::new());
        }
        self.comparison.telemetry.update(|state| {
            state.lane(id).current_sheet = Some(sheet.into());
            state.lane(id).current_range = Some(format!(
                "row heights {}..{} ({} spans)",
                spans[0].0,
                spans[spans.len() - 1].1,
                spans.len()
            ));
        })?;
        let key = (sheet.into(), spans.to_vec());
        if let Some(cached) = self.states[&id].heights.get(&key) {
            return Ok(cached.clone());
        }
        let response = self
            .comparison
            .excel
            .as_mut()
            .context("Excel host missing")?
            .height_runs(id, sheet, spans, &self.comparison.telemetry)?;
        let runs: Vec<HeightRun> = serde_json::from_value(response["runs"].clone())?;
        heights::validate(&runs, spans)?;
        self.states
            .get_mut(&id)
            .unwrap()
            .heights
            .insert(key, runs.clone());
        Ok(runs)
    }

    /// Stable candidate-derived filenames prevent late insertions from overwriting earlier images.
    fn capture_missing_regions(&mut self) -> Result<()> {
        for (&id, workbook) in &mut self.states {
            let directory = self
                .comparison
                .args
                .output
                .join(format!("evidence-{}", id.key()));
            fs::create_dir_all(&directory)?;
            let phase = if matches!(id, LaneId::O | LaneId::SD | LaneId::CP) {
                Phase::CaptureBefore
            } else {
                Phase::CaptureAfter
            };
            self.comparison
                .telemetry
                .update(|state| state.step(id, phase, StepStatus::Running))?;
            for candidate in &self.comparison.candidates {
                if workbook.captures.contains_key(candidate) {
                    continue;
                }
                let name = format!(
                    "{:x}",
                    Sha256::digest(format!("{}\0{}", candidate.sheet, candidate.cell).as_bytes())
                );
                let evidence = capture_candidate(
                    self.comparison
                        .excel
                        .as_mut()
                        .context("Excel host missing")?,
                    id,
                    candidate,
                    directory.join(format!("{name}.png")),
                    &self.comparison.telemetry,
                )?;
                workbook.captures.insert(candidate.clone(), evidence);
                *self
                    .comparison
                    .report
                    .capture_counts
                    .entry(id.key().into())
                    .or_default() += 1;
                self.comparison
                    .telemetry
                    .update(|state| state.lane(id).captures_done += 1)?;
            }
            self.comparison
                .telemetry
                .update(|state| state.step(id, phase, StepStatus::Done))?;
        }
        Ok(())
    }

    fn pairs(&self) -> Vec<(&'static str, LaneId, LaneId)> {
        use LaneId::*;
        [
            ("direct Default", O, SD),
            ("direct Check Performance", O, CP),
            ("Default rebuilt versus Original before rebuild", O, SDR),
            (
                "Check Performance rebuilt versus Original before rebuild",
                O,
                CPR,
            ),
            ("Default own rebuild", SD, SDR),
            ("Check Performance own rebuild", CP, CPR),
            ("Original own rebuild", O, OR),
            ("both rebuilt Default", OR, SDR),
            ("both rebuilt Check Performance", OR, CPR),
        ]
        .into_iter()
        .filter(|(_, a, b)| self.states.contains_key(a) && self.states.contains_key(b))
        .collect()
    }

    fn publish_available_comparisons(&mut self, phase: &str) -> Result<()> {
        let mut comparisons = Vec::new();
        for (label, left, right) in self.pairs() {
            let mut findings = Vec::new();
            for candidate in &self.comparison.candidates {
                let a = self.states[&left]
                    .captures
                    .get(candidate)
                    .context("Missing left evidence")?;
                let b = self.states[&right]
                    .captures
                    .get(candidate)
                    .context("Missing right evidence")?;
                findings.push(VisualFinding {
                    sheet: candidate.sheet.clone(),
                    cell: candidate.cell.clone(),
                    pixels_differ: images_differ(&a.path, &b.path)?,
                    geometry_differs: a.geometry != b.geometry,
                    left_png: a.path.clone(),
                    right_png: b.path.clone(),
                });
            }
            comparisons.push(VisualComparison {
                label: label.into(),
                left,
                right,
                findings,
            });
            self.comparison.telemetry.update(|state| {
                state.step(left, Phase::Compare, StepStatus::Done);
                state.step(right, Phase::Compare, StepStatus::Done);
            })?;
        }
        self.comparison.report.comparisons = comparisons;
        self.comparison.report.phases.push(phase.into());
        fs::write(
            self.comparison.args.output.join("partial-report.json"),
            serde_json::to_vec_pretty(&self.comparison.report)?,
        )?;
        Ok(())
    }

    fn project_legacy_findings(&mut self) -> Result<()> {
        self.comparison.captures.original_before = self.states[&LaneId::O].captures.clone();
        self.comparison.captures.shrink_before = self.states[&LaneId::SD].captures.clone();
        self.comparison.captures.shrink_after = self.states[&LaneId::SDR].captures.clone();
        self.comparison.captures.original_after = self.states[&LaneId::OR].captures.clone();
        self.comparison.compare_captured_evidence()?;
        if !self.comparison.args.skip_check_performance {
            self.comparison.performance.before = self.states[&LaneId::CP].captures.clone();
            self.comparison.performance.after = self.states[&LaneId::CPR].captures.clone();
            self.comparison.compare_performance_evidence()?;
        }
        Ok(())
    }

    fn annotate_overlapping_changes(&mut self) {
        let original = self
            .comparison
            .report
            .transitions
            .iter()
            .find(|t| t.label == "original opening/saving control")
            .map(|t| t.changes.clone())
            .unwrap_or_default();
        for transition in &mut self.comparison.report.transitions {
            for change in &mut transition.changes {
                change.also_in_original_open_save = original
                    .iter()
                    .any(|other| Snapshot::same_change(change, other));
            }
        }
    }

    fn close_states(&mut self) -> Result<()> {
        for id in self.states.keys().copied().rev().collect::<Vec<_>>() {
            self.comparison.close_workbook(id)?;
        }
        self.comparison.excel.take();
        self.comparison.telemetry.update(|state| {
            for lane in &mut state.lanes {
                for status in lane.steps.values_mut() {
                    if *status == StepStatus::Pending {
                        *status = StepStatus::Skipped;
                    }
                }
            }
        })?;
        Ok(())
    }
}
