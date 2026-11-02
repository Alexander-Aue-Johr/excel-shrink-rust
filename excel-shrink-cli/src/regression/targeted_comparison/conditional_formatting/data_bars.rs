//! Data bars have whole-area value dependencies; their display settings are static XML.
use crate::regression::{
    cf_inventory::scan::{Block, Rule},
    xml_views::{Attributes, Tag},
};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Links {
    standard: BTreeMap<String, Vec<(usize, String)>>,
    extension: BTreeMap<String, Vec<(usize, String)>>,
}

impl Links {
    /// Keep both representations conservative unless their IDs and application ranges agree.
    pub fn record(&mut self, block: &Block, index: usize) -> bool {
        let mut complete = true;
        for rule in &block.rules {
            if rule.attributes.get("type").map(String::as_str) != Some("dataBar") {
                continue;
            }
            if block.qualified_name == "x14:conditionalFormatting" {
                if let Some(id) = rule.attributes.get("id").filter(|id| !id.is_empty()) {
                    self.extension
                        .entry(id.clone())
                        .or_default()
                        .push((index, block.sqref.clone()));
                } else {
                    complete = false;
                }
            } else if rule.child_elements.contains("x14:id") {
                if rule.extension_ids.len() == 1 && !rule.extension_ids[0].is_empty() {
                    self.standard
                        .entry(rule.extension_ids[0].clone())
                        .or_default()
                        .push((index, block.sqref.clone()));
                } else {
                    complete = false;
                }
            }
        }
        complete
    }

    /// Reject missing, duplicate, differently scoped, or unsupported companion representations.
    pub fn constrain(&self, supported: &mut [bool]) {
        let initial = supported.to_vec();
        for (own, partners) in [
            (&self.standard, &self.extension),
            (&self.extension, &self.standard),
        ] {
            for (id, entries) in own {
                for (index, range) in entries {
                    let matched = entries.len() == 1
                        && partners.get(id).is_some_and(|matches| {
                            matches.len() == 1 && matches[0].1 == *range && initial[matches[0].0]
                        });
                    supported[*index] &= matched;
                }
            }
        }
    }
}

const LINK_EXTENSION: &[u8] = b"{B025F937-C7B1-47D3-B67F-A62EFF666E3E}";
const BAR_SETTINGS: &[&[u8]] = &[
    b"minLength",
    b"maxLength",
    b"showValue",
    b"border",
    b"gradient",
    b"direction",
    b"negativeBarColorSameAsPositive",
    b"negativeBarBorderColorSameAsPositive",
    b"axisPosition",
];

/// Unknown bar settings or extension payloads retain the namespace/content fallback.
pub(in crate::regression::targeted_comparison) fn static_attributes(tag: Tag<'_>) -> Result<bool> {
    let allowed: &[&[u8]] = match tag.name {
        b"dataBar" => &[b"minLength", b"maxLength", b"showValue"],
        b"x14:dataBar" => BAR_SETTINGS,
        b"x14:cfvo" => &[b"type", b"val", b"gte"],
        b"color"
        | b"x14:fillColor"
        | b"x14:borderColor"
        | b"x14:negativeFillColor"
        | b"x14:negativeBorderColor"
        | b"x14:axisColor" => &[b"rgb", b"theme", b"tint", b"indexed", b"auto"],
        b"extLst" | b"x14:id" => &[],
        b"ext" => &[b"uri"],
        _ => return Ok(false),
    };
    for attr in Attributes::new(tag.attrs) {
        let (name, value) = attr?;
        if [b"xmlns:x14".as_slice(), b"xmlns:xm"].contains(&name) {
            continue;
        }
        if !allowed.contains(&name)
            || (tag.name == b"ext" && name == b"uri" && value != LINK_EXTENSION)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Require two local thresholds; formula thresholds may depend on cells outside the area.
pub(super) fn local_thresholds(rule: &Rule) -> bool {
    rule.thresholds.len() == 2
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

/// Adapt only static x14 data bars; raw fingerprints retain settings, colours and link IDs.
pub(super) fn normalize_x14(block: &mut Block) -> bool {
    const CHILDREN: &[&str] = &[
        "x14:cfRule",
        "x14:dataBar",
        "x14:cfvo",
        "x14:fillColor",
        "x14:borderColor",
        "x14:negativeFillColor",
        "x14:negativeBorderColor",
        "x14:axisColor",
        "xm:sqref",
    ];
    if block.qualified_name != "x14:conditionalFormatting"
        || block.sqref.is_empty()
        || block
            .attributes
            .keys()
            .any(|key| !["xmlns:xm", "xmlns:x14"].contains(&key.as_str()))
        || block.rules.is_empty()
        || block
            .child_elements
            .iter()
            .any(|name| !CHILDREN.contains(&name.as_str()))
        || block.rules.iter().any(|rule| {
            rule.attributes.get("type").map(String::as_str) != Some("dataBar")
                || rule
                    .attributes
                    .keys()
                    .any(|key| !["type", "id", "priority", "stopIfTrue"].contains(&key.as_str()))
                || rule.truncated_formula
                || !rule.formulas.is_empty()
                || !rule.child_elements.contains("x14:dataBar")
                || rule.thresholds.len() != 2
                || rule.thresholds.iter().any(|threshold| {
                    threshold
                        .keys()
                        .any(|key| !["type", "val", "gte"].contains(&key.as_str()))
                        || !matches!(
                            threshold.get("type").map(String::as_str),
                            Some("autoMin" | "autoMax" | "min" | "max")
                        )
                })
        })
    {
        return false;
    }
    block.qualified_name = "conditionalFormatting".into();
    block.attributes.clear();
    block.attributes.insert("sqref".into(), block.sqref.clone());
    block.child_elements = BTreeSet::from([
        "cfRule".into(),
        "dataBar".into(),
        "cfvo".into(),
        "color".into(),
    ]);
    for rule in &mut block.rules {
        rule.attributes.remove("id");
        rule.child_elements = BTreeSet::from(["dataBar".into(), "cfvo".into(), "color".into()]);
        for threshold in &mut rule.thresholds {
            match threshold.get("type").map(String::as_str) {
                Some("autoMin") => {
                    threshold.insert("type".into(), "min".into());
                }
                Some("autoMax") => {
                    threshold.insert("type".into(), "max".into());
                }
                _ => {}
            }
        }
    }
    true
}
