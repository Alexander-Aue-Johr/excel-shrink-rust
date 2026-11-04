//! Compare live Original with Default before and after rebuilding the same Default instance.
use super::{
    capture_layout::{self, Geometry},
    charts,
    evidence::Evidence,
    execution_plan::StateDecision,
    heights::{self, HeightCheck, HeightRun},
    telemetry::{LaneId, Lifecycle, Phase, StepStatus},
    xml_changes::{cell_candidates, Snapshot, Transition},
    Candidate, Comparison,
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

struct WorkbookState {
    path: PathBuf,
    xml: Snapshot,
    captures: BTreeMap<Candidate, Evidence>,
    geometries: BTreeMap<Candidate, Geometry>,
    charts: charts::Captures,
    heights: BTreeMap<(String, Vec<(u32, u32)>), Vec<HeightRun>>,
}

#[derive(Serialize)]
pub(super) struct VisualFinding {
    sheet: String,
    cell: String,
    pixels_differ: bool,
    geometry_differs: bool,
    blank_margin_ignored: bool,
    pixels_compared: bool,
    rendering_differs: bool,
    left_rendering: &'static str,
    right_rendering: &'static str,
    left_geometry: serde_json::Value,
    right_geometry: serde_json::Value,
    left_png: Option<PathBuf>,
    right_png: Option<PathBuf>,
}

#[derive(Serialize)]
pub(super) struct VisualComparison {
    pub label: String,
    left: LaneId,
    right: LaneId,
    findings: Vec<VisualFinding>,
    selected_region_count: usize,
    pub charts: Vec<charts::Finding>,
}

impl VisualComparison {
    /// Summary counters refer only to an actually observed pair, never omitted states.
    pub(super) fn region_differences(&self, left: LaneId, right: LaneId) -> Option<usize> {
        (self.left == left && self.right == right).then(|| {
            self.findings
                .iter()
                .filter(|finding| {
                    finding.pixels_differ || finding.geometry_differs || finding.rendering_differs
                })
                .count()
        })
    }
}

struct Workflow<'a, 'b> {
    comparison: &'a mut Comparison<'b>,
    states: BTreeMap<LaneId, WorkbookState>,
    inspected: BTreeSet<(LaneId, LaneId)>,
    requested_candidates: BTreeSet<Candidate>,
    content_bounds: super::content_bounds::ContentBounds,
    original_save_keys: BTreeSet<[u8; 32]>,
}

impl Comparison<'_> {
    /// Compare two SD phases without reopening SD or explicitly rebuilding Original.
    pub(super) fn execute_staged_comparison(&mut self) -> Result<()> {
        Workflow {
            comparison: self,
            states: BTreeMap::new(),
            inspected: BTreeSet::new(),
            requested_candidates: BTreeSet::new(),
            content_bounds: super::content_bounds::ContentBounds::default(),
            original_save_keys: BTreeSet::new(),
        }
        .execute()
    }
}

impl Workflow<'_, '_> {
    /// Each call advances one observable domain phase; retained states make late discovery safe.
    fn execute(&mut self) -> Result<()> {
        self.open_and_save_controls()?;
        self.optimize_and_register_states()?;
        self.requested_candidates
            .extend(std::mem::take(&mut self.comparison.candidates));
        self.rebuild_in_place(LaneId::O, LaneId::OR)?;
        self.rebuild_in_place(LaneId::SD, LaneId::SDR)?;
        self.rebuild_in_place(LaneId::CP, LaneId::CPR)?;
        self.inspect_available_pairs()?;
        self.capture_missing_regions()?;
        self.publish_available_comparisons("Original and variants rebuilt in their existing processes")?;
        self.close_states()?;
        self.project_legacy_findings()?;
        self.comparison.complete()
    }

    fn open_and_save_controls(&mut self) -> Result<()> {
        self.comparison.inspect_initial_xml()?;
        let include_default = !self.comparison.report.input_identity.identical();
        if !include_default {
            self.skip_state(
                LaneId::SD,
                StateDecision::SkippedIdenticalInput { source: LaneId::O },
                "Default unchanged — identical input bytes",
            )?;
            self.skip_state(
                LaneId::SDR,
                StateDecision::SkippedIdenticalInput { source: LaneId::O },
                "Default rebuild omitted — identical input bytes",
            )?;
        }
        if self.comparison.args.skip_check_performance {
            self.comparison
                .report
                .state_decisions
                .insert(LaneId::CP, StateDecision::Disabled);
            self.comparison
                .report
                .state_decisions
                .insert(LaneId::CPR, StateDecision::Disabled);
        }
        self.comparison.open_workbooks(include_default)?;
        self.comparison.inspect_save_effects(include_default)?;
        let raw_original = self.comparison.args.original.clone();
        let raw_default = self.comparison.args.default.clone();
        let original = self.comparison.paths.original.clone();
        let default = self.comparison.paths.default.clone();
        self.audit("original opening/saving control", &raw_original, &original)?;
        if include_default {
            self.audit(
                "default input versus original input",
                &raw_original,
                &raw_default,
            )?;
            self.audit("default opening/saving control", &raw_default, &default)?;
        }
        self.register(LaneId::O, original)?;
        if include_default {
            self.register(LaneId::SD, default)?;
        }
        Ok(())
    }

    /// A skipped state has no Excel process and no invented screenshots or rebuild results.
    fn skip_state(&mut self, id: LaneId, decision: StateDecision, note: &str) -> Result<()> {
        self.comparison.report.state_decisions.insert(id, decision);
        self.comparison.telemetry.update(|state| {
            let lane = state.lane(id);
            lane.lifecycle = Lifecycle::Skipped;
            lane.decision_note = Some(note.into());
            lane.captures_total = Some(0);
            lane.capture_selection_complete = true;
            for step in lane.steps.values_mut() {
                *step = StepStatus::Skipped;
            }
        })
    }

    fn optimize_and_register_states(&mut self) -> Result<()> {
        self.comparison.optimize_performance_copy()?;
        if !self.comparison.args.skip_check_performance {
            let output = &self.comparison.args.output;
            let original = self.comparison.paths.original.clone();
            let before = output.join("check-performance.before-optimization.xlsx");
            let after = output.join("check-performance.after-optimization.xlsx");
            if self.comparison.performance.cache_hit {
                self.audit(
                    "current Original versus cached generation control",
                    &original,
                    &self
                        .comparison
                        .args
                        .output
                        .join("check-performance.cached-original.xlsx"),
                )?;
                self.audit(
                    "cached Check Performance reopening/saving control",
                    &after,
                    &self
                        .comparison
                        .performance
                        .path
                        .clone()
                        .context("Missing CP path")?,
                )?;
            }
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
        let xml = self.comparison.snapshots.read(&path)?;
        self.content_bounds.include(&xml);
        self.comparison
            .telemetry
            .update(|state| state.step(id, Phase::Discover, StepStatus::Done))?;
        let charts = if matches!(id, LaneId::OR | LaneId::SDR | LaneId::CPR) {
            self.capture_charts(id)?
        } else {
            charts::Captures::default()
        };
        self.comparison.report.state_decisions.insert(
            id,
            StateDecision::Observed {
                method: if matches!(id, LaneId::OR | LaneId::SDR | LaneId::CPR) {
                    "inplace-rebuild"
                } else {
                    "open-save"
                },
            },
        );
        self.states.insert(
            id,
            WorkbookState {
                path,
                xml,
                captures: BTreeMap::new(),
                geometries: BTreeMap::new(),
                charts,
                heights: BTreeMap::new(),
            },
        );
        Ok(())
    }

    /// Capture every chart while this state's pre-rebuild workbook is still retained.
    fn capture_charts(&mut self, id: LaneId) -> Result<charts::Captures> {
        let phase = if matches!(id, LaneId::O | LaneId::SD | LaneId::CP) {
            Phase::CaptureBefore
        } else {
            Phase::CaptureAfter
        };
        self.comparison
            .telemetry
            .update(|state| state.step(id, phase, StepStatus::Running))?;
        let captures = charts::capture(
            self.comparison
                .excel
                .as_mut()
                .context("Excel host missing")?,
            id,
            &self
                .comparison
                .args
                .output
                .join(format!("charts-{}", id.key())),
            &self.comparison.telemetry,
        )?;
        self.comparison.report.capture_counts.insert(
            format!("{}:charts", id.key()),
            charts::image_count(&captures),
        );
        self.comparison
            .telemetry
            .update(|state| state.step(id, phase, StepStatus::Done))?;
        Ok(captures)
    }

    fn audit(&mut self, label: &str, before: &Path, after: &Path) -> Result<()> {
        self.comparison.telemetry.update(|state| {
            for &id in self.states.keys() {
                state.lane(id).capture_selection_complete = false;
            }
        })?;
        let a = self.comparison.snapshots.read(before)?;
        let b = self.comparison.snapshots.read(after)?;
        self.content_bounds.include(&a);
        self.content_bounds.include(&b);
        let (mut changes, skipped_outside_content_cell_changes) = a.content_changes(&b)?;
        self.requested_candidates.extend(cell_candidates(&changes));
        self.publish_candidate_count()?;
        if label == "original opening/saving control" {
            self.original_save_keys = changes.iter().map(change_key).collect::<Result<_>>()?;
        }
        for change in &mut changes {
            change.also_in_original_open_save =
                self.original_save_keys.contains(&change_key(change)?);
        }
        let changes_file = super::change_evidence::write(
            &self.comparison.args.output,
            self.comparison.report.transitions.len(),
            &changes,
        )?;
        self.comparison.report.transitions.push(Transition {
            label: label.into(),
            before_file: before.into(),
            after_file: after.into(),
            change_count: changes.len(),
            changes_file,
            skipped_outside_content_cell_changes,
        });
        Ok(())
    }

    fn publish_candidate_count(&mut self) -> Result<()> {
        self.comparison.report.candidate_count = self.comparison.candidates.len();
        self.comparison
            .telemetry
            .update(|state| state.candidates(self.comparison.candidates.len()))
    }

    /// Rebuild one resident workbook, retaining its saved XML as the pre-state control.
    fn rebuild_in_place(&mut self, source: LaneId, target: LaneId) -> Result<()> {
        let Some(previous) = self.states.remove(&source) else {
            return Ok(());
        };
        let path = self.comparison.args.output.join(format!("{}-rebuilt.xlsx", target.key()));
        let pid = self.comparison.excel.as_mut().context("Excel host missing")?
            .move_to_rebuild(source, target, &path, &self.comparison.telemetry)?;
        self.comparison.report.excel_pids.insert(target.key().into(), pid);
        self.comparison.telemetry.update(|state| {
            let started = state.lane(source).process_started_at;
            state.lane(source).decision_note = Some(format!(
                "Same process transferred to {}; pre-state retained as XML only", target.key()));
            let lane = state.lane(target);
            lane.pid = Some(pid);
            lane.process_started_at = started;
            lane.process_alive = Some(true);
            lane.steps.insert(Phase::Open, StepStatus::Skipped);
            lane.lifecycle = Lifecycle::Open;
            lane.decision_note = Some("Rebuilt in the existing Excel process".into());
        })?;
        self.comparison.rebuild(target)?;
        self.audit(&format!("{} in-place rebuild and save", source.key()), &previous.path, &path)?;
        self.register(target, path)?;
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
            let (changes, _) = a.content_changes(&b)?;
            self.requested_candidates.extend(cell_candidates(&changes));
            let decisions = super::conditional_formatting::select(
                &a,
                &b,
                &changes,
                &mut self.requested_candidates,
            )?;
            self.comparison.report.conditional_formatting.push(
                super::conditional_formatting::Check {
                    left: left.key().into(),
                    right: right.key().into(),
                    decisions,
                },
            );
            self.comparison
                .telemetry
                .update(|state| state.report = self.comparison.report.telemetry_summary())?;
            self.publish_candidate_count()?;
            self.comparison.telemetry.update(|state| {
                state.step(left, Phase::MeasureHeights, StepStatus::Running);
                state.step(right, Phase::MeasureHeights, StepStatus::Running);
            })?;
            for sheet in a.sheets.keys() {
                let (spans, fixed) = heights::selection(&a, &b, sheet, self.content_bounds.last_relevant_row(sheet))?;
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
                self.requested_candidates.extend(selected);
                self.publish_candidate_count()?;
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
        let started = Instant::now();
        self.prepare_capture_partition()?;
        let mut pending: BTreeMap<_, VecDeque<_>> = self
            .states
            .iter()
            .map(|(&id, workbook)| {
                (
                    id,
                    self.comparison
                        .candidates
                        .iter()
                        .filter(|candidate| !workbook.captures.contains_key(*candidate))
                        .cloned()
                        .collect(),
                )
            })
            .collect();
        for &id in self.states.keys() {
            fs::create_dir_all(
                self.comparison
                    .args
                    .output
                    .join(format!("evidence-{}", id.key())),
            )?;
            self.comparison
                .telemetry
                .update(|state| state.step(id, capture_phase(id), StepStatus::Running))?;
        }
        loop {
            let jobs: Vec<_> = pending
                .iter()
                .filter_map(|(&id, queue)| {
                    queue.front().map(|candidate| {
                        let name = format!(
                            "{:x}",
                            Sha256::digest(
                                format!("{}\0{}", candidate.sheet, candidate.cell).as_bytes()
                            )
                        );
                        (
                            id,
                            candidate.clone(),
                            self.comparison
                                .args
                                .output
                                .join(format!("evidence-{}/{name}.png", id.key())),
                        )
                    })
                })
                .collect();
            if jobs.is_empty() {
                break;
            }
            let batch_size = if self.comparison.args.serial_captures {
                1
            } else {
                jobs.len()
            };
            for batch in jobs.chunks(batch_size) {
                self.comparison.telemetry.progress(|state| {
                    for (id, candidate, _) in batch {
                        state.lane(*id).current_sheet = Some(candidate.sheet.clone());
                        state.lane(*id).current_range = Some(candidate.cell.clone());
                    }
                })?;
                let mut core_ranges = BTreeMap::new();
                for (_, candidate, _) in batch {
                    if let Some(range) = self.content_bounds.core_range(candidate)? {
                        core_ranges.insert(candidate.clone(), range);
                    }
                }
                let geometry = self
                    .comparison
                    .excel
                    .as_mut()
                    .context("Excel host missing")?
                    .capture_ranges(batch, &core_ranges, &self.comparison.telemetry)?;
                for ((id, candidate, path), geometry) in batch.iter().zip(geometry) {
                    self.states.get_mut(id).unwrap().captures.insert(
                        candidate.clone(),
                        Evidence {
                            path: if serde_json::from_value::<Geometry>(geometry.clone())?.validate()?.empty() {
                                None
                            } else {
                                let (width, height) = image::image_dimensions(path)?;
                                ensure!(width <= capture_layout::MAX_EDGE && height <= capture_layout::MAX_EDGE
                                    && u64::from(width) * u64::from(height) <= capture_layout::MAX_PIXELS,
                                    "Export exceeds the measured image budget; no equality verdict was produced");
                                Some(path.clone())
                            },
                            geometry,
                        },
                    );
                    pending.get_mut(id).unwrap().pop_front();
                    *self
                        .comparison
                        .report
                        .capture_counts
                        .entry(id.key().into())
                        .or_default() += 1;
                }
                self.comparison.telemetry.update(|state| {
                    state.capture_success_sequence += batch.iter()
                        .filter(|(id, candidate, _)| self.states[id].captures[candidate].path.is_some())
                        .count() as u64;
                    for (id, _, _) in batch {
                        state.lane(*id).captures_done += 1;
                    }
                })?;
            }
        }
        for &id in self.states.keys() {
            self.comparison
                .telemetry
                .update(|state| state.step(id, capture_phase(id), StepStatus::Done))?;
        }
        *self
            .comparison
            .report
            .timings_ms
            .entry("capture_selection_and_transport".into())
            .or_default() += started.elapsed().as_millis() as u64;
        Ok(())
    }

    /// Preserve selected coverage, measure its visible partition, and record non-rendering leaves.
    fn prepare_capture_partition(&mut self) -> Result<()> {
        let (selected, coverage) = self.content_bounds.select(&self.requested_candidates)?;
        let plan = capture_layout::plan(&selected, |candidate| self.measure_candidate(candidate))?;
        self.comparison.candidates = plan.keys().cloned().collect();
        let mut coverage = coverage;
        super::content_bounds::record_partition(&mut coverage, &self.comparison.candidates);
        self.comparison.report.content_coverage = coverage;
        for (candidate, states) in &plan {
            for (&id, &geometry) in states {
                if geometry.empty() {
                    self.states.get_mut(&id).unwrap().captures.insert(
                        candidate.clone(),
                        Evidence {
                            path: None,
                            geometry: serde_json::to_value(geometry)?,
                        },
                    );
                }
            }
        }
        self.comparison.telemetry.update(|state| {
            for (&id, workbook) in &self.states {
                state.lane(id).capture_selection_complete = true;
                state.lane(id).skipped_zero_area_regions = self
                    .comparison
                    .candidates
                    .iter()
                    .filter(|candidate| {
                        workbook
                            .captures
                            .get(*candidate)
                            .is_some_and(|e| e.path.is_none())
                    })
                    .count();
                state.lane(id).captures_done = self
                    .comparison
                    .candidates
                    .iter()
                    .filter(|candidate| workbook.captures.contains_key(*candidate))
                    .count();
            }
        })?;
        self.publish_candidate_count()?;
        Ok(())
    }

    /// Cache immutable state geometry; later states must supply their own measurements.
    fn measure_candidate(&mut self, candidate: &Candidate) -> Result<BTreeMap<LaneId, Geometry>> {
        let missing: Vec<_> = self
            .states
            .iter()
            .filter(|(_, state)| !state.geometries.contains_key(candidate))
            .map(|(&id, _)| id)
            .collect();
        if !missing.is_empty() {
            self.comparison.telemetry.progress(|state| {
                for &id in &missing {
                    state.lane(id).current_sheet = Some(candidate.sheet.clone());
                    state.lane(id).current_range = Some(format!("measure {}", candidate.cell));
                }
            })?;
            let measured = self
                .comparison
                .excel
                .as_mut()
                .context("Excel host missing")?
                .measure_ranges(&missing, candidate, &self.comparison.telemetry)?;
            for (&id, geometry) in missing.iter().zip(measured) {
                self.states
                    .get_mut(&id)
                    .unwrap()
                    .geometries
                    .insert(candidate.clone(), geometry);
            }
        }
        self.states
            .iter()
            .map(|(&id, state)| {
                Ok((
                    id,
                    *state
                        .geometries
                        .get(candidate)
                        .context("Missing measured capture geometry")?,
                ))
            })
            .collect()
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
        let started = Instant::now();
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
                let verdict =
                    a.compare_content(b, &mut self.comparison.image_comparisons)?;
                findings.push(VisualFinding {
                    sheet: candidate.sheet.clone(),
                    cell: candidate.cell.clone(),
                    pixels_differ: verdict.pixels_differ,
                    blank_margin_ignored: verdict.blank_margin_ignored,
                    pixels_compared: a.path.is_some() && b.path.is_some(),
                    rendering_differs: a.path.is_some() != b.path.is_some(),
                    left_rendering: a.rendering(),
                    right_rendering: b.rendering(),
                    left_geometry: a.geometry.clone(),
                    right_geometry: b.geometry.clone(),
                    geometry_differs: verdict.geometry_differs,
                    left_png: a.path.clone(),
                    right_png: b.path.clone(),
                });
            }
            comparisons.push(VisualComparison {
                label: label.into(),
                left,
                right,
                selected_region_count: findings.len(),
                findings,
                charts: charts::compare(
                    &self.states[&left].charts,
                    &self.states[&right].charts,
                    &mut self.comparison.image_comparisons,
                )?,
            });
            self.comparison.telemetry.update(|state| {
                state.step(left, Phase::Compare, StepStatus::Done);
                state.step(right, Phase::Compare, StepStatus::Done);
            })?;
        }
        *self
            .comparison
            .report
            .timings_ms
            .entry("image_comparisons".into())
            .or_default() += started.elapsed().as_millis() as u64;
        // Preserve the frozen direct SD comparison with its own region selection.
        // Post-rebuild candidates can grow or repartition after that state is gone.
        for pair in comparisons {
            self.comparison
                .report
                .comparisons
                .retain(|old| !(old.left == pair.left && old.right == pair.right));
            self.comparison.report.comparisons.push(pair);
        }
        self.comparison.report.phases.push(phase.into());
        fs::write(
            self.comparison.args.output.join("partial-report.json"),
            serde_json::to_vec(&self.comparison.report)?,
        )?;
        Ok(())
    }

    fn project_legacy_findings(&mut self) -> Result<()> {
        self.comparison.original_save_changes =
            self.project_changes(&self.comparison.original_save_changes)?;
        self.comparison.performance.save_changes =
            self.project_changes(&self.comparison.performance.save_changes)?;
        self.comparison.performance.optimization_changes =
            self.project_changes(&self.comparison.performance.optimization_changes)?;
        if [LaneId::O, LaneId::SD, LaneId::SDR, LaneId::OR]
            .iter()
            .all(|id| self.states.contains_key(id))
        {
            self.comparison.captures.original_before = self.states[&LaneId::O].captures.clone();
            self.comparison.captures.shrink_before = self.states[&LaneId::SD].captures.clone();
            self.comparison.captures.shrink_after = self.states[&LaneId::SDR].captures.clone();
            self.comparison.captures.original_after = self.states[&LaneId::OR].captures.clone();
            self.comparison.compare_captured_evidence()?;
        }
        if [LaneId::O, LaneId::CP, LaneId::CPR, LaneId::OR]
            .iter()
            .all(|id| self.states.contains_key(id))
        {
            self.comparison.captures.original_before = self.states[&LaneId::O].captures.clone();
            self.comparison.captures.original_after = self.states[&LaneId::OR].captures.clone();
            self.comparison.performance.before = self.states[&LaneId::CP].captures.clone();
            self.comparison.performance.after = self.states[&LaneId::CPR].captures.clone();
            self.comparison.compare_performance_evidence()?;
        }
        Ok(())
    }

    /// Attribution uses overlap with the final partition, not equality with old grid addresses.
    fn project_changes(&self, changes: &BTreeSet<Candidate>) -> Result<BTreeSet<Candidate>> {
        use super::content_bounds::Rectangle;
        let selected = self.content_bounds.select(changes)?.0;
        let rectangles = selected
            .iter()
            .map(|c| Ok((&c.sheet, Rectangle::parse(&c.cell)?)))
            .collect::<Result<Vec<_>>>()?;
        let mut result = BTreeSet::new();
        for candidate in &self.comparison.candidates {
            let a = Rectangle::parse(&candidate.cell)?;
            if rectangles.iter().any(|(sheet, b)| {
                **sheet == candidate.sheet
                    && a.top <= b.bottom
                    && b.top <= a.bottom
                    && a.left <= b.right
                    && b.left <= a.right
            }) {
                result.insert(candidate.clone());
            }
        }
        Ok(result)
    }

    fn close_states(&mut self) -> Result<()> {
        for id in self.states.keys().copied().rev().collect::<Vec<_>>() {
            self.comparison.retain_performance_measurements(id);
            self.comparison.close_workbook(id)?;
        }
        self.comparison.telemetry.update(|state| {
            for (source, target) in [(LaneId::O, LaneId::OR), (LaneId::SD, LaneId::SDR), (LaneId::CP, LaneId::CPR)] {
                if state.lane(target).lifecycle == Lifecycle::Closed {
                    let closed_at = state.lane(target).closed_at;
                    let lane = state.lane(source);
                    lane.lifecycle = Lifecycle::Closed;
                    lane.process_alive = Some(false);
                    lane.closed_at = closed_at;
                }
            }
        })?;
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

/// Match exact transition evidence without retaining millions of duplicated strings.
fn change_key(change: &super::xml_changes::XmlChange) -> Result<[u8; 32]> {
    Ok(Sha256::digest(serde_json::to_vec(&Snapshot::change_key(change))?).into())
}

/// Rebuilt copies never count as pre-rebuild capture work.
fn capture_phase(id: LaneId) -> Phase {
    if matches!(id, LaneId::O | LaneId::SD | LaneId::CP) {
        Phase::CaptureBefore
    } else {
        Phase::CaptureAfter
    }
}
