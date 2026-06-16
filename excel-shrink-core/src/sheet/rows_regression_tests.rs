use super::{replace_empty_row_after_cell_range, replace_row};
use crate::models::{
    CleanSheetMetrics, CleanSheetParameters, SheetPreprocessConfig, WorkbookSpecificPatterns, Xf,
};
use crate::sheet::sheet_data::clean_sheet_rows_and_cells_in_cell_range;
use regex::bytes::Regex;
use std::collections::HashMap;
use zip::CompressionMethod;

fn relevant_style() -> Xf {
    Xf::new(Some(2), Some(1), true, true, true, true, false, false)
}

fn font_only_style() -> Xf {
    Xf::new(Some(0), Some(0), false, false, false, false, true, false)
}

fn non_font_empty_style() -> Xf {
    Xf::new(Some(0), Some(0), false, false, false, false, false, false)
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
        cleanup: Default::default(),
        collect_cleanup_stats: false,
    }
}

fn replace_single_row(row_xml: &[u8], params: &CleanSheetParameters) -> Vec<u8> {
    let row_re = Regex::new(
        r#"(?s)<row\b((?:[^>]*\br="([^"]+?)")?(?:[^>]*\bhidden="(1)")?(?:[^>]*\bs="([^"]+?)")?(?:[^>]*\bht="([^"]+?)")?[^/>]*?)(?:/>|>(.*?)</row>)"#,
    )
    .unwrap();

    let caps = row_re.captures(row_xml).unwrap();
    let mut metrics = CleanSheetMetrics::default();

    replace_row(&caps, params, &mut metrics)
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
fn keeps_empty_cell_when_cell_style_has_only_font_style_and_row_has_height() {
    let params = params_with_styles(&[(427, font_only_style()), (429, font_only_style())]);

    let row_xml =
        br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27"><c r="A15" s="427"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27"><c r="A15" s="427"/></row>"#
    );
}

#[test]
fn keeps_empty_font_only_cells_when_row_has_no_custom_height() {
    let params = params_with_styles(&[(427, font_only_style()), (429, font_only_style())]);

    let row_xml = br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27"><c r="A15" s="427"/><c r="B15" s="427"/><c r="C15" s="429"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27"><c r="A15" s="427"/><c r="B15" s="427"/><c r="C15" s="429"/></row>"#
    );
}

#[test]
fn keeps_repeated_empty_font_only_footer_cells_after_content() {
    let params = params_with_styles(&[(8, font_only_style()), (13, font_only_style())]);

    let row_xml = br#"<row r="16108" spans="1:20" x14ac:dyDescent="0.2"><c r="A16108" s="8" t="s"><v>41422</v></c><c r="B16108" s="13"/><c r="C16108" s="13"/><c r="D16108" s="13"/><c r="E16108" s="13"/><c r="F16108" s="13"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="16108" spans="1:20" x14ac:dyDescent="0.2"><c r="A16108" s="8" t="s"><v>41422</v></c><c r="B16108" s="13"/><c r="C16108" s="13"/><c r="D16108" s="13"/><c r="E16108" s="13"/><c r="F16108" s="13"/></row>"#
    );
}

#[test]
fn keeps_one_empty_default_style_footer_cell_after_content() {
    let params = params_with_styles(&[
        (8, font_only_style()),
        (18, font_only_style()),
        (87, font_only_style()),
    ]);

    let row_xml = br#"<row r="16107" spans="1:20" ht="15" x14ac:dyDescent="0.25"><c r="A16107" s="8" t="s"><v>41420</v></c><c r="B16107"/><c r="C16107"/><c r="H16107" s="87"/><c r="I16107" s="87"/><c r="M16107" s="18"/><c r="N16107"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="16107" spans="1:20" ht="15" x14ac:dyDescent="0.25"><c r="A16107" s="8" t="s"><v>41420</v></c><c r="B16107"/><c r="H16107" s="87"/><c r="I16107" s="87"/><c r="M16107" s="18"/></row>"#
    );
}

#[test]
fn removes_empty_font_only_cells_when_row_already_has_custom_height() {
    let params = params_with_styles(&[(427, font_only_style()), (429, font_only_style())]);

    let row_xml = br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27" customHeight="1"><c r="A15" s="427"/><c r="B15" s="429"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27" customHeight="1"/>"#
    );
}

#[test]
fn keeps_structural_empty_style_anchor_when_custom_height_row_would_lose_style() {
    let params = params_with_styles(&[
        (6, non_font_empty_style()),
        (12, font_only_style()),
        (47, font_only_style()),
    ]);

    let row_xml = br#"<row r="29" spans="1:13" s="12" customFormat="1" ht="13.2" customHeight="1"><c r="A29" s="12" t="s"><v>1</v></c><c r="C29" s="6"/><c r="D29" s="47" t="s"><v>2</v></c></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="29" spans="1:13" s="12" customFormat="1" ht="13.2" customHeight="1"><c r="A29" s="12" t="s"><v>1</v></c><c r="C29" s="6"/><c r="D29" s="47" t="s"><v>2</v></c></row>"#
    );
}

#[test]
fn does_not_add_custom_height_to_already_empty_row_with_height() {
    let params = params_with_styles(&[(429, font_only_style())]);

    let row_xml = br#"<row r="16" customFormat="1" ht="27"/>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(cleaned, br#"<row r="16" customFormat="1" ht="27"/>"#);
}

#[test]
fn does_not_add_custom_height_to_empty_row_after_cell_range_with_height() {
    let params = params_with_styles(&[(429, font_only_style())]);

    let row_xml = br#"<row r="16" customFormat="1" ht="27"/>"#;

    let cleaned = replace_single_empty_row_after_cell_range(row_xml, &params);

    assert_eq!(cleaned, br#"<row r="16" customFormat="1" ht="27"/>"#);
}

#[test]
fn postprocess_preserves_consecutive_hidden_empty_rows_but_cleans_attributes() {
    let params = params_with_styles(&[]);
    let sheet_data = br#"<row r="307905" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307906" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307907" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307908" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row>"#;
    let mut metrics = CleanSheetMetrics::default();

    let cleaned = clean_sheet_rows_and_cells_in_cell_range(sheet_data, &params, &mut metrics)
        .expect("hidden empty rows should clean");
    let cleaned = String::from_utf8(cleaned).unwrap();

    assert!(
        cleaned.contains(r#"<row r="307905" spans="1:117" hidden="1" x14ac:dyDescent="0.25"/>"#)
    );
    assert!(
        cleaned.contains(r#"<row r="307906" spans="1:117" hidden="1" x14ac:dyDescent="0.25"/>"#)
    );
    assert!(
        cleaned.contains(r#"<row r="307907" spans="1:117" hidden="1" x14ac:dyDescent="0.25"/>"#)
    );
    assert!(
        cleaned.contains(r#"<row r="307908" spans="1:117" hidden="1" x14ac:dyDescent="0.25"/>"#)
    );
    assert!(cleaned.contains("dyDescent"));
}
