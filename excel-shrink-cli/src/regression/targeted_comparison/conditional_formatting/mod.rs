//! CF selection proves only supported local contributions; every unresolved area remains a capture.
mod formulas;
mod ranges;
#[cfg(test)]
mod tests;
pub(super) mod xml;

use super::{
    xml_changes::{Sheet, Snapshot, XmlChange},
    Candidate,
};
use crate::regression::cf_inventory::scan::Block;
use anyhow::Result;
pub(super) use ranges::cell as cell_address;
pub(super) use ranges::Rect;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Serialize)]
pub(super) struct Decision {
    pub sheet: String,
    pub block_index: usize,
    pub sqref: String,
    pub catalogue_side: &'static str,
    pub outcome: String,
    pub reason: String,
    pub changed_inputs: usize,
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
    MissingDummyTargets,
    Fallback(&'static str),
}

fn proof(block: &Block, before: &Snapshot, after: &Snapshot) -> Proof {
    if block.qualified_name != "conditionalFormatting"
        || block.attributes.keys().any(|key| key != "sqref")
        || block.child_elements.iter().any(|name| {
            !["cfRule", "formula", "colorScale", "cfvo", "color"].contains(&name.as_str())
        })
        || block.rules.is_empty()
    {
        return Proof::Fallback("unsupported-rule-or-namespace-content");
    }
    if block.rules.iter().all(formulas::literal_rule) {
        if before.cf_context == after.cf_context {
            return Proof::LocalInputs;
        }
        return Proof::Fallback("format-string-theme-or-workbook-context-changed");
    }
    if block
        .rules
        .iter()
        .all(|rule| formulas::literal_rule(rule) || formulas::whole_area_rule(rule))
    {
        return if before.cf_context == after.cf_context {
            Proof::WholeAreaInputs
        } else {
            Proof::Fallback("format-string-theme-or-workbook-context-changed")
        };
    }
    for rule in &block.rules {
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
            return Proof::Fallback("unsupported-rule-semantics");
        }
        let Some(target) = formulas::dummy_target(&rule.formulas[0]) else {
            return Proof::Fallback("indirect-template-not-recognized");
        };
        let (Some(a), Some(b)) = (&before.sheet_names, &after.sheet_names) else {
            return Proof::Fallback("sheet-name-absence-not-proven");
        };
        if a.contains(&target) || b.contains(&target) {
            return Proof::Fallback("dummy-target-exists");
        }
    }
    Proof::MissingDummyTargets
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
    for change in changes.iter().filter(|change| change.sheet == *name) {
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
        let identical = a.conditional.fingerprints == b.conditional.fingerprints;
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
                } else if !catalogue.standard_namespace {
                    Proof::Fallback("worksheet-default-namespace-not-supported")
                } else if areas.is_none() {
                    Proof::Fallback("application-range-not-resolved")
                } else {
                    proof(block, before, after)
                };
                if matches!(mode, Proof::LocalInputs | Proof::WholeAreaInputs)
                    && a.cf_layout != b.cf_layout
                {
                    mode = Proof::Fallback("worksheet-layout-context-changed");
                }
                let mut decision = Decision {
                    sheet: name.clone(),
                    block_index: index,
                    sqref: block.sqref.clone(),
                    catalogue_side: side,
                    outcome: String::new(),
                    reason: String::new(),
                    changed_inputs: 0,
                    formula_input_observations: 0,
                    layout_rows: 0,
                    comparison_extent: extent,
                };
                match mode {
                    Proof::WholeAreaInputs => {
                        let areas = areas.as_deref().unwrap();
                        select_local_inputs(&mut decision, areas, a, b, changes, candidates)?;
                        if decision.changed_inputs > 0 || decision.formula_input_observations > 0 {
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
