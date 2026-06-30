use super::*;
use crate::models::{CleanSheetParameters, SheetPreprocessConfig, WorkbookSpecificPatterns, Xf};
use std::collections::HashMap;
use zip::CompressionMethod;

fn relevant_style() -> Xf {
    Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(2))
        .border_id(Some(1))
        .apply_border(true)
        .apply_fill(true)
        .has_visible_fill(true)
        .has_visible_border(true)
        .has_fill_or_border(true)
        .has_apply_fill_or_border(true)
        .build_xf()
}

fn params_with_relevant_style(style_id: u32) -> CleanSheetParameters {
    let mut xfs_by_index = HashMap::new();
    xfs_by_index.insert(style_id, relevant_style());

    CleanSheetParameters {
        xlsx_file_path: "test.xlsx".to_string(),
        zip_info_filename: "xl/worksheets/sheet1.xml".to_string(),
        compress_method: CompressionMethod::Deflated,
        patterns: WorkbookSpecificPatterns {
            xfs_by_index,
            no_empty_cell_visible_style_ids: Vec::new(),
            fill_or_border_or_protection_ids: vec![style_id],
            whitespace_indices: None,
        },
        advanced_cleanup: None,
        preprocess: SheetPreprocessConfig::default(),
        cleanup: Default::default(),
        collect_cleanup_stats: false,
    }
}

#[test]
fn does_not_treat_spans_as_style_attribute() {
    let params = params_with_relevant_style(8);

    let sheet_data = br#"<row r="99" spans="1:8"/>"#;

    let pos = find_last_fill_or_border_or_protection_style_or_hidden_attribute(
        sheet_data, &params, None, false,
    );

    assert_eq!(pos, None);
}

#[test]
fn finds_real_row_style_attribute_after_cell_range() {
    let params = params_with_relevant_style(8);

    let sheet_data = br#"<row r="99" s="8"/>"#;

    let pos = find_last_fill_or_border_or_protection_style_or_hidden_attribute(
        sheet_data, &params, None, false,
    );

    assert!(pos.is_some());
}

#[test]
fn last_visible_row_end_reports_row_at_boundary() {
    let sheet_data = br#"<row r="110"><c r="I110"><v>27</v></c></row><row r="111" spans="1:9"/>"#;
    let last_cell_row_end = find_last_row_with_cell_end_pos(sheet_data);

    let row_end = find_last_visible_row_end_position(sheet_data, last_cell_row_end).unwrap();

    assert_eq!(row_end.row, 110);
    assert_eq!(row_end.end, last_cell_row_end.unwrap());
}

#[test]
fn preserve_through_row_extends_range_after_last_visible_row() {
    let params = params_with_relevant_style(8);
    let sheet_data = br#"<row r="110"><c r="I110"><v>27</v></c></row><row r="111" spans="1:9"/><row r="112" spans="1:9"/>"#;

    let ranges_without_preserve =
        retrieve_sheet_data_range_containing_cells(sheet_data, &params, false, None);
    let rows_without_preserve = String::from_utf8_lossy(ranges_without_preserve.rows_range);
    assert!(!rows_without_preserve.contains(r#"r="111""#));

    let ranges_with_preserve =
        retrieve_sheet_data_range_containing_cells(sheet_data, &params, false, Some(111));
    let rows_with_preserve = String::from_utf8_lossy(ranges_with_preserve.rows_range);

    assert!(rows_with_preserve.contains(r#"r="111""#));
    assert!(!rows_with_preserve.contains(r#"r="112""#));
}
