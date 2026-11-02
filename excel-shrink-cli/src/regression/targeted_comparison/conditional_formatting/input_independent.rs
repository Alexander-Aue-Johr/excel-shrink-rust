//! Narrow proofs for unchanged CF expressions with no live cell-value dependency.
use super::{formulas, Snapshot};
use crate::regression::cf_inventory::scan::Rule;

/// Prove only whitelisted input-independent rules; never evaluate arbitrary CF formulas.
/// Callers separately verify catalogue, namespace and formatting/layout context equality.
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
        .trim_ascii()
        .strip_prefix('=')
        .unwrap_or(formula.trim_ascii())
        .trim_ascii();
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
    if row_parity(formula) {
        return Some("fixed-physical-row-parity");
    }
    if formula.eq_ignore_ascii_case("ISTEXT(#REF!)") {
        return Some("constant-false-istext-error-test");
    }
    let constant_error = formula.eq_ignore_ascii_case("#REF!")
        || formula.eq_ignore_ascii_case("#DIV/0!")
        || formula.split_once('=').is_some_and(|(left, right)| {
            left.trim().eq_ignore_ascii_case("#REF!")
                && (right.trim() == "1" || quoted_constant(right.trim()))
        });
    if constant_error {
        return Some("constant-error-expression");
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

/// Excel string literals escape a quote by doubling it; concatenation is not a literal.
fn quoted_constant(text: &str) -> bool {
    let Some(inner) = text
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
    else {
        return false;
    };
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '"' && chars.next() != Some('"') {
            return false;
        }
    }
    true
}

/// Match tokens separately so whitespace cannot join names or hide an extra dependency.
fn row_parity(formula: &str) -> bool {
    let upper = formula.to_ascii_uppercase();
    let mut rest = upper.as_str();
    for token in ["MOD", "(", "ROW", "(", ")", ",", "2", ")", "="] {
        let Some(next) = rest.trim_ascii_start().strip_prefix(token) else {
            return false;
        };
        rest = next;
    }
    matches!(rest.trim_ascii(), "0" | "1")
}
