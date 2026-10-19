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
fn preserves_font_representatives_even_at_fixed_height() {
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
fn preserves_distinct_font_only_cell_at_fixed_height() {
    let params = params_with_styles(&[
        (6, no_empty_cell_visible_style()),
        (12, font_only_style()),
        (47, font_only_style()),
    ]);

    let row_xml = br#"<row r="29" spans="1:13" s="12" customFormat="1" ht="13.2" customHeight="1"><c r="A29" s="12" t="s"><v>1</v></c><c r="C29" s="6"/><c r="D29" s="47" t="s"><v>2</v></c></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(cleaned, row_xml);
}

#[test]
fn preserves_font_representative_in_hidden_fixed_height_row() {
    let params = params_with_styles(&[(8, font_only_style()), (427, font_only_style())]);

    let row_xml = br#"<row r="15" hidden="1" spans="1:8" ht="27" customHeight="1"><c r="A15" s="8" t="s"><v>1</v></c><c r="B15" s="427"/></row>"#;

    let cleaned = replace_single_row(row_xml, &params);

    assert_eq!(cleaned, row_xml);
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
    assert!((metrics.deleted_cell_count == 0));
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
    assert!((metrics.deleted_cell_count == 0));
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
    assert!((metrics.deleted_cell_count == 0));
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
    assert!((metrics.deleted_cell_count == 0));
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
    assert!((metrics.deleted_cell_count == 0));
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
    assert!((metrics.deleted_cell_count == 0));
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
    assert!(metrics.kept_cell_states.is_empty() && (metrics.deleted_cell_count == 0));
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
    assert_eq!(metrics.kept_cell_states[0].col, 1);
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

fn reference_cell_block(bytes: &[u8]) -> Option<(usize, bool)> {
    let tag_end = crate::xml::find_byte_from(bytes, b'>', 0)?;
    let end = if tag_end > 0 && bytes[tag_end - 1] == b'/' {
        tag_end + 1
    } else {
        tag_end + 1 + crate::xml::find_bytes(&bytes[tag_end + 1..], b"</c>")? + 4
    };
    Some((end, crate::sheet::cells::has_content(&bytes[..end])))
}

#[test]
fn fused_cell_scan_preserves_content_and_boundary_semantics() {
    let headers = ["<c r=\"A1\">", "<c a=\"<v>\">", "<c>"];
    let contents = [
        "",
        "<v>1</v>",
        "<f/>",
        "<f x=\"1\">A1</f>",
        "<is><t>text</t></is>",
        "<foo/>",
        "<f\tx=\"1\"/>",
        "<!-- <v> -->",
        "<extension/>",
        "<is >x</is>",
    ];
    for header in headers {
        for content in contents {
            let xml = format!("{header}{content}</c><c><v>next</v></c>");
            for end in 0..=xml.len() {
                let bytes = &xml.as_bytes()[..end];
                assert_eq!(
                    super::parse_cell_block(bytes, 0).map(|c| (c.end, c.has_content)),
                    reference_cell_block(bytes)
                );
            }
        }
    }
    for bytes in [
        b"<c/>".as_slice(),
        b"<c a=\"<v>\"/>",
        b"<c/><c><v>next</v></c>",
    ] {
        assert_eq!(
            super::parse_cell_block(bytes, 0).map(|c| (c.end, c.has_content)),
            reference_cell_block(bytes)
        );
    }
}

#[test]
#[ignore = "manual timing with CELL_SCAN_FIXTURE; no timing assertion"]
fn benchmark_fused_cell_scan() {
    let bytes =
        std::fs::read(std::env::var("CELL_SCAN_FIXTURE").expect("set CELL_SCAN_FIXTURE")).unwrap();
    let mut cells = Vec::new();
    let mut position = 0;
    while let Some(relative) = crate::xml::find_bytes(&bytes[position..], b"<c ") {
        let start = position + relative;
        let (length, _) = reference_cell_block(&bytes[start..]).unwrap();
        cells.push(&bytes[start..start + length]);
        position = start + length;
    }
    assert!(!cells.is_empty());
    for candidate in [false, true, true, false, true, false, false, true] {
        let start = std::time::Instant::now();
        let mut checksum = 0;
        for _ in 0..100 {
            for &cell in &cells {
                let cell = std::hint::black_box(cell);
                let (end, content) = if candidate {
                    let block = super::parse_cell_block(cell, 0).unwrap();
                    (block.end, block.has_content)
                } else {
                    reference_cell_block(cell).unwrap()
                };
                checksum += end + usize::from(content);
            }
        }
        std::hint::black_box(checksum);
        eprintln!(
            "candidate={candidate} cells={} elapsed_us={} checksum={checksum}",
            cells.len(),
            start.elapsed().as_micros()
        );
    }
}

fn promotion_params() -> CleanSheetParameters {
    let xml = br#"<styleSheet><cellXfs count="4">
      <xf fontId="0" fillId="0" borderId="0" numFmtId="0" xfId="0"/>
      <xf fontId="8" fillId="0" borderId="0" numFmtId="0" xfId="0" applyFont="1"/>
      <xf fontId="8" fillId="0" borderId="0" numFmtId="0" xfId="0" applyFont="1"><alignment vertical="center"/></xf>
      <xf fontId="4" fillId="0" borderId="0" numFmtId="0" xfId="0" applyFont="1"/>
    </cellXfs></styleSheet>"#;
    let mut params = params_with_styles(&[]);
    params.patterns = WorkbookSpecificPatterns::new(crate::styles::parse_xfs(xml).unwrap(), None);
    params
}

fn promotion_columns(
    params: &CleanSheetParameters,
    trailing_font_matches: bool,
) -> CleanSheetMetrics {
    let mut metrics = CleanSheetMetrics::default();
    for (min, max, style) in [
        (1, 4, 3),
        (5, 16384, if trailing_font_matches { 1 } else { 3 }),
    ] {
        metrics.column_definitions.add(ColumnDefinition {
            min,
            max,
            cell_xf: Some(params.patterns.xfs_by_index[&style].clone()),
            hidden: false,
        });
    }
    metrics.column_definitions.finalize();
    metrics
}

const PROMOTION_ROW: &[u8] = br#"<row r="1" ht="9" customHeight="1"><c r="A1" s="2"><v>unchanged text</v></c><c r="B1" s="2"/><c r="C1" s="2"/><c r="D1" s="2"/></row>"#;

#[test]
fn explicit_row_font_keeps_cell_overrides_even_without_content_or_automatic_height() {
    let params = promotion_params();
    for height in ["", " ht=\"9\" customHeight=\"1\""] {
        for content in ["", "<c r=\"C1\" s=\"3\"><v>keep</v></c>"] {
            let input = format!("<row r=\"1\" s=\"2\" customFormat=\"1\"{height}><c r=\"A1\" s=\"3\"/><c r=\"B1\" s=\"3\"/>{content}</row>");
            let cleaned = replace_single_row_with_metrics(
                input.as_bytes(), &params, &mut promotion_columns(&params, false));
            assert_eq!(cleaned, input.as_bytes());
        }
    }
}

#[test]
fn row_font_promotion_reuses_existing_font_xf_and_preserves_text() {
    let params = promotion_params();
    assert_eq!(params.patterns.xfs_by_index[&2].row_font_style_id, Some(1));
    let mut metrics = promotion_columns(&params, true);
    let result = replace_single_row_with_metrics(PROMOTION_ROW, &params, &mut metrics);
    assert_eq!(result, br#"<row r="1" ht="9" customHeight="1" s="1" customFormat="1"><c r="A1" s="2"><v>unchanged text</v></c></row>"#);
    assert_eq!(metrics.deleted_cell_count, 3);
    let second =
        replace_single_row_with_metrics(&result, &params, &mut promotion_columns(&params, true));
    assert_eq!(result, second);
}

#[test]
fn row_font_promotion_rejects_different_implicit_font_or_unstyled_value() {
    let params = promotion_params();
    let a = replace_single_row_with_metrics(
        PROMOTION_ROW,
        &params,
        &mut promotion_columns(&params, false),
    );
    assert!(!String::from_utf8(a).unwrap().contains("customFormat"));
    let unstyled = String::from_utf8(PROMOTION_ROW.to_vec())
        .unwrap()
        .replace("r=\"A1\" s=\"2\"", "r=\"A1\"");
    let b = replace_single_row_with_metrics(
        unstyled.as_bytes(),
        &params,
        &mut promotion_columns(&params, true),
    );
    assert!(!String::from_utf8(b).unwrap().contains("customFormat"));
}

#[test]
fn row_font_promotion_keeps_existing_row_formats_and_merged_rows() {
    let params = promotion_params();
    let mut metrics = promotion_columns(&params, true);
    metrics.merge_cell_ranges.push(crate::models::CellRange {
        start_row: 1,
        end_row: 1,
        start_col: 1,
        end_col: 2,
    });
    let a = replace_single_row_with_metrics(PROMOTION_ROW, &params, &mut metrics);
    assert!(!String::from_utf8(a).unwrap().contains("customFormat"));
    let existing = String::from_utf8(PROMOTION_ROW.to_vec())
        .unwrap()
        .replace("<row ", "<row s=\"3\" customFormat=\"1\" ");
    let b = replace_single_row_with_metrics(
        existing.as_bytes(),
        &params,
        &mut promotion_columns(&params, true),
    );
    assert!(String::from_utf8(b)
        .unwrap()
        .starts_with("<row s=\"3\" customFormat=\"1\""));
}

#[test]
fn dense_full_width_row_compacts_identical_empty_styles_into_row_header() {
    let params = promotion_params();
    let mut input = String::from("<row r=\"1\" ht=\"9\" customHeight=\"1\">");
    for col in 1..=16384 {
        let mut n = col;
        let mut name = Vec::new();
        while n > 0 {
            n -= 1;
            name.push(b'A' + (n % 26) as u8);
            n /= 26;
        }
        name.reverse();
        let name = String::from_utf8(name).unwrap();
        if col == 1 {
            input.push_str("<c r=\"A1\" s=\"3\"><v>keep</v></c>");
        } else {
            input.push_str(&format!("<c r=\"{name}1\" s=\"2\"/>"));
        }
    }
    input.push_str("</row>");
    let mut metrics = promotion_columns(&params, false);
    let result = replace_single_row_with_metrics(input.as_bytes(), &params, &mut metrics);
    assert_eq!(result, br#"<row r="1" ht="9" customHeight="1" s="2" customFormat="1"><c r="A1" s="3"><v>keep</v></c></row>"#);
    assert_eq!(metrics.deleted_cell_count, 16383);
    let mut merged = promotion_columns(&params, false);
    merged.merge_cell_ranges.push(crate::models::CellRange {
        start_col: 3,
        end_col: 4,
        start_row: 1,
        end_row: 1,
    });
    let with_merge = replace_single_row_with_metrics(input.as_bytes(), &params, &mut merged);
    let text = String::from_utf8(with_merge).unwrap();
    assert!(text.contains("s=\"2\" customFormat=\"1\""));
    assert!(text.contains("<c r=\"C1\" s=\"2\"/>"));
    assert!(text.contains("<c r=\"D1\" s=\"2\"/>"));
    assert_eq!(merged.deleted_cell_count, 16381);
}

#[test]
fn column_font_coverage_checks_exact_overrides_inside_ranges() {
    let params = promotion_params();
    let mut metrics = promotion_columns(&params, true);
    metrics.column_definitions.add(ColumnDefinition {
        min: 50,
        max: 50,
        cell_xf: Some(params.patterns.xfs_by_index[&3].clone()),
        hidden: true,
    });
    metrics.column_definitions.finalize();
    assert!(!metrics.column_definitions.has_uniform_font(5, 16384, 8));
    assert!(metrics.column_definitions.has_uniform_font(51, 16384, 8));
}

#[test]
fn row_font_promotion_does_not_replace_a_different_inherited_fill() {
    let params = promotion_params();
    let mut metrics = promotion_columns(&params, true);
    let mut colored = params.patterns.xfs_by_index[&1].clone();
    colored.fill_id = Some(4);
    colored.has_visible_fill = true;
    colored.has_fill_or_border = true;
    colored.non_font_style_id = Some(999);
    metrics.column_definitions.add(ColumnDefinition {
        min: 50,
        max: 50,
        cell_xf: Some(colored),
        hidden: false,
    });
    metrics.column_definitions.finalize();
    let result = replace_single_row_with_metrics(PROMOTION_ROW, &params, &mut metrics);
    assert!(!String::from_utf8(result).unwrap().contains("customFormat"));
}

#[test]
fn row_font_promotion_reuses_full_style_when_other_inherited_components_match() {
    let params = promotion_params();
    let mut metrics = promotion_columns(&params, true);
    metrics.column_definitions.add(ColumnDefinition {
        min: 5,
        max: 16384,
        cell_xf: Some(params.patterns.xfs_by_index[&2].clone()),
        hidden: false,
    });
    metrics.column_definitions.finalize();
    let result = replace_single_row_with_metrics(PROMOTION_ROW, &params, &mut metrics);
    assert_eq!(result, br#"<row r="1" ht="9" customHeight="1" s="2" customFormat="1"><c r="A1" s="2"><v>unchanged text</v></c></row>"#);
}

#[test]
fn hidden_font_representatives_are_deduplicated_across_style_ids() {
    for aggressive in [false, true] {
        let mut params = params_with_styles(&[(1, apply_font_only_style(3)),
            (2, apply_font_only_style(3)), (3, apply_font_only_style(4))]);
        params.cleanup.keep_textless_style_representatives = !aggressive;
        params.cleanup.keep_textless_column_style_representatives = !aggressive;
        let mut metrics = CleanSheetMetrics::default();
        metrics.column_definitions.add(ColumnDefinition {
            min: 1, max: 3, cell_xf: Some(apply_font_only_style(4)), hidden: true,
        });
        metrics.column_definitions.finalize();
        let cleaned = replace_single_row_with_metrics(
            br#"<row r="3"><c r="A3" s="1"/><c r="B3" s="2"/><c r="C3" s="1"/><c r="D3" s="3"><v>1</v></c></row>"#,
            &params, &mut metrics);
        let text = String::from_utf8(cleaned).unwrap();
        assert!(text.contains("r=\"A3\""));
        assert!(!text.contains("r=\"B3\""));
        assert!(!text.contains("r=\"C3\""));
    }
}

#[test]
fn preceding_visible_content_represents_hidden_font() {
    let params = params_with_styles(&[(1, apply_font_only_style(3)), (2, apply_font_only_style(3))]);
    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 2, max: 3, cell_xf: Some(apply_font_only_style(4)), hidden: true,
    });
    metrics.column_definitions.finalize();
    let cleaned = replace_single_row_with_metrics(
        br#"<row r="3"><c r="A3" s="1"><v>1</v></c><c r="B3" s="2"/><c r="C3" s="1"/></row>"#,
        &params, &mut metrics);
    let text = String::from_utf8(cleaned).unwrap();
    assert!(text.contains("r=\"A3\""));
    assert!(!text.contains("r=\"B3\""));
    assert!(!text.contains("r=\"C3\""));
}

#[test]
fn removes_repeated_empty_cells_when_column_font_records_are_identical() {
    let params = duplicate_column_font_params(false);
    for height in ["", " ht=\"12.75\"", " ht=\"12.75\" customHeight=\"1\""] {
        let mut metrics = duplicate_column_font_metrics(&params);
        let row = format!("<row r=\"1\"{height}><c r=\"A1\" s=\"1\"><v>7</v></c><c r=\"B1\" s=\"1\"/><c r=\"C1\" s=\"1\"/></row>");
        let cleaned = replace_single_row_with_metrics(row.as_bytes(), &params, &mut metrics);
        let expected = format!("<row r=\"1\"{height}><c r=\"A1\" s=\"1\"><v>7</v></c><c r=\"B1\" s=\"1\"/></row>");
        assert_eq!(cleaned, expected.as_bytes());
        assert_eq!(metrics.deleted_cell_count, 1);
    }
}

#[test]
fn keeps_empty_column_font_overrides_when_font_records_differ() {
    let params = duplicate_column_font_params(true);
    for height in ["", " ht=\"12.75\"", " ht=\"12.75\" customHeight=\"1\""] {
        let mut metrics = duplicate_column_font_metrics(&params);
        let row = format!("<row r=\"1\"{height}><c r=\"A1\" s=\"1\"><v>7</v></c><c r=\"B1\" s=\"1\"/><c r=\"C1\" s=\"1\"/></row>");
        let cleaned = replace_single_row_with_metrics(row.as_bytes(), &params, &mut metrics);
        assert_eq!(cleaned, row.as_bytes());
        assert_eq!(metrics.deleted_cell_count, 0);
    }
}

fn duplicate_column_font_params(different_font: bool) -> CleanSheetParameters {
    let font_size = if different_font { 12 } else { 10 };
    let styles = format!(
        r#"<styleSheet><fonts count="3">
      <font><sz val="11"/><name val="Calibri"/></font>
      <font><sz val="10"/><color theme="1"/><name val="Arial"/><family val="2"/></font>
      <font><sz val="{font_size}"/><color theme="1"/><name val="Arial"/><family val="2"/></font>
    </fonts><cellXfs count="3">
      <xf fontId="0" fillId="0" borderId="0" numFmtId="0" xfId="0"/>
      <xf fontId="1" fillId="0" borderId="0" numFmtId="0" xfId="11" applyFont="1"/>
      <xf fontId="2" fillId="0" borderId="0" numFmtId="0" xfId="11"/>
    </cellXfs></styleSheet>"#
    );
    let mut params = params_with_styles(&[]);
    params.patterns = WorkbookSpecificPatterns::new(
        crate::styles::parse_xfs(styles.as_bytes()).unwrap(),
        None,
    );
    params
}

fn duplicate_column_font_metrics(params: &CleanSheetParameters) -> CleanSheetMetrics {
    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 2,
        max: 3,
        cell_xf: Some(params.patterns.xfs_by_index[&2].clone()),
        hidden: false,
    });
    metrics.column_definitions.finalize();
    metrics
}
