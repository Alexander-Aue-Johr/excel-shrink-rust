//! Byte-based XML evidence for selection and conservative change attribution.
use super::{
    cell_stream::{Cells, Writer},
    conditional_formatting::xml,
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
    pub value_namespace_supported: bool,
    pub cells: Cells,
    pub rows: Arc<BTreeMap<u32, BTreeMap<String, String>>>,
    pub columns: Vec<u8>,
    pub defaults: Vec<u8>,
    pub conditional: Arc<xml::Catalogue>,
    pub cf_layout: String,
    pub bottom: u32,
    pub right: u32,
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub sheets: BTreeMap<String, Sheet>,
    pub context: String,
    pub cf_context: String,
    pub sheet_names: Option<BTreeSet<String>>,
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
        let mut workbook = Vec::new();
        archive
            .by_name("xl/workbook.xml")?
            .read_to_end(&mut workbook)?;
        let sheet_names = xml::sheet_names(&workbook)?;
        let context = read_value_context(&mut archive)?;
        let cf_context = read_conditional_context(&mut archive, &workbook, &context)?;
        let mut sheets = BTreeMap::new();
        for sheet in plan.sheets {
            let model = read_sheet::<true>(
                &mut archive,
                &sheet.sheet_file,
                sheet.bottom_row + 1,
                sheet.base_right_column + 1,
            )?;
            sheets.insert(sheet.sheet, model);
        }
        Ok(Self {
            sheets,
            context,
            cf_context,
            sheet_names,
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

/// Hash shared value/style dependencies without decoding their XML.
fn read_value_context(archive: &mut ZipArchive<File>) -> Result<String> {
    let mut context = Sha256::new();
    for part in ["xl/styles.xml", "xl/sharedStrings.xml"] {
        let mut entry = match archive.by_name(part) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => continue,
            Err(error) => return Err(error.into()),
        };
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
    Ok(format!("{:x}", context.finalize()))
}

/// Add theme, format override and workbook-property dependencies relevant to CF rendering.
fn read_conditional_context(
    archive: &mut ZipArchive<File>,
    workbook: &[u8],
    context: &str,
) -> Result<String> {
    let mut cf_context = Sha256::new();
    cf_context.update(context.as_bytes());
    let mut properties = crate::regression::xml_views::Reader::<128>::new(&workbook)?;
    while let Some(tag) = properties.next()? {
        if tag.local() == b"workbookPr" && tag.kind != Kind::End {
            cf_context.update(tag.attrs);
        }
    }
    let themes = xml::visual_parts(archive)?;
    for part in themes {
        cf_context.update(part.as_bytes());
        let mut entry = archive.by_name(&part)?;
        let mut buffer = [0u8; 128 * 1024];
        loop {
            let count = entry.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            cf_context.update(&buffer[..count]);
        }
    }
    Ok(format!("{:x}", cf_context.finalize()))
}

/// Read one worksheet once for cells/layout; reopen only selected CF subtrees.
fn read_sheet<const INCLUDE_CELLS: bool>(
    archive: &mut ZipArchive<File>,
    part: &str,
    bottom: u32,
    right: u32,
) -> Result<Sheet> {
    let mut model = Sheet::default();
    model.bottom = bottom;
    model.right = right;
    let mut cells = if INCLUDE_CELLS {
        Some(Writer::new()?)
    } else {
        None
    };
    let mut spans = Vec::new();
    let mut cf_start = None;
    let mut standard_namespace = false;
    let mut x14_namespace = false;
    let mut xm_namespace = false;
    let mut x14_container = None;
    let mut namespace_scopes = Vec::new();
    let mut data_depth = None;
    let mut layout = Sha256::new();
    streaming_xml::scan_with_offsets(archive.by_name(part)?, |event| {
        if let Event::Tag(tag) = &event {
            // Cell-local namespace scopes cannot escape sheetData into a later CF block.
            let declaration = if data_depth.is_none() || tag.local() == b"sheetData" {
                tag.attr(b"xmlns")?
            } else {
                None
            };
            let tag_namespace = declaration
                .map(|namespace| {
                    namespace == b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                })
                .unwrap_or(standard_namespace);
            let (x14_declaration, xm_declaration) =
                if data_depth.is_none() || tag.local() == b"sheetData" {
                    (tag.attr(b"xmlns:x14")?, tag.attr(b"xmlns:xm")?)
                } else {
                    (None, None)
                };
            let tag_x14 = x14_declaration
                .map(|ns| ns == b"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main")
                .unwrap_or(x14_namespace);
            let tag_xm = xm_declaration
                .map(|ns| ns == b"http://schemas.microsoft.com/office/excel/2006/main")
                .unwrap_or(xm_namespace);
            if tag.depth == 0 && tag.kind == Kind::Start {
                model.value_namespace_supported = tag_namespace && tag.name == b"worksheet";
            } else if matches!(tag.kind, Kind::Start | Kind::Empty)
                && (data_depth.is_some() || tag.local() == b"sheetData")
                && (!tag_namespace
                    || tag.name.contains(&b':')
                    || memchr::memmem::find(tag.attrs, b"xmlns").is_some())
            {
                model.value_namespace_supported = false;
            }
            if tag.kind == Kind::Start
                && (declaration.is_some() || x14_declaration.is_some() || xm_declaration.is_some())
            {
                namespace_scopes.push((tag.depth, standard_namespace, x14_namespace, xm_namespace));
                standard_namespace = tag_namespace;
                x14_namespace = tag_x14;
                xm_namespace = tag_xm;
            }
            if matches!(tag.kind, Kind::Start | Kind::Empty) {
                if let Some((_, _, eligible, extension)) = &mut cf_start {
                    let known_prefix = *extension
                        && ((tag.name.starts_with(b"x14:") && tag_x14)
                            || (tag.name.starts_with(b"xm:") && tag_xm));
                    if !tag_namespace || (tag.name.contains(&b':') && !known_prefix) {
                        *eligible = false;
                    }
                    if *extension && !xml::static_format_attributes(*tag)? {
                        *eligible = false;
                    }
                }
            }
            if tag.name == b"x14:conditionalFormattings" {
                if tag.kind == Kind::Start && tag_x14 && tag_namespace {
                    x14_container = Some(tag.depth);
                }
                if tag.kind == Kind::End {
                    x14_container = None;
                }
            }
            if tag.local() == b"sheetData" {
                if tag.kind == Kind::Start {
                    data_depth = Some(tag.depth);
                } else {
                    data_depth = None;
                }
            } else if data_depth.is_none() && tag.local() != b"dimension" {
                layout.update(tag.name);
                layout.update(tag.attrs);
                layout.update([match tag.kind {
                    Kind::Start => 1,
                    Kind::Empty => 2,
                    Kind::End => 3,
                }]);
            }
            if tag.local() == b"conditionalFormatting" {
                let eligible =
                    tag.depth == 1 && tag.name == b"conditionalFormatting" && tag_namespace;
                let extension = tag.name == b"x14:conditionalFormatting"
                    && tag_namespace
                    && tag_x14
                    && tag_xm
                    && x14_container.is_some_and(|depth| depth + 1 == tag.depth);
                if tag.kind == Kind::Start && cf_start.is_none() {
                    cf_start = Some((tag.depth, tag.start, eligible || extension, extension));
                }
                if tag.kind == Kind::Empty && cf_start.is_none() {
                    spans.push(xml::Span {
                        start: tag.start,
                        end: tag.end,
                        namespace_supported: eligible,
                        x14_expression: extension,
                    });
                }
                if tag.kind == Kind::End
                    && cf_start.is_some_and(|(depth, _, _, _)| depth == tag.depth)
                {
                    let (_, start, eligible, extension) = cf_start.take().unwrap();
                    spans.push(xml::Span {
                        start,
                        end: tag.end,
                        namespace_supported: eligible,
                        x14_expression: extension,
                    });
                }
            }
            if INCLUDE_CELLS && matches!(tag.kind, Kind::Start | Kind::Empty) {
                match tag.local() {
                    b"row" => {
                        let row: u32 =
                            std::str::from_utf8(tag.attr(b"r")?.unwrap_or(b"0"))?.parse()?;
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
                            } else if key != b"r" && key != b"spans" {
                                layout.update(row.to_le_bytes());
                                layout.update(key);
                                layout.update(value);
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
            if tag.kind == Kind::End
                && namespace_scopes
                    .last()
                    .is_some_and(|(depth, _, _, _)| *depth == tag.depth)
            {
                let (_, main, x14, xm) = namespace_scopes.pop().unwrap();
                standard_namespace = main;
                x14_namespace = x14;
                xm_namespace = xm;
            }
        }
        match &mut cells {
            Some(cells) => cells.event(&event),
            None => Ok(()),
        }
    })?;
    if let Some(cells) = cells {
        model.cells = cells.finish()?;
    }
    model.cf_layout = format!("{:x}", layout.finalize());
    model.conditional = Arc::new(xml::catalogue(archive, part, &spans)?);
    Ok(model)
}

/// Audit CF eligibility through the production namespace scanner without indexing worksheet cells.
#[cfg(test)]
pub(super) fn conditional_catalogue_for_audit(
    archive: &mut ZipArchive<File>,
    part: &str,
) -> Result<Arc<xml::Catalogue>> {
    Ok(read_sheet::<false>(archive, part, 0, 0)?.conditional)
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
    fn metadata_audit_and_cell_scan_produce_the_same_conditional_catalogue() -> Result<()> {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("cf-audit-{}.xlsx", uuid::Uuid::new_v4()));
        let file = File::create(&path)?;
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("xl/worksheets/sheet.xml", zip::write::FileOptions::default())?;
        zip.write_all(br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData><conditionalFormatting sqref="A1:A10"><cfRule type="cellIs" operator="greaterThan" priority="1"><formula>5</formula></cfRule></conditionalFormatting></worksheet>"#)?;
        zip.finish()?;
        let mut archive = ZipArchive::new(File::open(&path)?)?;
        let full = read_sheet::<true>(&mut archive, "xl/worksheets/sheet.xml", 10, 1)?;
        let audit = conditional_catalogue_for_audit(&mut archive, "xl/worksheets/sheet.xml")?;
        assert_eq!(audit.fingerprints, full.conditional.fingerprints);
        assert_eq!(
            audit.namespace_supported,
            full.conditional.namespace_supported
        );
        assert_eq!(
            serde_json::to_value(&audit.blocks)?,
            serde_json::to_value(&full.conditional.blocks)?
        );
        drop(archive);
        std::fs::remove_file(path)?;
        Ok(())
    }
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
