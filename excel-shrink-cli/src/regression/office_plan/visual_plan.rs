#[path = "streaming_sheet.rs"]
mod streaming_sheet;

use super::visual_xml::{decimal, decode_attr, decode_sheet_name, Kind, Reader};
use anyhow::{ensure, Context, Result};
use memchr::memmem;
use serde::Serialize;
use std::{collections::HashMap, fs::File, io::Read, path::Path};
use zip::ZipArchive;

pub(crate) const MAX_ROWS: u32 = 1_048_576;
pub(crate) const MAX_COLUMNS: u32 = 16_384;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CaptureColumnSpan {
    pub min: u32,
    pub max: u32,
    pub width: Option<f64>,
    pub hidden: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CaptureColumns {
    pub default_width: f64,
    pub spans: Vec<CaptureColumnSpan>,
}

impl Default for CaptureColumns {
    fn default() -> Self {
        Self {
            default_width: 8.43,
            spans: Vec::new(),
        }
    }
}

impl CaptureColumns {
    fn trailing_hidden_start(&self) -> Result<Option<u32>> {
        let mut hidden = vec![false; MAX_COLUMNS as usize];
        for span in &self.spans {
            ensure!(
                span.min > 0 && span.min <= span.max && span.max <= MAX_COLUMNS,
                "invalid column span {}:{}",
                span.min,
                span.max
            );
            hidden[(span.min - 1) as usize..span.max as usize].fill(span.hidden);
        }
        if !hidden[MAX_COLUMNS as usize - 1] {
            return Ok(None);
        }
        let mut start = MAX_COLUMNS - 1;
        while start > 0 && hidden[(start - 1) as usize] {
            start -= 1;
        }
        Ok(Some(start))
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SheetVisualPlan {
    pub sheet: String,
    pub sheet_index: usize,
    pub sheet_file: String,
    pub base_right_column: u32,
    pub bottom_row: u32,
    pub row_height_bottom_row: u32,
    pub trailing_hidden_column_start: Option<u32>,
    pub trailing_hidden_row_start: Option<u32>,
    pub has_text: bool,
    pub horizontal_overflow_required: bool,
    pub drawing_fallback_required: bool,
    pub columns: CaptureColumns,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WorkbookVisualPlan {
    pub workbook: String,
    pub sheets: Vec<SheetVisualPlan>,
}

#[derive(Debug, Clone, Copy)]
struct Coordinate {
    row: u32,
    column: u32,
}

impl Coordinate {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let mut pos = 0;
        if bytes.get(pos) == Some(&b'$') {
            pos += 1;
        }
        let mut column = 0u32;
        while let Some(&byte) = bytes.get(pos) {
            if !byte.is_ascii_alphabetic() {
                break;
            }
            column = column
                .checked_mul(26)
                .and_then(|value| {
                    value.checked_add(u32::from(byte.to_ascii_uppercase() - b'A' + 1))
                })
                .context("column coordinate overflow")?;
            pos += 1;
        }
        if bytes.get(pos) == Some(&b'$') {
            pos += 1;
        }
        let row = decimal(&bytes[pos..])?;
        ensure!(
            (1..=MAX_ROWS).contains(&row) && (1..=MAX_COLUMNS).contains(&column),
            "cell reference outside XLSX grid"
        );
        Ok(Self { row, column })
    }
}

#[derive(Debug)]
struct SheetDescriptor {
    index: usize,
    name: String,
    file: String,
}

#[derive(Debug)]
struct Relationship {
    target: String,
    kind: String,
    external: bool,
}

#[derive(Debug, Default)]
struct SheetXmlSummary {
    max_row: u32,
    max_column: u32,
    has_text: bool,
    trailing_hidden_row_start: Option<u32>,
    columns: CaptureColumns,
}

#[derive(Debug, Default)]
struct DrawingSummary {
    max_row: u32,
    max_column: u32,
    fallback_required: bool,
}

fn read_zip_entry<R: Read + std::io::Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>> {
    let mut entry = zip
        .by_name(name)
        .with_context(|| format!("read OOXML part {name}"))?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_optional_zip_entry<R: Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>> {
    match zip.by_name(name) {
        Ok(mut entry) => {
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes)?;
            Ok(Some(bytes))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn resolve_part_target(source_part: &str, target: &str) -> Result<String> {
    let raw = if target.starts_with('/') {
        target.trim_start_matches('/').to_owned()
    } else {
        let directory = source_part
            .rsplit_once('/')
            .map(|(directory, _)| directory)
            .unwrap_or("");
        if directory.is_empty() {
            target.to_owned()
        } else {
            format!("{directory}/{target}")
        }
    };
    let mut parts = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                ensure!(parts.pop().is_some(), "OOXML target escapes package root");
            }
            _ => {
                ensure!(
                    !part.contains('\\'),
                    "backslash in OOXML relationship target"
                );
                parts.push(part);
            }
        }
    }
    Ok(parts.join("/"))
}

fn workbook_sheets<R: Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
) -> Result<Vec<SheetDescriptor>> {
    let workbook = read_zip_entry(zip, "xl/workbook.xml")?;
    let relationships = read_zip_entry(zip, "xl/_rels/workbook.xml.rels")?;

    let mut targets = HashMap::new();
    let mut rel_reader = Reader::<64>::new(&relationships)?;
    while let Some(tag) = rel_reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"Relationship" {
            continue;
        }
        if tag
            .attr(b"TargetMode")?
            .is_some_and(|value| value.eq_ignore_ascii_case(b"External"))
        {
            continue;
        }
        let id = decode_attr(tag.attr(b"Id")?.context("relationship Id missing")?)?;
        let target = decode_attr(
            tag.attr(b"Target")?
                .context("relationship Target missing")?,
        )?;
        targets.insert(id, resolve_part_target("xl/workbook.xml", &target)?);
    }

    let mut sheets = Vec::new();
    let mut reader = Reader::<64>::new(&workbook)?;
    let mut workbook_index = 0usize;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"sheet" {
            continue;
        }
        workbook_index += 1;
        let state = tag.attr(b"state")?.unwrap_or(b"visible");
        if !state.eq_ignore_ascii_case(b"visible") {
            continue;
        }
        let name = decode_sheet_name(tag.attr(b"name")?.context("sheet name missing")?)?;
        let rid = tag
            .attr(b"r:id")?
            .or(tag.attr(b"id")?)
            .context("sheet relationship id missing")?;
        let rid = decode_attr(rid)?;
        let Some(file) = targets.get(&rid) else {
            continue;
        };
        if file.starts_with("xl/worksheets/") {
            sheets.push(SheetDescriptor {
                index: workbook_index,
                name,
                file: file.clone(),
            });
        }
    }
    Ok(sheets)
}

pub(in crate::regression) fn parse_column_width(value: &[u8]) -> Result<f64> {
    let width: f64 = std::str::from_utf8(value)?.parse()?;
    // OOXML stores padding as part of the width, unlike Excel's 255-character
    // UI limit. Excel writes 255.7109375 for Calibri 11 and 256.5 for Arial 1.
    // Allow the five padding pixels even for a one-pixel digit width, without
    // clamping the source geometry or accepting unbounded capture dimensions.
    const MAX_STORED_COLUMN_WIDTH: f64 = 255.0 + 5.0;
    ensure!(
        width.is_finite() && (0.0..=MAX_STORED_COLUMN_WIDTH).contains(&width),
        "invalid column width {width}"
    );
    Ok(width)
}

#[cfg(test)]
fn scan_sheet(bytes: &[u8]) -> Result<SheetXmlSummary> {
    streaming_sheet::scan(bytes)
}

fn worksheet_relationships_path(sheet_part: &str) -> Result<String> {
    let (directory, file) = sheet_part
        .rsplit_once('/')
        .context("worksheet path has no directory")?;
    Ok(format!("{directory}/_rels/{file}.rels"))
}

fn parse_relationships(bytes: &[u8]) -> Result<Vec<Relationship>> {
    let mut relationships = Vec::new();
    let mut reader = Reader::<64>::new(bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"Relationship" {
            continue;
        }
        let target = decode_attr(
            tag.attr(b"Target")?
                .context("relationship target missing")?,
        )?;
        let kind = decode_attr(tag.attr(b"Type")?.context("relationship type missing")?)?;
        let external = tag
            .attr(b"TargetMode")?
            .is_some_and(|value| value.eq_ignore_ascii_case(b"External"));
        relationships.push(Relationship {
            target,
            kind,
            external,
        });
    }
    Ok(relationships)
}

fn marker_coordinate(bytes: &[u8]) -> Result<Coordinate> {
    let mut reader = Reader::<32>::new(bytes)?;
    let mut row = None;
    let mut column = None;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End {
            continue;
        }
        if tag.local() == b"row" {
            let element = reader.element(tag)?;
            row = Some(
                decimal(element.body)?
                    .checked_add(1)
                    .context("drawing row overflow")?,
            );
        } else if tag.local() == b"col" {
            let element = reader.element(tag)?;
            column = Some(
                decimal(element.body)?
                    .checked_add(1)
                    .context("drawing column overflow")?,
            );
        } else {
            let _ = reader.element(tag)?;
        }
    }
    let coordinate = Coordinate {
        row: row.context("drawing row missing")?,
        column: column.context("drawing column missing")?,
    };
    ensure!(
        coordinate.row <= MAX_ROWS && coordinate.column <= MAX_COLUMNS,
        "drawing outside grid"
    );
    Ok(coordinate)
}

fn anchor_marker(anchor: &[u8], wanted: &[u8]) -> Result<Option<Coordinate>> {
    let mut reader = Reader::<64>::new(anchor)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End {
            continue;
        }
        if tag.depth == 0 && tag.local() == wanted {
            let element = reader.element(tag)?;
            return marker_coordinate(element.body).map(Some);
        }
        if tag.depth == 0 {
            let _ = reader.element(tag)?;
        }
    }
    Ok(None)
}

fn anchor_has_unmodeled_visuals(raw: &[u8]) -> bool {
    [
        b"rot=".as_slice(),
        b"effectLst".as_slice(),
        b"effectDag".as_slice(),
        b"AlternateContent".as_slice(),
        b":ln w=".as_slice(),
    ]
    .iter()
    .any(|needle| memmem::find(raw, needle).is_some())
}

fn parse_drawing(bytes: &[u8]) -> Result<DrawingSummary> {
    let mut summary = DrawingSummary::default();
    let mut reader = Reader::<128>::new(bytes)?;
    let mut root_seen = false;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End {
            continue;
        }
        if tag.depth == 0 {
            ensure!(
                !root_seen && tag.local() == b"wsDr",
                "expected drawing root"
            );
            root_seen = true;
            continue;
        }
        if matches!(tag.local(), b"AlternateContent" | b"Choice" | b"Fallback") {
            summary.fallback_required = true;
            continue;
        }
        let element = reader.element(tag)?;
        match tag.local() {
            b"twoCellAnchor" => {
                match (
                    anchor_marker(element.body, b"from"),
                    anchor_marker(element.body, b"to"),
                ) {
                    (Ok(Some(from)), Ok(Some(to))) => {
                        summary.max_row = summary.max_row.max(from.row).max(to.row);
                        summary.max_column = summary.max_column.max(from.column).max(to.column);
                    }
                    _ => summary.fallback_required = true,
                }
                if anchor_has_unmodeled_visuals(element.raw) {
                    summary.fallback_required = true;
                }
            }
            b"oneCellAnchor" => {
                if let Ok(Some(from)) = anchor_marker(element.body, b"from") {
                    summary.max_row = summary.max_row.max(from.row);
                    summary.max_column = summary.max_column.max(from.column);
                }
                summary.fallback_required = true;
            }
            b"absoluteAnchor" => summary.fallback_required = true,
            _ => summary.fallback_required = true,
        }
    }
    ensure!(root_seen, "drawing root missing");
    Ok(summary)
}

fn hidden_note_shape(style: &[u8], body: &[u8]) -> Result<bool> {
    let style = decode_attr(style)?.to_ascii_lowercase();
    let visibility = style
        .split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            (name.trim() == "visibility").then(|| value.trim())
        })
        .last();
    if visibility != Some("hidden") {
        return Ok(false);
    }
    let mut reader = Reader::<64>::new(body)?;
    let mut note = false;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End {
            continue;
        }
        let element = reader.element(tag)?;
        match tag.local() {
            b"ClientData" => {
                if note || tag.attr(b"ObjectType")? != Some(b"Note") {
                    return Ok(false);
                }
                let mut data = Reader::<32>::new(element.body)?;
                while let Some(child) = data.next()? {
                    // Excel also persists a visibility flag separately from
                    // the VML style. A contradictory flag is not proof of a
                    // hidden note, even if its value is unfamiliar.
                    if child.kind != Kind::End && child.local() == b"Visible" {
                        return Ok(false);
                    }
                }
                note = true;
            }
            b"fill" | b"shadow" | b"path" | b"stroke" | b"textbox" => {}
            _ => return Ok(false),
        }
    }
    Ok(note)
}

fn known_vml_definition(name: &[u8], body: &[u8]) -> Result<bool> {
    let children: &[&[u8]] = match name {
        b"shapelayout" => &[b"idmap"],
        b"shapetype" => &[b"stroke", b"path"],
        _ => return Ok(false),
    };
    let mut reader = Reader::<64>::new(body)?;
    while let Some(tag) = reader.next()? {
        if tag.kind != Kind::End && (tag.depth != 0 || !children.contains(&tag.local())) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn vml_has_only_hidden_notes(bytes: &[u8]) -> Result<bool> {
    let mut document = Reader::<128>::new(bytes)?;
    let root = document.next()?.context("VML root missing")?;
    ensure!(
        root.depth == 0 && root.local() == b"xml",
        "Expected VML root"
    );
    let root = document.element(root)?;
    ensure!(
        document.next()?.is_none(),
        "Unexpected content after VML root"
    );
    let mut reader = Reader::<128>::new(root.body)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End {
            continue;
        }
        let element = reader.element(tag)?;
        match tag.local() {
            b"shapelayout" | b"shapetype" => {
                // Standard comment definitions contain only ID mappings and
                // path/stroke properties. Unknown template contents can carry
                // inherited visibility or objects; do not treat them as inert.
                if !known_vml_definition(tag.local(), element.body)? {
                    return Ok(false);
                }
            }
            b"shape" => {
                let Some(style) = tag.attr(b"style")? else {
                    return Ok(false);
                };
                if !hidden_note_shape(style, element.body)? {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

fn drawing_summary<R: Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
    sheet_part: &str,
) -> Result<DrawingSummary> {
    let rels_path = worksheet_relationships_path(sheet_part)?;
    let Some(rels) = read_optional_zip_entry(zip, &rels_path)? else {
        return Ok(DrawingSummary::default());
    };
    let mut summary = DrawingSummary::default();
    for relationship in parse_relationships(&rels)? {
        let kind = relationship.kind.rsplit('/').next().unwrap_or("");
        if kind == "vmlDrawing" {
            // Legacy comments are VML drawings but are absent from Office.js
            // worksheet.shapes. Hidden notes add no visible extent to capture.
            let hidden_notes_only = if relationship.external {
                false
            } else {
                let target = resolve_part_target(sheet_part, &relationship.target)?;
                read_optional_zip_entry(zip, &target)?
                    .is_some_and(|bytes| vml_has_only_hidden_notes(&bytes).unwrap_or(false))
            };
            summary.fallback_required |= !hidden_notes_only;
        }
        if matches!(kind, "oleObject" | "control") {
            summary.fallback_required = true;
        }
        if kind != "drawing" {
            continue;
        }
        if relationship.external {
            summary.fallback_required = true;
            continue;
        }
        let target = resolve_part_target(sheet_part, &relationship.target)?;
        let Some(drawing) = read_optional_zip_entry(zip, &target)? else {
            summary.fallback_required = true;
            continue;
        };
        match parse_drawing(&drawing) {
            Ok(parsed) => {
                summary.max_row = summary.max_row.max(parsed.max_row);
                summary.max_column = summary.max_column.max(parsed.max_column);
                summary.fallback_required |= parsed.fallback_required;
            }
            Err(_) => summary.fallback_required = true,
        }
    }
    Ok(summary)
}

pub(crate) fn build_workbook_visual_plan(path: &Path) -> Result<WorkbookVisualPlan> {
    let file = File::open(path).with_context(|| format!("open workbook {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("open XLSX as ZIP")?;
    let descriptors = workbook_sheets(&mut zip)?;
    let mut sheets = Vec::with_capacity(descriptors.len());

    for descriptor in descriptors {
        let sheet = zip.by_name(&descriptor.file)
            .with_context(|| format!("open worksheet {}", descriptor.file))?;
        let xml = streaming_sheet::scan(sheet)
            .with_context(|| format!("stream capture bounds for {}", descriptor.file))?;
        let drawing = drawing_summary(&mut zip, &descriptor.file)?;

        let trailing_column = xml.columns.trailing_hidden_start()?;
        let trailing_row = xml.trailing_hidden_row_start.map(|row| row - 1);
        let content_right = xml.max_column.saturating_sub(1);
        let content_bottom = xml.max_row.saturating_sub(1);
        // Capture one cell beyond drawings, but exclude that margin from the height gate.
        let drawing_right = drawing.max_column.min(MAX_COLUMNS - 1);
        let drawing_bottom = drawing.max_row.min(MAX_ROWS - 1);
        let row_height_bottom_row = content_bottom.max(drawing.max_row.saturating_sub(1));
        let natural_right = content_right.max(drawing_right);
        let natural_bottom = content_bottom.max(drawing_bottom);

        let base_right_column = match trailing_column {
            Some(start) => start.saturating_sub(1),
            None => natural_right,
        };
        let bottom_row = match trailing_row {
            Some(start) => start.saturating_sub(1),
            None => natural_bottom,
        };

        sheets.push(SheetVisualPlan {
            sheet: descriptor.name,
            sheet_index: descriptor.index,
            sheet_file: descriptor.file,
            base_right_column,
            bottom_row,
            row_height_bottom_row: row_height_bottom_row.min(bottom_row),
            trailing_hidden_column_start: trailing_column,
            trailing_hidden_row_start: trailing_row,
            has_text: xml.has_text,
            horizontal_overflow_required: trailing_column.is_none()
                && xml.has_text
                && base_right_column < MAX_COLUMNS - 1,
            drawing_fallback_required: drawing.fallback_required,
            columns: xml.columns,
        });
    }

    Ok(WorkbookVisualPlan {
        workbook: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workbook.xlsx")
            .to_owned(),
        sheets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_cdata_does_not_change_sheet_capture_bounds() {
        let xml = br#"<worksheet><sheetData><row r="3"><c r="B3"><v>7</v></c></row></sheetData>
            <headerFooter><oddFooter><![CDATA[&C Page < 1]]></oddFooter></headerFooter></worksheet>"#;
        let summary = scan_sheet(xml).unwrap();
        assert_eq!((summary.max_row, summary.max_column), (3, 2));
    }

    #[test]
    fn inline_cell_cdata_contributes_to_capture_bounds() {
        let xml = br#"<worksheet><sheetData><row r="3"><c r="B3" t="inlineStr"><is><t><![CDATA[<&text]]></t></is></c></row></sheetData></worksheet>"#;
        let summary = scan_sheet(xml).unwrap();
        assert_eq!((summary.max_row, summary.max_column), (3, 2));
        assert!(summary.has_text);
    }

    #[test]
    fn excel_maximum_widths_include_padding_and_retain_the_source_geometry() {
        let xml = br#"<worksheet><sheetFormatPr defaultColWidth="255.7109375"/>
            <cols>
                <col min="1" max="1" width="255.5703125"/>
                <col min="2" max="2" width="255.7109375"/>
                <col min="3" max="3" width="256.5"/>
            </cols>
            <sheetData><row r="1"><c r="C1"><v>1</v></c></row></sheetData>
        </worksheet>"#;
        let summary = scan_sheet(xml).unwrap();
        assert_eq!(summary.columns.default_width, 255.7109375);
        assert_eq!(
            summary
                .columns
                .spans
                .iter()
                .map(|span| span.width.unwrap())
                .collect::<Vec<_>>(),
            [255.5703125, 255.7109375, 256.5]
        );
        assert_eq!((summary.max_row, summary.max_column), (1, 3));
    }

    #[test]
    fn column_widths_remain_finite_nonnegative_and_bounded() {
        for value in ["0", "8.43", "255", "260"] {
            assert!(parse_column_width(value.as_bytes()).is_ok(), "{value}");
        }
        for value in [
            "-0.1",
            "260.00390625",
            "65536",
            "1e308",
            "NaN",
            "inf",
            "-inf",
        ] {
            for xml in [
                format!(
                    r#"<worksheet><cols><col min="1" max="1" width="{value}"/></cols></worksheet>"#
                ),
                format!(r#"<worksheet><sheetFormatPr defaultColWidth="{value}"/></worksheet>"#),
            ] {
                assert!(scan_sheet(xml.as_bytes()).is_err(), "{xml}");
            }
        }
    }

    #[test]
    fn trailing_hidden_columns_require_a_hidden_xfd_suffix() {
        let columns = CaptureColumns {
            default_width: 8.43,
            spans: vec![CaptureColumnSpan {
                min: 10,
                max: MAX_COLUMNS,
                width: None,
                hidden: true,
            }],
        };
        assert_eq!(columns.trailing_hidden_start().unwrap(), Some(9));

        let middle_only = CaptureColumns {
            default_width: 8.43,
            spans: vec![CaptureColumnSpan {
                min: 10,
                max: 20,
                width: None,
                hidden: true,
            }],
        };
        assert_eq!(middle_only.trailing_hidden_start().unwrap(), None);
    }

    #[test]
    fn explicit_trailing_hidden_rows_are_a_visual_tail_heuristic() {
        let xml = br#"<worksheet><sheetData>
            <row r="101" hidden="1"/>
            <row r="102"><c r="A102"><v>1</v></c></row>
            <row r="105"><c r="A105"><v>1</v></c></row>
            <row r="106" hidden="1"/><row r="107" hidden="1"/><row r="108" hidden="1"/>
        </sheetData></worksheet>"#;
        let summary = scan_sheet(xml).unwrap();
        assert_eq!(summary.trailing_hidden_row_start, Some(106));
        assert_eq!(summary.max_row, 105);
    }

    #[test]
    fn omitted_rows_do_not_cancel_the_visual_hidden_tail() {
        let xml = br#"<worksheet><sheetData>
            <row r="105"><c r="A105"><v>1</v></c></row>
            <row r="106" hidden="1"/>
            <row r="108" hidden="1"/>
        </sheetData></worksheet>"#;
        let summary = scan_sheet(xml).unwrap();
        assert_eq!(summary.trailing_hidden_row_start, Some(106));
    }

    #[test]
    fn two_cell_drawings_extend_bounds_without_excel_fallback() {
        let drawing = br#"<xdr:wsDr xmlns:xdr="x"><xdr:twoCellAnchor>
            <xdr:from><xdr:col>1</xdr:col><xdr:row>2</xdr:row></xdr:from>
            <xdr:to><xdr:col>9</xdr:col><xdr:row>39</xdr:row></xdr:to>
            <xdr:pic/>
        </xdr:twoCellAnchor></xdr:wsDr>"#;
        let plan = parse_drawing(drawing).unwrap();
        assert_eq!((plan.max_row, plan.max_column), (40, 10));
        assert!(!plan.fallback_required);
    }

    #[test]
    fn alternate_drawing_branches_extend_the_capture_bounds() {
        let drawing = br#"<xdr:wsDr xmlns:xdr="x" xmlns:mc="m">
            <mc:AlternateContent><mc:Choice Requires="a14">
                <xdr:twoCellAnchor>
                    <xdr:from><xdr:col>1</xdr:col><xdr:row>7</xdr:row></xdr:from>
                    <xdr:to><xdr:col>1</xdr:col><xdr:row>12</xdr:row><xdr:rowOff>157162</xdr:rowOff></xdr:to>
                    <xdr:sp/>
                </xdr:twoCellAnchor>
            </mc:Choice><mc:Fallback>
                <xdr:twoCellAnchor>
                    <xdr:from><xdr:col>1</xdr:col><xdr:row>7</xdr:row></xdr:from>
                    <xdr:to><xdr:col>3</xdr:col><xdr:row>9</xdr:row></xdr:to>
                    <xdr:pic/>
                </xdr:twoCellAnchor>
            </mc:Fallback></mc:AlternateContent>
        </xdr:wsDr>"#;
        let plan = parse_drawing(drawing).unwrap();
        assert_eq!((plan.max_row, plan.max_column), (13, 4));
        assert!(plan.fallback_required);
    }

    #[test]
    fn one_cell_drawings_request_the_narrow_office_shape_fallback() {
        let drawing = br#"<xdr:wsDr xmlns:xdr="x"><xdr:oneCellAnchor>
            <xdr:from><xdr:col>4</xdr:col><xdr:row>7</xdr:row></xdr:from>
            <xdr:ext cx="25400" cy="38100"/><xdr:pic/>
        </xdr:oneCellAnchor></xdr:wsDr>"#;
        let plan = parse_drawing(drawing).unwrap();
        assert!(plan.fallback_required);
        assert_eq!((plan.max_row, plan.max_column), (8, 5));
    }

    #[test]
    fn hidden_legacy_notes_do_not_require_office_shape_bounds() {
        let vml = br##"<xml xmlns:v="urn:schemas-microsoft-com:vml"
            xmlns:x="urn:schemas-microsoft-com:office:excel" xmlns:o="urn:schemas-microsoft-com:office:office">
            <o:shapelayout><o:idmap data="1"/></o:shapelayout>
            <v:shapetype id="_x0000_t202"><v:stroke joinstyle="miter"/></v:shapetype>
            <v:shape type="#_x0000_t202" style="position:absolute; width:108.6pt; height:57pt; visibility:hidden">
                <v:fill/><v:shadow/><v:path/><v:textbox><div>Note text</div></v:textbox>
                <x:ClientData ObjectType="Note"><x:MoveWithCells/><x:SizeWithCells/>
                    <x:Anchor>7, 10, 84, 1, 8, 56, 88, 17</x:Anchor>
                    <x:Row>84</x:Row><x:Column>8</x:Column>
                </x:ClientData>
            </v:shape>
        </xml>"##;
        assert!(vml_has_only_hidden_notes(vml).unwrap());
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        use std::io::Write;
        archive
            .start_file(
                "xl/worksheets/_rels/sheet1.xml.rels",
                zip::write::FileOptions::default(),
            )
            .unwrap();
        archive.write_all(br#"<Relationships><Relationship Type="x/vmlDrawing" Target="../drawings/notes.vml"/></Relationships>"#).unwrap();
        archive
            .start_file("xl/drawings/notes.vml", zip::write::FileOptions::default())
            .unwrap();
        archive.write_all(vml).unwrap();
        let mut archive = ZipArchive::new(archive.finish().unwrap()).unwrap();
        let summary = drawing_summary(&mut archive, "xl/worksheets/sheet1.xml").unwrap();
        assert!(!summary.fallback_required);
        assert_eq!((summary.max_row, summary.max_column), (0, 0));
    }

    #[test]
    fn visible_or_unknown_vml_keeps_conservative_drawing_fallback() {
        let note = r#"<xml xmlns:v="v" xmlns:x="x"><v:shape style="visibility:hidden">
            <x:ClientData ObjectType="Note"><x:Row>84</x:Row></x:ClientData></v:shape></xml>"#;
        assert!(vml_has_only_hidden_notes(note.as_bytes()).unwrap());
        for unknown in [
            note.replace("visibility:hidden", "visibility:visible"),
            note.replace("visibility:hidden", "visibility:hidden; visibility:visible"),
            note.replace("visibility:hidden", ""),
            note.replace("ObjectType=\"Note\"", "ObjectType=\"Button\""),
            note.replace("<x:Row>", "<x:Visible/><x:Row>"),
            note.replace("<v:shape", "<v:group")
                .replace("</v:shape>", "</v:group>"),
            note.replace("</xml>", "<v:rect/></xml>"),
        ] {
            assert!(
                !vml_has_only_hidden_notes(unknown.as_bytes()).unwrap(),
                "{unknown}"
            );
        }
        assert!(vml_has_only_hidden_notes(b"<xml><v:shape").is_err());
    }

    #[test]
    fn hidden_note_definitions_cannot_hide_unverified_visibility_or_objects() {
        let note =
            r#"<v:shape style="visibility:hidden"><x:ClientData ObjectType="Note"/></v:shape>"#;
        for definition in [
            "<v:shapetype><x:ClientData ObjectType=\"Note\"><x:Visible/></x:ClientData></v:shapetype>",
            "<v:shapetype><v:group><v:shape/></v:group></v:shapetype>",
            "<o:shapelayout><v:group/></o:shapelayout>",
            "<v:shapetype><v:path><x:Visible/></v:path></v:shapetype>",
        ] {
            let vml = format!("<xml xmlns:v=\"v\" xmlns:x=\"x\" xmlns:o=\"o\">{definition}{note}</xml>");
            assert!(!vml_has_only_hidden_notes(vml.as_bytes()).unwrap(), "{vml}");
        }
    }

    #[test]
    fn content_and_merge_bounds_ignore_declared_dimension() {
        let xml = br#"<worksheet><dimension ref="A1:XFD1048576"/><sheetData>
            <row r="2"><c r="C2" t="s"><v>0</v></c><c r="ZZ2" s="3"/></row>
            <row r="9"><c r="B9"><v>42</v></c></row>
        </sheetData><mergeCells><mergeCell ref="A10:K12"/></mergeCells></worksheet>"#;
        let summary = scan_sheet(xml).unwrap();
        assert_eq!((summary.max_row, summary.max_column), (12, 11));
    }

    #[test]
    fn worksheet_indices_retain_hidden_sheet_positions() {
        use std::io::{Cursor, Write};
        use zip::write::FileOptions;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
        writer
            .start_file("xl/workbook.xml", FileOptions::default())
            .unwrap();
        writer
            .write_all(
                br#"<workbook xmlns:r="r"><sheets>
            <sheet name="First_x0009_" sheetId="4" r:id="rId1"/>
            <sheet name="Hidden" sheetId="9" state="hidden" r:id="rId2"/>
            <sheet name=" Third" sheetId="20" r:id="rId3"/>
        </sheets></workbook>"#,
            )
            .unwrap();
        writer
            .start_file("xl/_rels/workbook.xml.rels", FileOptions::default())
            .unwrap();
        writer
            .write_all(
                br#"<Relationships>
            <Relationship Id="rId1" Target="worksheets/sheet1.xml"/>
            <Relationship Id="rId2" Target="worksheets/sheet2.xml"/>
            <Relationship Id="rId3" Target="worksheets/sheet3.xml"/>
        </Relationships>"#,
            )
            .unwrap();
        let buffer = writer.finish().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buffer.into_inner())).unwrap();
        let descriptors = workbook_sheets(&mut archive).unwrap();
        assert_eq!(descriptors.len(), 2);
        assert_eq!(descriptors[0].index, 1);
        assert_eq!(descriptors[1].index, 3);
        assert_eq!(descriptors[0].name, "First\t");
        assert_eq!(descriptors[1].name, " Third");
    }
}
