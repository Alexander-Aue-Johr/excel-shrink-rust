//! Fast value regression: borrowed byte views and persistent per-sheet digests.
//! Layout-only cells never create per-cell heap objects in the normal path.
use serde::{Serialize, Deserialize};
#[cfg(test)]
use super::visual_diagnostics;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellValueDifference {
    pub sheet_file: String,
    pub cell_ref: String,
    pub kind: CellDifferenceKind,
    pub left_value: Option<String>,
    pub right_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellDifferenceKind {
    Changed,
    MissingRight,
    ExtraRight,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkbookComparisonReport {
    pub compared_sheets: usize,
    pub difference_count: usize,
    pub differences: Vec<CellValueDifference>,
    pub right_capture_ranges: Vec<WorksheetCaptureRange>,
    pub left_capture_ranges: Vec<WorksheetCaptureRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorksheetCaptureRange {
    pub sheet_index: usize,
    pub sheet_file: String,
    pub max_cell: String,
    pub max_row: u32,
    pub max_column: u32,
    pub max_row_source: Option<String>,
    pub max_column_source: Option<String>,
}

#[path = "value_views.rs"]
mod views;
pub use views::{CaptureColumns, workbook_capture_columns, compare_saved_visual_values, compare_workbook_preservation, compare_workbook_values, workbook_visual_sheet_order};

/// Both source and cleaned visible content must be represented in screenshots.
pub fn combined_capture_ranges(report: &WorkbookComparisonReport) -> Vec<WorksheetCaptureRange> {
    let mut ranges = report.right_capture_ranges.clone();
    for left in &report.left_capture_ranges {
        match ranges.iter_mut().find(|r| r.sheet_index == left.sheet_index) {
            Some(range) => {
                if left.max_row > range.max_row { range.max_row = left.max_row; range.max_row_source = left.max_row_source.clone(); }
                if left.max_column > range.max_column { range.max_column = left.max_column; range.max_column_source = left.max_column_source.clone(); }
                let mut number = range.max_column;
                let mut letters = String::new();
                while number > 0 {
                    let rem = (number - 1) % 26;
                    letters.insert(0, (b'A' + rem as u8) as char); number = (number - 1) / 26;
                }
                range.max_cell = format!("{}{}", letters, range.max_row);
            }
            None => ranges.push(left.clone()),
        }
    }
    ranges.sort_by_key(|r| r.sheet_index);
    ranges
}


#[cfg(test)]
#[path = "xlsx_value_compare_legacy.rs"]
mod legacy;
#[cfg(test)]
#[path = "value_views_tests.rs"]
mod tests;
