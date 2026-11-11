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
pub use views::{compare_workbook_cell_contents, compare_workbook_changed_rows, compare_workbook_preservation, compare_workbook_values};
pub(in crate::regression) use views::removed_diagnostic_cells;

#[cfg(test)]
#[path = "xlsx_value_compare_legacy.rs"]
mod legacy;
#[cfg(test)]
#[path = "value_views_tests.rs"]
mod tests;
