use super::*;
use crate::models::{CleanSheetParameters, WorkbookSpecificPatterns, Xf};
use std::collections::HashMap;
use zip::CompressionMethod;

fn relevant_style() -> Xf {
    Xf::new(Some(2), Some(1), true, true, true, true, false, false)
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
            only_font_style_ids: Vec::new(),
            fill_or_border_or_protection_ids: vec![style_id],
            whitespace_indices: None,
        },
        advanced_cleanup: None,
    }
}

#[test]
fn does_not_treat_spans_as_style_attribute() {
    let params = params_with_relevant_style(8);

    let sheet_data = br#"<row r="99" spans="1:8"/>"#;

    let pos = find_last_fill_or_border_or_protection_style_or_hidden_attribute(
        sheet_data,
        &params,
        None,
    );

    assert_eq!(pos, None);
}

#[test]
fn finds_real_row_style_attribute_after_cell_range() {
    let params = params_with_relevant_style(8);

    let sheet_data = br#"<row r="99" s="8"/>"#;

    let pos = find_last_fill_or_border_or_protection_style_or_hidden_attribute(
        sheet_data,
        &params,
        None,
    );

    assert!(pos.is_some());
}