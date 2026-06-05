use crate::models::CellCoordinate;
use regex::bytes::Regex;
use std::sync::OnceLock;

static MERGE_CELL_PATTERN: OnceLock<Regex> = OnceLock::new();

pub fn find_max_merge_cells(merge_cell_data: &[u8]) -> Option<CellCoordinate> {
    let re = MERGE_CELL_PATTERN.get_or_init(|| 
        Regex::new(r#"<mergeCell\b[^/>]*\bref="([A-Za-z]+)(\d+):([A-Za-z]+)(\d+)"[^/>]*/>"#).unwrap()
    );

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

    // Finde das Maximum anhand der Länge und dann lexikografisch
    let max_col = distinct_cols.into_iter().max_by(|a, b| {
        a.len().cmp(&b.len()).then_with(|| a.cmp(b))
    }).unwrap();

    let max_row = distinct_rows.into_iter().max_by(|a, b| {
        a.len().cmp(&b.len()).then_with(|| a.cmp(b))
    }).unwrap();

    Some(CellCoordinate {
        column: max_col,
        row: max_row,
    })
}