//! Narrow proofs for unchanged CF expressions with no live cell-value dependency.
use super::{formulas, Snapshot};
use crate::regression::cf_inventory::scan::Rule;

/// Recognize only two constant error forms or a bare ISBLANK name proven absent in both states.
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
    let constant_error = formula.eq_ignore_ascii_case("#REF!")
        || formula.split_once('=').is_some_and(|(left, right)| {
            left.trim().eq_ignore_ascii_case("#REF!") && right.trim() == "1"
        });
    if constant_error {
        return Some("constant-ref-error-expression");
    }
    if !formula.eq_ignore_ascii_case("isblank") {
        return None;
    }
    let (Some(left), Some(right)) = (&before.defined_names, &after.defined_names) else {
        return None;
    };
    (!left.contains("isblank") && !right.contains("isblank"))
        .then_some("bare-isblank-name-absent-in-both-states")
}
