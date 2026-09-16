use crate::models::{ProtectedCellRefs, ProtectedWorkbookRefs, SheetScanBuffer};
use crate::utils::letters_byte_string_to_integer;
use regex::bytes::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

static UNSAFE_FUNCTIONS: OnceLock<Regex> = OnceLock::new();
static FORMULA_PATTERN: OnceLock<Regex> = OnceLock::new();
static DEFINED_NAME_PATTERN: OnceLock<Regex> = OnceLock::new();
static DEFINED_NAME_WITH_ATTRS_PATTERN: OnceLock<Regex> = OnceLock::new();
static REF_PATTERN: OnceLock<Regex> = OnceLock::new();
static SQREF_PATTERN: OnceLock<Regex> = OnceLock::new();
static SQREF_WITH_TAG_PATTERN: OnceLock<Regex> = OnceLock::new();
static COORD_PATTERN: OnceLock<Regex> = OnceLock::new();
static HINTS_PATTERN: OnceLock<Regex> = OnceLock::new();
static REFERENCE_TOKEN_PATTERN: OnceLock<Regex> = OnceLock::new();

pub enum ScanResult {
    Safe,
    Unsafe(String),
}

/// Packt Spalte und Zeile in einen u64 für extrem schnelles Hashing.
#[inline]
pub fn encode_coord(col: u32, row: u32) -> u64 {
    ((col as u64) << 32) | (row as u64)
}

struct FlatScanPatterns {
    unsafe_function: &'static Regex,
    formula: &'static Regex,
    defined_name: &'static Regex,
    reference_attribute: &'static Regex,
    sequence_reference_attribute: &'static Regex,
    coordinate: &'static Regex,
}

struct SheetScanPatterns {
    unsafe_function: &'static Regex,
    formula: &'static Regex,
    defined_name: &'static Regex,
    reference_attribute: &'static Regex,
    sequence_reference_attribute: &'static Regex,
    reference_token: &'static Regex,
}

fn flat_scan_patterns() -> FlatScanPatterns {
    FlatScanPatterns {
        unsafe_function: unsafe_function_pattern(),
        formula: formula_pattern(),
        defined_name: DEFINED_NAME_PATTERN
            .get_or_init(|| Regex::new(r"(?s)<definedName\b[^>]*>(.*?)</definedName>").unwrap()),
        reference_attribute: reference_attribute_pattern(),
        sequence_reference_attribute: SQREF_PATTERN
            .get_or_init(|| Regex::new(r#"sqref="([^"]+)""#).unwrap()),
        coordinate: COORD_PATTERN.get_or_init(|| {
            Regex::new(r"\$([A-Za-z]{1,3})\$?([1-9][0-9]{0,6})|([A-Za-z]{1,3})\$?([1-9][0-9]{0,6})")
                .unwrap()
        }),
    }
}

fn sheet_scan_patterns() -> SheetScanPatterns {
    SheetScanPatterns {
        unsafe_function: unsafe_function_pattern(),
        formula: formula_pattern(),
        defined_name: DEFINED_NAME_WITH_ATTRS_PATTERN
            .get_or_init(|| Regex::new(r"(?s)<definedName\b([^>]*)>(.*?)</definedName>").unwrap()),
        reference_attribute: reference_attribute_pattern(),
        sequence_reference_attribute: SQREF_WITH_TAG_PATTERN.get_or_init(|| {
            Regex::new(
                r#"(?s)<(?:[A-Za-z_][\w.-]*:)?([A-Za-z_][\w.-]*)\b[^>]*\bsqref="([^"]+)"[^>]*>"#,
            )
            .unwrap()
        }),
        reference_token: REFERENCE_TOKEN_PATTERN.get_or_init(|| {
            Regex::new(
                r"(?i)(?:'[^']+'!|[A-Z_][A-Z0-9_.\-]*!)?\$?[A-Z]{1,3}\$?[1-9][0-9]{0,6}(?::\$?[A-Z]{1,3}\$?[1-9][0-9]{0,6})?",
            )
            .unwrap()
        }),
    }
}

fn unsafe_function_pattern() -> &'static Regex {
    UNSAFE_FUNCTIONS.get_or_init(|| Regex::new(r"(?i)\b(INDIRECT|OFFSET|ADDRESS|CELL)\b").unwrap())
}

fn formula_pattern() -> &'static Regex {
    FORMULA_PATTERN.get_or_init(|| Regex::new(r"(?s)<f[^>]*>(.*?)</f>").unwrap())
}

fn reference_attribute_pattern() -> &'static Regex {
    REF_PATTERN.get_or_init(|| {
        Regex::new(r#"(?s)<(?:[A-Za-z_][\w.-]*:)?([A-Za-z_][\w.-]*)\b[^>]*\bref="([^"]+)"[^>]*>"#)
            .unwrap()
    })
}

pub fn scan_for_references(
    buffers: &[&[u8]],
    protected_cells: &mut ProtectedCellRefs,
) -> ScanResult {
    let patterns = flat_scan_patterns();

    for &buffer in buffers {
        if let Some(function) = scan_flat_buffer(buffer, protected_cells, &patterns) {
            return ScanResult::Unsafe(function);
        }
    }

    ScanResult::Safe
}

fn scan_flat_buffer(
    buffer: &[u8],
    protected_cells: &mut ProtectedCellRefs,
    patterns: &FlatScanPatterns,
) -> Option<String> {
    if let Some(function) = scan_flat_formulas(buffer, protected_cells, patterns) {
        return Some(function);
    }
    if let Some(function) = scan_flat_defined_names(buffer, protected_cells, patterns) {
        return Some(function);
    }
    scan_flat_reference_attributes(buffer, protected_cells, patterns);
    None
}

fn scan_flat_formulas(
    buffer: &[u8],
    protected_cells: &mut ProtectedCellRefs,
    patterns: &FlatScanPatterns,
) -> Option<String> {
    for captures in patterns.formula.captures_iter(buffer) {
        let formula = captures.get(1).unwrap().as_bytes();
        if let Some(function) = unsafe_function_name(formula, patterns.unsafe_function) {
            return Some(function);
        }
        extract_coordinates(formula, protected_cells, patterns.coordinate);
    }
    None
}

fn scan_flat_defined_names(
    buffer: &[u8],
    protected_cells: &mut ProtectedCellRefs,
    patterns: &FlatScanPatterns,
) -> Option<String> {
    for captures in patterns.defined_name.captures_iter(buffer) {
        let content = captures.get(1).unwrap().as_bytes();
        if let Some(function) = unsafe_function_name(content, patterns.unsafe_function) {
            return Some(function);
        }
        extract_coordinates(content, protected_cells, patterns.coordinate);
    }
    None
}

fn scan_flat_reference_attributes(
    buffer: &[u8],
    protected_cells: &mut ProtectedCellRefs,
    patterns: &FlatScanPatterns,
) {
    for captures in patterns.reference_attribute.captures_iter(buffer) {
        let tag_name = captures.get(1).unwrap().as_bytes();
        if !ref_tag_should_be_ignored(tag_name) {
            extract_range(captures.get(2).unwrap().as_bytes(), protected_cells);
        }
    }

    for captures in patterns.sequence_reference_attribute.captures_iter(buffer) {
        extract_range(captures.get(1).unwrap().as_bytes(), protected_cells);
    }
}

pub fn scan_for_references_by_sheet(
    buffers: &[SheetScanBuffer<'_>],
    protected_refs: &mut ProtectedWorkbookRefs,
    sheet_name_to_file_seed: Option<&HashMap<String, String>>,
    sheet_files_by_index: Option<&[String]>,
) -> ScanResult {
    PreparedReferenceScanner::new(buffers, sheet_name_to_file_seed, sheet_files_by_index)
        .scan(buffers, protected_refs)
}

/// Immutable workbook context reusable across independent, possibly concurrent scans.
/// Supply all sheet names when preparing: scan() does not extend the mapping.
pub struct PreparedReferenceScanner<'a> {
    sheet_name_to_file: HashMap<String, String>,
    sheet_files_by_index: Option<&'a [String]>,
    patterns: SheetScanPatterns,
}

impl<'a> PreparedReferenceScanner<'a> {
    pub fn new(
        buffers: &[SheetScanBuffer<'_>],
        seed: Option<&HashMap<String, String>>,
        sheet_files_by_index: Option<&'a [String]>,
    ) -> Self {
        Self {
            sheet_name_to_file: build_sheet_name_to_file_map(buffers, seed),
            sheet_files_by_index,
            patterns: sheet_scan_patterns(),
        }
    }

    pub fn scan(
        &self,
        buffers: &[SheetScanBuffer<'_>],
        protected_refs: &mut ProtectedWorkbookRefs,
    ) -> ScanResult {
        for buffer in buffers {
            let _profile = tracing::info_span!(target: "excel_shrink::profile", "scan_sheet_references", sheet = buffer.sheet_file, input_bytes = buffer.xml.len()).entered();
            if let Some(function) = scan_sheet_buffer(
                buffer,
                protected_refs,
                &self.sheet_name_to_file,
                self.sheet_files_by_index,
                &self.patterns,
            ) {
                return ScanResult::Unsafe(function);
            }
        }
        ScanResult::Safe
    }
}

fn build_sheet_name_to_file_map(
    buffers: &[SheetScanBuffer<'_>],
    seed: Option<&HashMap<String, String>>,
) -> HashMap<String, String> {
    let mut map = seed.cloned().unwrap_or_default();
    map.extend(
        buffers
            .iter()
            .map(|buffer| (buffer.sheet_name.to_string(), buffer.sheet_file.to_string())),
    );
    map
}

fn scan_sheet_buffer(
    buffer: &SheetScanBuffer<'_>,
    protected_refs: &mut ProtectedWorkbookRefs,
    sheet_name_to_file: &HashMap<String, String>,
    sheet_files_by_index: Option<&[String]>,
    patterns: &SheetScanPatterns,
) -> Option<String> {
    if let Some(function) =
        scan_sheet_formulas(buffer, protected_refs, sheet_name_to_file, patterns)
    {
        return Some(function);
    }
    if let Some(function) = scan_sheet_defined_names(
        buffer,
        protected_refs,
        sheet_name_to_file,
        sheet_files_by_index,
        patterns,
    ) {
        return Some(function);
    }
    scan_sheet_reference_attributes(buffer, protected_refs, sheet_name_to_file, patterns);
    None
}

fn scan_sheet_formulas(
    buffer: &SheetScanBuffer<'_>,
    protected_refs: &mut ProtectedWorkbookRefs,
    sheet_name_to_file: &HashMap<String, String>,
    patterns: &SheetScanPatterns,
) -> Option<String> {
    for captures in patterns.formula.captures_iter(buffer.xml) {
        let formula = captures.get(1).unwrap().as_bytes();
        if let Some(function) = unsafe_function_name(formula, patterns.unsafe_function) {
            return Some(function);
        }
        extract_tokens_for_sheet(
            formula,
            Some(buffer.sheet_file),
            sheet_name_to_file,
            protected_refs,
            patterns.reference_token,
        );
    }
    None
}

fn scan_sheet_defined_names(
    buffer: &SheetScanBuffer<'_>,
    protected_refs: &mut ProtectedWorkbookRefs,
    sheet_name_to_file: &HashMap<String, String>,
    sheet_files_by_index: Option<&[String]>,
    patterns: &SheetScanPatterns,
) -> Option<String> {
    for captures in patterns.defined_name.captures_iter(buffer.xml) {
        let attrs = captures
            .get(1)
            .map(|capture| capture.as_bytes())
            .unwrap_or_default();
        if defined_name_should_be_ignored(attrs) {
            continue;
        }

        let content = captures.get(2).unwrap().as_bytes();
        if let Some(function) = unsafe_function_name(content, patterns.unsafe_function) {
            return Some(function);
        }

        extract_tokens_for_sheet(
            content,
            default_defined_name_sheet(buffer.sheet_file, attrs, sheet_files_by_index),
            sheet_name_to_file,
            protected_refs,
            patterns.reference_token,
        );
    }
    None
}

fn default_defined_name_sheet<'a>(
    current_sheet: &'a str,
    attrs: &[u8],
    sheet_files_by_index: Option<&'a [String]>,
) -> Option<&'a str> {
    if current_sheet != "xl/workbook.xml" {
        return Some(current_sheet);
    }

    extract_local_sheet_id(attrs)
        .and_then(|index| sheet_files_by_index.and_then(|order| order.get(index)))
        .map(String::as_str)
}

fn scan_sheet_reference_attributes(
    buffer: &SheetScanBuffer<'_>,
    protected_refs: &mut ProtectedWorkbookRefs,
    sheet_name_to_file: &HashMap<String, String>,
    patterns: &SheetScanPatterns,
) {
    for captures in patterns.reference_attribute.captures_iter(buffer.xml) {
        let tag_name = captures.get(1).unwrap().as_bytes();
        if !ref_tag_should_be_ignored(tag_name) {
            extract_range_for_sheet(
                captures.get(2).unwrap().as_bytes(),
                buffer.sheet_file,
                sheet_name_to_file,
                protected_refs,
            );
        }
    }

    for captures in patterns
        .sequence_reference_attribute
        .captures_iter(buffer.xml)
    {
        let tag_name = captures.get(1).unwrap().as_bytes();
        if !tag_name.eq_ignore_ascii_case(b"selection") {
            extract_range_for_sheet(
                captures.get(2).unwrap().as_bytes(),
                buffer.sheet_file,
                sheet_name_to_file,
                protected_refs,
            );
        }
    }
}

fn unsafe_function_name(text: &[u8], pattern: &Regex) -> Option<String> {
    pattern
        .find(text)
        .map(|matched| String::from_utf8_lossy(matched.as_bytes()).to_string())
}

fn extract_coordinates(text: &[u8], protected: &mut ProtectedCellRefs, coord_re: &Regex) {
    for caps in coord_re.captures_iter(text) {
        let (col_bytes, row_bytes) = if let Some(c) = caps.get(1) {
            (c.as_bytes(), caps.get(2).unwrap().as_bytes())
        } else {
            (
                caps.get(3).unwrap().as_bytes(),
                caps.get(4).unwrap().as_bytes(),
            )
        };

        let col = letters_byte_string_to_integer(col_bytes);
        if let Ok(row_str) = std::str::from_utf8(row_bytes) {
            if let Ok(row) = row_str.parse::<u32>() {
                protected.insert_cell(col, row);
            }
        }
    }
}

fn ref_tag_should_be_ignored(tag_name: &[u8]) -> bool {
    tag_name.eq_ignore_ascii_case(b"dimension")
        || tag_name.eq_ignore_ascii_case(b"autoFilter")
        || tag_name.eq_ignore_ascii_case(b"sortState")
}

fn defined_name_should_be_ignored(attrs: &[u8]) -> bool {
    let Some(name) = extract_quoted_attr_value(attrs, b"name") else {
        return false;
    };

    name.eq_ignore_ascii_case(b"_xlnm._FilterDatabase")
        || name.eq_ignore_ascii_case(b"_FilterDatabase")
        || bytes_ends_with_ignore_ascii_case(name, b".wvu.FilterData")
}

fn extract_tokens_for_sheet(
    text: &[u8],
    default_sheet_for_unprefixed: Option<&str>,
    sheet_name_to_file: &std::collections::HashMap<String, String>,
    protected: &mut ProtectedWorkbookRefs,
    token_re: &Regex,
) {
    for m in token_re.find_iter(text) {
        extract_reference_token_for_sheet_bytes(
            m.as_bytes(),
            default_sheet_for_unprefixed,
            sheet_name_to_file,
            protected,
        );
    }
}

const MAX_EXCEL_COL: u32 = 16_384; // XFD
const MAX_EXCEL_ROW: u32 = 1_048_576;

fn extract_range(text: &[u8], protected: &mut ProtectedCellRefs) -> bool {
    for token in text.split(u8::is_ascii_whitespace) {
        extract_reference_token_bytes(token, protected);
    }

    true
}

fn extract_range_for_sheet(
    text: &[u8],
    current_sheet: &str,
    sheet_name_to_file: &std::collections::HashMap<String, String>,
    protected: &mut ProtectedWorkbookRefs,
) {
    for token in text.split(u8::is_ascii_whitespace) {
        extract_reference_token_for_sheet_bytes(
            token,
            Some(current_sheet),
            sheet_name_to_file,
            protected,
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum ParsedReferenceToken {
    Single(ParsedRefPart),
    Range(ParsedRefPart, ParsedRefPart),
}

fn extract_reference_token_bytes(token: &[u8], protected: &mut ProtectedCellRefs) {
    let (_, raw_reference) = split_sheet_prefix_bytes(trim_ascii(token));
    if let Some(reference) = parse_reference_token(raw_reference) {
        insert_parsed_reference(reference, protected);
    }
}

fn extract_reference_token_for_sheet_bytes(
    token: &[u8],
    default_sheet_for_unprefixed: Option<&str>,
    sheet_name_to_file: &HashMap<String, String>,
    protected: &mut ProtectedWorkbookRefs,
) {
    let (sheet_prefix, raw_reference) = split_sheet_prefix_bytes(trim_ascii(token));
    let Some(reference) = parse_reference_token(raw_reference) else {
        return;
    };

    let target_sheet = match sheet_prefix {
        Some(sheet) => resolve_sheet_name(sheet, sheet_name_to_file),
        None => default_sheet_for_unprefixed,
    };
    insert_parsed_reference(reference, refs_for_target_sheet(target_sheet, protected));
}

fn parse_reference_token(raw_reference: &[u8]) -> Option<ParsedReferenceToken> {
    let Some(colon_index) = crate::xml::find_bytes(raw_reference, b":") else {
        return parse_ref_part_bytes(raw_reference).map(ParsedReferenceToken::Single);
    };

    let start = parse_ref_part_bytes(&raw_reference[..colon_index])?;
    let end = parse_ref_part_bytes(&raw_reference[colon_index + 1..])?;
    Some(ParsedReferenceToken::Range(start, end))
}

fn refs_for_target_sheet<'a>(
    target_sheet: Option<&str>,
    protected: &'a mut ProtectedWorkbookRefs,
) -> &'a mut ProtectedCellRefs {
    match target_sheet {
        Some(sheet) => protected.refs_for_sheet_mut(sheet),
        None => &mut protected.global_or_unknown,
    }
}

fn insert_parsed_reference(reference: ParsedReferenceToken, protected: &mut ProtectedCellRefs) {
    match reference {
        ParsedReferenceToken::Single(reference) => insert_single_reference(reference, protected),
        ParsedReferenceToken::Range(start, end) => protected.insert_range(
            start.col.unwrap_or(1),
            start.row.unwrap_or(1),
            end.col.unwrap_or(MAX_EXCEL_COL),
            end.row.unwrap_or(MAX_EXCEL_ROW),
        ),
    }
}

#[inline]
fn insert_single_reference(reference: ParsedRefPart, protected: &mut ProtectedCellRefs) {
    match (reference.col, reference.row) {
        (Some(col), Some(row)) => protected.insert_cell(col, row),
        (Some(col), None) => protected.insert_range(col, 1, col, MAX_EXCEL_ROW),
        (None, Some(row)) => protected.insert_range(1, row, MAX_EXCEL_COL, row),
        (None, None) => {}
    }
}

fn split_sheet_prefix_bytes(reference: &[u8]) -> (Option<&[u8]>, &[u8]) {
    if let Some(idx) = crate::xml::rfind_bytes(reference, b"!") {
        return (Some(&reference[..idx]), &reference[idx + 1..]);
    }
    (None, reference)
}

fn resolve_sheet_name<'a>(
    mut sheet: &[u8],
    sheet_name_to_file: &'a HashMap<String, String>,
) -> Option<&'a str> {
    if sheet.first() == Some(&b'\'') && sheet.last() == Some(&b'\'') && sheet.len() >= 2 {
        sheet = &sheet[1..sheet.len() - 1];
    }
    let s = std::str::from_utf8(sheet).unwrap_or("");
    sheet_name_to_file
        .get(&s.replace("''", "'"))
        .map(|s| s.as_str())
}

#[derive(Debug, Clone, Copy)]
struct ParsedRefPart {
    col: Option<u32>,
    row: Option<u32>,
}

fn parse_ref_part_bytes(part: &[u8]) -> Option<ParsedRefPart> {
    let mut col = 0;
    let mut row = 0;
    let mut has_col = false;
    let mut has_row = false;

    for &b in part {
        if b >= b'A' && b <= b'Z' {
            col = col * 26 + (b - b'A' + 1) as u32;
            has_col = true;
        } else if b >= b'a' && b <= b'z' {
            col = col * 26 + (b - b'a' + 1) as u32;
            has_col = true;
        } else if b >= b'0' && b <= b'9' {
            row = row * 10 + (b - b'0') as u32;
            has_row = true;
        }
    }

    if !has_col && !has_row {
        return None;
    }

    Some(ParsedRefPart {
        col: if has_col { Some(col) } else { None },
        row: if has_row { Some(row) } else { None },
    })
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = bytes.len();

    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }

    &bytes[start..end]
}

fn extract_local_sheet_id(attrs: &[u8]) -> Option<usize> {
    let raw = extract_quoted_attr_value(attrs, b"localSheetId")?;
    let value = std::str::from_utf8(raw).ok()?;
    value.parse::<usize>().ok()
}

fn extract_quoted_attr_value<'a>(attrs: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let mut index = 0;

    while index < attrs.len() {
        while index < attrs.len() && attrs[index].is_ascii_whitespace() {
            index += 1;
        }

        let name_start = index;
        while index < attrs.len()
            && (attrs[index].is_ascii_alphanumeric()
                || matches!(attrs[index], b':' | b'_' | b'-' | b'.'))
        {
            index += 1;
        }

        if name_start == index {
            index += 1;
            continue;
        }

        let name = &attrs[name_start..index];
        while index < attrs.len() && attrs[index].is_ascii_whitespace() {
            index += 1;
        }

        if attrs.get(index) != Some(&b'=') {
            continue;
        }
        index += 1;

        while index < attrs.len() && attrs[index].is_ascii_whitespace() {
            index += 1;
        }

        let quote = *attrs.get(index)?;
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        index += 1;

        let value_start = index;
        let end_rel = attrs[index..].iter().position(|byte| *byte == quote)?;
        let value = &attrs[value_start..value_start + end_rel];
        index = value_start + end_rel + 1;

        if name.eq_ignore_ascii_case(attr_name) {
            return Some(value);
        }
    }

    None
}

fn bytes_ends_with_ignore_ascii_case(value: &[u8], suffix: &[u8]) -> bool {
    value.len() >= suffix.len() && value[value.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

#[cfg(test)]
#[path = "reference_scanner_regression_tests.rs"]
mod regression_tests;

#[cfg(test)]
#[path = "reference_scanner_sheet_scope_tests.rs"]
mod sheet_scope_tests;

pub fn check_for_reference_hints(xml_data: &[u8]) -> bool {
    if crate::xml::find_bytes(xml_data, b"!").is_some() {
        return true;
    }
    let re = HINTS_PATTERN
        .get_or_init(|| Regex::new(r"(?i)\b(INDIRECT|OFFSET|ADDRESS|CELL)\b").unwrap());
    re.is_match(xml_data)
}
