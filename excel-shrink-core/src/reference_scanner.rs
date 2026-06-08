use crate::models::ProtectedCellRefs;
use crate::utils::letters_byte_string_to_integer;
use regex::bytes::Regex;
use std::sync::OnceLock;

static UNSAFE_FUNCTIONS: OnceLock<Regex> = OnceLock::new();
static FORMULA_PATTERN: OnceLock<Regex> = OnceLock::new();
static DEFINED_NAME_PATTERN: OnceLock<Regex> = OnceLock::new();
static REF_PATTERN: OnceLock<Regex> = OnceLock::new();
static SQREF_PATTERN: OnceLock<Regex> = OnceLock::new();
static COORD_PATTERN: OnceLock<Regex> = OnceLock::new();

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
        .get_or_init(|| Regex::new(r"(?s)<definedName[^>]*>(.*?)</definedName>").unwrap());
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

            if tag_name.eq_ignore_ascii_case(b"dimension") {
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

const MAX_EXCEL_COL: u32 = 16_384; // XFD
const MAX_EXCEL_ROW: u32 = 1_048_576;

fn extract_range(text: &[u8], protected: &mut ProtectedCellRefs) -> bool {
    let raw = String::from_utf8_lossy(text);

    // sqref kann mehrere Bereiche enthalten, z. B. "A1:B10 C20:D30"
    for token in raw.split_whitespace() {
        extract_reference_token(token, protected);
    }

    true
}

fn extract_reference_token(token: &str, protected: &mut ProtectedCellRefs) {
    let token = strip_sheet_prefix(token.trim());

    if token.is_empty() {
        return;
    }

    if let Some((left, right)) = token.split_once(':') {
        let Some(left_ref) = parse_ref_part(left) else {
            return;
        };
        let Some(right_ref) = parse_ref_part(right) else {
            return;
        };

        let col1 = left_ref.col.unwrap_or(1);
        let col2 = right_ref.col.unwrap_or(MAX_EXCEL_COL);
        let row1 = left_ref.row.unwrap_or(1);
        let row2 = right_ref.row.unwrap_or(MAX_EXCEL_ROW);

        protected.insert_range(col1, row1, col2, row2);
        return;
    }

    let Some(single_ref) = parse_ref_part(token) else {
        return;
    };

    match (single_ref.col, single_ref.row) {
        (Some(col), Some(row)) => {
            protected.insert_cell(col, row);
        }
        (Some(col), None) => {
            // Ganze Spalte, z. B. A
            protected.insert_range(col, 1, col, MAX_EXCEL_ROW);
        }
        (None, Some(row)) => {
            // Ganze Zeile, z. B. 1
            protected.insert_range(1, row, MAX_EXCEL_COL, row);
        }
        (None, None) => {}
    }
}

#[derive(Debug, Clone, Copy)]
struct ParsedRefPart {
    col: Option<u32>,
    row: Option<u32>,
}

fn parse_ref_part(part: &str) -> Option<ParsedRefPart> {
    let part = strip_sheet_prefix(part.trim());
    let part = part.trim_matches('\'');
    let part = part.replace('$', "");

    if part.is_empty() {
        return None;
    }

    let col_str: String = part.chars().filter(|c| c.is_ascii_alphabetic()).collect();

    let row_str: String = part.chars().filter(|c| c.is_ascii_digit()).collect();

    let col = if col_str.is_empty() {
        None
    } else {
        Some(letters_byte_string_to_integer(col_str.as_bytes()))
    };

    let row = if row_str.is_empty() {
        None
    } else {
        row_str.parse::<u32>().ok()
    };

    if col.is_none() && row.is_none() {
        return None;
    }

    Some(ParsedRefPart { col, row })
}

fn strip_sheet_prefix(reference: &str) -> &str {
    reference
        .rsplit_once('!')
        .map(|(_, cell_ref)| cell_ref)
        .unwrap_or(reference)
}

#[cfg(test)]
#[path = "reference_scanner_regression_tests.rs"]
mod regression_tests;