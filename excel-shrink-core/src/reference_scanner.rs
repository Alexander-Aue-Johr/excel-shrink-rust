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
pub fn encode_coord(col: u32, row: u32) -> u64 {
    ((col as u64) << 32) | (row as u64)
}

pub fn scan_for_references(
    buffers: &[&[u8]],
    protected_cells: &mut ProtectedCellRefs,
) -> ScanResult {
    let unsafe_re = UNSAFE_FUNCTIONS
        .get_or_init(|| Regex::new(r"(?i)\b(INDIRECT|OFFSET|ADDRESS|CELL)\b").unwrap());
    let formula_re = FORMULA_PATTERN.get_or_init(|| Regex::new(r"(?s)<f[^>]*>(.*?)</f>").unwrap());
    let defined_name_re = DEFINED_NAME_PATTERN
        .get_or_init(|| Regex::new(r"(?s)<definedName\b[^>]*>(.*?)</definedName>").unwrap());
    let ref_re = REF_PATTERN.get_or_init(|| {
        Regex::new(r#"(?s)<(?:[A-Za-z_][\w.-]*:)?([A-Za-z_][\w.-]*)\b[^>]*\bref="([^"]+)"[^>]*>"#)
            .unwrap()
    });
    let sqref_re = SQREF_PATTERN.get_or_init(|| Regex::new(r#"sqref="([^"]+)""#).unwrap());

    // Matched A1, $A$1, etc.
    let coord_re = COORD_PATTERN.get_or_init(|| {
        Regex::new(r"\$([A-Za-z]{1,3})\$?([1-9][0-9]{0,6})|([A-Za-z]{1,3})\$?([1-9][0-9]{0,6})")
            .unwrap()
    });

    for &buf in buffers {
        // 1. Check for unsafe functions anywhere in formulas
        for caps in formula_re.captures_iter(buf) {
            let formula = caps.get(1).unwrap().as_bytes();
            if let Some(m) = unsafe_re.find(formula) {
                let func = String::from_utf8_lossy(m.as_bytes()).to_string();
                return ScanResult::Unsafe(func);
            }
            extract_coordinates(formula, protected_cells, &coord_re);
        }

        // 2. Check defined names
        for caps in defined_name_re.captures_iter(buf) {
            let content = caps.get(1).unwrap().as_bytes();
            if let Some(m) = unsafe_re.find(content) {
                let func = String::from_utf8_lossy(m.as_bytes()).to_string();
                return ScanResult::Unsafe(func);
            }
            extract_coordinates(content, protected_cells, &coord_re);
        }

        // 3. Extract simple ranges from attributes like ref="A1:B10"
        for caps in ref_re.captures_iter(buf) {
            let tag_name = caps.get(1).unwrap().as_bytes();

            if ref_tag_should_be_ignored(tag_name) {
                continue;
            }

            let content = caps.get(2).unwrap().as_bytes();
            extract_range(content, protected_cells);
        }

        for caps in sqref_re.captures_iter(buf) {
            let content = caps.get(1).unwrap().as_bytes();
            extract_range(content, protected_cells);
        }
    }

    ScanResult::Safe
}

pub fn scan_for_references_by_sheet(
    buffers: &[SheetScanBuffer<'_>],
    protected_refs: &mut ProtectedWorkbookRefs,
    sheet_name_to_file_seed: Option<&HashMap<String, String>>,
    sheet_files_by_index: Option<&[String]>,
) -> ScanResult {
    let unsafe_re = UNSAFE_FUNCTIONS
        .get_or_init(|| Regex::new(r"(?i)\b(INDIRECT|OFFSET|ADDRESS|CELL)\b").unwrap());
    let formula_re = FORMULA_PATTERN.get_or_init(|| Regex::new(r"(?s)<f[^>]*>(.*?)</f>").unwrap());
    let defined_name_with_attrs_re = DEFINED_NAME_WITH_ATTRS_PATTERN
        .get_or_init(|| Regex::new(r"(?s)<definedName\b([^>]*)>(.*?)</definedName>").unwrap());
    let ref_re = REF_PATTERN.get_or_init(|| {
        Regex::new(r#"(?s)<(?:[A-Za-z_][\w.-]*:)?([A-Za-z_][\w.-]*)\b[^>]*\bref="([^"]+)"[^>]*>"#)
            .unwrap()
    });
    let sqref_with_tag_re = SQREF_WITH_TAG_PATTERN.get_or_init(|| {
        Regex::new(r#"(?s)<(?:[A-Za-z_][\w.-]*:)?([A-Za-z_][\w.-]*)\b[^>]*\bsqref="([^"]+)"[^>]*>"#)
            .unwrap()
    });
    let reference_token_re = REFERENCE_TOKEN_PATTERN.get_or_init(|| {
        Regex::new(
            r"(?i)(?:'[^']+'!|[A-Z_][A-Z0-9_.\-]*!)?\$?[A-Z]{1,3}\$?[1-9][0-9]{0,6}(?::\$?[A-Z]{1,3}\$?[1-9][0-9]{0,6})?",
        )
        .unwrap()
    });

    let mut sheet_name_to_file: HashMap<String, String> = HashMap::new();

    if let Some(seed) = sheet_name_to_file_seed {
        for (name, file) in seed {
            sheet_name_to_file.insert(name.clone(), file.clone());
        }
    }

    for buffer in buffers {
        sheet_name_to_file.insert(buffer.sheet_name.to_string(), buffer.sheet_file.to_string());
    }

    for buffer in buffers {
        let current_sheet = buffer.sheet_file;
        let xml = buffer.xml;

        for caps in formula_re.captures_iter(xml) {
            let formula = caps.get(1).unwrap().as_bytes();
            if let Some(m) = unsafe_re.find(formula) {
                let func = String::from_utf8_lossy(m.as_bytes()).to_string();
                return ScanResult::Unsafe(func);
            }
            extract_tokens_for_sheet(
                formula,
                Some(current_sheet),
                &sheet_name_to_file,
                protected_refs,
                reference_token_re,
            );
        }

        for caps in defined_name_with_attrs_re.captures_iter(xml) {
            let attrs = caps.get(1).map(|m| m.as_bytes()).unwrap_or_default();
            let content = caps.get(2).unwrap().as_bytes();
            if defined_name_should_be_ignored(attrs) {
                continue;
            }

            if let Some(m) = unsafe_re.find(content) {
                let func = String::from_utf8_lossy(m.as_bytes()).to_string();
                return ScanResult::Unsafe(func);
            }

            let default_sheet_for_unprefixed = if current_sheet == "xl/workbook.xml" {
                extract_local_sheet_id(attrs)
                    .and_then(|index| sheet_files_by_index.and_then(|order| order.get(index)))
                    .map(String::as_str)
            } else {
                Some(current_sheet)
            };

            extract_tokens_for_sheet(
                content,
                default_sheet_for_unprefixed,
                &sheet_name_to_file,
                protected_refs,
                reference_token_re,
            );
        }

        for caps in ref_re.captures_iter(xml) {
            let tag_name = caps.get(1).unwrap().as_bytes();

            if ref_tag_should_be_ignored(tag_name) {
                continue;
            }

            let content = caps.get(2).unwrap().as_bytes();
            extract_range_for_sheet(content, current_sheet, &sheet_name_to_file, protected_refs);
        }

        for caps in sqref_with_tag_re.captures_iter(xml) {
            let tag_name = caps.get(1).unwrap().as_bytes();
            if tag_name.eq_ignore_ascii_case(b"selection") {
                continue;
            }

            let content = caps.get(2).unwrap().as_bytes();
            extract_range_for_sheet(content, current_sheet, &sheet_name_to_file, protected_refs);
        }
    }

    ScanResult::Safe
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

fn extract_reference_token_bytes(token: &[u8], protected: &mut ProtectedCellRefs) {
    let (_, raw_ref) = split_sheet_prefix_bytes(trim_ascii(token));

    if let Some(colon_idx) = crate::xml::find_bytes(raw_ref, b":") {
        let left = &raw_ref[..colon_idx];
        let right = &raw_ref[colon_idx + 1..];

        let Some(left_ref) = parse_ref_part_bytes(left) else {
            return;
        };
        let Some(right_ref) = parse_ref_part_bytes(right) else {
            return;
        };

        let col1 = left_ref.col.unwrap_or(1);
        let col2 = right_ref.col.unwrap_or(MAX_EXCEL_COL);
        let row1 = left_ref.row.unwrap_or(1);
        let row2 = right_ref.row.unwrap_or(MAX_EXCEL_ROW);

        protected.insert_range(col1, row1, col2, row2);
        return;
    }

    let Some(single_ref) = parse_ref_part_bytes(raw_ref) else {
        return;
    };

    match (single_ref.col, single_ref.row) {
        (Some(col), Some(row)) => {
            protected.insert_cell(col, row);
        }
        (Some(col), None) => {
            protected.insert_range(col, 1, col, MAX_EXCEL_ROW);
        }
        (None, Some(row)) => {
            protected.insert_range(1, row, MAX_EXCEL_COL, row);
        }
        (None, None) => {}
    }
}

fn extract_reference_token_for_sheet_bytes(
    token: &[u8],
    default_sheet_for_unprefixed: Option<&str>,
    sheet_name_to_file: &HashMap<String, String>,
    protected: &mut ProtectedWorkbookRefs,
) {
    let (sheet_opt, raw_ref) = split_sheet_prefix_bytes(trim_ascii(token));

    let target_sheet = if let Some(raw_sheet) = sheet_opt {
        resolve_sheet_name(raw_sheet, sheet_name_to_file)
    } else {
        default_sheet_for_unprefixed
    };

    if let Some(colon_idx) = crate::xml::find_bytes(raw_ref, b":") {
        let left = &raw_ref[..colon_idx];
        let right = &raw_ref[colon_idx + 1..];

        let Some(left_ref) = parse_ref_part_bytes(left) else {
            return;
        };
        let Some(right_ref) = parse_ref_part_bytes(right) else {
            return;
        };

        let col1 = left_ref.col.unwrap_or(1);
        let col2 = right_ref.col.unwrap_or(MAX_EXCEL_COL);
        let row1 = left_ref.row.unwrap_or(1);
        let row2 = right_ref.row.unwrap_or(MAX_EXCEL_ROW);

        if let Some(sheet) = target_sheet {
            protected
                .refs_for_sheet_mut(sheet)
                .insert_range(col1, row1, col2, row2);
        } else {
            protected
                .global_or_unknown
                .insert_range(col1, row1, col2, row2);
        }
    } else {
        let Some(single_ref) = parse_ref_part_bytes(raw_ref) else {
            return;
        };

        let refs = if let Some(sheet) = target_sheet {
            protected.refs_for_sheet_mut(sheet)
        } else {
            &mut protected.global_or_unknown
        };

        match (single_ref.col, single_ref.row) {
            (Some(col), Some(row)) => refs.insert_cell(col, row),
            (Some(col), None) => refs.insert_range(col, 1, col, MAX_EXCEL_ROW),
            (None, Some(row)) => refs.insert_range(1, row, MAX_EXCEL_COL, row),
            (None, None) => {}
        }
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
