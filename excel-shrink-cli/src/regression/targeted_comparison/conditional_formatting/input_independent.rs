//! Narrow proofs for unchanged CF expressions with no live cell-value dependency.
use super::{formulas, Snapshot};
use crate::regression::cf_inventory::scan::Rule;

/// Recognize exact constant errors and absent names, never evaluate an arbitrary CF formula.
pub(super) fn proof(rule: &Rule, before: &Snapshot, after: &Snapshot) -> Option<&'static str> {
    if rule.truncated_formula
        || rule.formula_cdata
        || rule.attributes.keys().any(|key| {
            !["type", "operator", "dxfId", "priority", "stopIfTrue"].contains(&key.as_str())
        })
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
    if rule.attributes.get("type").map(String::as_str) == Some("cellIs") {
        let single_comparison = matches!(
            rule.attributes.get("operator").map(String::as_str),
            Some(
                "equal"
                    | "notEqual"
                    | "lessThan"
                    | "lessThanOrEqual"
                    | "greaterThan"
                    | "greaterThanOrEqual"
            )
        );
        return (single_comparison && formula.eq_ignore_ascii_case("#REF!"))
            .then_some("cell-is-constant-ref-error-operand");
    }
    if rule.attributes.get("type").map(String::as_str) != Some("expression")
        || rule.attributes.contains_key("operator")
    {
        return None;
    }
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
