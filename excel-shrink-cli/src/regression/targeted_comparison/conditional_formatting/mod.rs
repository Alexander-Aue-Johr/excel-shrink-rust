//! CF selection proves only supported local contributions; every unresolved area remains a capture.
pub(super) mod data_bars;
mod expression;
mod formulas;
mod input_independent;
#[cfg(test)]
mod inventory_audit;
mod ranges;
mod references;
#[cfg(test)]
mod tests;
pub(super) mod xml;

use super::{
    xml_changes::{Sheet, Snapshot, XmlChange},
    Candidate,
};
use crate::regression::cf_inventory::scan::{Block, Rule};
use anyhow::{Context, Result};
pub(super) use ranges::areas as application_areas;
pub(super) use ranges::cell as cell_address;
pub(super) use ranges::Rect;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize)]
pub(super) struct Decision {
    pub sheet: String,
    pub block_index: usize,
    pub sqref: String,
    pub catalogue_side: &'static str,
    pub outcome: String,
    pub reason: String,
    pub changed_inputs: usize,
    pub referenced_input_changes: usize,
    pub referenced_formula_observations: usize,
    /// Formula appearances across both states, not a deduplicated cell count.
    pub formula_input_observations: usize,
    pub layout_rows: usize,
    pub comparison_extent: Rect,
}

#[derive(Serialize)]
pub(super) struct Check {
    pub left: String,
    pub right: String,
    pub decisions: Vec<Decision>,
}

enum Proof {
    LocalInputs,
    WholeAreaInputs,
    ReferencedInputs(Vec<references::Reference>),
    CombinedInputs(CombinedInputs),
    MissingDummyTargets,
    InputIndependent(&'static str),
    Fallback(&'static str),
}

struct CombinedInputs {
    whole_area: bool,
    references: Vec<references::Reference>,
}

fn proof(block: &Block, before: &Snapshot, after: &Snapshot) -> Proof {
    if block.attributes.contains_key("pivot") {
        return Proof::Fallback("pivot-formatting-dependencies-not-modeled");
    }
    if block.qualified_name != "conditionalFormatting"
        || block.attributes.keys().any(|key| key != "sqref")
        || block.child_elements.iter().any(|name| {
            ![
                "cfRule",
                "formula",
                "colorScale",
                "cfvo",
                "color",
                "dataBar",
                "extLst",
                "ext",
                "x14:id",
            ]
            .contains(&name.as_str())
        })
        || block.rules.is_empty()
    {
        return Proof::Fallback("unsupported-rule-or-namespace-content");
    }
    let complete_context_unchanged = before.cf_context == after.cf_context
        && before.context == after.context;
    if block.rules.iter().all(formulas::literal_rule) {
        if before.cf_context == after.cf_context {
            return Proof::LocalInputs;
        }
        return Proof::Fallback("format-string-theme-or-workbook-context-changed");
    }
    if block
        .rules
        .iter()
        .all(|rule| input_independent::proof(rule, before, after).is_some())
    {
        return if complete_context_unchanged {
            Proof::InputIndependent("unchanged-input-independent-rules")
        } else {
            Proof::Fallback("format-string-theme-or-workbook-context-changed")
        };
    }
    if block
        .rules
        .iter()
        .all(|rule| formulas::literal_rule(rule) || formulas::whole_area_rule(rule))
    {
        // Actual cell-format changes count as uncertain inputs below. Unused
        // formats cannot affect an aggregate rule over unchanged input cells.
        return if before.cf_context == after.cf_context {
            Proof::WholeAreaInputs
        } else {
            Proof::Fallback("format-string-theme-or-workbook-context-changed")
        };
    }
    if let Some(inputs) = reference_inputs(block, before, after) {
        // These recognized formulas inspect values, never cell formatting.
        // Changed applied formats already produce their own cell candidates;
        // unused XF entries cannot alter the rule's unchanged value inputs.
        return if before.cf_context == after.cf_context {
            Proof::ReferencedInputs(inputs)
        } else {
            Proof::Fallback("format-string-theme-or-workbook-context-changed")
        };
    }
    if let Some(inputs) = combined_inputs(block, before, after) {
        return if complete_context_unchanged {
            Proof::CombinedInputs(inputs)
        } else {
            Proof::Fallback("format-string-theme-or-workbook-context-changed")
        };
    }
    for rule in &block.rules {
        if let Some(reason) = dummy_target_error(rule, before, after) {
            return Proof::Fallback(reason);
        }
    }
    Proof::MissingDummyTargets
}

/// Missing-target rules contribute no visual dependency; any uncertainty keeps their fallback.
fn dummy_target_error(rule: &Rule, before: &Snapshot, after: &Snapshot) -> Option<&'static str> {
    if rule.truncated_formula
        || rule.formula_cdata
        || rule.attributes.get("type").map(String::as_str) != Some("cellIs")
        || rule.attributes.get("operator").map(String::as_str) != Some("notEqual")
        || rule.attributes.keys().any(|key| {
            !["type", "operator", "dxfId", "priority", "stopIfTrue"].contains(&key.as_str())
        })
        || rule.child_elements.iter().any(|name| name != "formula")
        || rule.formulas.len() != 1
    {
        return Some("unsupported-rule-semantics");
    }
    let Some(target) = formulas::dummy_target(&rule.formulas[0]) else {
        return Some("indirect-template-not-recognized");
    };
    let (Some(a), Some(b)) = (&before.sheet_names, &after.sheet_names) else {
        return Some("sheet-name-absence-not-proven");
    };
    if a.contains(&target) || b.contains(&target) {
        return Some("dummy-target-exists");
    }
    None
}

/// Union every understood rule's dependencies; one unknown member invalidates the whole plan.
fn combined_inputs(block: &Block, before: &Snapshot, after: &Snapshot) -> Option<CombinedInputs> {
    let mut plan = CombinedInputs {
        whole_area: false,
        references: Vec::new(),
    };
    let mut active_rules = 0;
    for rule in &block.rules {
        if input_independent::proof(rule, before, after).is_some() {
            continue;
        } else if formulas::literal_rule(rule) {
            active_rules += 1;
        } else if formulas::whole_area_rule(rule) {
            plan.whole_area = true;
            active_rules += 1;
        } else if let Some(inputs) = rule_references(rule) {
            plan.references.extend(inputs);
            active_rules += 1;
        } else if dummy_target_error(rule, before, after).is_some() {
            return None;
        }
    }
    (active_rules > 0).then_some(plan)
}

/// Every rule must have a known dependency footprint; one unknown rule keeps the block conservative.
fn reference_inputs(
    block: &Block,
    before: &Snapshot,
    after: &Snapshot,
) -> Option<Vec<references::Reference>> {
    let mut inputs = Vec::new();
    for rule in &block.rules {
        if formulas::literal_rule(rule) || input_independent::proof(rule, before, after).is_some() {
            continue;
        }
        inputs.extend(rule_references(rule)?);
    }
    (!inputs.is_empty()).then_some(inputs)
}

/// Extract explicit scalar/range dependencies only from understood rule types and content.
fn rule_references(rule: &Rule) -> Option<Vec<references::Reference>> {
    let mut inputs = Vec::new();
    if rule.truncated_formula
        || rule.formula_cdata
        || !matches!(
            rule.attributes.get("type").map(String::as_str),
            Some(
                "containsBlanks"
                    | "notContainsBlanks"
                    | "containsErrors"
                    | "notContainsErrors"
                    | "containsText"
                    | "notContainsText"
                    | "beginsWith"
                    | "endsWith"
                    | "expression"
                    | "cellIs"
            )
        )
        || rule.attributes.keys().any(|key| {
            ![
                "type",
                "dxfId",
                "priority",
                "stopIfTrue",
                "operator",
                "text",
            ]
            .contains(&key.as_str())
        })
        || rule.child_elements.iter().any(|name| name != "formula")
        || rule.formulas.is_empty()
    {
        return None;
    }
    for raw in &rule.formulas {
        let formula = formulas::decode_entities(raw)?;
        inputs.extend(expression::dependencies(&formula)?);
    }
    Some(inputs)
}

/// Changes and uncertain formula inputs select all dependent targets, without rule evaluation.
fn select_references(
    decision: &mut Decision,
    inputs: &[references::Reference],
    areas: &[Rect],
    before: &Snapshot,
    after: &Snapshot,
    changes: &[XmlChange],
    candidates: &mut BTreeSet<Candidate>,
) -> Result<()> {
    select_local_inputs(
        decision,
        areas,
        &before.sheets[&decision.sheet],
        &after.sheets[&decision.sheet],
        changes,
        candidates,
    )?;
    let mut sources: BTreeMap<&str, Vec<&references::Reference>> = BTreeMap::new();
    for reference in inputs {
        let (source, _) = reference
            .source(before, &decision.sheet)
            .context("CF source not resolved")?;
        sources.entry(source).or_default().push(reference);
    }
    let mut changed = 0;
    let mut uncertain = 0;
    for (source, inputs) in sources {
        let mut select = |input| {
            let mut affected = false;
            for reference in &inputs {
                let targets = reference.targets(input, areas, decision.comparison_extent);
                affected |= !targets.is_empty();
                fallback(
                    &decision.sheet,
                    &targets,
                    decision.comparison_extent,
                    candidates,
                );
            }
            affected
        };
        for change in changes.iter().filter(|change| {
            change.sheet == source && change.may_change_cell_input
        }) {
            if let Some(input) = change.location.strip_prefix("cell:").and_then(ranges::cell) {
                changed += usize::from(select(input));
            }
        }
        for cells in [&before.sheets[source].cells, &after.sheets[source].cells] {
            cells.formula_cells(|cell| {
                uncertain += usize::from(select((cell.row, u32::from(cell.column))));
                Ok(())
            })?;
        }
    }
    decision.referenced_input_changes = changed;
    decision.referenced_formula_observations = uncertain;
    if changed > 0 || uncertain > 0 {
        decision.outcome = "partial".into();
    }
    decision.reason =
        "worksheet-reference-dependencies; changed or uncertain inputs select dependent targets"
            .into();
    Ok(())
}

fn fallback(sheet: &str, areas: &[Rect], extent: Rect, candidates: &mut BTreeSet<Candidate>) {
    for area in areas {
        if let Some(region) = area.intersect(extent) {
            for tile in region.tiles() {
                candidates.insert(Candidate {
                    sheet: sheet.into(),
                    cell: tile.address(),
                });
            }
        }
    }
}

/// Select only changed XML inputs, uncertain formula inputs and changed row layouts.
fn select_local_inputs(
    decision: &mut Decision,
    areas: &[Rect],
    a: &Sheet,
    b: &Sheet,
    changes: &[XmlChange],
    candidates: &mut BTreeSet<Candidate>,
) -> Result<()> {
    let name = decision.sheet.clone();
    let name = &name;
    let extent = decision.comparison_extent;
    let mut selected = BTreeSet::new();
    for change in changes.iter().filter(|change| {
        change.sheet == *name && change.may_change_cell_input
    }) {
        let Some(address) = change.location.strip_prefix("cell:") else {
            continue;
        };
        let Some((row, column)) = ranges::cell(address) else {
            continue;
        };
        if areas.iter().any(|area| area.contains(row, column)) {
            selected.insert(address.to_owned());
            decision.changed_inputs += 1;
        }
    }
    let rows: BTreeSet<_> = a.rows.keys().chain(b.rows.keys()).copied().collect();
    for row in rows {
        if a.rows.get(&row) != b.rows.get(&row) {
            let changed = Rect {
                top: row,
                bottom: row,
                left: 1,
                right: 16384,
            };
            let affected: Vec<_> = areas
                .iter()
                .filter_map(|area| area.intersect(changed))
                .collect();
            if !affected.is_empty() {
                decision.layout_rows += 1;
                fallback(name, &affected, extent, candidates);
            }
        }
    }
    for cells in [&a.cells, &b.cells] {
        cells.formula_cells(|cell| {
            if areas
                .iter()
                .any(|area| area.contains(cell.row, u32::from(cell.column)))
            {
                selected.insert(cell.address());
                decision.formula_input_observations += 1;
            }
            Ok(())
        })?;
    }
    decision.outcome = if selected.is_empty() && decision.layout_rows == 0 {
        "excluded"
    } else {
        "partial"
    }
    .into();
    decision.reason =
        "Literal cellIs: unchanged XML inputs and visual context; formula inputs remain captures."
            .into();
    candidates.extend(selected.into_iter().map(|cell| Candidate {
        sheet: name.clone(),
        cell,
    }));
    Ok(())
}

/// Aggregate rules may compare formatted values inherited from their input rows.
/// Row-height changes alone remain local layout candidates.
fn inherited_row_formats_changed(a: &Sheet, b: &Sheet, areas: &[Rect]) -> Result<bool> {
    let rows: BTreeSet<_> = a
        .rows
        .keys()
        .chain(b.rows.keys())
        .copied()
        .filter(|row| {
            areas.iter().any(|area| area.top <= *row && *row <= area.bottom)
                && ["s", "customFormat"].iter().any(|key| {
                    a.rows.get(row).and_then(|attributes| attributes.get(*key))
                        != b.rows.get(row).and_then(|attributes| attributes.get(*key))
                })
        })
        .collect();
    if rows.is_empty() {
        return Ok(false);
    }
    for cells in [&a.cells, &b.cells] {
        if cells.any_input(|cell| {
            rows.contains(&cell.row)
                && areas
                    .iter()
                    .any(|area| area.contains(cell.row, u32::from(cell.column)))
        })? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Compare complete rule catalogues before excluding any CF area; removed rules keep their old scope.
pub(super) fn select(
    before: &Snapshot,
    after: &Snapshot,
    changes: &[XmlChange],
    candidates: &mut BTreeSet<Candidate>,
) -> Result<Vec<Decision>> {
    let mut decisions = Vec::new();
    for (name, a) in &before.sheets {
        let Some(b) = after.sheets.get(name) else {
            continue;
        };
        let extent = Rect {
            top: 1,
            left: 1,
            bottom: a.bottom.max(b.bottom),
            right: a.right.max(b.right),
        };
        let identical = a.conditional.fingerprints == b.conditional.fingerprints
            && a.conditional.namespace_supported == b.conditional.namespace_supported;
        let catalogues = if identical {
            vec![("both", &a.conditional)]
        } else {
            vec![("left", &a.conditional), ("right", &b.conditional)]
        };
        for (side, catalogue) in catalogues {
            for (index, block) in catalogue.blocks.iter().enumerate() {
                let areas = ranges::areas(&block.sqref);
                let mut mode = if !identical {
                    Proof::Fallback("rule-catalogue-changed")
                } else if !catalogue.namespace_supported[index] {
                    Proof::Fallback("worksheet-default-namespace-not-supported")
                } else if areas.is_none() {
                    Proof::Fallback("application-range-not-resolved")
                } else {
                    proof(block, before, after)
                };
                if matches!(
                    mode,
                    Proof::LocalInputs
                        | Proof::WholeAreaInputs
                        | Proof::ReferencedInputs(_)
                        | Proof::CombinedInputs(_)
                        | Proof::InputIndependent(_)
                ) && (a.cf_layout != b.cf_layout
                    || areas.as_ref().is_some_and(|areas| {
                        let right = areas.iter().map(|area| area.right).max().unwrap_or(16_384);
                        !a.cf_columns.same_prefix(&b.cf_columns, right)
                    }))
                {
                    mode = Proof::Fallback("worksheet-layout-context-changed");
                }
                let references = match &mode {
                    Proof::ReferencedInputs(inputs) => inputs.as_slice(),
                    Proof::CombinedInputs(plan) => plan.references.as_slice(),
                    _ => &[],
                };
                if references.iter().any(|reference| {
                    match (
                        reference.source(before, name),
                        reference.source(after, name),
                    ) {
                        (Some((a, _)), Some((b, _))) => a != b,
                        _ => true,
                    }
                }) {
                    mode = Proof::Fallback("referenced-worksheet-not-resolved-in-both-states");
                }
                let mut decision = Decision {
                    sheet: name.clone(),
                    block_index: index,
                    sqref: block.sqref.clone(),
                    catalogue_side: side,
                    outcome: String::new(),
                    reason: String::new(),
                    changed_inputs: 0,
                    referenced_input_changes: 0,
                    referenced_formula_observations: 0,
                    formula_input_observations: 0,
                    layout_rows: 0,
                    comparison_extent: extent,
                };
                match mode {
                    Proof::CombinedInputs(plan) => {
                        let areas = areas.as_deref().unwrap();
                        select_references(
                            &mut decision,
                            &plan.references,
                            areas,
                            before,
                            after,
                            changes,
                            candidates,
                        )?;
                        if plan.whole_area
                            && (decision.changed_inputs > 0
                                || decision.formula_input_observations > 0)
                        {
                            decision.outcome = "capture".into();
                            decision.reason =
                                "combined-whole-area-input-changed-or-uncertain".into();
                            fallback(name, areas, extent, candidates);
                        } else {
                            decision.reason = if decision.outcome == "excluded" {
                                "combined-supported-inputs-unchanged"
                            } else {
                                "combined-reference-or-layout-dependency"
                            }
                            .into();
                        }
                    }
                    Proof::ReferencedInputs(inputs) => select_references(
                        &mut decision,
                        &inputs,
                        areas.as_deref().unwrap(),
                        before,
                        after,
                        changes,
                        candidates,
                    )?,
                    Proof::WholeAreaInputs => {
                        let areas = areas.as_deref().unwrap();
                        select_local_inputs(&mut decision, areas, a, b, changes, candidates)?;
                        if decision.changed_inputs > 0
                            || decision.formula_input_observations > 0
                            || inherited_row_formats_changed(a, b, areas)?
                        {
                            decision.outcome = "capture".into();
                            decision.reason =
                                "whole-area-dependency-changed-or-formula-input-uncertain".into();
                            fallback(name, areas, extent, candidates);
                        } else {
                            decision.reason =
                                "whole-area-XML-inputs-unchanged; no rule evaluation".into();
                        }
                    }
                    Proof::MissingDummyTargets => {
                        decision.outcome = "excluded".into();
                        decision.reason = "fixed-dummy-targets-absent-in-both-states".into();
                    }
                    Proof::InputIndependent(reason) => {
                        decision.outcome = "excluded".into();
                        decision.reason = reason.into();
                    }
                    Proof::LocalInputs => select_local_inputs(
                        &mut decision,
                        areas.as_deref().unwrap(),
                        a,
                        b,
                        changes,
                        candidates,
                    )?,
                    Proof::Fallback(reason) => {
                        decision.outcome = "capture".into();
                        decision.reason = reason.into();
                        fallback(
                            name,
                            areas.as_deref().unwrap_or(&[extent]),
                            extent,
                            candidates,
                        );
                    }
                }
                decisions.push(decision);
            }
        }
    }
    Ok(decisions)
}
