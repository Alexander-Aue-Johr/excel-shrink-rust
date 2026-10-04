use crate::visual_xml::{Kind, Reader, decode_attr, decimal, truth};
use anyhow::{Context, Result, ensure};
use memchr::memmem;
use serde::Serialize;
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
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
    pub sheet_index_basis: String,
    pub bounds_evidence: BoundsEvidence,
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

// Witness references explain an envelope without exporting cell contents or changing policy.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct BoundsEvidence {
    dimension_ref: Option<String>,
    content_cells: usize,
    cells_without_content: usize,
    merge_count: usize,
    rightmost_content: Option<BoundsWitness>,
    bottommost_content: Option<BoundsWitness>,
    rightmost_merge: Option<BoundsWitness>,
    bottommost_merge: Option<BoundsWitness>,
}
#[derive(Debug, Clone, Serialize)]
struct BoundsWitness {
    reference: String,
    row_1based: u32,
    column_1based: u32,
    kind: String,
}
fn witness(slot: &mut Option<BoundsWitness>, coordinate: Coordinate, reference: &str, kind: &str, horizontal: bool) {
    let previous = slot.as_ref().map(|w| if horizontal { w.column_1based } else { w.row_1based }).unwrap_or(0);
    let current = if horizontal { coordinate.column } else { coordinate.row };
    if current > previous {
        *slot = Some(BoundsWitness { reference: reference.to_owned(), row_1based: coordinate.row,
            column_1based: coordinate.column, kind: kind.to_owned() });
    }
}
fn coordinate_reference(coordinate: Coordinate) -> String {
    let mut n=coordinate.column; let mut letters=Vec::new();
    while n>0 { n-=1; letters.push((b'A'+(n%26) as u8) as char); n/=26; }
    format!("{}{}", letters.into_iter().rev().collect::<String>(), coordinate.row)
}
#[derive(Debug, Default)]
struct SheetXmlSummary {
    evidence: BoundsEvidence,
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
                index: workbook_index,
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
    summary.evidence.merge_count += 1;
    let reference_text = decode_attr(reference)?;
    for cell in reference.split(|byte| *byte == b':') {
        let coordinate = Coordinate::parse(cell)?;
        witness(&mut summary.evidence.rightmost_merge, coordinate, &reference_text, "merge", true);
        witness(&mut summary.evidence.bottommost_merge, coordinate, &reference_text, "merge", false);
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

        if tag.depth == 1 && local == b"dimension" {
            summary.evidence.dimension_ref = tag.attr(b"ref")?.map(decode_attr).transpose()?;
            let _ = reader.element(tag)?;
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
                // Visual-tail heuristic requested by the experiment: once an
                // explicit hidden row starts a trailing suffix, omitted row
                // definitions do not cancel it. A later explicit visible row
                // does cancel it and may start a new suffix further down.
                if summary.trailing_hidden_row_start.is_none() {
                    summary.trailing_hidden_row_start = Some(current_row);
                }
            } else {
                summary.trailing_hidden_row_start = None;
            }
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
                summary.evidence.content_cells += 1;
                let reference_text = coordinate_reference(coordinate);
                let kind = String::from_utf8_lossy(cell_type);
                witness(&mut summary.evidence.rightmost_content, coordinate, &reference_text, &kind, true);
                witness(&mut summary.evidence.bottommost_content, coordinate, &reference_text, &kind, false);
                summary.max_row = summary.max_row.max(coordinate.row);
                summary.max_column = summary.max_column.max(coordinate.column);
                summary.has_text |= inline_text
                    || matches!(cell_type, b"s" | b"inlineStr" | b"str");
            } else { summary.evidence.cells_without_content += 1; }
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

    for descriptor in descriptors {
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
            None => (natural_right, "wsl-rust-xml-content-drawing".to_string()),
        };
        let (bottom_row, bottom_bound_source) = match trailing_row {
            Some(start) => (start.saturating_sub(1), "trailing-hidden".to_string()),
            None => (natural_bottom, "wsl-rust-xml-content-drawing".to_string()),
        };

        sheets.push(SheetVisualPlan {
            sheet: descriptor.name,
            sheet_index: descriptor.index,
            sheet_index_basis: "workbook-sheet-order-1-based".to_owned(),
            bounds_evidence: xml.evidence,
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
    fn diagnostic_witnesses_explain_bounds_without_using_dimension() {
        let xml = br#"<worksheet><dimension ref="A1:XFD1048576"/><sheetData>
            <row r="2"><c r="C2" t="s"><v>0</v></c><c r="ZZ2" s="3"/></row>
            <row r="9"><c r="B9"><v>42</v></c></row>
        </sheetData><mergeCells><mergeCell ref="A10:K12"/></mergeCells></worksheet>"#;
        let summary = scan_sheet(xml).unwrap();
        assert_eq!((summary.max_row, summary.max_column), (12, 11));
        assert_eq!(summary.evidence.dimension_ref.as_deref(), Some("A1:XFD1048576"));
        assert_eq!(summary.evidence.content_cells, 2);
        assert_eq!(summary.evidence.cells_without_content, 1);
        assert_eq!(summary.evidence.merge_count, 1);
        assert_eq!(summary.evidence.rightmost_content.unwrap().reference, "C2");
        assert_eq!(summary.evidence.bottommost_content.unwrap().reference, "B9");
        assert_eq!(summary.evidence.rightmost_merge.unwrap().reference, "A10:K12");
    }

    #[test]
    fn witness_payload_never_contains_cell_text_or_formula_text() {
        let xml = br#"<worksheet><sheetData><row r="1">
            <c r="A1" t="inlineStr"><is><t>PRIVATE-CELL-TEXT</t></is></c>
            <c r="B1"><f>PRIVATE-FORMULA</f></c>
        </row></sheetData></worksheet>"#;
        let summary = scan_sheet(xml).unwrap();
        let json = serde_json::to_string(&summary.evidence).unwrap();
        assert!(!json.contains("PRIVATE"));
        assert_eq!(summary.evidence.content_cells, 2);
    }

    #[test]
    fn worksheet_indices_retain_hidden_sheet_positions() {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
        writer.start_file("xl/workbook.xml", SimpleFileOptions::default()).unwrap();
        writer.write_all(br#"<workbook xmlns:r="r"><sheets>
            <sheet name="First" sheetId="4" r:id="rId1"/>
            <sheet name="Hidden" sheetId="9" state="hidden" r:id="rId2"/>
            <sheet name=" Third" sheetId="20" r:id="rId3"/>
        </sheets></workbook>"#).unwrap();
        writer.start_file("xl/_rels/workbook.xml.rels", SimpleFileOptions::default()).unwrap();
        writer.write_all(br#"<Relationships>
            <Relationship Id="rId1" Target="worksheets/sheet1.xml"/>
            <Relationship Id="rId2" Target="worksheets/sheet2.xml"/>
            <Relationship Id="rId3" Target="worksheets/sheet3.xml"/>
        </Relationships>"#).unwrap();
        let buffer = writer.finish().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buffer.into_inner())).unwrap();
        let descriptors = workbook_sheets(&mut archive).unwrap();
        assert_eq!(descriptors.len(), 2);
        assert_eq!(descriptors[0].index, 1);
        assert_eq!(descriptors[1].index, 3);
        assert_eq!(descriptors[1].name, " Third");
    }

}
