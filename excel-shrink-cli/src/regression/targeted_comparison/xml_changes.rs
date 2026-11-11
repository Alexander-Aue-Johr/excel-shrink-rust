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
    pub visual_bounds: Option<super::content_bounds::Rectangle>,
    pub background_tail_sampling: bool,
    pub cf_blank_tail_excluded: bool,
    pub rebuild_blockers: BTreeSet<String>,
    pub cells: Cells,
    pub rows: Arc<BTreeMap<u32, BTreeMap<String, String>>>,
    pub columns: Vec<u8>,
    pub defaults: Vec<u8>,
    pub conditional: Arc<xml::Catalogue>,
    pub cf_layout: String,
    pub cf_columns: super::column_layout::ColumnLayout,
    pub bottom: u32,
    pub right: u32,
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub sheets: BTreeMap<String, Sheet>,
    pub context: String,
    /// Shared rendering dependencies and the default format, excluding unused XFs.
    pub layout_context: String,
    pub cf_context: String,
    pub sheet_names: Option<BTreeSet<String>>,
    pub defined_names: Option<BTreeSet<String>>,
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
    /// Formatting-only blank cells remain visual candidates, not CF value dependencies.
    pub may_change_cell_input: bool,
}

#[derive(Serialize)]
pub(super) struct AuditTimings {
    pub read_snapshots: u64,
    pub differences: u64,
    pub attribution: u64,
    pub write_evidence: u64,
    pub total: u64,
}

#[derive(Serialize)]
pub(super) struct Transition {
    pub label: String,
    pub before_file: std::path::PathBuf,
    pub after_file: std::path::PathBuf,
    pub change_count: usize,
    pub changes_file: std::path::PathBuf,
    pub skipped_outside_content_cell_changes: BTreeMap<String, usize>,
    pub timings_ms: Option<AuditTimings>,
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

fn hash_layout_tag(
    layout: &mut Sha256,
    tag: crate::regression::xml_views::Tag<'_>,
    styles: &super::style_identity::StyleIdentity,
) -> Result<()> {
    layout.update(tag.name);
    if tag.local() == b"col" {
        styles.hash_attributes(layout, tag.attrs, b"style")?;
    } else {
        layout.update(tag.attrs);
    }
    layout.update([match tag.kind {
        Kind::Start => 1,
        Kind::Empty => 2,
        Kind::End => 3,
    }]);
    Ok(())
}

impl Snapshot {
    /// Stream cell fingerprints to owned scratch files; retain only layout metadata in memory.
    pub fn read(path: &Path) -> Result<Self> {
        Self::read_with_scratch(path, &std::env::temp_dir())
    }

    /// Select scratch ownership explicitly for retained analyses in corpus attempts.
    pub fn read_with_scratch(path: &Path, scratch: &Path) -> Result<Self> {
        Self::read_with_bounded_drawings(path, scratch, &BTreeSet::new())
    }

    /// Native-qualified drawings affect only this snapshot's background-tail bounds.
    pub(super) fn read_with_bounded_drawings(
        path: &Path,
        scratch: &Path,
        bounded_drawings: &BTreeSet<String>,
    ) -> Result<Self> {
        let plan = build_workbook_visual_plan(path)?;
        let mut archive = ZipArchive::new(File::open(path)?)?;
        let mut workbook = Vec::new();
        archive
            .by_name("xl/workbook.xml")?
            .read_to_end(&mut workbook)?;
        let sheet_names = xml::sheet_names(&workbook)?;
        let defined_names = xml::defined_names_in_package(&mut archive, &workbook)?;
        let (style_bytes, styles) = match archive.by_name("xl/styles.xml") {
            Ok(mut entry) => {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                let styles = excel_shrink_core::styles::parse_xfs(&bytes)?;
                (bytes, styles)
            }
            Err(zip::result::ZipError::FileNotFound) => (Vec::new(), vec![Default::default()]),
            Err(error) => return Err(error.into()),
        };
        let style_identity = super::style_identity::StyleIdentity::read(&style_bytes)?;
        let solid_fills = super::style_identity::solid_fill_ids(&style_bytes)?;
        let (context, conditional_values) = read_value_context(&mut archive, &style_identity)?;
        let cf_context = read_conditional_context(&mut archive, &workbook, &conditional_values)?;
        let mut sheets = BTreeMap::new();
        for sheet in plan.sheets {
            let model = read_sheet::<true>(
                &mut archive,
                &sheet.sheet_file,
                sheet.bottom_row + 1,
                sheet.base_right_column + 1,
                Some(scratch),
                &styles,
                &style_identity,
                &solid_fills,
                !sheet.drawing_fallback_required || bounded_drawings.contains(&sheet.sheet),
            )?;
            sheets.insert(sheet.sheet, model);
        }
        Ok(Self {
            sheets,
            context,
            layout_context: conditional_values,
            cf_context,
            sheet_names,
            defined_names,
        })
    }

    /// Compare actual XML content, with style/string context, rather than addresses alone.
    #[cfg(test)]
    pub fn changes(&self, after: &Self) -> Result<Vec<XmlChange>> {
        self.compare(after, None)
    }

    /// Count exterior formatting-only changes without materializing millions of unused candidates.
    pub fn content_changes(
        &self,
        after: &Self,
    ) -> Result<(Vec<XmlChange>, BTreeMap<String, usize>)> {
        let mut skipped = BTreeMap::new();
        let changes = self.compare(after, Some(&mut skipped))?;
        Ok((changes, skipped))
    }

    fn compare(
        &self,
        after: &Self,
        mut skipped: Option<&mut BTreeMap<String, usize>>,
    ) -> Result<Vec<XmlChange>> {
        let mut content = super::content_bounds::ContentBounds::default();
        if skipped.is_some() {
            content.include(self);
            content.include(after);
        }
        let mut changes = Vec::new();
        let names: BTreeSet<_> = self.sheets.keys().chain(after.sheets.keys()).collect();
        for name in names {
            let empty = Sheet::default();
            let left = self.sheets.get(name).unwrap_or(&empty);
            let right = after.sheets.get(name).unwrap_or(&empty);
            left.cells.changes(&right.cells, |change| {
                let coordinate = change.coordinate();
                if let Some(counts) = skipped.as_deref_mut() {
                    if !change.visible()
                        && !content.intersects_cell(
                            name,
                            coordinate.row,
                            u32::from(coordinate.column),
                        )
                    {
                        *counts.entry(name.clone()).or_default() += 1;
                        return Ok(());
                    }
                }
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
                    may_change_cell_input: change.may_change_input(),
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
            may_change_cell_input: false,
        });
    }

    /// An exact matching transition is evidence of overlap, never proof of exclusive causality.
    #[cfg(test)]
    pub fn same_change(a: &XmlChange, b: &XmlChange) -> bool {
        Self::change_key(a) == Self::change_key(b)
    }

    pub fn change_key(change: &XmlChange) -> (&str, &str, Option<&str>, Option<&str>, &str, &str) {
        (
            &change.sheet,
            &change.location,
            change.before.as_deref(),
            change.after.as_deref(),
            &change.context_before,
            &change.context_after,
        )
    }
}

/// Hash shared value/style dependencies without decoding their XML.
fn read_value_context(
    archive: &mut ZipArchive<File>,
    styles: &super::style_identity::StyleIdentity,
) -> Result<(String, String)> {
    let mut context = Sha256::new();
    let mut conditional = Sha256::new();
    for part in ["xl/styles.xml", "xl/sharedStrings.xml"] {
        let mut entry = match archive.by_name(part) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => continue,
            Err(error) => return Err(error.into()),
        };
        context.update(part.as_bytes());
        conditional.update(part.as_bytes());
        if part == "xl/styles.xml" {
            context.update(&styles.context);
            conditional.update(&styles.conditional_context);
            continue;
        }
        let mut buffer = [0u8; 128 * 1024];
        loop {
            let count = entry.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            context.update(&buffer[..count]);
            conditional.update(&buffer[..count]);
        }
    }
    Ok((format!("{:x}", context.finalize()), format!("{:x}", conditional.finalize())))
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
            cf_context.update(workbook_property_identity(tag.attrs)?);
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

/// The automatic earlier-version compatibility check cannot change CF rendering.
/// Keep every other property, including date1904 and showObjects, in the proof.
fn workbook_property_identity(attributes: &[u8]) -> Result<[u8; 32]> {
    let mut hash = Sha256::new();
    for attribute in crate::regression::xml_views::Attributes::new(attributes) {
        let (key, value) = attribute?;
        if key == b"checkCompatibility" {
            continue;
        }
        hash.update((key.len() as u64).to_le_bytes());
        hash.update(key);
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value);
    }
    Ok(hash.finalize().into())
}

/// Read one worksheet once for cells/layout; reopen only selected CF subtrees.
fn read_sheet<const INCLUDE_CELLS: bool>(
    archive: &mut ZipArchive<File>,
    part: &str,
    bottom: u32,
    right: u32,
    scratch: Option<&Path>,
    styles: &[excel_shrink_core::models::Xf],
    style_identity: &super::style_identity::StyleIdentity,
    solid_fills: &BTreeSet<u32>,
    drawings_bounded: bool,
) -> Result<Sheet> {
    let mut visual = super::visual_bounds::VisualBounds::new(styles);
    let mut model = Sheet::default();
    model.cf_columns = super::column_layout::ColumnLayout::new();
    model.bottom = bottom;
    model.right = right;
    let mut cells = if INCLUDE_CELLS {
        Some(match scratch {
            Some(path) => Writer::new_in(path)?,
            None => Writer::new()?,
        })
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
    let mut columns_depth = None;
    let mut layout = Sha256::new();
    let mut canonical_layout = Sha256::new();
    let mut merge_order = super::merge_order::MergeOrder::default();
    let mut uniform_background_supported = true;
    let mut sheet_views = 0;
    let mut sheet_views_supported = true;
    streaming_xml::scan_with_offsets(archive.by_name(part)?, |event| {
        if let Event::Tag(tag) = &event {
            let filter_bounds = if standard_namespace {
                bounded_empty_auto_filter(*tag)?
            } else {
                None
            };
            if let Some(area) = filter_bounds {
                visual.include_rectangle(area.top, area.left, area.bottom, area.right);
            }
            if let Some(reason) = super::execution_plan::worksheet_blocker(*tag) {
                // Known drawing anchors already contribute to bottom/right. Their
                // presence does not give a uniform background a finite sheet edge.
                if reason != "worksheet formula"
                    && !(tag.name == b"drawing" && drawings_bounded)
                    && !(tag.name == b"conditionalFormatting" && tag.depth == 1)
                    && filter_bounds.is_none()
                {
                    uniform_background_supported = false;
                }
                if !model.rebuild_blockers.contains(reason) {
                    model.rebuild_blockers.insert(reason.into());
                }
            }
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
            if tag.local() == b"sheetView" && tag.kind != Kind::End {
                sheet_views += 1;
                sheet_views_supported &= tag_namespace
                    && tag.name == b"sheetView"
                    && tag.depth == 2
                    && matches!(
                        tag.attr(b"showGridLines")?,
                        None | Some(b"0") | Some(b"1") | Some(b"false") | Some(b"true")
                    );
            }
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
                            || (tag.name.starts_with(b"xm:") && tag_xm))
                        || (!*extension && tag.name == b"x14:id" && tag_x14);
                    if !tag_namespace || (tag.name.contains(&b':') && !known_prefix) {
                        *eligible = false;
                    }
                    if *extension && !xml::static_format_attributes(*tag)? {
                        *eligible = false;
                    }
                    if !*extension
                        && matches!(tag.name, b"dataBar" | b"extLst" | b"ext" | b"x14:id")
                        && !super::conditional_formatting::data_bars::static_attributes(*tag)?
                    {
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
            if tag.name == b"cols" && tag_namespace && tag.depth == 1 {
                columns_depth = (tag.kind == Kind::Start).then_some(tag.depth);
            }
            let worksheet_column = tag.name == b"col" && tag_namespace
                && columns_depth.is_some_and(|depth| tag.depth == depth + 1);
            let merge_member = data_depth.is_none() && merge_order.observe(*tag, tag_namespace)?;
            if tag.local() == b"sheetData" {
                if tag.kind == Kind::Start {
                    data_depth = Some(tag.depth);
                } else {
                    data_depth = None;
                }
            } else if data_depth.is_none() && tag.local() != b"dimension" && !worksheet_column {
                hash_layout_tag(&mut layout, *tag, style_identity)?;
                if !merge_member {
                    hash_layout_tag(&mut canonical_layout, *tag, style_identity)?;
                }
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
                        x14_extension: extension,
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
                        x14_extension: extension,
                    });
                }
            }
            if INCLUDE_CELLS
                && data_depth.is_some()
                && matches!(tag.local(), b"c" | b"v" | b"f" | b"is")
            {
                visual.observe(*tag)?;
            }
            if INCLUDE_CELLS && matches!(tag.kind, Kind::Start | Kind::Empty) {
                if (tag.local() == b"row" && data_depth.is_some())
                    || (tag.local() == b"col" && columns_depth.is_some())
                {
                    visual.observe(*tag)?;
                }
                match tag.local() {
                    b"row"
                        if tag.name == b"row"
                            && tag_namespace
                            && data_depth.is_some_and(|depth| tag.depth == depth + 1) =>
                    {
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
                                b"x14ac:dyDescent",
                                b"thickTop",
                                b"thickBot",
                            ]
                            .contains(&key)
                                && (!matches!(key, b"thickTop" | b"thickBot")
                                    || matches!(value, b"0" | b"1" | b"false" | b"true"))
                            {
                                attributes.insert(
                                    String::from_utf8(key.to_vec())?,
                                    if key == b"s" {
                                        style_identity.index_text(value)?
                                    } else {
                                        String::from_utf8(value.to_vec())?
                                    },
                                );
                            } else if key != b"r" && key != b"spans" {
                                layout.update(row.to_le_bytes());
                                layout.update(key);
                                layout.update(value);
                                canonical_layout.update(row.to_le_bytes());
                                canonical_layout.update(key);
                                canonical_layout.update(value);
                            }
                        }
                        if !attributes.is_empty() {
                            Arc::make_mut(&mut model.rows).insert(row, attributes);
                        }
                    }
                    b"col"
                        if tag.name == b"col"
                            && tag_namespace
                            && columns_depth.is_some_and(|depth| tag.depth == depth + 1) =>
                    {
                        model.cf_columns.observe(*tag, style_identity)?;
                        model.columns.extend_from_slice(tag.name);
                        model.columns.extend_from_slice(&style_identity.attributes(tag.attrs, b"style")?);
                    }
                    b"sheetFormatPr"
                        if tag.name == b"sheetFormatPr" && tag_namespace && tag.depth == 1 =>
                    {
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
        match &event {
            Event::Text(bytes) => merge_order.observe_text(bytes, false),
            Event::Cdata(bytes) => merge_order.observe_text(bytes, true),
            Event::Tag(_) => {}
        }
        match &mut cells {
            Some(cells) => cells.event_with_styles(&event, style_identity),
            None => Ok(()),
        }
    })?;
    if let Some(cells) = cells {
        model.cells = cells.finish()?;
    }
    model.cf_layout = if let Some(merges) = merge_order.finish() {
        canonical_layout.update(merges);
        format!("{:x}", canonical_layout.finalize())
    } else {
        format!("{:x}", layout.finalize())
    };
    model.conditional = Arc::new(xml::catalogue(archive, part, &spans)?);
    let cf_blank_tail_inactive = model.value_namespace_supported
        && model.cf_columns.is_resolved()
        && uniform_background_supported
        && super::conditional_formatting::blank_tail_inactive(&model.conditional);
    model.cf_blank_tail_excluded = cf_blank_tail_inactive && !model.conditional.blocks.is_empty();
    if !cf_blank_tail_inactive {
        for block in &model.conditional.blocks {
            if let Some(areas) = super::conditional_formatting::application_areas(&block.sqref) {
                for area in areas {
                    visual.include_rectangle(area.top, area.left, area.bottom, area.right);
                }
            }
        }
    }
    let mut finite = super::content_bounds::Rectangle {
        top: 1,
        left: 1,
        bottom: bottom.max(1),
        right: right.max(1),
    };
    if !cf_blank_tail_inactive {
        for block in &model.conditional.blocks {
            if let Some(areas) = super::conditional_formatting::application_areas(&block.sqref) {
                for area in areas {
                    finite.bottom = finite.bottom.max(area.bottom.saturating_add(1).min(1_048_576));
                    finite.right = finite.right.max(area.right);
                }
            }
        }
    }
    // A rule confined to the finite capture envelope cannot affect its tail.
    // Its application area still contributes to capture selection above.
    let cf_inside_finite = model.value_namespace_supported
        && model.cf_columns.is_resolved()
        && uniform_background_supported
        && finite.bottom < 1_048_576
        && finite.right < 16_384
        && model.conditional.namespace_supported.len() == model.conditional.blocks.len()
        && model.conditional.namespace_supported.iter().all(|supported| *supported)
        && model.conditional.blocks.iter().all(|block| {
            super::conditional_formatting::application_areas(&block.sqref).is_some_and(|areas| {
                !areas.is_empty()
                    && areas.iter().all(|area| {
                        area.bottom <= finite.bottom && area.right <= finite.right
                    })
            })
        });
    let background_tail_supported = cf_blank_tail_inactive || cf_inside_finite;
    model.background_tail_sampling = visual.finish_background_bands(
        solid_fills,
        finite,
        background_tail_supported && sheet_views > 0 && sheet_views_supported,
    );
    if !model.background_tail_sampling {
        visual.finish_columns(solid_fills, finite, background_tail_supported);
    }
    model.visual_bounds = visual.bounds;
    if let Some(bounds) = model.visual_bounds {
        model.bottom = model.bottom.max(bounds.bottom);
        model.right = model.right.max(bounds.right);
    }
    Ok(model)
}

/// A filter without criteria adds buttons within its explicit rectangle.
/// Filter children, extensions and unfamiliar attributes keep the existing fallback.
fn bounded_empty_auto_filter(
    tag: crate::regression::xml_views::Tag<'_>,
) -> Result<Option<super::content_bounds::Rectangle>> {
    if tag.name != b"autoFilter" || tag.depth != 1 || tag.kind != Kind::Empty {
        return Ok(None);
    }
    for attribute in Attributes::new(tag.attrs) {
        let (name, _) = attribute?;
        if !matches!(name, b"ref" | b"xr:uid") {
            return Ok(None);
        }
    }
    let Some(reference) = tag.attr(b"ref")? else {
        return Ok(None);
    };
    let Some(area) = std::str::from_utf8(reference)
        .ok()
        .and_then(|address| super::content_bounds::Rectangle::parse(address).ok())
    else {
        return Ok(None);
    };
    Ok((area.top > 0 && area.left > 0 && area.bottom <= 1_048_576 && area.right <= 16_384)
        .then_some(area))
}

/// Audit CF eligibility through the production namespace scanner without indexing worksheet cells.
#[cfg(test)]
pub(super) fn conditional_catalogue_for_audit(
    archive: &mut ZipArchive<File>,
    part: &str,
) -> Result<Arc<xml::Catalogue>> {
    Ok(read_sheet::<false>(archive, part, 0, 0, None, &[], &crate::regression::targeted_comparison::style_identity::StyleIdentity::default(), &BTreeSet::new(), false)?.conditional)
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

/// Preserve actual coordinates, including empty formatting changes within the content envelope.
pub(super) fn cell_candidates(changes: &[XmlChange]) -> BTreeSet<Candidate> {
    changes
        .iter()
        .filter_map(|change| {
            let cell = change.location.strip_prefix("cell:")?;
            Some(Candidate {
                sheet: change.sheet.clone(),
                cell: cell.to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_ui_setting_is_not_a_conditional_rendering_dependency() -> Result<()> {
        let before = br#" date1904="0" checkCompatibility="1" showObjects="all""#;
        let after = br#" date1904="0" checkCompatibility="0" showObjects="all""#;
        assert_eq!(
            workbook_property_identity(before)?,
            workbook_property_identity(after)?
        );
        assert_ne!(
            workbook_property_identity(before)?,
            workbook_property_identity(br#" date1904="1" checkCompatibility="0" showObjects="all""#)?
        );
        assert_ne!(
            workbook_property_identity(before)?,
            workbook_property_identity(br#" date1904="0" checkCompatibility="0" showObjects="none""#)?
        );
        Ok(())
    }
    #[test]
    fn exterior_blank_changes_are_counted_but_deleted_real_values_are_retained() -> Result<()> {
        fn snapshot(xml: &[u8]) -> Result<Snapshot> {
            let mut writer = Writer::new()?;
            streaming_xml::scan(xml, |event| writer.event(&event))?;
            Ok(Snapshot {
                sheets: BTreeMap::from([(
                    "Data".into(),
                    Sheet {
                        cells: writer.finish()?,
                        ..Sheet::default()
                    },
                )]),
                context: "same".into(),
                layout_context: "same".into(),
                cf_context: "same".into(),
                sheet_names: None,
                defined_names: None,
            })
        }
        let mut xml =
            String::from("<sheetData><row><c r=\"C1\"><v>1</v></c><c r=\"F10\"><v>2</v></c>");
        for row in 20..100_020 {
            xml.push_str(&format!("<c r=\"A{row}\" s=\"1\"/>"));
        }
        xml.push_str("</row></sheetData>");
        let before = snapshot(xml.as_bytes())?;
        let after = snapshot(br#"<sheetData><row><c r="C1"><v>1</v></c></row></sheetData>"#)?;
        let (changes, skipped) = before.content_changes(&after)?;
        assert_eq!(skipped["Data"], 100_000);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].location, "cell:F10");
        assert_eq!(before.changes(&after)?.len(), 100_001);
        Ok(())
    }

    #[test]
    fn drawing_anchors_do_not_become_worksheet_rows_or_columns() -> Result<()> {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("drawing-row-{}.xlsx", uuid::Uuid::new_v4()));
        let mut zip = zip::ZipWriter::new(File::create(&path)?);
        zip.start_file(
            "xl/worksheets/sheet.xml",
            zip::write::FileOptions::default(),
        )?;
        zip.write_all(br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"><sheetFormatPr defaultRowHeight="15"/><cols><col min="1" max="2" width="12"/></cols><sheetData><row r="1" ht="25" customHeight="1"><c r="A1"><v>7</v></c></row></sheetData><extLst><xdr:from><xdr:row>5</xdr:row><xdr:col>2</xdr:col></xdr:from><row r="0"/><col min="99" max="100"/><sheetFormatPr defaultRowHeight="99"/></extLst></worksheet>"#)?;
        zip.finish()?;
        let mut archive = ZipArchive::new(File::open(&path)?)?;
        let sheet = read_sheet::<true>(&mut archive, "xl/worksheets/sheet.xml", 10, 2, None, &[], &crate::regression::targeted_comparison::style_identity::StyleIdentity::default(), &BTreeSet::new(), false)?;
        assert_eq!(sheet.rows.len(), 1);
        assert_eq!(sheet.fixed_height(1), Some(("25", false)));
        assert_eq!(
            String::from_utf8(sheet.columns.clone())?,
            "col min=\"1\" max=\"2\" width=\"12\""
        );
        assert!(!String::from_utf8(sheet.defaults.clone())?.contains("99"));
        assert!(sheet.value_namespace_supported);
        drop(archive);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn metadata_audit_and_cell_scan_produce_the_same_conditional_catalogue() -> Result<()> {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("cf-audit-{}.xlsx", uuid::Uuid::new_v4()));
        let file = File::create(&path)?;
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            "xl/worksheets/sheet.xml",
            zip::write::FileOptions::default(),
        )?;
        zip.write_all(br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData><conditionalFormatting sqref="A1:A10"><cfRule type="cellIs" operator="greaterThan" priority="1"><formula>5</formula></cfRule></conditionalFormatting></worksheet>"#)?;
        zip.finish()?;
        let mut archive = ZipArchive::new(File::open(&path)?)?;
        let full = read_sheet::<true>(&mut archive, "xl/worksheets/sheet.xml", 10, 1, None, &[], &crate::regression::targeted_comparison::style_identity::StyleIdentity::default(), &BTreeSet::new(), false)?;
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
            may_change_cell_input: false,
        };
        let mut b = a.clone();
        b.before = Some("different content".into());
        assert!(!Snapshot::same_change(&a, &b));
        assert!(Snapshot::same_change(&a, &a));
    }

    #[test]
    fn large_change_audit_roundtrips_without_growing_the_main_report() -> Result<()> {
        use std::io::{BufRead, BufReader};
        let directory =
            crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "change-audit")?;
        let changes: Vec<_> = (1..=100_000)
            .map(|row| XmlChange {
                sheet: "Rows".into(),
                location: format!("cell:A{row}"),
                before: Some("a".repeat(64)),
                after: None,
                context_before: "b".repeat(64),
                context_after: "c".repeat(64),
                also_in_original_open_save: row % 2 == 0,
                visible_cell: true,
                may_change_cell_input: true,
            })
            .collect();
        let path = super::super::change_evidence::write(&directory, 0, &changes)?;
        let mut lines =
            BufReader::new(flate2::read::GzDecoder::new(std::fs::File::open(&path)?)).lines();
        let header: serde_json::Value = serde_json::from_str(&lines.next().unwrap()?)?;
        assert_eq!(header["contexts"].as_array().unwrap().len(), 2);
        let mut count = 0;
        for line in lines {
            let row: serde_json::Value = serde_json::from_str(&line?)?;
            count += 1;
            assert_eq!(row[1], format!("cell:A{count}"));
            assert_eq!(row[2], "a".repeat(64));
            assert!(row[3].is_null());
            assert_eq!(row[6], count % 2 == 0);
        }
        assert_eq!(count, changes.len());
        let transition = Transition {
            label: "test".into(),
            before_file: "a.xlsx".into(),
            after_file: "b.xlsx".into(),
            change_count: count,
            changes_file: path.clone(),
            skipped_outside_content_cell_changes: BTreeMap::new(),
            timings_ms: None,
        };
        assert!(serde_json::to_vec(&transition)?.len() < 1024);
        assert!(std::fs::metadata(path)?.len() < 1_000_000);
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
    #[test]
    fn format_permutations_preserve_cell_row_column_and_cf_layout_identity() -> Result<()> {
        use super::super::style_identity::StyleIdentity;
        use std::io::Write;

        fn model(xml: &[u8], identity: &StyleIdentity) -> Result<Sheet> {
            let path = std::env::temp_dir().join(format!("style-identity-{}.zip", uuid::Uuid::new_v4()));
            let mut zip = zip::ZipWriter::new(File::create(&path)?);
            zip.start_file("sheet.xml", zip::write::FileOptions::default())?;
            zip.write_all(xml)?;
            zip.finish()?;
            let mut archive = ZipArchive::new(File::open(&path)?)?;
            let model = read_sheet::<true>(
                &mut archive, "sheet.xml", 1, 1, None,
                &[Default::default(), Default::default()], identity, &BTreeSet::new(), false,
            )?;
            drop(archive);
            std::fs::remove_file(path)?;
            Ok(model)
        }

        let left_styles = StyleIdentity::read(br#"<styleSheet><cellXfs count="2"><xf fontId="0"/><xf fontId="1"/></cellXfs></styleSheet>"#)?;
        let right_styles = StyleIdentity::read(br#"<styleSheet><cellXfs count="2"><xf fontId="1"/><xf fontId="0"/></cellXfs></styleSheet>"#)?;
        let left = model(br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="1" max="1" style="0"/></cols><sheetData><row r="1" s="0" customFormat="1"><c r="A1" s="0"><v>7</v></c></row></sheetData></worksheet>"#, &left_styles)?;
        let right_xml = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="1" max="1" style="1"/></cols><sheetData><row r="1" s="1" customFormat="1"><c r="A1" s="1"><v>7</v></c></row></sheetData></worksheet>"#;
        let right = model(right_xml, &right_styles)?;
        let mut differences = 0;
        left.cells.changes(&right.cells, |_| { differences += 1; Ok(()) })?;
        assert_eq!(differences, 0);
        assert_eq!(left.rows, right.rows);
        assert_eq!(left.columns, right.columns);
        assert_eq!(left.cf_layout, right.cf_layout);
        assert!(left.cf_columns.same_prefix(&right.cf_columns, 16_384));
        let changed_xml = std::str::from_utf8(right_xml)?.replace("<v>7</v>", "<v>8</v>");
        let changed = model(changed_xml.as_bytes(), &right_styles)?;
        left.cells.changes(&changed.cells, |_| { differences += 1; Ok(()) })?;
        assert_eq!(differences, 1);
        Ok(())
    }

    #[test]
    fn border_height_hints_use_row_checks_and_unknown_values_keep_layout_fallback() -> Result<()> {
        use std::io::Write;
        let identity = super::super::style_identity::StyleIdentity::read(b"")?;
        let path = std::env::temp_dir().join(format!("row-hint-{}.zip", uuid::Uuid::new_v4()));
        let mut models = Vec::new();
        for hint in ["", "thickTop=\"1\"", "thickBot=\"true\"", "thickTop=\"unknown\""] {
            let mut zip = zip::ZipWriter::new(File::create(&path)?);
            zip.start_file("sheet.xml", zip::write::FileOptions::default())?;
            write!(zip, "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\" {hint}/></sheetData></worksheet>")?;
            zip.finish()?;
            let mut archive = ZipArchive::new(File::open(&path)?)?;
            models.push(read_sheet::<true>(
                &mut archive, "sheet.xml", 1, 1, None,
                &[Default::default()], &identity, &BTreeSet::new(), false,
            )?);
        }
        assert_eq!(models[0].cf_layout, models[1].cf_layout);
        assert_eq!(models[0].cf_layout, models[2].cf_layout);
        assert_ne!(models[0].cf_layout, models[3].cf_layout);
        assert_eq!(models[1].rows[&1]["thickTop"], "1");
        assert_eq!(models[2].rows[&1]["thickBot"], "true");
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn merge_reordering_preserves_layout_but_changed_or_ambiguous_merges_do_not() -> Result<()> {
        use std::io::Write;
        let identity = super::super::style_identity::StyleIdentity::read(b"")?;
        let path = std::env::temp_dir().join(format!("merge-layout-{}.zip", uuid::Uuid::new_v4()));
        let layout = |body: &str| -> Result<String> {
            let mut zip = zip::ZipWriter::new(File::create(&path)?);
            zip.start_file("sheet.xml", zip::write::FileOptions::default())?;
            write!(zip, "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData/>{body}</worksheet>")?;
            zip.finish()?;
            let mut archive = ZipArchive::new(File::open(&path)?)?;
            let sheet = read_sheet::<false>(
                &mut archive, "sheet.xml", 1, 1, None,
                &[Default::default()], &identity, &BTreeSet::new(), false,
            )?;
            Ok(sheet.cf_layout)
        };
        let left = r#"<mergeCells count="2"><mergeCell ref="A1:B1"/><mergeCell ref="C2:D2"/></mergeCells>"#;
        let right = r#"<mergeCells count="2"><mergeCell ref="C2:D2"/><mergeCell ref="A1:B1"/></mergeCells>"#;
        let expected = layout(left)?;
        assert_eq!(expected, layout(right)?);
        assert_ne!(expected, layout(&right.replace("C2:D2", "C3:D3"))?);
        assert_ne!(expected, layout(&right.replace("C2:D2", "A1:C2"))?);
        assert_ne!(expected, layout(&format!("{right}<sheetProtection sheet=\"1\"/>"))?);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn bounded_drawings_preserve_uniform_background_proof_and_unknown_markup_fallback() -> Result<()> {
        use std::io::Write;
        let styles_xml = br#"<styleSheet><fills><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFFFFF"/></patternFill></fill></fills><cellXfs><xf fontId="0" fillId="0" borderId="0"/><xf fontId="0" fillId="1" borderId="0" applyFill="1"/></cellXfs></styleSheet>"#;
        let styles = excel_shrink_core::styles::parse_xfs(styles_xml)?;
        let identity = super::super::style_identity::StyleIdentity::read(styles_xml)?;
        let fills = super::super::style_identity::solid_fill_ids(styles_xml)?;
        for (bounded, extra, expected_bottom) in [
            (true, "", 80),
            (false, "", 1_048_576),
            (true, "<tableParts count=\"0\"/>", 1_048_576),
        ] {
            let xml = format!(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><cols><col min="1" max="16384" style="1"/></cols><sheetData><row r="60"><c r="B60"><v>1</v></c></row></sheetData><drawing r:id="one"/>{extra}</worksheet>"#);
            let path = std::env::temp_dir().join(format!("bounded-background-{}.zip", uuid::Uuid::new_v4()));
            let mut writer = zip::ZipWriter::new(File::create(&path)?);
            writer.start_file("sheet.xml", zip::write::FileOptions::default())?;
            writer.write_all(xml.as_bytes())?;
            writer.finish()?;
            let mut archive = ZipArchive::new(File::open(&path)?)?;
            let model = read_sheet::<true>(
                &mut archive, "sheet.xml", 80, 20, None, &styles, &identity, &fills, bounded,
            )?;
            assert_eq!(model.visual_bounds.unwrap().bottom, expected_bottom);
            assert!(!model.rebuild_blockers.is_empty());
            drop(archive);
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    #[test]
    fn finite_conditional_formatting_keeps_background_tail_bounded() -> Result<()> {
        use std::io::Write;
        let styles_xml = br#"<styleSheet><fills><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFFFFF"/></patternFill></fill></fills><cellXfs><xf fontId="0" fillId="0" borderId="0"/><xf fontId="0" fillId="1" borderId="0" applyFill="1"/></cellXfs></styleSheet>"#;
        let styles = excel_shrink_core::styles::parse_xfs(styles_xml)?;
        let identity = super::super::style_identity::StyleIdentity::read(styles_xml)?;
        let fills = super::super::style_identity::solid_fill_ids(styles_xml)?;
        for (application, expected_bounds, expected_sampling) in [
            ("B2:C10", "A1:U21", true),
            ("B2:AC25", "A1:AD26", true),
            ("B2:XFD1048576", "A1:XFD1048576", false),
        ] {
            let xml = format!(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView workbookViewId="0"/></sheetViews><cols><col min="1" max="16384" style="1"/></cols><sheetData><row r="20"><c r="T20"><v>7</v></c></row></sheetData><conditionalFormatting sqref="{application}"><cfRule type="colorScale" priority="1"><colorScale><cfvo type="min"/><cfvo type="max"/><color rgb="FF92D050"/><color rgb="FFFF0000"/></colorScale></cfRule></conditionalFormatting></worksheet>"#);
            let path = std::env::temp_dir()
                .join(format!("finite-cf-background-{}.zip", uuid::Uuid::new_v4()));
            let mut writer = zip::ZipWriter::new(File::create(&path)?);
            writer.start_file("sheet.xml", zip::write::FileOptions::default())?;
            writer.write_all(xml.as_bytes())?;
            writer.finish()?;
            let mut archive = ZipArchive::new(File::open(&path)?)?;
            let model = read_sheet::<true>(
                &mut archive,
                "sheet.xml",
                20,
                20,
                None,
                &styles,
                &identity,
                &fills,
                true,
            )?;
            assert_eq!(model.background_tail_sampling, expected_sampling);
            assert!(!model.cf_blank_tail_excluded);
            assert_eq!(model.conditional.blocks.len(), 1);
            assert_eq!(model.visual_bounds.unwrap().address(), expected_bounds);
            drop(archive);
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    #[test]
    fn background_band_sampling_requires_supported_views_and_blank_inactive_cf() -> Result<()> {
        use std::io::Write;
        let styles_xml = br#"<styleSheet><fills><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFFFFF"/></patternFill></fill></fills><cellXfs><xf fillId="0" borderId="0"/><xf fillId="1" borderId="0" applyFill="1"/></cellXfs></styleSheet>"#;
        let styles = excel_shrink_core::styles::parse_xfs(styles_xml)?;
        let identity = super::super::style_identity::StyleIdentity::read(styles_xml)?;
        let fills = super::super::style_identity::solid_fill_ids(styles_xml)?;
        for (views, cf, sampled) in [
            (
                r#"<sheetViews><sheetView showGridLines="0" workbookViewId="0"/></sheetViews>"#,
                "",
                true,
            ),
            (
                r#"<sheetViews><sheetView workbookViewId="0"/></sheetViews>"#,
                "",
                true,
            ),
            (
                r#"<sheetViews><sheetView workbookViewId="0"/></sheetViews>"#,
                r#"<autoFilter ref="A3:O19" xr:uid="{test}"/>"#,
                true,
            ),
            (
                r#"<sheetViews><sheetView workbookViewId="0"/></sheetViews>"#,
                r#"<autoFilter ref="A3:O19"><filterColumn colId="1"><filters><filter val="x"/></filters></filterColumn></autoFilter>"#,
                false,
            ),
            (
                r#"<sheetViews><sheetView workbookViewId="0"/></sheetViews>"#,
                r#"<autoFilter ref="A3:O19" unknown="1"/>"#,
                false,
            ),
            (
                r#"<sheetViews><sheetView showGridLines="0" workbookViewId="0"/><sheetView showGridLines="1" workbookViewId="1"/></sheetViews>"#,
                "",
                true,
            ),
            (
                r#"<sheetViews><sheetView showGridLines="unknown" workbookViewId="0"/></sheetViews>"#,
                "",
                false,
            ),
            (
                r#"<sheetViews><sheetView xmlns="urn:unsupported" workbookViewId="0"/></sheetViews>"#,
                "",
                false,
            ),
            (
                "",
                "",
                false,
            ),
            (
                r#"<sheetViews><sheetView showGridLines="0" workbookViewId="0"/></sheetViews>"#,
                r#"<conditionalFormatting sqref="A1:XFD1048576"><cfRule type="cellIs" operator="equal" priority="1"><formula>1</formula></cfRule></conditionalFormatting>"#,
                false,
            ),
            (
                r#"<sheetViews><sheetView showGridLines="0" workbookViewId="0"/></sheetViews>"#,
                r#"<conditionalFormatting sqref="A1:XFD1048576"><cfRule type="cellIs" operator="equal" priority="1"><formula>"."</formula></cfRule></conditionalFormatting>"#,
                true,
            ),
            (
                r#"<sheetViews><sheetView showGridLines="0" workbookViewId="0"/></sheetViews>"#,
                r#"<conditionalFormatting sqref="A1:XFD1048576"><cfRule type="cellIs" operator="equal" priority="1"><formula>"."</formula></cfRule></conditionalFormatting><tableParts count="0"/>"#,
                false,
            ),
        ] {
            let xml = format!(
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{views}<cols><col min="1" max="3" style="1"/><col min="5" max="16384" style="1"/></cols><sheetData><row r="20" s="1" customFormat="1"><c r="T20"><v>7</v></c></row></sheetData>{cf}</worksheet>"#
            );
            let path =
                std::env::temp_dir().join(format!("background-bands-{}.zip", uuid::Uuid::new_v4()));
            let mut writer = zip::ZipWriter::new(File::create(&path)?);
            writer.start_file("sheet.xml", zip::write::FileOptions::default())?;
            writer.write_all(xml.as_bytes())?;
            writer.finish()?;
            let mut archive = ZipArchive::new(File::open(&path)?)?;
            let model = read_sheet::<true>(
                &mut archive,
                "sheet.xml",
                20,
                20,
                None,
                &styles,
                &identity,
                &fills,
                true,
            )?;
            assert_eq!(model.background_tail_sampling, sampled);
            assert_eq!(model.cf_blank_tail_excluded, sampled && cf.contains("cfRule"));
            assert_eq!(
                model.visual_bounds.unwrap().address(),
                if sampled { "A1:T21" } else { "A1:XFD1048576" }
            );
            drop(archive);
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Inspect retained states without opening Excel or changing corpus results.
    #[test]
    #[ignore = "Requires LINE_DIAG_REFERENCE with the retained Abwasser native inventory"]
    fn retained_abwasser_native_lines_bound_background_tail() -> Result<()> {
        let receipt = std::path::PathBuf::from(std::env::var("LINE_DIAG_REFERENCE")?);
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(&receipt)?)?;
        let workbook = receipt.parent().unwrap().join("original.xlsx");
        assert_eq!(crate::regression::artifact_cache::hash_file(&workbook)?,
            report["original_rebuilt_sha256"].as_str().unwrap());
        let observation = &report["object_coverage"][0];
        let lines = super::super::ole_metadata::line_frames(&workbook, "Inhalt")?;
        let areas: Vec<super::super::capture_layout::Geometry> =
            serde_json::from_value(observation["areas"].clone())?;
        super::super::reference_objects::validate_line_sheet(&observation["inventory"], &areas, &lines)?;
        let raw = Snapshot::read(&workbook)?;
        let bounded = Snapshot::read_with_bounded_drawings(&workbook, &std::env::temp_dir(),
            &BTreeSet::from(["Inhalt".to_owned()]))?;
        assert_eq!(raw.sheets["Inhalt"].visual_bounds.unwrap().bottom, 1_048_576);
        let bounds = bounded.sheets["Inhalt"].visual_bounds.unwrap();
        assert_eq!(bounds.bottom, 50);
        // The saved workbook still has a distant column-style boundary. Keep it.
        assert_eq!(bounds.right, 16_136);
        assert!(bounded.sheets["Inhalt"].background_tail_sampling);
        assert!(raw.changes(&bounded)?.is_empty());
        Ok(())
    }

    /// Inspect retained states without opening Excel or changing corpus results.
    #[test]
    #[ignore = "Requires LINE_BOUNDS_FIXTURE with the retained Bevoelkerungsbewegung workbook"]
    fn retained_qualified_line_sheet_bounds_are_finite() -> Result<()> {
        let path = std::path::PathBuf::from(std::env::var("LINE_BOUNDS_FIXTURE")?);
        let snapshot = Snapshot::read_with_bounded_drawings(
            &path, &std::env::temp_dir(), &BTreeSet::from(["2.11".to_owned()]),
        )?;
        let sheet = &snapshot.sheets["2.11"];
        eprintln!("bounds {:?}, blockers {:?}", sheet.visual_bounds, sheet.rebuild_blockers);
        assert!(sheet.background_tail_sampling);
        let bounds = sheet.visual_bounds.expect("Missing visual bounds");
        assert!(bounds.bottom < 100 && bounds.right < 20);
        Ok(())
    }

    /// Inspect retained states without opening Excel or changing corpus results.
    #[test]
    #[ignore = "Requires XML_DIAG_LEFT and XML_DIAG_OUTPUT retained snapshot paths"]
    fn inspect_retained_background_extents() -> Result<()> {
        let snapshot = Snapshot::read(Path::new(&std::env::var("XML_DIAG_LEFT")?))?;
        let report: Vec<_> = snapshot.sheets.iter().map(|(name, sheet)| {
            serde_json::json!({
                "sheet": name,
                "visual_bounds": sheet.visual_bounds.map(|area| area.address()),
                "content_bounds": sheet.cells.content_bounds().map(|area| area.address()),
                "background_tail_sampling": sheet.background_tail_sampling,
                    "cf_blank_tail_excluded": sheet.cf_blank_tail_excluded,
                "cf_blocks": sheet.conditional.blocks.len(),
                "cf_blank_tail_inactive": super::super::conditional_formatting::blank_tail_inactive(&sheet.conditional),
            })
        }).collect();
        std::fs::write(
            std::env::var("XML_DIAG_OUTPUT")?,
            serde_json::to_vec_pretty(&report)?,
        )?;
        Ok(())
    }

    /// Inspect retained states without opening Excel or changing corpus results.
    #[test]
    #[ignore = "Requires XML_DIAG_LEFT and XML_DIAG_RIGHT retained XLSX paths"]
    fn inspect_retained_state_selection() -> Result<()> {
        let left = Snapshot::read(Path::new(&std::env::var("XML_DIAG_LEFT")?))?;
        let right = Snapshot::read(Path::new(&std::env::var("XML_DIAG_RIGHT")?))?;
        let (changes, skipped) = left.content_changes(&right)?;
        let mut candidates = cell_candidates(&changes);
        let cell_candidates_count = candidates.len();
        let decisions = super::super::conditional_formatting::select(
            &left, &right, &changes, &mut candidates,
        )?;
        let locations: BTreeMap<_, usize> = changes.iter().fold(BTreeMap::new(), |mut counts, change| {
            let kind = change.location.split(':').next().unwrap_or("unknown");
            *counts.entry(format!("{}:{kind}", change.sheet)).or_default() += 1;
            counts
        });
        let reasons: Vec<_> = decisions.iter().map(|decision| serde_json::json!({
            "sheet": decision.sheet, "outcome": decision.outcome, "reason": decision.reason,
            "changed_inputs": decision.changed_inputs,
            "referenced_input_changes": decision.referenced_input_changes,
            "formula_input_observations": decision.formula_input_observations,
            "referenced_formula_observations": decision.referenced_formula_observations,
            "layout_rows": decision.layout_rows,
            "sqref_length": decision.sqref.len(), "sqref_prefix": decision.sqref.chars().take(120).collect::<String>()
        })).collect();
        let mut last_candidate_rows = BTreeMap::<String, u32>::new();
        for candidate in &candidates {
            let range = super::super::content_bounds::Rectangle::parse(&candidate.cell)?;
            last_candidate_rows.entry(candidate.sheet.clone())
                .and_modify(|last| *last = (*last).max(range.bottom)).or_insert(range.bottom);
        }
        let mut bounds = super::super::content_bounds::ContentBounds::default();
        bounds.include(&left);
        bounds.include(&right);
        super::super::layout_candidates::select(&left, &right, &bounds, &mut candidates)?;
        let (selected, coverage) = bounds.select(&candidates)?;
        let coalesced = super::super::capture_layout::coalesce(&selected)?;
        let report = serde_json::json!({"changes": locations, "skipped": skipped,
            "cf_decisions": reasons, "cell_candidates": cell_candidates_count,
            "all_candidates": candidates.len(), "last_candidate_rows":last_candidate_rows,
            "coverage": coverage, "selected_grid_tiles": selected.len(),
            "coalesced_regions": coalesced,
            "note": "XML-only selection. Effective row heights and measured image-size partitioning require live Excel; charts are additional independent exports."});
        if let Ok(path) = std::env::var("XML_DIAG_OUTPUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        }
        eprintln!("{report}");
        Ok(())
    }

}
