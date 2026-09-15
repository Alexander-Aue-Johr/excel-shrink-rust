use super::{replace_empty_row_after_cell_range, replace_row};
use crate::models::{
    CleanSheetMetrics, CleanSheetParameters, CleanupSwitches, ColumnDefinition,
    SheetPreprocessConfig, WorkbookSpecificPatterns, Xf,
};
use crate::sheet::output::trim_trailing_layout_only_rows;
use crate::sheet::sheet_data::clean_sheet_rows_and_cells_in_cell_range;
use regex::bytes::Regex;
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

fn font_only_style() -> Xf {
    Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(0))
        .border_id(Some(0))
        .has_no_empty_cell_visible_style(true)
        .build_xf()
}

fn apply_font_only_style(font_id: u32) -> Xf {
    Xf::builder()
        .font_id(Some(font_id))
        .fill_id(Some(0))
        .border_id(Some(0))
        .apply_font(true)
        .has_no_empty_cell_visible_style(true)
        .build_xf()
}

fn visible_fill_style() -> Xf {
    Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(4))
        .border_id(Some(0))
        .has_visible_fill(true)
        .has_fill_or_border(true)
        .build_xf()
}

fn fill_style(fill_id: u32, apply_fill: bool) -> Xf {
    Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(fill_id))
        .border_id(Some(0))
        .apply_fill(apply_fill)
        .has_visible_fill(fill_id != 0)
        .has_fill_or_border(fill_id != 0)
        .has_apply_fill_or_border(apply_fill)
        .build_xf()
}

fn font_and_apply_fill_style(fill_id: u32) -> Xf {
    Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(fill_id))
        .border_id(Some(0))
        .apply_fill(true)
        .has_visible_fill(fill_id != 0)
        .has_fill_or_border(fill_id != 0)
        .has_apply_fill_or_border(true)
        .build_xf()
}

fn apply_font_and_fill_style(font_id: u32, fill_id: u32) -> Xf {
    Xf::builder()
        .font_id(Some(font_id))
        .fill_id(Some(fill_id))
        .border_id(Some(0))
        .apply_font(true)
        .apply_fill(true)
        .has_visible_fill(fill_id != 0)
        .has_fill_or_border(fill_id != 0)
        .has_apply_fill_or_border(true)
        .build_xf()
}

fn no_empty_cell_visible_style() -> Xf {
    font_only_style()
}

fn params_with_styles(styles: &[(u32, Xf)]) -> CleanSheetParameters {
    let mut xfs_by_index = HashMap::new();
    let mut no_empty_cell_visible_style_ids = Vec::new();
    let mut fill_or_border_or_protection_ids = Vec::new();

    for (id, xf) in styles {
        if xf.has_no_empty_cell_visible_style {
            no_empty_cell_visible_style_ids.push(*id);
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
            no_empty_cell_visible_style_ids,
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
    let mut metrics = CleanSheetMetrics::default();
    replace_single_row_with_metrics(row_xml, params, &mut metrics)
}

fn replace_single_row_with_metrics(
    row_xml: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let row_re = Regex::new(
        r#"(?s)<row\b((?:[^>]*\br="([^"]+?)")?(?:[^>]*\bhidden="(1)")?(?:[^>]*\bs="([^"]+?)")?(?:[^>]*\bht="([^"]+?)")?[^/>]*?)(?:/>|>(.*?)</row>)"#,
    )
    .unwrap();

    let caps = row_re.captures(row_xml).unwrap();

    replace_row(&caps, params, metrics)
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

fn metrics_with_column_style(style: Xf, hidden: bool) -> CleanSheetMetrics {
    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(style),
        hidden,
    });
    metrics.column_definitions.finalize();
    metrics
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
fn keeps_empty_cell_when_cell_style_has_no_empty_cell_visible_style_and_row_has_height() {
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
fn keeps_one_empty_no_empty_cell_visible_style_cell_per_style_when_row_has_no_custom_height() {
    let params = params_with_styles(&[(427, font_only_style()), (429, font_only_style())]);

    let row_xml = br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27"><c r="A15" s="427"/><c r="B15" s="427"/><c r="C15" s="429"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="15" spans="1:3" s="429" customFormat="1" ht="27"><c r="A15" s="427"/><c r="C15" s="429"/></row>"#
    );
}

#[test]
fn keeps_one_empty_no_empty_cell_visible_style_footer_cell_after_content() {
    let params = params_with_styles(&[(8, font_only_style()), (13, font_only_style())]);

    let row_xml = br#"<row r="16108" spans="1:20" x14ac:dyDescent="0.2"><c r="A16108" s="8" t="s"><v>41422</v></c><c r="B16108" s="13"/><c r="C16108" s="13"/><c r="D16108" s="13"/><c r="E16108" s="13"/><c r="F16108" s="13"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="16108" spans="1:2" x14ac:dyDescent="0.2"><c r="A16108" s="8" t="s"><v>41422</v></c><c r="B16108" s="13"/></row>"#
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
        br#"<row r="16107" spans="1:13" ht="15" x14ac:dyDescent="0.25"><c r="A16107" s="8" t="s"><v>41420</v></c><c r="B16107"/><c r="H16107" s="87"/><c r="M16107" s="18"/></row>"#
    );
}

#[test]
fn disables_default_styleless_representatives_without_disabling_font_style_representatives() {
    let mut params = params_with_styles(&[(8, font_only_style()), (87, font_only_style())]);
    params.cleanup = CleanupSwitches {
        keep_textless_default_font_representatives: false,
        ..CleanupSwitches::default()
    };

    let row_xml = br#"<row r="16107" spans="1:20" ht="15" x14ac:dyDescent="0.25"><c r="A16107" s="8" t="s"><v>41420</v></c><c r="B16107"/><c r="C16107"/><c r="H16107" s="87"/><c r="I16107" s="87"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="16107" spans="1:8" ht="15" x14ac:dyDescent="0.25"><c r="A16107" s="8" t="s"><v>41420</v></c><c r="H16107" s="87"/></row>"#
    );
}

#[test]
fn disables_font_style_representatives_without_disabling_default_styleless_representatives() {
    let mut params = params_with_styles(&[(8, font_only_style()), (87, font_only_style())]);
    params.cleanup = CleanupSwitches {
        keep_textless_style_representatives: false,
        ..CleanupSwitches::default()
    };

    let row_xml = br#"<row r="16107" spans="1:20" ht="15" x14ac:dyDescent="0.25"><c r="A16107" s="8" t="s"><v>41420</v></c><c r="B16107"/><c r="C16107"/><c r="H16107" s="87"/><c r="I16107" s="87"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="16107" spans="1:2" ht="15" x14ac:dyDescent="0.25"><c r="A16107" s="8" t="s"><v>41420</v></c><c r="B16107"/></row>"#
    );
}

#[test]
fn removes_empty_representatives_when_both_features_are_disabled() {
    let mut params = params_with_styles(&[
        (8, font_only_style()),
        (18, font_only_style()),
        (87, font_only_style()),
    ]);
    params.cleanup = CleanupSwitches {
        keep_textless_default_font_representatives: false,
        keep_textless_style_representatives: false,
        ..CleanupSwitches::default()
    };

    let row_xml = br#"<row r="16107" spans="1:20" ht="15" x14ac:dyDescent="0.25"><c r="A16107" s="8" t="s"><v>41420</v></c><c r="B16107"/><c r="C16107"/><c r="H16107" s="87"/><c r="I16107" s="87"/><c r="M16107" s="18"/><c r="N16107"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="16107" spans="1:1" ht="15" x14ac:dyDescent="0.25"><c r="A16107" s="8" t="s"><v>41420</v></c></row>"#
    );
}

#[test]
fn removes_empty_no_empty_cell_visible_style_cells_when_row_already_has_custom_height() {
    let params = params_with_styles(&[(427, font_only_style()), (429, font_only_style())]);

    let row_xml = br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27" customHeight="1"><c r="A15" s="427"/><c r="B15" s="429"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27" customHeight="1"/>"#
    );
}

#[test]
fn keeps_all_empty_cells_that_reset_custom_height_row_fill() {
    let row_red_style = fill_style(5, true);
    let default_fill_style = apply_font_only_style(4);
    let number_default_fill_style = apply_font_only_style(4);
    let params = params_with_styles(&[
        (272, default_fill_style),
        (288, number_default_fill_style),
        (456, row_red_style),
    ]);

    let row_xml = br#"<row r="54" spans="1:252" s="456" customFormat="1" ht="19.149999999999999" customHeight="1"><c r="B54" s="272"/><c r="C54" s="272"/><c r="I54" s="288"/><c r="J54" s="288"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(cleaned, row_xml);
}

#[test]
fn keeps_empty_default_font_cell_in_auto_height_row_to_preserve_spacing() {
    let text_style = apply_font_and_fill_style(109, 5);
    let column_style = apply_font_and_fill_style(114, 5);
    let default_font_fill_style = font_and_apply_fill_style(5);
    let params = params_with_styles(&[
        (1045, default_font_fill_style),
        (1760, column_style.clone()),
        (1766, text_style),
    ]);
    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 2,
        max: 3,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let row_xml = br#"<row r="8" spans="1:3" ht="12.75" x14ac:dyDescent="0.2"><c r="A8" s="1766"><v>2</v></c><c r="C8" s="1045"/></row>"#;

    let cleaned = replace_single_row_with_metrics(row_xml, &params, &mut metrics);

    assert_eq!(cleaned, row_xml);
}

#[test]
fn keeps_empty_cell_that_activates_column_font_in_auto_height_row() {
    let row_style = apply_font_and_fill_style(8, 2);
    let column_style = apply_font_and_fill_style(14, 2);
    let cell_style = apply_font_and_fill_style(14, 2);
    let params = params_with_styles(&[
        (161, row_style),
        (241, column_style.clone()),
        (1186, cell_style),
    ]);
    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 12,
        max: 12,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let row_xml = br#"<row r="91" spans="1:13" s="161" customFormat="1" ht="9.75" x14ac:dyDescent="0.2"><c r="L91" s="1186"/></row>"#;

    let cleaned = replace_single_row_with_metrics(row_xml, &params, &mut metrics);

    assert_eq!(cleaned, row_xml);
}

#[test]
fn keeps_empty_row_even_without_height_when_cells_are_removed() {
    let mut params = params_with_styles(&[]);
    params.cleanup = CleanupSwitches {
        keep_textless_default_font_representatives: false,
        ..CleanupSwitches::default()
    };

    let row_xml = br#"<row r="15"><c r="A15"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(cleaned, br#"<row r="15"/>"#);
}

#[test]
fn keeps_empty_row_after_cell_range_even_without_height() {
    let params = params_with_styles(&[]);

    let row_xml = br#"<row r="16"></row>"#;

    let cleaned = replace_single_empty_row_after_cell_range(row_xml, &params);

    assert_eq!(cleaned, br#"<row r="16"/>"#);
}

#[test]
fn keeps_empty_style_representatives_when_custom_height_row_has_content() {
    let params = params_with_styles(&[(427, font_only_style()), (429, font_only_style())]);

    let row_xml = br#"<row r="15" spans="1:8" s="429" customFormat="1" ht="27" customHeight="1"><c r="A15" s="8" t="s"><v>1</v></c><c r="B15" s="427"/><c r="C15" s="429"/><c r="D15" s="429"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="15" spans="1:3" s="429" customFormat="1" ht="27" customHeight="1"><c r="A15" s="8" t="s"><v>1</v></c><c r="B15" s="427"/><c r="C15" s="429"/></row>"#
    );
}

#[test]
fn keeps_structural_empty_style_anchor_when_custom_height_row_would_lose_style() {
    let params = params_with_styles(&[
        (6, relevant_style()),
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
fn keeps_empty_font_only_cell_when_custom_height_row_has_content() {
    let params = params_with_styles(&[
        (6, no_empty_cell_visible_style()),
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
fn keeps_empty_style_representative_in_hidden_custom_height_row_with_content() {
    let params = params_with_styles(&[(8, font_only_style()), (427, font_only_style())]);

    let row_xml = br#"<row r="15" hidden="1" spans="1:8" ht="27" customHeight="1"><c r="A15" s="8" t="s"><v>1</v></c><c r="B15" s="427"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(
        cleaned,
        br#"<row r="15" hidden="1" spans="1:8" ht="27" customHeight="1"><c r="A15" s="8" t="s"><v>1</v></c><c r="B15" s="427"/></row>"#
    );
}

#[test]
fn keeps_column_font_style_representative_without_tail_anchor_when_row_has_no_custom_height() {
    let column_style = apply_font_only_style(4);
    let cell_style = apply_font_only_style(8);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = metrics_with_column_style(column_style, false);

    let row_xml = br#"<row r="3"><c r="A3" s="8"/></row>"#;

    let cleaned = replace_single_row_with_metrics(row_xml, &params, &mut metrics);

    assert_eq!(cleaned, row_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert!(metrics.deleted_cell_states.is_empty());
    assert_eq!(metrics.tail_anchor_max_row, None);
}

#[test]
fn keeps_hidden_column_font_style_representative_without_tail_anchor_when_row_has_no_custom_height()
{
    let column_style = apply_font_only_style(4);
    let cell_style = apply_font_only_style(8);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = metrics_with_column_style(column_style, true);

    let row_xml = br#"<row r="3"><c r="A3" s="8"/></row>"#;

    let cleaned = replace_single_row_with_metrics(row_xml, &params, &mut metrics);

    assert_eq!(cleaned, row_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert!(metrics.deleted_cell_states.is_empty());
    assert_eq!(metrics.tail_anchor_max_row, None);
}

#[test]
fn keeps_column_font_style_representative_when_row_has_content_cell() {
    let column_style = apply_font_only_style(4);
    let cell_style = apply_font_only_style(8);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = metrics_with_column_style(column_style, false);

    let row_xml = br#"<row r="3"><c r="A3" s="8"/><c r="B3"><v>1</v></c></row>"#;

    let cleaned = replace_single_row_with_metrics(row_xml, &params, &mut metrics);

    assert_eq!(cleaned, row_xml);
    assert_eq!(metrics.kept_cell_states.len(), 2);
    assert!(metrics.deleted_cell_states.is_empty());
    assert_eq!(metrics.tail_anchor_max_row, Some(3));
}

#[test]
fn keeps_column_font_style_representative_when_only_other_content_cell_is_hidden() {
    let column_style = apply_font_only_style(4);
    let cell_style = apply_font_only_style(8);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.add(ColumnDefinition {
        min: 2,
        max: 2,
        cell_xf: None,
        hidden: true,
    });
    metrics.column_definitions.finalize();

    let row_xml = br#"<row r="3"><c r="A3" s="8"/><c r="B3"><v>1</v></c></row>"#;

    let cleaned = replace_single_row_with_metrics(row_xml, &params, &mut metrics);

    assert_eq!(cleaned, row_xml);
    assert_eq!(metrics.kept_cell_states.len(), 2);
    assert!(metrics.deleted_cell_states.is_empty());
}

#[test]
fn keeps_column_font_style_representative_when_row_has_visible_fill_cell() {
    let column_style = apply_font_only_style(4);
    let font_cell_style = apply_font_only_style(8);
    let fill_cell_style = visible_fill_style();
    let params = params_with_styles(&[
        (4, column_style.clone()),
        (8, font_cell_style),
        (9, fill_cell_style),
    ]);
    let mut metrics = metrics_with_column_style(column_style, false);

    let row_xml = br#"<row r="3"><c r="A3" s="8"/><c r="B3" s="9"/></row>"#;

    let cleaned = replace_single_row_with_metrics(row_xml, &params, &mut metrics);

    assert_eq!(cleaned, row_xml);
    assert_eq!(metrics.kept_cell_states.len(), 2);
    assert!(metrics.deleted_cell_states.is_empty());
    assert_eq!(metrics.tail_anchor_max_row, Some(3));
}

#[test]
fn visible_row_style_keeps_column_font_style_representative_as_tail_part() {
    let column_style = apply_font_only_style(4);
    let font_cell_style = apply_font_only_style(8);
    let row_style = visible_fill_style();
    let params = params_with_styles(&[
        (4, column_style.clone()),
        (8, font_cell_style),
        (9, row_style),
    ]);
    let mut metrics = metrics_with_column_style(column_style, false);

    let row_xml = br#"<row r="3" s="9"><c r="A3" s="8"/></row>"#;

    let cleaned = replace_single_row_with_metrics(row_xml, &params, &mut metrics);

    assert_eq!(cleaned, row_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert!(metrics.deleted_cell_states.is_empty());
    assert_eq!(metrics.tail_anchor_max_row, Some(3));
}

#[test]
fn removing_later_custom_height_representative_recomputes_tail_anchor() {
    let params = params_with_styles(&[]);
    let sheet_data = br#"<row r="10"><c r="A10"><v>keep</v></c></row><row r="20" customFormat="1" ht="12.75" customHeight="1"><c r="A20"/></row>"#;
    let mut metrics = CleanSheetMetrics::default();

    let cleaned = clean_sheet_rows_and_cells_in_cell_range(sheet_data, &params, &mut metrics)
        .expect("row cleanup should succeed");

    assert_eq!(metrics.tail_anchor_max_row, Some(10));
    assert!(!String::from_utf8_lossy(&cleaned).contains(r#"r="A20""#));
    let trimmed = trim_trailing_layout_only_rows(&cleaned, &params, &mut metrics);
    let trimmed = String::from_utf8_lossy(&trimmed);

    assert!(trimmed.contains(r#"r="10""#));
    assert!(trimmed.contains(r#"<v>keep</v>"#));
    assert!(!trimmed.contains(r#"r="20""#));
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
fn postprocess_preserves_hidden_empty_rows_but_cleans_attributes() {
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

#[test]
fn row_view_borrows_header_and_style_in_arbitrary_attribute_order() {
    use crate::sheet::row_view::RowView;
    let params = params_with_styles(&[(7, relevant_style())]);
    let attributes = br#" customHeight='1' data-note='hidden="1" r="999"' ht = "12" s='7' r='42' spans='1:8' hidden='0'"#;
    let row = RowView::parse(attributes, &params).unwrap();
    assert_eq!(row.reference, b"42");
    assert_eq!(row.number, Some(42));
    assert!(!row.hidden);
    assert!(row.has_height && row.has_custom_height);
    assert!(std::ptr::eq(
        row.raw_attributes.as_ptr(),
        attributes.as_ptr()
    ));
    assert!(std::ptr::eq(
        row.style.unwrap(),
        &params.patterns.xfs_by_index[&7]
    ));
    let (start, end) = row.spans.unwrap();
    assert_eq!(&attributes[start..end], b"1:8");
}

#[test]
fn invalid_row_header_is_preserved_without_partial_cleanup() {
    use super::replace_row_bytes;
    let params = params_with_styles(&[]);
    let raw = br#"<row r="7" hidden="1><c r="A7"/></row>"#;
    let mut metrics = CleanSheetMetrics::default();
    let result = replace_row_bytes(
        raw,
        br#" r="7" hidden="1"#,
        Some(br#"<c r="A7"/>"#),
        &params,
        &mut metrics,
    );
    assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
    assert_eq!(result.as_ref(), raw);
    assert!(metrics.kept_cell_states.is_empty() && metrics.deleted_cell_states.is_empty());
}

#[test]
fn self_closing_row_stays_borrowed_and_still_records_visible_style() {
    let params = params_with_styles(&[(7, relevant_style())]);
    let raw = br#"<row r="42" s="7"/>"#;
    let attr = br#" r="42" s="7""#;
    for after_range in [false, true] {
        let mut metrics = CleanSheetMetrics::default();
        let result = if after_range {
            super::replace_empty_row_after_cell_range_bytes(raw, attr, &params, &mut metrics)
        } else {
            super::replace_row_bytes(raw, attr, None, &params, &mut metrics)
        };
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
        assert_eq!(result.as_ref(), raw);
        assert_eq!(metrics.visible_content_bounds.max_row, Some(42));
        assert_eq!(metrics.tail_anchor_max_row, Some(42));
    }
}

#[test]
fn recleaned_row_does_not_reuse_old_content_or_remove_old_cell_records() {
    let params = params_with_styles(&[(9, font_only_style())]);
    let mut metrics = CleanSheetMetrics::default();
    let original =
        br#"<row r="3" customHeight="1"><c r="A3"><v>keep</v></c><c r="B3" s="9"/></row>"#;
    let first = replace_single_row_with_metrics(original, &params, &mut metrics);
    assert_eq!(first, original);
    let previous_cells = metrics.kept_cell_states.len();
    // The reference pass removed the content cell. The remaining representative
    // must be judged against the current row, even with older metrics present.
    let changed = br#"<row r="3" customHeight="1"><c r="B3" s="9"/></row>"#;
    let second = replace_single_row_with_metrics(changed, &params, &mut metrics);
    assert_eq!(second, br#"<row r="3" customHeight="1"/>"#);
    assert_eq!(metrics.kept_cell_states.len(), previous_cells);
    assert_eq!(metrics.kept_cell_states[1].col, 2);
}

#[test]
fn spans_use_only_current_row_cells_after_recleaning() {
    let mut params = params_with_styles(&[]);
    params.cleanup.keep_textless_default_font_representatives = false;
    let mut metrics = CleanSheetMetrics::default();
    let previous = br#"<row r="3" spans="1:26"><c r="Z3"><v>old</v></c></row>"#;
    replace_single_row_with_metrics(previous, &params, &mut metrics);
    let current = br#"<row r="3" spans="1:26"><c r="A3"><v>keep</v></c><c r="Z3"/></row>"#;
    let cleaned = replace_single_row_with_metrics(current, &params, &mut metrics);
    assert_eq!(
        cleaned,
        br#"<row r="3" spans="1:1"><c r="A3"><v>keep</v></c></row>"#
    );
}

#[test]
fn spans_exclude_pruned_representatives_but_include_retained_visible_cells() {
    let params = params_with_styles(&[(7, relevant_style()), (9, font_only_style())]);
    let original =
        br#"<row r="3" spans="1:26" customHeight="1"><c r="A3" s="7"/><c r="Z3" s="9"/></row>"#;
    assert_eq!(
        replace_single_row(original, &params),
        br#"<row r="3" spans="1:1" customHeight="1"><c r="A3" s="7"/></row>"#
    );
}

#[test]
fn cell_attributes_are_scanned_once_in_any_order_and_quoted_values_are_skipped() {
    let params = params_with_styles(&[(7, relevant_style())]);
    let original = br#"<row r="3"><c s = '7' data-note='r="Z99" s="0"' r='A3'/></row>"#;
    let mut metrics = CleanSheetMetrics::default();
    let cleaned =
        clean_sheet_rows_and_cells_in_cell_range(original, &params, &mut metrics).unwrap();
    assert_eq!(cleaned, original);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.kept_cell_states[0].col, 1);
    assert_eq!(metrics.kept_cell_states[0].row, 3);
}

#[test]
fn spans_match_numeric_row_references_with_leading_zeroes() {
    let mut params = params_with_styles(&[]);
    params.cleanup.keep_textless_default_font_representatives = false;
    let original = br#"<row r="03" spans="1:26"><c r="A3"><v>keep</v></c><c r="Z3"/></row>"#;
    assert_eq!(
        replace_single_row(original, &params),
        br#"<row r="03" spans="1:1"><c r="A3"><v>keep</v></c></row>"#
    );
}
