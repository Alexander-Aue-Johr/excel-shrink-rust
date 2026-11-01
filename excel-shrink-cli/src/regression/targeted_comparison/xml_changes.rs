//! Byte-based XML evidence for selection and conservative change attribution.
use super::{candidates::read_cells, Candidate};
use crate::regression::{
    office_plan::build_workbook_visual_plan,
    worksheet_stream::WorksheetStream,
    xml_views::{Attributes, Kind, Reader},
};
use anyhow::{ensure, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::Path,
};
use zip::ZipArchive;

#[derive(Clone, Default)]
pub(super) struct Sheet {
    pub cells: BTreeMap<String, Vec<u8>>,
    pub rows: BTreeMap<u32, BTreeMap<String, String>>,
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
}

#[derive(Serialize)]
pub(super) struct Transition {
    pub label: String,
    pub before_file: std::path::PathBuf,
    pub after_file: std::path::PathBuf,
    pub changes: Vec<XmlChange>,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl Snapshot {
    /// Retain bounded cell XML and layout attributes, without constructing an XML DOM.
    pub fn read(path: &Path) -> Result<Self> {
        let plan = build_workbook_visual_plan(path)?;
        let mut archive = ZipArchive::new(File::open(path)?)?;
        let mut context = Sha256::new();
        for part in ["xl/styles.xml", "xl/sharedStrings.xml"] {
            if let Ok(mut entry) = archive.by_name(part) {
                ensure!(
                    entry.size() <= 64 * 1024 * 1024,
                    "XML context exceeds analysis limit"
                );
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                context.update(part.as_bytes());
                context.update(bytes);
            }
        }
        let mut sheets = BTreeMap::new();
        for sheet in plan.sheets {
            let mut model = Sheet {
                cells: read_cells(path, &sheet.sheet_file)?,
                ..Sheet::default()
            };
            let mut stream = WorksheetStream::new(archive.by_name(&sheet.sheet_file)?);
            let mut fragment = Vec::new();
            while stream.next(&mut fragment)? {
                let mut reader = Reader::<128>::new(&fragment)?;
                while let Some(tag) = reader.next()? {
                    if !matches!(tag.kind, Kind::Start | Kind::Empty) {
                        continue;
                    }
                    match tag.local() {
                        b"row" => {
                            let mut attributes = BTreeMap::new();
                            let row: u32 =
                                std::str::from_utf8(tag.attr(b"r")?.unwrap_or(b"0"))?.parse()?;
                            ensure!(
                                (1..=1_048_576).contains(&row),
                                "Unsupported or invalid worksheet row index"
                            );
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
                            model.rows.insert(row, attributes);
                        }
                        b"col" => model
                            .columns
                            .extend_from_slice(&fragment[tag.start..tag.end]),
                        b"sheetFormatPr" => model
                            .defaults
                            .extend_from_slice(&fragment[tag.start..tag.end]),
                        _ => {}
                    }
                }
            }
            sheets.insert(sheet.sheet, model);
        }
        Ok(Self {
            sheets,
            context: format!("{:x}", context.finalize()),
        })
    }

    /// Compare actual XML content, with style/string context, rather than addresses alone.
    pub fn changes(&self, after: &Self) -> Vec<XmlChange> {
        let mut changes = Vec::new();
        let names: BTreeSet<_> = self.sheets.keys().chain(after.sheets.keys()).collect();
        for name in names {
            let empty = Sheet::default();
            let left = self.sheets.get(name).unwrap_or(&empty);
            let right = after.sheets.get(name).unwrap_or(&empty);
            let cells: BTreeSet<_> = left.cells.keys().chain(right.cells.keys()).collect();
            for cell in cells {
                self.push_change(
                    after,
                    &mut changes,
                    name,
                    &format!("cell:{cell}"),
                    left.cells.get(cell).map(Vec::as_slice),
                    right.cells.get(cell).map(Vec::as_slice),
                );
            }
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
        changes
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
pub(super) fn cell_candidates(
    before: &Snapshot,
    after: &Snapshot,
    changes: &[XmlChange],
) -> BTreeSet<Candidate> {
    changes
        .iter()
        .filter_map(|change| {
            let cell = change.location.strip_prefix("cell:")?;
            let visible = [before, after].iter().any(|snapshot| {
                snapshot
                    .sheets
                    .get(&change.sheet)
                    .and_then(|sheet| sheet.cells.get(cell))
                    .is_some_and(|raw| {
                        [b"<v".as_slice(), b"<f", b"<is"].iter().any(|pattern| {
                            raw.windows(pattern.len()).any(|bytes| bytes == *pattern)
                        })
                    })
            });
            let address = if visible {
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
        };
        let mut b = a.clone();
        b.before = Some("different content".into());
        assert!(!Snapshot::same_change(&a, &b));
        assert!(Snapshot::same_change(&a, &a));
    }
}
