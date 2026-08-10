use crate::models::{CellCoordinate, CellRange};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use regex::bytes::Regex;
use std::sync::OnceLock;

static MERGE_CELL_PATTERN: OnceLock<Regex> = OnceLock::new();

pub fn find_max_merge_cells(merge_cell_data: &[u8]) -> Option<CellCoordinate> {
    let re = MERGE_CELL_PATTERN.get_or_init(|| {
        Regex::new(r#"<mergeCell\b[^/>]*\bref="([A-Za-z]+)(\d+):([A-Za-z]+)(\d+)"[^/>]*/>"#)
            .unwrap()
    });

    let mut distinct_cols = Vec::new();
    let mut distinct_rows = Vec::new();

    for caps in re.captures_iter(merge_cell_data) {
        distinct_cols.push(caps.get(1).unwrap().as_bytes().to_vec());
        distinct_rows.push(caps.get(2).unwrap().as_bytes().to_vec());
        distinct_cols.push(caps.get(3).unwrap().as_bytes().to_vec());
        distinct_rows.push(caps.get(4).unwrap().as_bytes().to_vec());
    }

    if distinct_cols.is_empty() {
        return None;
    }

    let max_col = distinct_cols
        .into_iter()
        .max_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)))
        .unwrap();

    let max_row = distinct_rows
        .into_iter()
        .max_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)))
        .unwrap();

    Some(CellCoordinate {
        column: max_col,
        row: max_row,
    })
}

pub fn find_merge_cell_ranges(merge_cell_data: &[u8]) -> Vec<CellRange> {
    let re = MERGE_CELL_PATTERN.get_or_init(|| {
        Regex::new(r#"<mergeCell\b[^/>]*\bref="([A-Za-z]+)(\d+):([A-Za-z]+)(\d+)"[^/>]*/>"#)
            .unwrap()
    });

    re.captures_iter(merge_cell_data)
        .map(|caps| {
            let start_col = letters_byte_string_to_integer(caps.get(1).unwrap().as_bytes());
            let start_row = digits_byte_string_to_integer(caps.get(2).unwrap().as_bytes());
            let end_col = letters_byte_string_to_integer(caps.get(3).unwrap().as_bytes());
            let end_row = digits_byte_string_to_integer(caps.get(4).unwrap().as_bytes());

            CellRange {
                start_col: start_col.min(end_col),
                start_row: start_row.min(end_row),
                end_col: start_col.max(end_col),
                end_row: start_row.max(end_row),
            }
        })
        .collect()
}
