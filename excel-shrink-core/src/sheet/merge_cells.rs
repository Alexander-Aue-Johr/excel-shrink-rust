use crate::models::{CellCoordinate, CellRange};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use regex::bytes::{Captures, Regex};
use std::sync::OnceLock;

static MERGE_CELL_PATTERN: OnceLock<Regex> = OnceLock::new();

pub fn find_max_merge_cells(merge_cell_data: &[u8]) -> Option<CellCoordinate> {
    let mut max_column = None;
    let mut max_row = None;

    for captures in merge_cell_pattern().captures_iter(merge_cell_data) {
        max_column = Some(max_coordinate_component(
            max_column,
            capture_bytes(&captures, 1),
        ));
        max_column = Some(max_coordinate_component(
            max_column,
            capture_bytes(&captures, 3),
        ));
        max_row = Some(max_coordinate_component(
            max_row,
            capture_bytes(&captures, 2),
        ));
        max_row = Some(max_coordinate_component(
            max_row,
            capture_bytes(&captures, 4),
        ));
    }

    Some(CellCoordinate {
        column: max_column?.to_vec(),
        row: max_row?.to_vec(),
    })
}

pub fn find_merge_cell_ranges(merge_cell_data: &[u8]) -> Vec<CellRange> {
    merge_cell_pattern()
        .captures_iter(merge_cell_data)
        .map(merge_cell_range)
        .collect()
}

fn merge_cell_pattern() -> &'static Regex {
    MERGE_CELL_PATTERN.get_or_init(|| {
        Regex::new(r#"<mergeCell\b[^/>]*\bref="([A-Za-z]+)(\d+):([A-Za-z]+)(\d+)"[^/>]*/>"#)
            .unwrap()
    })
}

#[inline]
fn max_coordinate_component<'a>(current: Option<&'a [u8]>, candidate: &'a [u8]) -> &'a [u8] {
    match current {
        Some(current)
            if current
                .len()
                .cmp(&candidate.len())
                .then_with(|| current.cmp(candidate))
                .is_ge() =>
        {
            current
        }
        _ => candidate,
    }
}

#[inline]
fn capture_bytes<'a>(captures: &Captures<'a>, index: usize) -> &'a [u8] {
    captures.get(index).unwrap().as_bytes()
}

fn merge_cell_range(captures: Captures<'_>) -> CellRange {
    let start_col = letters_byte_string_to_integer(capture_bytes(&captures, 1));
    let start_row = digits_byte_string_to_integer(capture_bytes(&captures, 2));
    let end_col = letters_byte_string_to_integer(capture_bytes(&captures, 3));
    let end_row = digits_byte_string_to_integer(capture_bytes(&captures, 4));

    CellRange {
        start_col: start_col.min(end_col),
        start_row: start_row.min(end_row),
        end_col: start_col.max(end_col),
        end_row: start_row.max(end_row),
    }
}
