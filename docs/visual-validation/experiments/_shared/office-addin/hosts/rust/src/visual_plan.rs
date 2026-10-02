use crate::visual_xml::{Kind, Reader, decode_attr, decimal, truth};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use image::{DynamicImage, ImageFormat, ImageReader, Rgba, RgbaImage};
use memchr::memmem;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Cursor, Read},
    path::Path,
    time::Instant,
};
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
    fn expanded_widths(&self) -> Result<Vec<f64>> {
        ensure!(self.default_width.is_finite() && self.default_width >= 0.0, "invalid default column width");
        let mut widths = vec![self.default_width; MAX_COLUMNS as usize];
        for span in &self.spans {
            ensure!(
                span.min > 0 && span.min <= span.max && span.max <= MAX_COLUMNS,
                "invalid column span {}:{}",
                span.min,
                span.max
            );
            let width = if span.hidden {
                0.0
            } else {
                span.width.unwrap_or(self.default_width)
            };
            ensure!(width.is_finite() && width >= 0.0, "invalid column width");
            widths[(span.min - 1) as usize..span.max as usize].fill(width);
        }
        Ok(widths)
    }

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
    pub content_right_column: u32,
    pub content_bottom_row: u32,
    pub drawing_right_column: u32,
    pub drawing_bottom_row: u32,
    pub base_right_column: u32,
    pub bottom_row: u32,
    pub trailing_hidden_column_start: Option<u32>,
    pub trailing_hidden_row_start: Option<u32>,
    pub right_bound_source: String,
    pub bottom_bound_source: String,
    pub has_text: bool,
    pub horizontal_overflow_required: bool,
    pub drawing_fallback_required: bool,
    pub drawing_anchor_count: usize,
    pub columns: CaptureColumns,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WorkbookVisualPlan {
    pub workbook: String,
    pub rust_plan_ms: f64,
    pub sheets: Vec<SheetVisualPlan>,
}

impl WorkbookVisualPlan {
    fn sheet(&self, name: &str) -> Option<&SheetVisualPlan> {
        self.sheets.iter().find(|sheet| sheet.sheet == name)
    }
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
    anchor_count: usize,
}

fn read_zip_entry<R: Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>> {
    let mut entry = zip.by_name(name).with_context(|| format!("read OOXML part {name}"))?;
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
                ensure!(!part.contains('\\'), "backslash in OOXML relationship target");
                parts.push(part);
            }
        }
    }
    Ok(parts.join("/"))
}

fn workbook_sheets<R: Read + std::io::Seek>(zip: &mut ZipArchive<R>) -> Result<Vec<SheetDescriptor>> {
    let workbook = read_zip_entry(zip, "xl/workbook.xml")?;
    let relationships = read_zip_entry(zip, "xl/_rels/workbook.xml.rels")?;

    let mut targets = HashMap::new();
    let mut rel_reader = Reader::<64>::new(&relationships)?;
    while let Some(tag) = rel_reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"Relationship" {
            continue;
        }
        if tag.attr(b"TargetMode")?.is_some_and(|value| value.eq_ignore_ascii_case(b"External")) {
            continue;
        }
        let id = decode_attr(tag.attr(b"Id")?.context("relationship Id missing")?)?;
        let target = decode_attr(tag.attr(b"Target")?.context("relationship Target missing")?)?;
        targets.insert(id, resolve_part_target("xl/workbook.xml", &target)?);
    }

    let mut sheets = Vec::new();
    let mut reader = Reader::<64>::new(&workbook)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"sheet" {
            continue;
        }
        let state = tag.attr(b"state")?.unwrap_or(b"visible");
        if !state.eq_ignore_ascii_case(b"visible") {
            continue;
        }
        let name = decode_attr(tag.attr(b"name")?.context("sheet name missing")?)?;
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
                name,
                file: file.clone(),
            });
        }
    }
    Ok(sheets)
}

fn parse_column_width(value: &[u8]) -> Result<f64> {
    let width: f64 = std::str::from_utf8(value)?.parse()?;
    ensure!(width.is_finite() && (0.0..=255.0).contains(&width), "invalid column width");
    Ok(width)
}

fn scan_columns(body: &[u8], columns: &mut CaptureColumns) -> Result<()> {
    let mut reader = Reader::<64>::new(body)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"col" {
            continue;
        }
        let min = decimal(tag.attr(b"min")?.context("column min missing")?)?;
        let max = decimal(tag.attr(b"max")?.context("column max missing")?)?;
        ensure!(min > 0 && min <= max && max <= MAX_COLUMNS, "invalid column range");
        let width = tag.attr(b"width")?.map(parse_column_width).transpose()?;
        columns.spans.push(CaptureColumnSpan {
            min,
            max,
            width,
            hidden: truth(tag.attr(b"hidden")?),
        });
    }
    Ok(())
}

fn cell_body_flags(body: &[u8]) -> Result<(bool, bool)> {
    if body.is_empty() {
        return Ok((false, false));
    }
    let mut reader = Reader::<32>::new(body)?;
    let mut content = false;
    let mut inline_text = false;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End {
            continue;
        }
        match tag.local() {
            b"f" => {
                content = true;
                let _ = reader.element(tag)?;
            }
            b"v" => {
                let element = reader.element(tag)?;
                if element.body.iter().any(|byte| !byte.is_ascii_whitespace()) {
                    content = true;
                }
            }
            b"is" => {
                let element = reader.element(tag)?;
                let mut inline_reader = Reader::<32>::new(element.body)?;
                while let Some(child) = inline_reader.next()? {
                    if child.kind == Kind::End || child.local() != b"t" {
                        continue;
                    }
                    let text = inline_reader.element(child)?;
                    if text.body.iter().any(|byte| !byte.is_ascii_whitespace()) {
                        content = true;
                        inline_text = true;
                    }
                }
            }
            _ => {
                let _ = reader.element(tag)?;
            }
        }
    }
    Ok((content, inline_text))
}

fn include_merge(summary: &mut SheetXmlSummary, reference: &[u8]) -> Result<()> {
    for cell in reference.split(|byte| *byte == b':') {
        let coordinate = Coordinate::parse(cell)?;
        summary.max_row = summary.max_row.max(coordinate.row);
        summary.max_column = summary.max_column.max(coordinate.column);
    }
    Ok(())
}

fn scan_merge_cells(body: &[u8], summary: &mut SheetXmlSummary) -> Result<()> {
    let mut reader = Reader::<64>::new(body)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"mergeCell" {
            continue;
        }
        if let Some(reference) = tag.attr(b"ref")? {
            include_merge(summary, reference)?;
        }
        let _ = reader.element(tag)?;
    }
    Ok(())
}

fn scan_sheet(bytes: &[u8]) -> Result<SheetXmlSummary> {
    let mut summary = SheetXmlSummary {
        max_row: 1,
        max_column: 1,
        ..Default::default()
    };
    let mut reader = Reader::<128>::new(bytes)?;
    let mut current_row = 0u32;
    let mut current_column = 0u32;
    let mut in_sheet_data = false;
    let mut in_row = false;
    let mut previous_explicit_row = None;
    let mut previous_row_hidden = false;

    while let Some(tag) = reader.next()? {
        let local = tag.local();
        if tag.kind == Kind::End {
            if local == b"row" && tag.depth == 2 {
                in_row = false;
            } else if local == b"sheetData" && tag.depth == 1 {
                in_sheet_data = false;
            }
            continue;
        }

        if tag.depth == 1 && local == b"sheetFormatPr" {
            if let Some(width) = tag.attr(b"defaultColWidth")? {
                summary.columns.default_width = parse_column_width(width)?;
            }
            let _ = reader.element(tag)?;
            continue;
        }
        if tag.depth == 1 && local == b"cols" {
            let element = reader.element(tag)?;
            scan_columns(element.body, &mut summary.columns)?;
            continue;
        }
        if tag.depth == 1 && local == b"sheetData" {
            in_sheet_data = tag.kind == Kind::Start;
            continue;
        }
        if tag.depth == 1 && local == b"mergeCells" {
            let element = reader.element(tag)?;
            scan_merge_cells(element.body, &mut summary)?;
            continue;
        }
        if in_sheet_data && tag.depth == 2 && local == b"row" {
            current_row = match tag.attr(b"r")? {
                Some(value) => decimal(value)?,
                None => current_row.checked_add(1).context("row overflow")?,
            };
            ensure!((1..=MAX_ROWS).contains(&current_row), "row outside XLSX grid");
            current_column = 0;
            let hidden = truth(tag.attr(b"hidden")?);
            if hidden {
                let continues = previous_row_hidden
                    && previous_explicit_row.is_some_and(|row| row.checked_add(1) == Some(current_row));
                if !continues {
                    summary.trailing_hidden_row_start = Some(current_row);
                }
            } else {
                summary.trailing_hidden_row_start = None;
            }
            previous_explicit_row = Some(current_row);
            previous_row_hidden = hidden;
            in_row = tag.kind == Kind::Start;
            continue;
        }
        if in_row && tag.depth == 3 && local == b"c" {
            let reference = tag.attr(b"r")?;
            let coordinate = match reference {
                Some(value) => Coordinate::parse(value)?,
                None => Coordinate {
                    row: current_row,
                    column: current_column.checked_add(1).context("column overflow")?,
                },
            };
            ensure!(coordinate.row == current_row && coordinate.column <= MAX_COLUMNS, "cell reference disagrees with row");
            current_column = coordinate.column;
            let cell_type = tag.attr(b"t")?.unwrap_or(b"n");
            let element = reader.element(tag)?;
            let (content, inline_text) = cell_body_flags(element.body)?;
            if content {
                summary.max_row = summary.max_row.max(coordinate.row);
                summary.max_column = summary.max_column.max(coordinate.column);
                summary.has_text |= inline_text
                    || matches!(cell_type, b"s" | b"inlineStr" | b"str");
            }
            continue;
        }
        if tag.depth == 1 {
            let _ = reader.element(tag)?;
        }
    }

    Ok(summary)
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
        let target = decode_attr(tag.attr(b"Target")?.context("relationship target missing")?)?;
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
            row = Some(decimal(element.body)?.checked_add(1).context("drawing row overflow")?);
        } else if tag.local() == b"col" {
            let element = reader.element(tag)?;
            column = Some(decimal(element.body)?.checked_add(1).context("drawing column overflow")?);
        } else {
            let _ = reader.element(tag)?;
        }
    }
    let coordinate = Coordinate {
        row: row.context("drawing row missing")?,
        column: column.context("drawing column missing")?,
    };
    ensure!(coordinate.row <= MAX_ROWS && coordinate.column <= MAX_COLUMNS, "drawing outside grid");
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
            ensure!(!root_seen && tag.local() == b"wsDr", "expected drawing root");
            root_seen = true;
            continue;
        }
        if tag.depth != 1 {
            continue;
        }
        let element = reader.element(tag)?;
        summary.anchor_count += 1;
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
        if matches!(kind, "vmlDrawing" | "oleObject" | "control") {
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
                summary.anchor_count += parsed.anchor_count;
            }
            Err(_) => summary.fallback_required = true,
        }
    }
    Ok(summary)
}

pub(crate) fn build_workbook_visual_plan(path: &Path) -> Result<WorkbookVisualPlan> {
    let started = Instant::now();
    let file = File::open(path).with_context(|| format!("open workbook {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("open XLSX as ZIP")?;
    let descriptors = workbook_sheets(&mut zip)?;
    let mut sheets = Vec::with_capacity(descriptors.len());

    for (index, descriptor) in descriptors.into_iter().enumerate() {
        let sheet_xml = read_zip_entry(&mut zip, &descriptor.file)?;
        let xml = scan_sheet(&sheet_xml)?;
        let drawing = drawing_summary(&mut zip, &descriptor.file)?;

        let trailing_column = xml.columns.trailing_hidden_start()?;
        let trailing_row = xml.trailing_hidden_row_start.map(|row| row - 1);
        let content_right = xml.max_column.saturating_sub(1);
        let content_bottom = xml.max_row.saturating_sub(1);
        let drawing_right = drawing.max_column.saturating_sub(1);
        let drawing_bottom = drawing.max_row.saturating_sub(1);
        let natural_right = content_right.max(drawing_right);
        let natural_bottom = content_bottom.max(drawing_bottom);

        let (base_right_column, right_bound_source) = match trailing_column {
            Some(start) => (start.saturating_sub(1), "trailing-hidden".to_string()),
            None => (natural_right, "rust-xml-content-drawing".to_string()),
        };
        let (bottom_row, bottom_bound_source) = match trailing_row {
            Some(start) => (start.saturating_sub(1), "trailing-hidden".to_string()),
            None => (natural_bottom, "rust-xml-content-drawing".to_string()),
        };

        sheets.push(SheetVisualPlan {
            sheet: descriptor.name,
            sheet_index: index + 1,
            sheet_file: descriptor.file,
            content_right_column: content_right,
            content_bottom_row: content_bottom,
            drawing_right_column: drawing_right,
            drawing_bottom_row: drawing_bottom,
            base_right_column,
            bottom_row,
            trailing_hidden_column_start: trailing_column,
            trailing_hidden_row_start: trailing_row,
            right_bound_source,
            bottom_bound_source,
            has_text: xml.has_text,
            horizontal_overflow_required: trailing_column.is_none()
                && xml.has_text
                && base_right_column < MAX_COLUMNS - 1,
            drawing_fallback_required: drawing.fallback_required,
            drawing_anchor_count: drawing.anchor_count,
            columns: xml.columns,
        });
    }

    Ok(WorkbookVisualPlan {
        workbook: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workbook.xlsx")
            .to_owned(),
        rust_plan_ms: started.elapsed().as_secs_f64() * 1000.0,
        sheets,
    })
}

fn safe_file_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => ch,
        })
        .collect()
}

fn column_name(index_zero_based: u32) -> String {
    let mut value = index_zero_based + 1;
    let mut name = String::new();
    while value > 0 {
        value -= 1;
        name.insert(0, (b'A' + (value % 26) as u8) as char);
        value /= 26;
    }
    name
}

fn capture_address(end_column: u32, bottom_row: u32) -> String {
    format!("A1:{}{}", column_name(end_column), bottom_row + 1)
}

fn end_column_for_visible_count(widths: &[f64], base: u32, extra: u32) -> u32 {
    let mut found = 0u32;
    for column in base.saturating_add(1)..MAX_COLUMNS {
        if widths[column as usize] > 0.0 {
            found += 1;
            if found >= extra {
                return column;
            }
        }
    }
    if found == 0 { base } else { MAX_COLUMNS - 1 }
}

fn column_edges(widths: &[f64], end_column: u32, image_width: u32) -> Result<Vec<u32>> {
    ensure!(end_column < MAX_COLUMNS && image_width > 0, "invalid probe image extent");
    let slice = &widths[..=end_column as usize];
    let total: f64 = slice.iter().sum();
    ensure!(total.is_finite() && total > 0.0, "capture has no visible XML column width");
    let mut edges = Vec::with_capacity(slice.len() + 1);
    edges.push(0);
    let mut cumulative = 0.0;
    for width in slice {
        cumulative += *width;
        edges.push((cumulative / total * f64::from(image_width)).round() as u32);
    }
    if let Some(last) = edges.last_mut() {
        *last = image_width;
    }
    Ok(edges)
}

fn estimate_background(
    image: &RgbaImage,
    edges: &[u32],
    base_column: u32,
    end_column: u32,
) -> Rgba<u8> {
    let start_column = base_column.saturating_add(1).min(end_column);
    let x0 = edges
        .get(start_column as usize)
        .copied()
        .unwrap_or(0)
        .min(image.width().saturating_sub(1));
    let x1 = edges
        .get(end_column as usize + 1)
        .copied()
        .unwrap_or(image.width())
        .max(x0 + 1)
        .min(image.width());
    let area = u64::from(x1.saturating_sub(x0)) * u64::from(image.height());
    let stride = ((area / 100_000).max(1) as f64).sqrt().floor().max(1.0) as u32;
    let mut histogram = HashMap::<u16, u32>::new();
    let mut y = 0;
    while y < image.height() {
        let mut x = x0;
        while x < x1 {
            let pixel = image.get_pixel(x, y);
            let key = (u16::from(pixel[0] >> 4) << 8)
                | (u16::from(pixel[1] >> 4) << 4)
                | u16::from(pixel[2] >> 4);
            *histogram.entry(key).or_default() += 1;
            x = x.saturating_add(stride);
        }
        y = y.saturating_add(stride);
    }
    let key = histogram
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(key, _)| key)
        .unwrap_or(0x0fff);
    Rgba([
        (((key >> 8) & 0xf) as u8) * 16 + 8,
        (((key >> 4) & 0xf) as u8) * 16 + 8,
        ((key & 0xf) as u8) * 16 + 8,
        255,
    ])
}

fn differs(pixel: &Rgba<u8>, background: &Rgba<u8>) -> bool {
    pixel[3] > 0
        && (pixel[0].abs_diff(background[0]) > 14
            || pixel[1].abs_diff(background[1]) > 14
            || pixel[2].abs_diff(background[2]) > 14)
}

fn horizontal_rule_mask(image: &RgbaImage, background: &Rgba<u8>) -> Vec<bool> {
    let mut candidates = vec![false; image.height() as usize];
    if image.width() < 16 {
        return candidates;
    }
    for y in 1..image.height().saturating_sub(1) {
        let reference = *image.get_pixel(1, y);
        if !differs(&reference, background) {
            continue;
        }
        candidates[y as usize] = (2..image.width().saturating_sub(1)).all(|x| {
            let pixel = image.get_pixel(x, y);
            (0..3).all(|channel| pixel[channel].abs_diff(reference[channel]) <= 8)
        });
    }
    let mut mask = vec![false; candidates.len()];
    let mut start = 1usize;
    while start < candidates.len() {
        if !candidates[start] {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < candidates.len() && candidates[end] {
            end += 1;
        }
        if end - start <= 4 {
            mask[start..end].fill(true);
        }
        start = end;
    }
    mask
}

struct OverflowDecision {
    keep_column: u32,
    done: bool,
    next_column: u32,
    next_extra: u32,
    scanned_pixels: u64,
}

fn analyze_overflow(
    image: &RgbaImage,
    widths: &[f64],
    base_column: u32,
    end_column: u32,
    extra: u32,
) -> Result<OverflowDecision> {
    ensure!(base_column <= end_column && end_column < MAX_COLUMNS, "invalid overflow bounds");
    let edges = column_edges(widths, end_column, image.width())?;
    let background = estimate_background(image, &edges, base_column, end_column);
    let rules = horizontal_rule_mask(image, &background);
    let last_visible = (base_column.saturating_add(1)..=end_column)
        .rev()
        .find(|column| widths[*column as usize] > 0.0);
    let more_visible = (end_column.saturating_add(1)..MAX_COLUMNS)
        .any(|column| widths[column as usize] > 0.0);
    let mut keep = base_column;
    let mut edge_content = false;
    let mut scanned_pixels = 0u64;

    for column in base_column.saturating_add(1)..=end_column {
        if widths[column as usize] <= 0.0 {
            continue;
        }
        let left = edges[column as usize];
        let right = edges[column as usize + 1].min(image.width());
        if right.saturating_sub(left) <= 2 {
            continue;
        }
        let mut content = false;
        'scan: for y in 1..image.height().saturating_sub(1) {
            if rules.get(y as usize).copied().unwrap_or(false) {
                continue;
            }
            for x in left.saturating_add(1)..right.saturating_sub(1) {
                scanned_pixels += 1;
                if differs(image.get_pixel(x, y), &background) {
                    content = true;
                    break 'scan;
                }
            }
        }
        if content {
            keep = keep.max(column);
            if Some(column) == last_visible {
                edge_content = true;
            }
        }
    }

    let next_extra = if extra < 8 {
        8
    } else {
        extra.saturating_mul(2).min(MAX_COLUMNS)
    };
    let next_column = end_column_for_visible_count(widths, base_column, next_extra);
    let done = !edge_content || !more_visible || next_column <= end_column;
    Ok(OverflowDecision {
        keep_column: keep,
        done,
        next_column,
        next_extra,
        scanned_pixels,
    })
}

fn encode_png(image: RgbaImage) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image).write_to(&mut cursor, ImageFormat::Png)?;
    Ok(cursor.into_inner())
}

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    ensure!(bytes.len() >= 24 && &bytes[..8] == b"\x89PNG\r\n\x1a\n", "payload is not a PNG");
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    Ok((width, height))
}

fn persist_capture(
    workbook: &str,
    id: &str,
    sheet: &str,
    address: &str,
    bytes: &[u8],
    image_root: &Path,
    extra: Value,
) -> Result<Value> {
    let workbook_dir = image_root.join(safe_file_name(workbook));
    fs::create_dir_all(&workbook_dir)?;
    let file_name = format!("{id}.png");
    fs::write(workbook_dir.join(&file_name), bytes)?;
    let (width, height) = png_dimensions(bytes)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut value = json!({
        "id": id,
        "workbook": workbook,
        "sheet": sheet,
        "address": address,
        "relative_png": format!("{}/{}", safe_file_name(workbook), file_name),
        "png_bytes": bytes.len(),
        "png_width": width,
        "png_height": height,
        "sha256": format!("{:x}", hasher.finalize())
    });
    if let (Value::Object(target), Value::Object(source)) = (&mut value, extra) {
        target.extend(source);
    }
    Ok(value)
}

pub(crate) fn process_planned_image_batch(
    plan: &WorkbookVisualPlan,
    payload: &Value,
    image_root: &Path,
) -> Result<Value> {
    let started = Instant::now();
    let workbook = payload["workbook"].as_str().context("workbook")?;
    ensure!(workbook == plan.workbook, "visual plan workbook mismatch");
    let images = payload["images"].as_array().context("images")?;
    let mut captures = Vec::new();
    let mut pending = Vec::new();
    let mut scanned_pixels = 0u64;

    for value in images {
        let id = value["id"].as_str().context("image id")?;
        let sheet = value["sheet"].as_str().context("image sheet")?;
        let bottom_row = value["bottom_row"].as_u64().context("bottom_row")? as u32;
        let base_right_column = value["base_right_column"].as_u64().context("base_right_column")? as u32;
        let end_column = value["end_column"].as_u64().context("end_column")? as u32;
        let overflow_probe = value["overflow_probe"].as_bool().unwrap_or(false);
        let probe_extra = value["probe_extra"].as_u64().unwrap_or(0) as u32;
        let bytes = BASE64.decode(value["base64"].as_str().context("image base64")?)?;

        if !overflow_probe {
            captures.push(persist_capture(
                workbook,
                id,
                sheet,
                &capture_address(end_column, bottom_row),
                &bytes,
                image_root,
                json!({
                    "rust_xml_base_right_column": base_right_column,
                    "overflow_checked": false
                }),
            )?);
            continue;
        }

        let sheet_plan = plan.sheet(sheet).with_context(|| format!("missing visual plan for sheet {sheet}"))?;
        let widths = sheet_plan.columns.expanded_widths()?;
        let decoded = ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()?
            .decode()?
            .to_rgba8();
        let decision = analyze_overflow(
            &decoded,
            &widths,
            base_right_column,
            end_column,
            probe_extra,
        )?;
        scanned_pixels += decision.scanned_pixels;

        if !decision.done {
            pending.push(json!({
                "id": id,
                "sheet": sheet,
                "next_column": decision.next_column,
                "next_extra": decision.next_extra
            }));
            continue;
        }

        let edges = column_edges(&widths, end_column, decoded.width())?;
        let crop_width = edges[(decision.keep_column + 1) as usize]
            .max(1)
            .min(decoded.width());
        let cropped = image::imageops::crop_imm(
            &decoded,
            0,
            0,
            crop_width,
            decoded.height(),
        )
        .to_image();
        let cropped_bytes = encode_png(cropped)?;
        captures.push(persist_capture(
            workbook,
            id,
            sheet,
            &capture_address(decision.keep_column, bottom_row),
            &cropped_bytes,
            image_root,
            json!({
                "rust_xml_base_right_column": base_right_column,
                "overflow_checked": true,
                "overflow_capture_end_column": end_column,
                "overflow_keep_column": decision.keep_column,
                "overflow_probe_extra": probe_extra,
                "overflow_scanned_pixels": decision.scanned_pixels
            }),
        )?);
    }

    Ok(json!({
        "captures": captures,
        "pending": pending,
        "analysis_ms": started.elapsed().as_secs_f64() * 1000.0,
        "scanned_pixels": scanned_pixels
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            <row r="105"><c r="A105"><v>1</v></c></row>
            <row r="106" hidden="1"/><row r="107" hidden="1"/><row r="108" hidden="1"/>
        </sheetData></worksheet>"#;
        let summary = scan_sheet(xml).unwrap();
        assert_eq!(summary.trailing_hidden_row_start, Some(106));
        assert_eq!(summary.max_row, 105);
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
    fn visible_probe_budget_counts_visible_columns_not_sheet_indices() {
        let mut widths = vec![10.0; MAX_COLUMNS as usize];
        widths[1] = 0.0;
        widths[2] = 0.0;
        assert_eq!(end_column_for_visible_count(&widths, 0, 2), 4);
    }
}
