//! Deliberately narrow formula recognition; unknown encodings/grammar never prove equality.

/// These rules depend on the complete application-area value set, not an evaluated result.
pub(super) fn whole_area_rule(rule: &crate::regression::cf_inventory::scan::Rule) -> bool {
    if rule.truncated_formula
        || !rule.formulas.is_empty()
        || rule
            .attributes
            .keys()
            .any(|key| !["type", "dxfId", "priority", "stopIfTrue"].contains(&key.as_str()))
    {
        return false;
    }
    match rule.attributes.get("type").map(String::as_str) {
        Some("duplicateValues") => rule.child_elements.is_empty(),
        Some("dataBar") => {
            rule.child_elements.contains("dataBar")
                && rule.child_elements.iter().all(|name| {
                    ["dataBar", "cfvo", "color", "extLst", "ext", "x14:id"].contains(&name.as_str())
                })
                && super::data_bars::local_thresholds(rule)
        }
        Some("colorScale") => {
            !rule.thresholds.is_empty()
                && rule
                    .child_elements
                    .iter()
                    .all(|name| ["colorScale", "cfvo", "color"].contains(&name.as_str()))
                && rule.thresholds.iter().all(|threshold| {
                    if threshold
                        .keys()
                        .any(|key| !["type", "val", "gte"].contains(&key.as_str()))
                    {
                        return false;
                    }
                    match threshold.get("type").map(String::as_str) {
                        Some("min" | "max") => true,
                        Some("num" | "percent" | "percentile") => threshold
                            .get("val")
                            .and_then(|value| value.parse::<f64>().ok())
                            .is_some_and(f64::is_finite),
                        _ => false,
                    }
                })
        }
        _ => false,
    }
}

/// Decode XML text once before literal classification; CDATA is already literal text.
pub(super) fn literal_rule(rule: &crate::regression::cf_inventory::scan::Rule) -> bool {
    let mut normalized = rule.clone();
    if !rule.formula_cdata {
        let decoded: Option<Vec<_>> = rule
            .formulas
            .iter()
            .map(|formula| decode_entities(formula))
            .collect();
        let Some(formulas) = decoded else {
            return false;
        };
        normalized.formulas = formulas;
    }
    crate::regression::cf_inventory::scan::classify(&normalized) == "constant-cell-is-eligible"
}

pub(super) fn dummy_target(raw: &str) -> Option<String> {
    let decoded = decode_entities(raw)?;
    let mut quoted = false;
    let compact: String = decoded
        .chars()
        .filter(|&ch| {
            if ch == '"' {
                quoted = !quoted;
            }
            quoted || !ch.is_ascii_whitespace()
        })
        .collect();
    if quoted || !compact.is_ascii() {
        return None;
    }
    let upper = compact
        .strip_prefix('=')
        .unwrap_or(&compact)
        .to_ascii_uppercase();
    let digits = upper
        .strip_prefix("INDIRECT(\"DUMMY_FOR_COMPARISON")?
        .strip_suffix("!\"&ADDRESS(ROW(),COLUMN()))")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let target = format!("dummy_for_comparison{digits}");
    (target.len() <= 31).then_some(target)
}

pub(super) fn decode_entities(raw: &str) -> Option<String> {
    let mut result = String::new();
    let mut remaining = raw;
    while let Some(index) = remaining.find('&') {
        result.push_str(&remaining[..index]);
        remaining = &remaining[index + 1..];
        let end = remaining.find(';')?;
        let entity = &remaining[..end];
        let ch = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let value = if let Some(hex) = entity.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16).ok()?
                } else {
                    entity.strip_prefix('#')?.parse().ok()?
                };
                char::from_u32(value)?
            }
        };
        result.push(ch);
        remaining = &remaining[end + 1..];
    }
    result.push_str(remaining);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_fixed_same_coordinate_template_is_recognized() {
        assert_eq!(
            dummy_target("INDIRECT(\"Dummy_for_Comparison1!\"&amp;ADDRESS(ROW(),COLUMN()))"),
            Some("dummy_for_comparison1".into())
        );
        let spaced =
            " indirect ( &quot;Dummy_for_Comparison2!&quot; &amp; ADDRESS( ROW(), COLUMN() ) ) ";
        assert_eq!(dummy_target(spaced), Some("dummy_for_comparison2".into()));
        for formula in [
            "INDIRECT(\"Dummy_for_Comparison1!\"&ADDRESS(ROW()+1,COLUMN()))",
            "INDIRECT(\"Other!\"&ADDRESS(ROW(),COLUMN()))",
            "= =INDIRECT(\"Dummy_for_Comparison1!\"&ADDRESS(ROW(),COLUMN()))",
            "INDIRECT(\"Dummy_for_Comparison1!\"&ADDRESS(ROW(),COLUMN()))+1",
        ] {
            assert_eq!(dummy_target(formula), None, "{formula}");
        }
    }
}
