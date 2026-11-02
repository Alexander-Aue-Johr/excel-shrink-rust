//! Narrow proofs for unchanged CF expressions with no live cell-value dependency.
use super::{formulas, Snapshot};
use crate::regression::cf_inventory::scan::Rule;

/// A bare ISBLANK token is a name, not ISBLANK(cell). Prove its absence in both workbooks.
pub(super) fn proof(rule: &Rule, before: &Snapshot, after: &Snapshot) -> Option<&'static str> {
    if rule.truncated_formula
        || rule.formula_cdata
        || rule.attributes.get("type").map(String::as_str) != Some("expression")
        || rule
            .attributes
            .keys()
            .any(|key| !["type", "dxfId", "priority", "stopIfTrue"].contains(&key.as_str()))
        || rule.child_elements.iter().any(|name| name != "formula")
        || rule.formulas.len() != 1
    {
        return None;
    }
    let formula = formulas::decode_entities(&rule.formulas[0])?;
    let formula = formula
        .trim()
        .strip_prefix('=')
        .unwrap_or(formula.trim())
        .trim();
    if !formula.eq_ignore_ascii_case("isblank") {
        return None;
    }
    let (Some(left), Some(right)) = (&before.defined_names, &after.defined_names) else {
        return None;
    };
    (!left.contains("isblank") && !right.contains("isblank"))
        .then_some("bare-isblank-name-absent-in-both-states")
}
