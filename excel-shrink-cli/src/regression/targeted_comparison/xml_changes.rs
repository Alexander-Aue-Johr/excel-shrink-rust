//! Byte-based XML evidence for selection and conservative change attribution.
use super::{
    cell_stream::{Cells, Writer},
    Candidate,
};
use crate::regression::{
    office_plan::build_workbook_visual_plan,
    streaming_xml::{self, Event},
    xml_views::{Attributes, Kind},
};
use anyhow::{ensure, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::Path,
    sync::Arc,
};
use zip::ZipArchive;

#[derive(Clone, Default)]
pub(super) struct Sheet {
    pub cells: Cells,
    pub rows: Arc<BTreeMap<u32, BTreeMap<String, String>>>,
    pub columns: Vec<u8>,
    pub defaults: Vec<u8>,
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub sheets: BTreeMap<String, Sheet>,
    pub context: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct XmlChange {
    pub sheet: String,
    pub location: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub context_before: String,
    pub context_after: String,
    pub also_in_original_open_save: bool,
    #[serde(skip)]
    visible_cell: bool,
}

#[derive(Serialize)]
pub(super) struct Transition {
    pub label: String,
    pub before_file: std::path::PathBuf,
    pub after_file: std::path::PathBuf,
    pub changes: Vec<XmlChange>,
}

fn cell_digest(cell: super::cell_stream::Cell) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(64);
    for byte in cell.digest {
        write!(result, "{byte:02x}").expect("String write");
    }
    result
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl Snapshot {
    /// Stream cell fingerprints to owned scratch files; retain only layout metadata in memory.
    pub fn read(path: &Path) -> Result<Self> {
        let plan = build_workbook_visual_plan(path)?;
        let mut archive = ZipArchive::new(File::open(path)?)?;
        let mut context = Sha256::new();
        for part in ["xl/styles.xml", "xl/sharedStrings.xml"] {
            if let Ok(mut entry) = archive.by_name(part) {
                context.update(part.as_bytes());
                let mut buffer = [0u8; 128 * 1024];
                loop {
                    let count = entry.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    context.update(&buffer[..count]);
                }
            }
        }
        let mut sheets = BTreeMap::new();
        for sheet in plan.sheets {
            let mut model = Sheet::default();
            let mut cells = Writer::new()?;
            streaming_xml::scan(archive.by_name(&sheet.sheet_file)?, |event| {
                if let Event::Tag(tag) = &event {
                    if matches!(tag.kind, Kind::Start | Kind::Empty) {
                        match tag.local() {
                            b"row" => {
                                let row: u32 =
                                    std::str::from_utf8(tag.attr(b"r")?.unwrap_or(b"0"))?
                                        .parse()?;
                                ensure!(
                                    (1..=1_048_576).contains(&row),
                                    "Unsupported or invalid worksheet row index"
                                );
                                let mut attributes = BTreeMap::new();
                                for attribute in Attributes::new(tag.attrs) {
                                    let (key, value) = attribute?;
                                    if [
                                        b"ht".as_slice(),
                                        b"customHeight",
                                        b"hidden",
                                        b"s",
                                        b"customFormat",
                                    ]
                                    .contains(&key)
                                    {
                                        attributes.insert(
                                            String::from_utf8(key.to_vec())?,
                                            String::from_utf8(value.to_vec())?,
                                        );
                                    }
                                }
                                if !attributes.is_empty() {
                                    Arc::make_mut(&mut model.rows).insert(row, attributes);
                                }
                            }
                            b"col" => {
                                model.columns.extend_from_slice(tag.name);
                                model.columns.extend_from_slice(tag.attrs);
                            }
                            b"sheetFormatPr" => {
                                model.defaults.extend_from_slice(tag.name);
                                model.defaults.extend_from_slice(tag.attrs);
                            }
                            _ => {}
                        }
                    }
                }
                cells.event(&event)
            })?;
            model.cells = cells.finish()?;
            sheets.insert(sheet.sheet, model);
        }
        Ok(Self {
            sheets,
            context: format!("{:x}", context.finalize()),
        })
    }

    /// Compare actual XML content, with style/string context, rather than addresses alone.
    pub fn changes(&self, after: &Self) -> Result<Vec<XmlChange>> {
        let mut changes = Vec::new();
        let names: BTreeSet<_> = self.sheets.keys().chain(after.sheets.keys()).collect();
        for name in names {
            let empty = Sheet::default();
            let left = self.sheets.get(name).unwrap_or(&empty);
            let right = after.sheets.get(name).unwrap_or(&empty);
            left.cells.changes(&right.cells, |change| {
                let coordinate = change.coordinate();
                let address = coordinate.address();
                changes.push(XmlChange {
                    sheet: name.clone(),
                    location: format!("cell:{address}"),
                    before: change.before.map(cell_digest),
                    after: change.after.map(cell_digest),
                    context_before: if change.before.is_some() {
                        self.context.clone()
                    } else {
                        "absent".into()
                    },
                    context_after: if change.after.is_some() {
                        after.context.clone()
                    } else {
                        "absent".into()
                    },
                    also_in_original_open_save: false,
                    visible_cell: change.visible(),
                });
                Ok(())
            })?;
            let rows: BTreeSet<_> = left.rows.keys().chain(right.rows.keys()).collect();
            for row in rows {
                let a = left
                    .rows
                    .get(row)
                    .map(serde_json::to_vec)
                    .transpose()
                    .expect("string attributes serialize");
                let b = right
                    .rows
                    .get(row)
                    .map(serde_json::to_vec)
                    .transpose()
                    .expect("string attributes serialize");
                self.push_change(
                    after,
                    &mut changes,
                    name,
                    &format!("row:{row}"),
                    a.as_deref(),
                    b.as_deref(),
                );
            }
            self.push_change(
                after,
                &mut changes,
                name,
                "columns",
                Some(&left.columns),
                Some(&right.columns),
            );
            self.push_change(
                after,
                &mut changes,
                name,
                "defaults",
                Some(&left.defaults),
                Some(&right.defaults),
            );
        }
        Ok(changes)
    }

    fn push_change(
        &self,
        after: &Self,
        changes: &mut Vec<XmlChange>,
        sheet: &str,
        location: &str,
        a: Option<&[u8]>,
        b: Option<&[u8]>,
    ) {
        if a == b {
            return;
        }
        changes.push(XmlChange {
            sheet: sheet.into(),
            location: location.into(),
            before: a.map(hash),
            after: b.map(hash),
            context_before: if a.is_some() {
                self.context.clone()
            } else {
                "absent".into()
            },
            context_after: if b.is_some() {
                after.context.clone()
            } else {
                "absent".into()
            },
            also_in_original_open_save: false,
            visible_cell: false,
        });
    }

    /// An exact matching transition is evidence of overlap, never proof of exclusive causality.
    pub fn same_change(a: &XmlChange, b: &XmlChange) -> bool {
        a.sheet == b.sheet
            && a.location == b.location
            && a.before == b.before
            && a.after == b.after
            && a.context_before == b.context_before
            && a.context_after == b.context_after
    }
}

impl Sheet {
    pub fn fixed_height(&self, row: u32) -> Option<(&str, bool)> {
        let attributes = self.rows.get(&row)?;
        if !matches!(
            attributes.get("customHeight").map(String::as_str),
            Some("1" | "true")
        ) {
            return None;
        }
        Some((
            attributes.get("ht")?.as_str(),
            matches!(
                attributes.get("hidden").map(String::as_str),
                Some("1" | "true")
            ),
        ))
    }
}

/// Content-bearing cells retain their own location; empty changes also select their row probe.
pub(super) fn cell_candidates(changes: &[XmlChange]) -> BTreeSet<Candidate> {
    changes
        .iter()
        .filter_map(|change| {
            let cell = change.location.strip_prefix("cell:")?;
            let address = if change.visible_cell {
                cell.to_owned()
            } else {
                format!(
                    "A{}",
                    cell.trim_start_matches(|c: char| c.is_ascii_alphabetic())
                )
            };
            Some(Candidate {
                sheet: change.sheet.clone(),
                cell: address,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equal_addresses_are_not_equal_changes() {
        let a = XmlChange {
            sheet: "Rows".into(),
            location: "cell:A1".into(),
            before: Some("one".into()),
            after: None,
            context_before: "styles".into(),
            context_after: "styles".into(),
            also_in_original_open_save: false,
            visible_cell: false,
        };
        let mut b = a.clone();
        b.before = Some("different content".into());
        assert!(!Snapshot::same_change(&a, &b));
        assert!(Snapshot::same_change(&a, &a));
    }
}
