//! Resolve the column-layout prefix that can position a conditional-format area.
use super::style_identity::StyleIdentity;
use crate::regression::xml_views::{Attributes, Kind, Tag};
use anyhow::Result;
use std::collections::BTreeMap;

type Properties = BTreeMap<Vec<u8>, Vec<u8>>;

#[derive(Clone)]
struct Run {
    first: u32,
    last: u32,
    properties: Properties,
}

#[derive(Clone, Default)]
pub(super) struct ColumnLayout {
    runs: Vec<Run>,
    resolved: bool,
}

impl ColumnLayout {
    /// Only a parsed worksheet can establish that an absent column definition is known.
    pub fn new() -> Self {
        Self {
            runs: Vec::new(),
            resolved: true,
        }
    }

    /// Keep all supported properties and resolve style IDs through their actual formats.
    pub fn observe(&mut self, tag: Tag<'_>, styles: &StyleIdentity) -> Result<()> {
        let number = |key| -> Result<Option<u32>> {
            Ok(tag
                .attr(key)?
                .and_then(|value| std::str::from_utf8(value).ok()?.parse().ok()))
        };
        let (Some(first), Some(last)) = (number(b"min")?, number(b"max")?) else {
            self.resolved = false;
            return Ok(());
        };
        if tag.kind != Kind::Empty || first == 0 || first > last || last > 16_384 {
            self.resolved = false;
            return Ok(());
        }
        let mut properties = Properties::new();
        for attribute in Attributes::new(tag.attrs) {
            let (key, value) = attribute?;
            if key == b"min" || key == b"max" {
                continue;
            }
            if ![
                b"width".as_slice(),
                b"style",
                b"customWidth",
                b"bestFit",
                b"hidden",
                b"outlineLevel",
                b"collapsed",
                b"phonetic",
            ]
            .contains(&key)
            {
                self.resolved = false;
                return Ok(());
            }
            let value = if key == b"style" {
                styles.index_text(value)?.into_bytes()
            } else {
                value.to_vec()
            };
            if properties.insert(key.to_vec(), value).is_some() {
                self.resolved = false;
            }
        }
        if let Some(previous) = self.runs.last_mut() {
            if previous.last >= first {
                self.resolved = false;
            }
            if previous.last + 1 == first && previous.properties == properties {
                previous.last = last;
                return Ok(());
            }
        }
        self.runs.push(Run {
            first,
            last,
            properties,
        });
        Ok(())
    }

    /// Columns through the area's right edge determine its width and horizontal position.
    /// Later columns do not change this prefix. Unknown metadata cannot prove equality.
    pub fn same_prefix(&self, other: &Self, through: u32) -> bool {
        self.resolved && other.resolved && self.prefix(through).eq(other.prefix(through))
    }

    fn prefix(&self, through: u32) -> impl Iterator<Item = (u32, u32, &Properties)> {
        self.runs
            .iter()
            .filter(move |run| run.first <= through)
            .map(move |run| (run.first, run.last.min(through), &run.properties))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regression::xml_views::Reader;

    fn columns(xml: &[u8]) -> Result<ColumnLayout> {
        let mut result = ColumnLayout::new();
        let mut reader = Reader::<32>::new(xml)?;
        while let Some(tag) = reader.next()? {
            if tag.name == b"col" {
                result.observe(tag, &StyleIdentity::default())?;
            }
        }
        Ok(result)
    }

    #[test]
    fn columns_after_the_scope_do_not_change_its_prefix() -> Result<()> {
        let a = columns(br#"<cols><col min="1" max="62" width="12"/><col min="63" max="71" width="15"/></cols>"#)?;
        let b = columns(br#"<cols><col width="12" min="1" max="62"/><col min="63" max="71" width="10"/></cols>"#)?;
        assert!(a.same_prefix(&b, 49));
        assert!(!a.same_prefix(&b, 63));
        let split = columns(br#"<cols><col min="1" max="20" width="12"/><col min="21" max="62" width="12"/></cols>"#)?;
        assert!(a.same_prefix(&split, 49));
        Ok(())
    }

    #[test]
    fn earlier_columns_hidden_state_and_unresolved_metadata_keep_the_fallback() -> Result<()> {
        let a = columns(br#"<cols><col min="1" max="20" width="12"/></cols>"#)?;
        for xml in [
            br#"<cols><col min="1" max="20" width="13"/></cols>"#.as_slice(),
            br#"<cols><col min="1" max="20" width="12" hidden="1"/></cols>"#,
            br#"<cols><col min="1" max="20" width="12"/><col min="63" max="71" unknown="1"/></cols>"#,
            br#"<cols><col min="1" max="20" width="12"/><col min="63" max="10"/></cols>"#,
            br#"<cols><col min="1" max="20" width="12"/><col min="10" max="30" width="12"/></cols>"#,
        ] {
            assert!(!a.same_prefix(&columns(xml)?, 15));
        }
        assert!(!a.same_prefix(&ColumnLayout::default(), 15));
        Ok(())
    }
}
