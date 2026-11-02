//! Stream worksheet structure and collect only conditional-formatting metadata.
use crate::regression::{
    streaming_xml::{self, Event},
    xml_views::{Attributes, Kind},
};
use anyhow::Result;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

#[derive(Clone, Default, Serialize)]
pub(in crate::regression) struct Rule {
    pub attributes: BTreeMap<String, String>,
    pub formulas: Vec<String>,
    pub child_elements: BTreeSet<String>,
    pub truncated_formula: bool,
    pub formula_cdata: bool,
    /// Raw IDs linking legacy data bars to their x14 representation in the same worksheet.
    pub extension_ids: Vec<String>,
    pub thresholds: Vec<BTreeMap<String, String>>,
    pub classification: String,
}

#[derive(Default, Serialize)]
pub(in crate::regression) struct Block {
    pub start: usize,
    pub end: usize,
    pub qualified_name: String,
    pub attributes: BTreeMap<String, String>,
    pub sqref: String,
    pub rules: Vec<Rule>,
    pub child_elements: BTreeSet<String>,
    pub fallback_reason: String,
    pub tile_proxy_upper_bound: Option<u64>,
    pub raw_xml: Option<String>,
    pub raw_file: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextTarget {
    Formula,
    ApplicationRange,
    ExtensionId,
}

fn attributes(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    Attributes::new(bytes)
        .map(|entry| {
            let (name, value) = entry?;
            Ok((
                String::from_utf8(name.to_vec())?,
                String::from_utf8(value.to_vec())?,
            ))
        })
        .collect()
}

/// Recognize literal numbers and Excel strings with doubled quotes, never expressions.
fn literal(value: &str) -> bool {
    let value = value.trim();
    // Entity-encoded quotes can hide an expression boundary; do not classify undecoded entities.
    if value.contains('&') {
        return false;
    }
    if value.parse::<f64>().is_ok_and(f64::is_finite) {
        return true;
    }
    let bytes = value.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || bytes[bytes.len() - 1] != b'"' {
        return false;
    }
    let mut index = 1;
    while index < bytes.len() - 1 {
        if bytes[index] == b'"' {
            if index + 1 >= bytes.len() - 1 || bytes[index + 1] != b'"' {
                return false;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    true
}

/// Keep literal-threshold eligibility narrow: no expression evaluation or equality claim.
pub(in crate::regression) fn classify(rule: &Rule) -> &'static str {
    if rule.truncated_formula {
        return "formula-summary-truncated";
    }
    let kind = rule
        .attributes
        .get("type")
        .map(String::as_str)
        .unwrap_or("missing");
    if kind != "cellIs" {
        return "rule-semantics-not-supported";
    }
    if rule
        .attributes
        .keys()
        .any(|key| !["type", "operator", "dxfId", "priority", "stopIfTrue"].contains(&key.as_str()))
        || rule.child_elements.iter().any(|name| name != "formula")
    {
        return "unmodeled-rule-content";
    }
    let expected = match rule.attributes.get("operator").map(String::as_str) {
        Some("between" | "notBetween") => 2,
        Some(
            "equal" | "notEqual" | "lessThan" | "lessThanOrEqual" | "greaterThan"
            | "greaterThanOrEqual",
        ) => 1,
        _ => return "operator-not-supported",
    };
    if rule.formulas.len() != expected {
        return "unexpected-formula-count";
    }
    if !rule.formulas.iter().all(|value| literal(value)) {
        return "nonconstant-or-unparsed-operand";
    }
    "constant-cell-is-eligible"
}

/// Parse all XML tags, including extension rules, without accumulating rows or cells.
/// Partial blocks stay available to the caller if later malformed XML aborts the sheet.
pub(in crate::regression) fn scan(
    input: impl Read,
    blocks: &mut Vec<Block>,
    rows_per_tile: u32,
    columns_per_tile: u32,
) -> Result<()> {
    let mut block_depth = None;
    let mut rule_depth = None;
    let mut text_target: Option<(usize, TextTarget)> = None;
    let mut text_bytes = Vec::new();
    streaming_xml::scan_with_offsets(input, |event| {
        let cdata = matches!(&event, Event::Cdata(_));
        match event {
            Event::Tag(tag) => {
                let local = tag.local();
                let opening = matches!(tag.kind, Kind::Start | Kind::Empty);
                if opening && local == b"conditionalFormatting" && block_depth.is_none() {
                    let attrs = attributes(tag.attrs)?;
                    blocks.push(Block {
                        start: tag.start,
                        qualified_name: String::from_utf8(tag.name.to_vec())?,
                        sqref: attrs.get("sqref").cloned().unwrap_or_default(),
                        attributes: attrs,
                        ..Block::default()
                    });
                    block_depth = Some(tag.depth);
                }
                if let Some(depth) = block_depth {
                    let block = blocks.last_mut().unwrap();
                    if opening && tag.depth > depth {
                        block
                            .child_elements
                            .insert(String::from_utf8(tag.name.to_vec())?);
                    }
                    if opening && local == b"cfRule" {
                        block.rules.push(Rule {
                            attributes: attributes(tag.attrs)?,
                            ..Rule::default()
                        });
                        rule_depth = Some(tag.depth);
                    } else if opening && rule_depth.is_some_and(|depth| tag.depth > depth) {
                        block
                            .rules
                            .last_mut()
                            .unwrap()
                            .child_elements
                            .insert(String::from_utf8(tag.name.to_vec())?);
                    }
                    if opening && local == b"cfvo" && rule_depth.is_some() {
                        block
                            .rules
                            .last_mut()
                            .unwrap()
                            .thresholds
                            .push(attributes(tag.attrs)?);
                    }
                    if tag.kind == Kind::Start
                        && matches!(local, b"formula" | b"f")
                        && rule_depth.is_some()
                    {
                        block.rules.last_mut().unwrap().formulas.push(String::new());
                        text_target = Some((tag.depth, TextTarget::Formula));
                        text_bytes.clear();
                    }
                    if tag.kind == Kind::Start && local == b"sqref" {
                        text_target = Some((tag.depth, TextTarget::ApplicationRange));
                        text_bytes.clear();
                    }
                    if tag.kind == Kind::Start && tag.name == b"x14:id" && rule_depth.is_some() {
                        text_target = Some((tag.depth, TextTarget::ExtensionId));
                        text_bytes.clear();
                    }
                    if tag.kind == Kind::End
                        && text_target.is_some_and(|(depth, _)| depth == tag.depth)
                    {
                        let bytes = std::mem::take(&mut text_bytes);
                        let truncated = text_target.unwrap().1 == TextTarget::Formula
                            && block.rules.last().unwrap().truncated_formula;
                        let text = if truncated {
                            String::from_utf8_lossy(&bytes).into_owned()
                        } else {
                            String::from_utf8(bytes)?
                        };
                        if text_target.unwrap().1 == TextTarget::ApplicationRange {
                            block.sqref = text;
                        } else if text_target.unwrap().1 == TextTarget::ExtensionId {
                            block.rules.last_mut().unwrap().extension_ids.push(text);
                        } else {
                            *block.rules.last_mut().unwrap().formulas.last_mut().unwrap() = text;
                        }
                        text_target = None;
                    }
                    if matches!(tag.kind, Kind::End | Kind::Empty) && rule_depth == Some(tag.depth)
                    {
                        let rule = block.rules.last_mut().unwrap();
                        rule.classification = classify(rule).into();
                        rule_depth = None;
                    }
                    if matches!(tag.kind, Kind::End | Kind::Empty) && tag.depth == depth {
                        block.end = tag.end;
                        block.tile_proxy_upper_bound =
                            tile_proxy(&block.sqref, rows_per_tile, columns_per_tile);
                        block.fallback_reason = if block.tile_proxy_upper_bound.is_none() {
                            "unparsed-application-range"
                        } else if block.qualified_name != "conditionalFormatting" {
                            "namespace-or-extension-needs-review"
                        } else if block.attributes.keys().any(|name| name != "sqref")
                            || block
                                .child_elements
                                .iter()
                                .any(|name| !["cfRule", "formula"].contains(&name.as_str()))
                        {
                            "unmodeled-block-content"
                        } else if block.rules.is_empty() {
                            "missing-rules"
                        } else if block
                            .rules
                            .iter()
                            .all(|rule| rule.classification == "constant-cell-is-eligible")
                        {
                            "eligible-pending-values-and-format-comparison"
                        } else {
                            "unsupported-rule-in-block"
                        }
                        .into();
                        block_depth = None;
                    }
                }
            }
            Event::Text(bytes) | Event::Cdata(bytes) => {
                if let Some((_, target)) = text_target {
                    let block = blocks.last_mut().unwrap();
                    if target != TextTarget::Formula {
                        text_bytes.extend_from_slice(bytes);
                    } else {
                        let rule = block.rules.last_mut().unwrap();
                        rule.formula_cdata |= cdata;
                        if text_bytes.len() + bytes.len() <= 8192 {
                            text_bytes.extend_from_slice(bytes);
                        } else {
                            rule.truncated_formula = true;
                        }
                    }
                }
            }
        }
        Ok(())
    })
}

fn coordinate(raw: &str) -> Option<(u32, u32)> {
    let raw = raw.replace('$', "");
    let split = raw.find(|c: char| c.is_ascii_digit()).unwrap_or(raw.len());
    let mut column = 0u32;
    for byte in raw[..split].bytes() {
        if !byte.is_ascii_uppercase() {
            return None;
        }
        column = column
            .checked_mul(26)?
            .checked_add(u32::from(byte - b'A') + 1)?;
    }
    let row = if split == raw.len() {
        0
    } else {
        raw[split..].parse().ok()?
    };
    if column > 16384 || row > 1048576 || (column == 0 && row == 0) {
        return None;
    }
    Some((row, column))
}

/// A fixed-grid cost proxy, not Excel pixel geometry. Overlapping references are summed conservatively.
fn tile_proxy(sqref: &str, rows: u32, columns: u32) -> Option<u64> {
    if rows == 0 || columns == 0 || sqref.trim().is_empty() {
        return None;
    }
    let mut total = 0u64;
    for area in sqref.split_ascii_whitespace() {
        let (start, end) = area.split_once(':').unwrap_or((area, area));
        let (mut top, mut left) = coordinate(start)?;
        let (mut bottom, mut right) = coordinate(end)?;
        if top == 0 && bottom == 0 {
            top = 1;
            bottom = 1048576;
        }
        if left == 0 && right == 0 {
            left = 1;
            right = 16384;
        }
        if top == 0 || left == 0 || top > bottom || left > right {
            return None;
        }
        total += u64::from((bottom - 1) / rows - (top - 1) / rows + 1)
            * u64::from((right - 1) / columns - (left - 1) / columns + 1);
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_constants_are_distinct_from_references_and_expressions() {
        assert!(!literal("\"a&#34;&amp;A1&amp;&#34;b\""));
        for value in [
            "0",
            "-1.2E3",
            "\"\"",
            "\".\"",
            "\"\"\" \"\"\"",
            "\"SUM(A1)\"",
        ] {
            assert!(literal(value), "{value}");
        }
        for value in [
            "A1",
            "NOW()",
            "1+2",
            "\"a\"&A1&\"b\"",
            "NaN",
            "1e999",
            "\"unterminated",
        ] {
            assert!(!literal(value), "{value}");
        }
    }
    #[test]
    fn numeric_rules_are_eligible_but_references_and_extensions_fall_back() -> Result<()> {
        let xml = br#"<worksheet><conditionalFormatting sqref="J5:J1048576"><cfRule type="cellIs" operator="equal"><formula>0</formula></cfRule></conditionalFormatting><conditionalFormatting sqref="A1:A9"><cfRule type="cellIs" operator="equal"><formula>B1</formula></cfRule></conditionalFormatting><extLst><x:conditionalFormatting xmlns:x="extension"><x:cfRule type="expression"><xm:f xmlns:xm="m">A1</xm:f></x:cfRule><xm:sqref xmlns:xm="m">B2:B7</xm:sqref></x:conditionalFormatting></extLst></worksheet>"#;
        let mut blocks = Vec::new();
        scan(xml.as_slice(), &mut blocks, 256, 32)?;
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].tile_proxy_upper_bound, Some(4096));
        assert!(blocks[0].fallback_reason.starts_with("eligible-"));
        assert_eq!(
            blocks[1].rules[0].classification,
            "nonconstant-or-unparsed-operand"
        );
        assert_eq!(blocks[2].sqref, "B2:B7");
        assert_eq!(&xml[blocks[2].start..blocks[2].end][..4], b"<x:c");
        assert_eq!(blocks[2].rules[0].formulas, vec!["A1"]);
        Ok(())
    }
    #[test]
    fn whole_columns_and_disjoint_ranges_have_explicit_costs() {
        assert_eq!(tile_proxy("$A:$A", 256, 32), Some(4096));
        assert_eq!(tile_proxy("A1:B2 D513:D514", 256, 32), Some(2));
        assert_eq!(tile_proxy("invalid", 256, 32), None);
    }
}
