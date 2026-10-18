// Retained only for differential tests and benchmarks; not compiled into release builds.
use memchr::memmem;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;
use zip::ZipArchive;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingCellPolicy {
    pub allow_whitespace_values: bool,
    pub allow_hidden_rows_or_columns: bool,
}

impl MissingCellPolicy {
    pub fn strict() -> Self {
        Self {
            allow_whitespace_values: false,
            allow_hidden_rows_or_columns: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct CellValueDifference {
    pub sheet_file: String,
    pub cell_ref: String,
    pub kind: CellDifferenceKind,
    pub left_value: Option<String>,
    pub right_value: Option<String>,
}

impl CellValueDifference {
    fn missing_right(sheet_file: &str, cell_ref: &[u8], left_value: &CellValue) -> Self {
        Self {
            sheet_file: sheet_file.to_string(),
            cell_ref: bytes_to_lossy_string(cell_ref),
            kind: CellDifferenceKind::MissingRight,
            left_value: Some(left_value.to_report_string()),
            right_value: None,
        }
    }

    fn extra_right(sheet_file: &str, cell_ref: &[u8], right_value: &CellValue) -> Self {
        Self {
            sheet_file: sheet_file.to_string(),
            cell_ref: bytes_to_lossy_string(cell_ref),
            kind: CellDifferenceKind::ExtraRight,
            left_value: None,
            right_value: Some(right_value.to_report_string()),
        }
    }

    fn changed(
        sheet_file: &str,
        cell_ref: &[u8],
        left_value: &CellValue,
        right_value: &CellValue,
    ) -> Self {
        Self {
            sheet_file: sheet_file.to_string(),
            cell_ref: bytes_to_lossy_string(cell_ref),
            kind: CellDifferenceKind::Changed,
            left_value: Some(left_value.to_report_string()),
            right_value: Some(right_value.to_report_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellDifferenceKind {
    Changed,
    MissingRight,
    ExtraRight,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkbookComparisonReport {
    pub compared_sheets: usize,
    pub difference_count: usize,
    pub differences: Vec<CellValueDifference>,
    pub right_capture_ranges: Vec<WorksheetCaptureRange>,
    pub left_capture_ranges: Vec<WorksheetCaptureRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorksheetCaptureRange {
    pub sheet_index: usize,
    pub sheet_file: String,
    pub max_cell: String,
    pub max_row: u32,
    pub max_column: u32,
    pub max_row_source: Option<String>,
    pub max_column_source: Option<String>,
}

#[derive(Debug)]
struct WorkbookCells {
    sheet_order: Vec<String>,
    sheets: HashMap<String, SheetCells>,
}

#[derive(Debug)]
struct SheetCells {
    cells: HashMap<Vec<u8>, CellValue>,
    hidden_rows: HashSet<Vec<u8>>,
    hidden_columns: Vec<ColumnRange>,
    capture_bounds: SheetCaptureBounds,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CellValue {
    Empty,
    Raw(Vec<u8>),
    SharedString(Vec<u8>),
    InlineString(Vec<u8>),
}

#[derive(Debug, Clone, Copy)]
struct ColumnRange {
    min: u32,
    max: u32,
}

#[derive(Debug, Clone)]
struct SheetCaptureBounds {
    max_column: u32,
    max_row: u32,
    max_column_source: Option<Vec<u8>>,
    max_row_source: Option<Vec<u8>>,
}

pub fn compare_workbook_values(
    left_path: &Path,
    right_path: &Path,
    missing_policy: MissingCellPolicy,
    only_sheets: Option<&HashSet<String>>,
) -> anyhow::Result<WorkbookComparisonReport> {
    let left = read_workbook_cells(left_path)?;
    let right = read_workbook_cells(right_path)?;
    Ok(compare_workbooks(
        &left,
        &right,
        missing_policy,
        only_sheets,
    ))
}

fn compare_workbooks(
    left: &WorkbookCells,
    right: &WorkbookCells,
    missing_policy: MissingCellPolicy,
    only_sheets: Option<&HashSet<String>>,
) -> WorkbookComparisonReport {
    let _phase = super::visual_diagnostics::phase("values.compare_cells", "in-memory workbook comparison");
    let mut differences = Vec::new();
    let sheet_names = compared_sheet_names(left, right, only_sheets);

    for sheet_file in &sheet_names {
        compare_workbook_sheet(
            sheet_file,
            left.sheets.get(sheet_file),
            right.sheets.get(sheet_file),
            missing_policy,
            &mut differences,
        );
    }

    WorkbookComparisonReport {
        compared_sheets: sheet_names.len(),
        difference_count: differences.len(),
        differences,
        right_capture_ranges: workbook_capture_ranges(right, only_sheets),
        left_capture_ranges: workbook_capture_ranges(left, only_sheets),
    }
}

fn compared_sheet_names(
    left: &WorkbookCells,
    right: &WorkbookCells,
    only_sheets: Option<&HashSet<String>>,
) -> Vec<String> {
    let mut sheet_names: Vec<_> = left
        .sheets
        .keys()
        .chain(right.sheets.keys())
        .filter(|sheet| only_sheets.map_or(true, |only| only.contains(*sheet)))
        .cloned()
        .collect();
    sheet_names.sort();
    sheet_names.dedup();
    sheet_names
}

fn compare_workbook_sheet(
    sheet_file: &str,
    left_sheet: Option<&SheetCells>,
    right_sheet: Option<&SheetCells>,
    missing_policy: MissingCellPolicy,
    differences: &mut Vec<CellValueDifference>,
) {
    match (left_sheet, right_sheet) {
        (Some(left_sheet), Some(right_sheet)) => compare_sheets(
            sheet_file,
            left_sheet,
            right_sheet,
            missing_policy,
            differences,
        ),
        (Some(left_sheet), None) => {
            append_missing_sheet_differences(sheet_file, left_sheet, missing_policy, differences);
        }
        (None, Some(right_sheet)) => {
            append_extra_sheet_differences(sheet_file, right_sheet, differences);
        }
        (None, None) => {}
    }
}

fn append_missing_sheet_differences(
    sheet_file: &str,
    left_sheet: &SheetCells,
    missing_policy: MissingCellPolicy,
    differences: &mut Vec<CellValueDifference>,
) {
    for (cell_ref, value) in &left_sheet.cells {
        if !missing_right_cell_is_allowed(left_sheet, cell_ref, value, missing_policy) {
            differences.push(CellValueDifference::missing_right(
                sheet_file, cell_ref, value,
            ));
        }
    }
}

fn append_extra_sheet_differences(
    sheet_file: &str,
    right_sheet: &SheetCells,
    differences: &mut Vec<CellValueDifference>,
) {
    for (cell_ref, value) in &right_sheet.cells {
        if !value.is_empty() {
            differences.push(CellValueDifference::extra_right(
                sheet_file, cell_ref, value,
            ));
        }
    }
}

fn compare_sheets(
    sheet_file: &str,
    left_sheet: &SheetCells,
    right_sheet: &SheetCells,
    missing_policy: MissingCellPolicy,
    differences: &mut Vec<CellValueDifference>,
) {
    for cell_ref in compared_cell_refs(left_sheet, right_sheet) {
        if let Some(difference) = compare_cell_values(
            sheet_file,
            &cell_ref,
            left_sheet,
            right_sheet,
            missing_policy,
        ) {
            differences.push(difference);
        }
    }
}

fn compared_cell_refs(left_sheet: &SheetCells, right_sheet: &SheetCells) -> Vec<Vec<u8>> {
    let mut cell_refs: Vec<_> = left_sheet
        .cells
        .keys()
        .chain(right_sheet.cells.keys())
        .cloned()
        .collect();
    cell_refs.sort();
    cell_refs.dedup();
    cell_refs
}

fn compare_cell_values(
    sheet_file: &str,
    cell_ref: &[u8],
    left_sheet: &SheetCells,
    right_sheet: &SheetCells,
    missing_policy: MissingCellPolicy,
) -> Option<CellValueDifference> {
    let left_value = left_sheet.cells.get(cell_ref).unwrap_or(&CellValue::Empty);
    let right_value = right_sheet.cells.get(cell_ref).unwrap_or(&CellValue::Empty);

    if left_value == right_value {
        return None;
    }

    if !left_value.is_empty() && right_value.is_empty() {
        return (!missing_right_cell_is_allowed(left_sheet, cell_ref, left_value, missing_policy))
            .then(|| CellValueDifference::missing_right(sheet_file, cell_ref, left_value));
    }

    if left_value.is_empty() {
        return Some(CellValueDifference::extra_right(
            sheet_file,
            cell_ref,
            right_value,
        ));
    }

    Some(CellValueDifference::changed(
        sheet_file,
        cell_ref,
        left_value,
        right_value,
    ))
}

#[inline]
fn missing_right_cell_is_allowed(
    left_sheet: &SheetCells,
    cell_ref: &[u8],
    left_value: &CellValue,
    policy: MissingCellPolicy,
) -> bool {
    if left_value.is_empty() {
        return true;
    }
    if policy.allow_whitespace_values && left_value.is_whitespace() {
        return true;
    }
    policy.allow_hidden_rows_or_columns && left_sheet.cell_is_hidden(cell_ref)
}

fn read_workbook_cells(path: &Path) -> anyhow::Result<WorkbookCells> {
    let _phase = super::visual_diagnostics::phase("values.read_workbook", path.display().to_string());
    let file_bytes = read_file_bytes(path)?;
    let mut archive = ZipArchive::new(Cursor::new(file_bytes.as_slice()))?;
    let sheet_order = read_workbook_sheet_order(&mut archive)?;
    let shared_strings = read_shared_strings(&mut archive)?;
    let sheets = read_worksheet_cells(&mut archive, &shared_strings)?;

    Ok(WorkbookCells {
        sheet_order,
        sheets,
    })
}

fn read_file_bytes(path: &Path) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_worksheet_cells(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    shared_strings: &[Vec<u8>],
) -> anyhow::Result<HashMap<String, SheetCells>> {
    let mut sheets = HashMap::new();

    for index in 0..archive.len() {
        if let Some((name, sheet)) = read_worksheet_cells_at(archive, index, shared_strings)? {
            sheets.insert(name, sheet);
        }
    }

    Ok(sheets)
}

fn read_worksheet_cells_at(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    index: usize,
    shared_strings: &[Vec<u8>],
) -> anyhow::Result<Option<(String, SheetCells)>> {
    let mut file = archive.by_index(index)?;
    let name = file.name().to_string();
    if !is_worksheet_xml(&name) {
        return Ok(None);
    }

    let mut xml = Vec::new();
    file.read_to_end(&mut xml)?;
    Ok(Some((name, parse_sheet_cells(&xml, shared_strings))))
}

fn workbook_capture_ranges(
    workbook: &WorkbookCells,
    only_sheets: Option<&HashSet<String>>,
) -> Vec<WorksheetCaptureRange> {
    ordered_sheet_files(workbook)
        .into_iter()
        .enumerate()
        .filter(|(_, sheet_file)| only_sheets.map_or(true, |only| only.contains(sheet_file)))
        .filter_map(|(index, sheet_file)| {
            workbook
                .sheets
                .get(&sheet_file)
                .map(|sheet| sheet.capture_bounds.to_capture_range(index + 1, sheet_file))
        })
        .collect()
}

fn ordered_sheet_files(workbook: &WorkbookCells) -> Vec<String> {
    let mut ordered = if workbook.sheet_order.is_empty() {
        let mut sheet_files = workbook.sheets.keys().cloned().collect::<Vec<_>>();
        sheet_files.sort_by_key(|name| worksheet_sort_key(name));
        sheet_files
    } else {
        workbook.sheet_order.clone()
    };

    for sheet_file in workbook.sheets.keys() {
        if !ordered.contains(sheet_file) {
            ordered.push(sheet_file.clone());
        }
    }

    ordered
}

fn read_workbook_sheet_order(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
) -> anyhow::Result<Vec<String>> {
    let Some(workbook_xml) = read_optional_zip_entry(archive, "xl/workbook.xml")? else {
        return Ok(Vec::new());
    };

    let Some(rels_xml) = read_optional_zip_entry(archive, "xl/_rels/workbook.xml.rels")? else {
        return Ok(Vec::new());
    };

    let relationship_targets = parse_workbook_relationship_targets(&rels_xml);
    Ok(parse_workbook_sheet_relationship_ids(&workbook_xml)
        .into_iter()
        .filter_map(|relationship_id| relationship_targets.get(&relationship_id).cloned())
        .filter(|target| is_worksheet_xml(target))
        .collect())
}

fn read_optional_zip_entry(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    let Ok(mut file) = archive.by_name(name) else {
        return Ok(None);
    };

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn parse_workbook_sheet_relationship_ids(workbook_xml: &[u8]) -> Vec<Vec<u8>> {
    let mut ids = Vec::new();
    let mut pos = 0;

    while let Some(sheet_start_rel) = find_bytes(&workbook_xml[pos..], b"<sheet") {
        let sheet_start = pos + sheet_start_rel;
        if !is_tag_start(workbook_xml, sheet_start, b"<sheet") {
            pos = sheet_start + b"<sheet".len();
            continue;
        }

        let Some(sheet_tag_end) = find_byte_from(workbook_xml, b'>', sheet_start) else {
            break;
        };
        let sheet_tag = &workbook_xml[sheet_start..sheet_tag_end];
        if let Some(relationship_id) = extract_attr_value(sheet_tag, b"r:id") {
            ids.push(relationship_id.to_vec());
        }
        pos = sheet_tag_end + 1;
    }

    ids
}

fn parse_workbook_relationship_targets(rels_xml: &[u8]) -> HashMap<Vec<u8>, String> {
    let mut targets = HashMap::new();
    let mut pos = 0;

    while let Some(rel_start_rel) = find_bytes(&rels_xml[pos..], b"<Relationship") {
        let rel_start = pos + rel_start_rel;
        if !is_tag_start(rels_xml, rel_start, b"<Relationship") {
            pos = rel_start + b"<Relationship".len();
            continue;
        }

        let Some(rel_tag_end) = find_byte_from(rels_xml, b'>', rel_start) else {
            break;
        };
        let rel_tag = &rels_xml[rel_start..rel_tag_end];
        if let (Some(id), Some(target)) = (
            extract_attr_value(rel_tag, b"Id"),
            extract_attr_value(rel_tag, b"Target"),
        ) {
            if let Some(zip_name) = normalize_workbook_relationship_target(target) {
                targets.insert(id.to_vec(), zip_name);
            }
        }
        pos = rel_tag_end + 1;
    }

    targets
}

fn normalize_workbook_relationship_target(target: &[u8]) -> Option<String> {
    let target = bytes_to_lossy_string(target).replace('\\', "/");
    let raw_path = if let Some(path) = target.strip_prefix('/') {
        path.to_string()
    } else {
        format!("xl/{target}")
    };

    normalize_zip_path(&raw_path)
}

fn normalize_zip_path(path: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

fn parse_merge_cell_refs(sheet_xml: &[u8]) -> Vec<Vec<u8>> {
    let mut refs = Vec::new();
    let mut pos = 0;

    while let Some(merge_start_rel) = find_bytes(&sheet_xml[pos..], b"<mergeCell") {
        let merge_start = pos + merge_start_rel;
        if !is_tag_start(sheet_xml, merge_start, b"<mergeCell") {
            pos = merge_start + b"<mergeCell".len();
            continue;
        }

        let Some(merge_tag_end) = find_byte_from(sheet_xml, b'>', merge_start) else {
            break;
        };
        let merge_tag = &sheet_xml[merge_start..merge_tag_end];
        if let Some(range_ref) = extract_attr_value(merge_tag, b"ref") {
            refs.push(range_ref.to_vec());
        }
        pos = merge_tag_end + 1;
    }

    refs
}

fn cell_has_formula(cell: &[u8]) -> bool {
    let mut pos = 0;
    while let Some(formula_start_rel) = find_bytes(&cell[pos..], b"<f") {
        let formula_start = pos + formula_start_rel;
        if is_tag_start(cell, formula_start, b"<f") {
            return true;
        }
        pos = formula_start + b"<f".len();
    }
    false
}

impl SheetCaptureBounds {
    fn new() -> Self {
        Self {
            max_column: 1,
            max_row: 1,
            max_column_source: None,
            max_row_source: None,
        }
    }

    fn include_cell_ref(&mut self, cell_ref: &[u8]) {
        if let Some((column, row)) = parse_cell_reference_bounds(cell_ref) {
            self.include_bound(column, row, cell_ref);
        }
    }

    fn include_range_ref(&mut self, range_ref: &[u8]) {
        if let Some((column, row)) = parse_cell_range_max_bounds(range_ref) {
            self.include_bound(column, row, range_ref);
        }
    }

    fn include_bound(&mut self, column: u32, row: u32, source: &[u8]) {
        if column > self.max_column {
            self.max_column = column;
            self.max_column_source = Some(source.to_vec());
        }
        if row > self.max_row {
            self.max_row = row;
            self.max_row_source = Some(source.to_vec());
        }
    }

    fn to_capture_range(&self, sheet_index: usize, sheet_file: String) -> WorksheetCaptureRange {
        WorksheetCaptureRange {
            sheet_index,
            sheet_file,
            max_cell: format!(
                "{}{}",
                column_number_to_letters(self.max_column),
                self.max_row
            ),
            max_row: self.max_row,
            max_column: self.max_column,
            max_row_source: self.max_row_source.as_deref().map(bytes_to_lossy_string),
            max_column_source: self.max_column_source.as_deref().map(bytes_to_lossy_string),
        }
    }
}

fn read_shared_strings(archive: &mut ZipArchive<Cursor<&[u8]>>) -> anyhow::Result<Vec<Vec<u8>>> {
    let Some(xml) = read_optional_zip_entry(archive, "xl/sharedStrings.xml")? else {
        return Ok(Vec::new());
    };

    Ok(parse_shared_strings(&xml))
}

fn parse_shared_strings(xml: &[u8]) -> Vec<Vec<u8>> {
    let mut values = Vec::new();
    let mut pos = 0;

    while let Some(si_start_rel) = find_bytes(&xml[pos..], b"<si") {
        let si_start = pos + si_start_rel;
        if !is_tag_start(xml, si_start, b"<si") {
            pos = si_start + b"<si".len();
            continue;
        }

        let Some(si_tag_end) = find_byte_from(xml, b'>', si_start) else {
            break;
        };
        let si_end = if si_tag_end > si_start && xml[si_tag_end - 1] == b'/' {
            si_tag_end + 1
        } else {
            let content_start = si_tag_end + 1;
            let Some(close_rel) = find_bytes(&xml[content_start..], b"</si>") else {
                break;
            };
            content_start + close_rel + b"</si>".len()
        };

        values.push(extract_text_runs(&xml[si_start..si_end]));
        pos = si_end;
    }

    values
}

fn parse_sheet_cells(sheet_xml: &[u8], shared_strings: &[Vec<u8>]) -> SheetCells {
    let (cells, mut capture_bounds) = parse_cells(sheet_xml, shared_strings);
    for merge_ref in parse_merge_cell_refs(sheet_xml) {
        capture_bounds.include_range_ref(&merge_ref);
    }

    SheetCells {
        cells,
        hidden_rows: parse_hidden_rows(sheet_xml),
        hidden_columns: parse_hidden_columns(sheet_xml),
        capture_bounds,
    }
}

fn parse_cells(
    sheet_xml: &[u8],
    shared_strings: &[Vec<u8>],
) -> (HashMap<Vec<u8>, CellValue>, SheetCaptureBounds) {
    let mut cells = HashMap::new();
    let mut capture_bounds = SheetCaptureBounds::new();
    let mut pos = 0;

    while let Some(cell_start_rel) = find_bytes(&sheet_xml[pos..], b"<c") {
        let cell_start = pos + cell_start_rel;
        if !is_tag_start(sheet_xml, cell_start, b"<c") {
            pos = cell_start + b"<c".len();
            continue;
        }

        let Some(cell_tag_end) = find_byte_from(sheet_xml, b'>', cell_start) else {
            break;
        };
        let cell_tag = &sheet_xml[cell_start..cell_tag_end];
        let Some(cell_ref) = extract_attr_value(cell_tag, b"r") else {
            pos = cell_tag_end + 1;
            continue;
        };

        let cell_end = if cell_tag_end > cell_start && sheet_xml[cell_tag_end - 1] == b'/' {
            cell_tag_end + 1
        } else {
            let content_start = cell_tag_end + 1;
            let Some(close_rel) = find_bytes(&sheet_xml[content_start..], b"</c>") else {
                break;
            };
            content_start + close_rel + b"</c>".len()
        };

        let cell = &sheet_xml[cell_start..cell_end];
        let cell_value = extract_cell_value(cell, cell_tag, shared_strings);
        if cell_value.has_content() || cell_has_formula(cell) {
            capture_bounds.include_cell_ref(cell_ref);
        }
        cells.insert(cell_ref.to_vec(), cell_value);
        pos = cell_end;
    }

    (cells, capture_bounds)
}

fn extract_cell_value(cell: &[u8], cell_tag: &[u8], shared_strings: &[Vec<u8>]) -> CellValue {
    let cell_type = extract_attr_value(cell_tag, b"t");

    if cell_type == Some(b"inlineStr".as_slice()) {
        return extract_xml_block(cell, b"<is", b"</is>")
            .map(CellValue::InlineString)
            .unwrap_or(CellValue::Empty);
    }

    let Some(value) = extract_xml_block(cell, b"<v", b"</v>") else {
        return CellValue::Empty;
    };

    if cell_type == Some(b"s".as_slice()) {
        return parse_usize(&value)
            .and_then(|index| shared_strings.get(index).cloned())
            .map(CellValue::SharedString)
            .unwrap_or(CellValue::Raw(value));
    }

    CellValue::Raw(value)
}

fn parse_hidden_rows(sheet_xml: &[u8]) -> HashSet<Vec<u8>> {
    let mut rows = HashSet::new();
    let mut pos = 0;

    while let Some(row_start_rel) = find_bytes(&sheet_xml[pos..], b"<row") {
        let row_start = pos + row_start_rel;
        if !is_tag_start(sheet_xml, row_start, b"<row") {
            pos = row_start + b"<row".len();
            continue;
        }

        let Some(row_tag_end) = find_byte_from(sheet_xml, b'>', row_start) else {
            break;
        };
        let row_tag = &sheet_xml[row_start..row_tag_end];
        if attr_is_true(row_tag, b"hidden") {
            if let Some(row_ref) = extract_attr_value(row_tag, b"r") {
                rows.insert(row_ref.to_vec());
            }
        }
        pos = row_tag_end + 1;
    }

    rows
}

fn parse_hidden_columns(sheet_xml: &[u8]) -> Vec<ColumnRange> {
    let mut ranges = Vec::new();
    let mut pos = 0;

    while let Some(col_start_rel) = find_bytes(&sheet_xml[pos..], b"<col") {
        let col_start = pos + col_start_rel;
        if !is_tag_start(sheet_xml, col_start, b"<col") {
            pos = col_start + b"<col".len();
            continue;
        }

        let Some(col_tag_end) = find_byte_from(sheet_xml, b'>', col_start) else {
            break;
        };
        let col_tag = &sheet_xml[col_start..col_tag_end];
        if attr_is_true(col_tag, b"hidden") {
            let min = extract_attr_value(col_tag, b"min").and_then(parse_u32);
            let max = extract_attr_value(col_tag, b"max").and_then(parse_u32);
            if let (Some(min), Some(max)) = (min, max) {
                ranges.push(ColumnRange { min, max });
            }
        }
        pos = col_tag_end + 1;
    }

    ranges
}

fn extract_xml_block(cell: &[u8], start_tag: &[u8], end_tag: &[u8]) -> Option<Vec<u8>> {
    let start = find_bytes(cell, start_tag)?;
    if !is_tag_start(cell, start, start_tag) {
        return None;
    }
    let open_end = find_byte_from(cell, b'>', start)? + 1;
    let close_rel = find_bytes(&cell[open_end..], end_tag)?;
    Some(cell[open_end..open_end + close_rel].to_vec())
}

fn extract_text_runs(xml: &[u8]) -> Vec<u8> {
    let mut value = Vec::new();
    let mut pos = 0;

    while let Some(text_start_rel) = find_bytes(&xml[pos..], b"<t") {
        let text_start = pos + text_start_rel;
        if !is_tag_start(xml, text_start, b"<t") {
            pos = text_start + b"<t".len();
            continue;
        }

        let Some(text_tag_end) = find_byte_from(xml, b'>', text_start) else {
            break;
        };
        if text_tag_end > text_start && xml[text_tag_end - 1] == b'/' {
            pos = text_tag_end + 1;
            continue;
        }

        let content_start = text_tag_end + 1;
        let Some(close_rel) = find_bytes(&xml[content_start..], b"</t>") else {
            break;
        };
        value.extend_from_slice(&xml[content_start..content_start + close_rel]);
        pos = content_start + close_rel + b"</t>".len();
    }

    value
}

impl SheetCells {
    fn cell_is_hidden(&self, cell_ref: &[u8]) -> bool {
        let Some(split) = cell_ref.iter().position(|byte| byte.is_ascii_digit()) else {
            return false;
        };
        if split == 0 || split >= cell_ref.len() {
            return false;
        }

        if self.hidden_rows.contains(&cell_ref[split..]) {
            return true;
        }

        let Some(column_index) = column_letters_to_number(&cell_ref[..split]) else {
            return false;
        };
        self.hidden_columns
            .iter()
            .any(|range| range.min <= column_index && column_index <= range.max)
    }
}

impl CellValue {
    fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    fn has_content(&self) -> bool {
        match self {
            Self::Empty => false,
            Self::Raw(value) | Self::SharedString(value) | Self::InlineString(value) => {
                !value.is_empty()
            }
        }
    }

    fn is_whitespace(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Raw(value) | Self::SharedString(value) | Self::InlineString(value) => value
                .iter()
                .all(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r')),
        }
    }

    fn to_report_string(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Raw(value) | Self::SharedString(value) | Self::InlineString(value) => {
                bytes_to_lossy_string(value)
            }
        }
    }
}

fn attr_is_true(tag: &[u8], attr: &[u8]) -> bool {
    extract_attr_value(tag, attr)
        .is_some_and(|value| value == b"1" || value.eq_ignore_ascii_case(b"true"))
}

fn extract_attr_value<'a>(tag: &'a [u8], attr: &[u8]) -> Option<&'a [u8]> {
    let mut needle = Vec::with_capacity(attr.len() + 2);
    needle.extend_from_slice(attr);
    needle.extend_from_slice(b"=\"");

    let mut pos = 0;
    while let Some(found_rel) = find_bytes(&tag[pos..], &needle) {
        let found = pos + found_rel;
        if found > 0 && !tag[found - 1].is_ascii_whitespace() {
            pos = found + 1;
            continue;
        }

        let value_start = found + needle.len();
        let value_end_rel = tag[value_start..].iter().position(|byte| *byte == b'"')?;
        return Some(&tag[value_start..value_start + value_end_rel]);
    }

    None
}

fn parse_u32(bytes: &[u8]) -> Option<u32> {
    let mut value = 0u32;
    if bytes.is_empty() {
        return None;
    }
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(u32::from(byte - b'0'));
    }
    Some(value)
}

fn parse_usize(bytes: &[u8]) -> Option<usize> {
    let mut value = 0usize;
    if bytes.is_empty() {
        return None;
    }
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(usize::from(byte - b'0'));
    }
    Some(value)
}

fn column_letters_to_number(bytes: &[u8]) -> Option<u32> {
    let mut value = 0u32;
    for byte in bytes {
        let upper = byte.to_ascii_uppercase();
        if !upper.is_ascii_uppercase() {
            return None;
        }
        value = value
            .saturating_mul(26)
            .saturating_add(u32::from(upper - b'A' + 1));
    }
    Some(value)
}

fn column_number_to_letters(mut index: u32) -> String {
    if index == 0 {
        return "A".to_string();
    }

    let mut letters = Vec::new();
    while index > 0 {
        index -= 1;
        letters.push(b'A' + (index % 26) as u8);
        index /= 26;
    }
    letters.reverse();
    String::from_utf8(letters).unwrap_or_else(|_| "A".to_string())
}

fn parse_cell_reference_bounds(cell_ref: &[u8]) -> Option<(u32, u32)> {
    let split = cell_ref.iter().position(|byte| byte.is_ascii_digit())?;
    if split == 0 || split >= cell_ref.len() {
        return None;
    }

    let column = column_letters_to_number(&cell_ref[..split])?;
    let row = parse_u32(&cell_ref[split..])?;
    Some((column, row))
}

fn parse_cell_range_max_bounds(range_ref: &[u8]) -> Option<(u32, u32)> {
    let mut max_column = 0;
    let mut max_row = 0;

    for cell_ref in range_ref.split(|byte| *byte == b':') {
        let (column, row) = parse_cell_reference_bounds(cell_ref)?;
        max_column = max_column.max(column);
        max_row = max_row.max(row);
    }

    if max_column == 0 || max_row == 0 {
        None
    } else {
        Some((max_column, max_row))
    }
}

fn worksheet_sort_key(name: &str) -> (u32, String) {
    let number = name
        .strip_prefix("xl/worksheets/sheet")
        .and_then(|rest| rest.strip_suffix(".xml"))
        .and_then(|number| number.parse::<u32>().ok())
        .unwrap_or(u32::MAX);
    (number, name.to_string())
}

fn is_worksheet_xml(name: &str) -> bool {
    name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml")
}

fn is_tag_start(data: &[u8], tag_start: usize, tag_name: &[u8]) -> bool {
    matches!(
        data.get(tag_start + tag_name.len()),
        Some(byte) if byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>')
    )
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    memmem::find(haystack, needle)
}

fn find_byte_from(haystack: &[u8], needle: u8, start: usize) -> Option<usize> {
    haystack[start..]
        .iter()
        .position(|byte| *byte == needle)
        .map(|offset| start + offset)
}

fn bytes_to_lossy_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shared_string_cell_values_by_text_not_index() {
        let sst = br#"<sst><si><t>Hello</t></si><si><r><t>A</t></r><r><t>B</t></r></si></sst>"#;
        let sheet = br#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>1</v></c></row></sheetData></worksheet>"#;
        let shared = parse_shared_strings(sst);
        let parsed = parse_sheet_cells(sheet, &shared);

        assert_eq!(
            parsed.cells.get(b"A1".as_slice()),
            Some(&CellValue::SharedString(b"AB".to_vec()))
        );
    }

    #[test]
    fn missing_whitespace_cell_can_be_allowed() {
        let left = SheetCells {
            cells: HashMap::from([(b"A1".to_vec(), CellValue::Raw(b"   ".to_vec()))]),
            hidden_rows: HashSet::new(),
            hidden_columns: Vec::new(),
            capture_bounds: SheetCaptureBounds::new(),
        };
        let right = SheetCells {
            cells: HashMap::new(),
            hidden_rows: HashSet::new(),
            hidden_columns: Vec::new(),
            capture_bounds: SheetCaptureBounds::new(),
        };
        let mut differences = Vec::new();

        compare_sheets(
            "xl/worksheets/sheet1.xml",
            &left,
            &right,
            MissingCellPolicy {
                allow_whitespace_values: true,
                allow_hidden_rows_or_columns: false,
            },
            &mut differences,
        );

        assert!(differences.is_empty());
    }

    #[test]
    fn missing_hidden_cell_can_be_allowed() {
        let left = SheetCells {
            cells: HashMap::from([(b"C7".to_vec(), CellValue::Raw(b"secret".to_vec()))]),
            hidden_rows: HashSet::from([b"7".to_vec()]),
            hidden_columns: Vec::new(),
            capture_bounds: SheetCaptureBounds::new(),
        };
        let right = SheetCells {
            cells: HashMap::new(),
            hidden_rows: HashSet::new(),
            hidden_columns: Vec::new(),
            capture_bounds: SheetCaptureBounds::new(),
        };
        let mut differences = Vec::new();

        compare_sheets(
            "xl/worksheets/sheet1.xml",
            &left,
            &right,
            MissingCellPolicy {
                allow_whitespace_values: false,
                allow_hidden_rows_or_columns: true,
            },
            &mut differences,
        );

        assert!(differences.is_empty());
    }

    #[test]
    fn parses_sheet_max_content_bounds_from_values_and_formulas() {
        let sheet = br#"<worksheet><sheetData>
            <row r="1"><c r="A1"><v>1</v></c><c r="ZZ1"/></row>
            <row r="42"><c r="B42"><v>1</v></c></row>
            <row r="5"><c r="AAA5"><f>SUM(A1:B42)</f></c></row>
        </sheetData></worksheet>"#;

        let parsed = parse_sheet_cells(sheet, &[]);

        assert_eq!(parsed.capture_bounds.max_column, 703);
        assert_eq!(parsed.capture_bounds.max_row, 42);
        assert_eq!(
            parsed
                .capture_bounds
                .max_column_source
                .as_deref()
                .map(bytes_to_lossy_string),
            Some("AAA5".to_string())
        );
        assert_eq!(
            parsed
                .capture_bounds
                .max_row_source
                .as_deref()
                .map(bytes_to_lossy_string),
            Some("B42".to_string())
        );
        assert_eq!(column_number_to_letters(703), "AAA");
    }

    #[test]
    fn merge_cells_expand_content_bounds() {
        let sheet = br#"<worksheet>
            <sheetData><row r="1"><c r="A1"><v>Title</v></c></row></sheetData>
            <mergeCells><mergeCell ref="A1:H24"/></mergeCells>
        </worksheet>"#;

        let parsed = parse_sheet_cells(sheet, &[]);

        assert_eq!(parsed.capture_bounds.max_column, 8);
        assert_eq!(parsed.capture_bounds.max_row, 24);
        assert_eq!(
            parsed
                .capture_bounds
                .max_column_source
                .as_deref()
                .map(bytes_to_lossy_string),
            Some("A1:H24".to_string())
        );
    }

    #[test]
    fn parses_workbook_sheet_relationship_order() {
        let workbook = br#"<workbook><sheets>
            <sheet name="Second" sheetId="2" r:id="rId2"/>
            <sheet name="First" sheetId="1" r:id="rId1"/>
        </sheets></workbook>"#;
        let rels = br#"<Relationships>
            <Relationship Id="rId1" Target="worksheets/sheet1.xml"/>
            <Relationship Id="rId2" Target="/xl/worksheets/sheet2.xml"/>
        </Relationships>"#;
        let targets = parse_workbook_relationship_targets(rels);
        let ordered = parse_workbook_sheet_relationship_ids(workbook)
            .into_iter()
            .filter_map(|id| targets.get(&id).cloned())
            .collect::<Vec<_>>();

        assert_eq!(
            ordered,
            vec![
                "xl/worksheets/sheet2.xml".to_string(),
                "xl/worksheets/sheet1.xml".to_string()
            ]
        );
    }
}
