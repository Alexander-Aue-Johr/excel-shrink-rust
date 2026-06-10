use super::{replace_empty_row_after_cell_range, replace_row};
use crate::models::{
    AdvancedCleanupConfig, CellCleanerMode, CleanSheetMetrics, CleanSheetParameters,
    SheetPreprocessConfig, WorkbookSpecificPatterns, Xf,
};
use regex::bytes::Regex;
use std::collections::HashMap;
use std::sync::Arc;
use zip::CompressionMethod;

fn relevant_style() -> Xf {
    Xf::new(Some(2), Some(1), true, true, true, true, false, false)
}

fn font_only_style() -> Xf {
    Xf::new(Some(0), Some(0), false, false, false, false, true, false)
}

fn params_with_styles(styles: &[(u32, Xf)]) -> CleanSheetParameters {
    let mut xfs_by_index = HashMap::new();
    let mut only_font_style_ids = Vec::new();
    let mut fill_or_border_or_protection_ids = Vec::new();

    for (id, xf) in styles {
        if xf.has_only_font_style {
            only_font_style_ids.push(*id);
        }

        if xf.has_fill_or_border || xf.has_apply_protection {
            fill_or_border_or_protection_ids.push(*id);
        }

        xfs_by_index.insert(*id, xf.clone());
    }

    CleanSheetParameters {
        xlsx_file_path: "test.xlsx".to_string(),
        zip_info_filename: "xl/worksheets/sheet1.xml".to_string(),
        compress_method: CompressionMethod::Deflated,
        patterns: WorkbookSpecificPatterns {
            xfs_by_index,
            only_font_style_ids,
            fill_or_border_or_protection_ids,
            whitespace_indices: None,
        },
        advanced_cleanup: None,
        preprocess: SheetPreprocessConfig::default(),
        cell_cleaner_mode: CellCleanerMode::Regex,
    }
}

fn replace_single_row(row_xml: &[u8], params: &CleanSheetParameters) -> Vec<u8> {
    replace_single_row_with_metrics(row_xml, params).0
}

fn replace_single_row_with_metrics(
    row_xml: &[u8],
    params: &CleanSheetParameters,
) -> (Vec<u8>, CleanSheetMetrics) {
    let row_re = Regex::new(
        r#"(?s)<row\b((?:[^>]*\br="([^"]+?)")?(?:[^>]*\bhidden="(1)")?(?:[^>]*\bs="([^"]+?)")?(?:[^>]*\bht="([^"]+?)")?[^/>]*?)(?:/>|>(.*?)</row>)"#,
    )
    .unwrap();

    let caps = row_re.captures(row_xml).unwrap();
    let mut metrics = CleanSheetMetrics::default();

    let cleaned = replace_row(&caps, params, &mut metrics);
    (cleaned, metrics)
}

fn replace_single_row_with_cell_cleaner(
    row_xml: &[u8],
    params: &CleanSheetParameters,
    cell_cleaner_mode: CellCleanerMode,
) -> Vec<u8> {
    let mut params = params.clone();
    params.cell_cleaner_mode = cell_cleaner_mode;
    replace_single_row(row_xml, &params)
}

fn replace_single_empty_row_after_cell_range(
    row_xml: &[u8],
    params: &CleanSheetParameters,
) -> Vec<u8> {
    let row_re = Regex::new(
        r#"(?s)<row\b((?:[^>]*\br="([^"]+?)")?(?:[^>]*\bhidden="(1)")?(?:[^>]*\bs="([^"]+?)")?(?:[^>]*\bht="([^"]+?)")?[^/>]*?)(?:/>|></row>)"#,
    )
    .unwrap();

    let caps = row_re.captures(row_xml).unwrap();
    let mut metrics = CleanSheetMetrics::default();

    replace_empty_row_after_cell_range(&caps, params, &mut metrics)
}

#[test]
fn keeps_empty_cell_when_cell_style_has_fill_or_border_regression_row_15() {
    let params = params_with_styles(&[(427, relevant_style()), (429, font_only_style())]);

    let row_xml =
        br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27"><c r="A15" s="427"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27"><c r="A15" s="427"/></row>"#
    );
}

#[test]
fn collapses_row_when_empty_cell_has_only_font_style_and_row_has_height() {
    let params = params_with_styles(&[(427, font_only_style()), (429, font_only_style())]);

    let row_xml =
        br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27"><c r="A15" s="427"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="15" customFormat="1" ht="27" customHeight="1"/>"#
    );
}

#[test]
fn adds_custom_height_to_already_empty_row_with_height() {
    let params = params_with_styles(&[(429, font_only_style())]);

    let row_xml = br#"<row r="16" customFormat="1" ht="27"/>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="16" customFormat="1" ht="27" customHeight="1"/>"#
    );
}

#[test]
fn adds_custom_height_to_empty_row_after_cell_range_with_height() {
    let params = params_with_styles(&[(429, font_only_style())]);

    let row_xml = br#"<row r="16" customFormat="1" ht="27"/>"#;

    let cleaned = replace_single_empty_row_after_cell_range(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="16" customFormat="1" ht="27" customHeight="1"/>"#
    );
}

#[test]
fn byte_cell_cleaner_matches_regex_for_self_closing_cells() {
    let params = params_with_styles(&[(427, font_only_style()), (429, font_only_style())]);
    let row_xml =
        br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27"><c r="A15" s="427"/></row>"#;

    assert_eq!(
        replace_single_row_with_cell_cleaner(row_xml, &params, CellCleanerMode::Bytes),
        replace_single_row_with_cell_cleaner(row_xml, &params, CellCleanerMode::Regex)
    );
}

#[test]
fn byte_cell_cleaner_matches_regex_and_skips_content_cells_without_advanced_cleanup() {
    let params = params_with_styles(&[]);
    let row_xml = br#"<row r="3"><c r="A3"><v>123</v></c><c r="B3"/></row>"#;

    assert_eq!(
        replace_single_row_with_cell_cleaner(row_xml, &params, CellCleanerMode::Bytes),
        replace_single_row_with_cell_cleaner(row_xml, &params, CellCleanerMode::Regex)
    );
}

#[test]
fn byte_cell_cleaner_matches_regex_for_content_cells_with_advanced_cleanup() {
    let mut params = params_with_styles(&[]);
    params.advanced_cleanup = Some(Arc::new(AdvancedCleanupConfig {
        remove_unreferenced_whitespace: false,
        remove_unreferenced_hidden_content: true,
    }));
    let row_xml = br#"<row r="3" hidden="1"><c r="A3"><v>secret</v></c></row>"#;

    let mut regex_params = params.clone();
    regex_params.cell_cleaner_mode = CellCleanerMode::Regex;
    let (regex_cleaned, regex_metrics) = replace_single_row_with_metrics(row_xml, &regex_params);

    let mut byte_params = params;
    byte_params.cell_cleaner_mode = CellCleanerMode::Bytes;
    let (byte_cleaned, byte_metrics) = replace_single_row_with_metrics(row_xml, &byte_params);

    assert_eq!(byte_cleaned, regex_cleaned);
    assert_eq!(
        byte_metrics.hidden_candidates,
        regex_metrics.hidden_candidates
    );
    assert_eq!(
        byte_metrics.candidates.len(),
        regex_metrics.candidates.len()
    );
}
