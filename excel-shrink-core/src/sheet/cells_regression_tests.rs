use super::{replace_parsed_cell, replace_parsed_cell_with_empty_style_policy, ParsedCell};
#[cfg(feature = "cell-tracing")]
use crate::cleaning_trace::{self, CellDecision, RecordedCellDecision};
#[cfg(feature = "cell-tracing")]
use crate::models::CleanupCause;
use crate::models::{
    AdvancedCleanupConfig, CleanSheetMetrics, CleanSheetParameters, ColumnDefinition,
    InactiveProtectionCellPolicy, SheetPreprocessConfig, WorkbookSpecificPatterns, Xf,
};
use crate::shared_strings::find_whitespace_sst_indices;
use crate::sheet::row_view::RowView;

use std::collections::HashMap;
use std::sync::Arc;
use zip::CompressionMethod;

fn default_params() -> CleanSheetParameters {
    CleanSheetParameters {
        xlsx_file_path: "test.xlsx".to_string(),
        zip_info_filename: "xl/worksheets/sheet1.xml".to_string(),
        compress_method: CompressionMethod::Deflated,
        patterns: WorkbookSpecificPatterns {
            xfs_by_index: HashMap::new(),
            no_empty_cell_visible_style_ids: Vec::new(),
            fill_or_border_or_protection_ids: Vec::new(),
            whitespace_indices: None,
        },
        advanced_cleanup: None,
        preprocess: SheetPreprocessConfig::default(),
        cleanup: Default::default(),
        collect_cleanup_stats: false,
    }
}

fn params_with_styles(styles: &[(u32, Xf)]) -> CleanSheetParameters {
    let mut xfs_by_index = HashMap::new();
    let mut no_empty_cell_visible_style_ids = Vec::new();
    let mut fill_or_border_or_protection_ids = Vec::new();

    for (id, xf) in styles {
        if xf.has_no_empty_cell_visible_style {
            no_empty_cell_visible_style_ids.push(*id);
        }

        if xf.has_fill_or_border
            || xf.applies_default_fill_or_border_override()
            || xf.has_apply_protection
        {
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

fn hidden_row_3() -> RowView<'static, 'static> {
    RowView {
        promoted_style_id: None,
        raw_attributes: br#" r="3" hidden="1""#,
        reference: b"3",
        hidden: true,
        style: None,
        has_height: false,
        has_custom_height: false,
        number: Some(3),
        spans: None,
    }
}

fn parsed_a3_cell<'a>(
    raw: &'a [u8],
    attr: &'a [u8],
    style_id: Option<u32>,
) -> ParsedCell<'a, 'static> {
    ParsedCell {
        column: None,
        row_view: None,
        raw,
        attr,
        reference_attr: br#"r="A3""#,
        row: b"3",
        col_num: 1,
        style_id,
    }
}

fn parsed_x4_cell<'a>(
    raw: &'a [u8],
    attr: &'a [u8],
    style_id: Option<u32>,
) -> ParsedCell<'a, 'static> {
    ParsedCell {
        column: None,
        row_view: None,
        raw,
        attr,
        reference_attr: br#"r="X4""#,
        row: b"4",
        col_num: 24,
        style_id,
    }
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

fn border_style(border_id: u32, apply_border: bool) -> Xf {
    Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(0))
        .border_id(Some(border_id))
        .apply_border(apply_border)
        .has_visible_border(border_id != 0)
        .has_fill_or_border(border_id != 0)
        .has_apply_fill_or_border(apply_border)
        .build_xf()
}

fn font_and_apply_fill_style(fill_id: u32) -> Xf {
    font_and_apply_fill_style_with_font(0, fill_id)
}

fn font_and_apply_fill_style_with_font(font_id: u32, fill_id: u32) -> Xf {
    Xf::builder()
        .font_id(Some(font_id))
        .fill_id(Some(fill_id))
        .border_id(Some(0))
        .apply_fill(true)
        .has_visible_fill(fill_id != 0)
        .has_fill_or_border(fill_id != 0)
        .has_apply_fill_or_border(true)
        .build_xf()
}

fn font_and_apply_default_border_style() -> Xf {
    Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(0))
        .border_id(Some(0))
        .apply_border(true)
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

fn font_only_style_with_font(font_id: u32) -> Xf {
    Xf::builder()
        .font_id(Some(font_id))
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

fn font_and_apply_default_fill_style() -> Xf {
    Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(0))
        .border_id(Some(0))
        .apply_fill(true)
        .has_apply_fill_or_border(true)
        .build_xf()
}

fn apply_protection_style() -> Xf {
    Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(0))
        .border_id(Some(0))
        .has_apply_protection(true)
        .build_xf()
}

fn empty_style_without_visible_parts() -> Xf {
    Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(0))
        .border_id(Some(0))
        .build_xf()
}

fn shared_strings_with_self_closing_text_at(index: usize) -> Vec<u8> {
    let mut xml = Vec::new();
    xml.extend_from_slice(b"<sst>");
    for _ in 0..index {
        xml.extend_from_slice(b"<si><t>value</t></si>");
    }
    xml.extend_from_slice(b"<si><t /></si></sst>");
    xml
}

#[test]
fn keeps_hidden_cell_with_content_without_advanced_cleanup() {
    let params = default_params();
    let mut metrics = CleanSheetMetrics::default();

    let row_view = hidden_row_3();

    let cell_xml = br#"<c r="A3"><v>secret</v></c>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3""#, None);
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn removes_empty_hidden_cell_without_style_representative() {
    let params = default_params();
    let mut metrics = CleanSheetMetrics::default();

    let row_view = hidden_row_3();

    let cell_xml = br#"<c r="A3"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3""#, None);
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn keeps_empty_hidden_column_cell_when_style_has_visible_fill() {
    let params = params_with_styles(&[(7, fill_style(2, true))]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: None,
        hidden: true,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="7"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="7""#, Some(7));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert!(matches!(cleaned, std::borrow::Cow::Borrowed(_)));
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn keeps_empty_cell_when_apply_font_differs_from_column_apply_font() {
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
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
    assert_eq!(metrics.tail_anchor_max_row, None);
}

#[test]
fn removes_empty_hidden_column_cell_when_apply_font_differs_from_column_apply_font() {
    let column_style = apply_font_only_style(4);
    let cell_style = apply_font_only_style(8);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: true,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
    assert_eq!(metrics.tail_anchor_max_row, None);
}

#[test]
fn removes_empty_hidden_column_cell_when_only_font_differs_from_column_font() {
    let column_style = apply_font_only_style(4);
    let cell_style = apply_font_only_style(8);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 16_384,
        max: 16_384,
        cell_xf: Some(column_style),
        hidden: true,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="XFD49" s="8"/>"#;
    let cell = ParsedCell {
        column: None,
        row_view: None,
        raw: cell_xml,
        attr: br#" r="XFD49" s="8""#,
        reference_attr: br#"r="XFD49""#,
        row: b"49",
        col_num: 16_384,
        style_id: Some(8),
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
    assert_eq!(metrics.tail_anchor_max_row, None);
}

#[test]
fn keeps_empty_cell_when_nonzero_font_differs_from_column_without_cell_apply_font() {
    let column_style = apply_font_only_style(4);
    let cell_style = font_only_style_with_font(8);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
    assert_eq!(metrics.tail_anchor_max_row, None);
}

#[test]
fn nonzero_font_override_stays_explicit_when_it_duplicates_font_zero() {
    let column_style = apply_font_only_style(4);
    let mut cell_style = font_only_style_with_font(8);
    cell_style.font_identity_id = Some(0);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();
    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn keeps_empty_cell_when_apply_font_resets_column_font_to_default() {
    let column_style = apply_font_only_style(4);
    let cell_style = apply_font_only_style(0);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
    assert_eq!(metrics.tail_anchor_max_row, None);
}

#[test]
fn keeps_empty_cell_when_apply_font_differs_from_column_font_without_column_apply_font() {
    let column_style = font_and_apply_fill_style(5);
    let cell_style = apply_font_and_fill_style(8, 5);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
    assert_eq!(metrics.tail_anchor_max_row, Some(3));
}

#[test]
fn removes_empty_cell_when_apply_font_matches_column_apply_font() {
    let column_style = apply_font_only_style(4);
    let cell_style = apply_font_only_style(4);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn removes_empty_cell_when_apply_font_matches_column_font_without_column_apply_font() {
    let column_style = font_and_apply_fill_style_with_font(8, 5);
    let cell_style = apply_font_and_fill_style(8, 5);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn removes_empty_cell_when_default_font_differs_from_column_without_cell_apply_font() {
    let column_style = apply_font_only_style(4);
    let cell_style = font_only_style();
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn keeps_empty_default_font_cell_in_auto_height_row_when_column_font_differs() {
    let column_style = apply_font_and_fill_style(117, 5);
    let cell_style = font_and_apply_fill_style(5);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();
    let row_view = RowView {
        promoted_style_id: None,
        raw_attributes: br#" r="3" ht="12.75""#,
        reference: b"3",
        hidden: false,
        style: None,
        has_height: true,
        has_custom_height: false,
        number: Some(3),
        spans: None,
    };

    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn keeps_empty_cell_in_auto_height_row_when_column_font_matches_cell_but_row_font_differs() {
    let row_style = apply_font_and_fill_style(8, 2);
    let column_style = apply_font_and_fill_style(14, 2);
    let cell_style = apply_font_and_fill_style(14, 2);
    let params = params_with_styles(&[
        (161, row_style.clone()),
        (241, column_style.clone()),
        (1186, cell_style),
    ]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();
    let row_view = RowView {
        promoted_style_id: None,
        raw_attributes: br#" r="3" s="161" customFormat="1" ht="9.75""#,
        reference: b"3",
        hidden: false,
        style: Some(&row_style),
        has_height: true,
        has_custom_height: false,
        number: Some(3),
        spans: None,
    };

    let cell_xml = br#"<c r="A3" s="1186"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="1186""#, Some(1186));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn removes_empty_cell_when_default_font_and_same_fill_do_not_apply_font() {
    let column_style = apply_font_and_fill_style(117, 5);
    let cell_style = font_and_apply_fill_style(5);
    let params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn opt_out_removes_empty_cell_when_apply_font_differs_from_column_apply_font() {
    let column_style = apply_font_only_style(4);
    let cell_style = apply_font_only_style(8);
    let mut params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
    params.cleanup.keep_textless_column_style_representatives = false;
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="8"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="8""#, Some(8));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn keeps_formula_cell_with_attributes_and_empty_cached_value() {
    let params = default_params();
    let mut metrics = CleanSheetMetrics::default();

    let cell_xml =
        br#"<c r="A3" s="39" t="str"><f t="shared" ref="A3:A10" si="0">IF(B3=1,"","")</f><v/></c>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="39" t="str""#, Some(39));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn keeps_empty_cell_when_style_applies_default_border_override() {
    let params = params_with_styles(&[(43, font_and_apply_default_border_style())]);
    let mut metrics = CleanSheetMetrics::default();

    let cell_xml = br#"<c r="F2" s="43"/>"#;
    let cell = ParsedCell {
        column: None,
        row_view: None,
        raw: cell_xml,
        attr: br#" r="F2" s="43""#,
        reference_attr: br#"r="F2""#,
        row: b"2",
        col_num: 6,
        style_id: Some(43),
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn removes_empty_cell_when_default_border_override_matches_borderless_column_style() {
    let column_style = font_only_style();
    let cell_default_border_style = font_and_apply_default_border_style();
    let params = params_with_styles(&[(2, column_style.clone()), (43, cell_default_border_style)]);

    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="43"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="43""#, Some(43));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn keeps_empty_cell_when_default_border_override_clears_visible_row_border() {
    let row_border_style = border_style(4, true);
    let cell_default_border_style = font_and_apply_default_border_style();
    let params = params_with_styles(&[
        (4, row_border_style.clone()),
        (43, cell_default_border_style),
    ]);

    let mut metrics = CleanSheetMetrics::default();
    let row_view = RowView {
        promoted_style_id: None,
        raw_attributes: br#" r="3" s="4" customFormat="1""#,
        reference: b"3",
        hidden: false,
        style: Some(&row_border_style),
        has_height: false,
        has_custom_height: false,
        number: Some(3),
        spans: None,
    };

    let cell_xml = br#"<c r="A3" s="43"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="43""#, Some(43));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn keeps_empty_cell_when_style_applies_default_fill_override() {
    let params = params_with_styles(&[(44, font_and_apply_default_fill_style())]);
    let mut metrics = CleanSheetMetrics::default();

    let cell_xml = br#"<c r="G2" s="44"/>"#;
    let cell = ParsedCell {
        column: None,
        row_view: None,
        raw: cell_xml,
        attr: br#" r="G2" s="44""#,
        reference_attr: br#"r="G2""#,
        row: b"2",
        col_num: 7,
        style_id: Some(44),
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn removes_empty_cell_when_default_fill_override_matches_fillless_column_style() {
    let column_style = font_only_style();
    let cell_default_fill_style = font_and_apply_default_fill_style();
    let params = params_with_styles(&[(2, column_style.clone()), (44, cell_default_fill_style)]);

    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="44"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="44""#, Some(44));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn keeps_empty_cell_when_default_fill_override_clears_visible_row_fill() {
    let row_fill_style = fill_style(4, true);
    let cell_default_fill_style = font_and_apply_default_fill_style();
    let params = params_with_styles(&[(4, row_fill_style.clone()), (44, cell_default_fill_style)]);

    let mut metrics = CleanSheetMetrics::default();
    let row_view = RowView {
        promoted_style_id: None,
        raw_attributes: br#" r="3" s="4" customFormat="1""#,
        reference: b"3",
        hidden: false,
        style: Some(&row_fill_style),
        has_height: false,
        has_custom_height: false,
        number: Some(3),
        spans: None,
    };

    let cell_xml = br#"<c r="A3" s="44"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="44""#, Some(44));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn does_not_mark_hidden_formula_cell_as_advanced_cleanup_candidate() {
    let mut params = default_params();

    params.advanced_cleanup = Some(Arc::new(AdvancedCleanupConfig {
        remove_unreferenced_whitespace: false,
        remove_unreferenced_hidden_content: true,
    }));

    let mut metrics = CleanSheetMetrics::default();
    let row_view = hidden_row_3();

    let cell_xml = br#"<c r="A3" s="39" t="str"><f t="shared" si="0"/><v/></c>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="39" t="str""#, Some(39));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.hidden_candidates, 0);
    assert_eq!(metrics.candidates.len(), 0);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn marks_hidden_content_as_candidate_with_advanced_cleanup() {
    let mut params = default_params();

    params.advanced_cleanup = Some(Arc::new(AdvancedCleanupConfig {
        remove_unreferenced_whitespace: false,
        remove_unreferenced_hidden_content: true,
    }));

    let mut metrics = CleanSheetMetrics::default();

    let row_view = hidden_row_3();

    let cell_xml = br#"<c r="A3"><v>secret</v></c>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3""#, None);
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.hidden_candidates, 1);
    assert_eq!(metrics.candidates.len(), 1);
    assert_eq!(metrics.candidates[0].col, 1);
    assert_eq!(metrics.candidates[0].row, 3);
}

#[test]
fn does_not_mark_hidden_content_candidate_when_cell_is_style_representative() {
    let mut params = default_params();

    params.advanced_cleanup = Some(Arc::new(AdvancedCleanupConfig {
        remove_unreferenced_whitespace: false,
        remove_unreferenced_hidden_content: true,
    }));

    let mut metrics = CleanSheetMetrics::default();
    let row_view = hidden_row_3();

    let cell_xml = br#"<c r="A3"><v>secret</v></c>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3""#, None);
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell_with_empty_style_policy(
        &cell,
        &params,
        &mut metrics,
        true,
        false,
        false,
        true,
    );

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.hidden_candidates, 0);
    assert!(metrics.candidates.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 1);
}

#[test]
fn does_not_mark_candidate_when_cell_style_has_apply_protection_and_sheet_is_protected() {
    let protected_style = apply_protection_style();

    let mut params = params_with_styles(&[(7, protected_style)]);

    params.advanced_cleanup = Some(Arc::new(AdvancedCleanupConfig {
        remove_unreferenced_whitespace: false,
        remove_unreferenced_hidden_content: true,
    }));

    let mut metrics = CleanSheetMetrics::default();
    metrics.sheet_protection_enabled = true;

    let row_view = hidden_row_3();

    let cell_xml = br#"<c r="A3" s="7"><v>secret</v></c>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="7""#, Some(7));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.hidden_candidates, 1);
    assert_eq!(metrics.candidates.len(), 0);
}

#[test]
fn default_marks_candidate_when_cell_style_has_apply_protection_without_sheet_protection() {
    let protected_style = apply_protection_style();

    let mut params = params_with_styles(&[(7, protected_style)]);

    params.advanced_cleanup = Some(Arc::new(AdvancedCleanupConfig {
        remove_unreferenced_whitespace: false,
        remove_unreferenced_hidden_content: true,
    }));

    let mut metrics = CleanSheetMetrics::default();
    let row_view = hidden_row_3();

    let cell_xml = br#"<c r="A3" s="7"><v>secret</v></c>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="7""#, Some(7));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.hidden_candidates, 1);
    assert_eq!(metrics.candidates.len(), 1);
    assert_eq!(metrics.candidates[0].col, 1);
    assert_eq!(metrics.candidates[0].row, 3);
}

#[test]
fn protection_only_cell_still_anchors_tail_when_column_style_is_visible() {
    let column_style = fill_style(4, false);
    let protected_style = apply_protection_style();
    let mut params = params_with_styles(&[(4, column_style.clone()), (7, protected_style)]);
    params.cleanup.inactive_protection_cell_policy = InactiveProtectionCellPolicy::KeepAll;
    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="7"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="7""#, Some(7));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert!(metrics.kept_cell_states[0].kept_only_for_protection);
    assert_eq!(metrics.tail_anchor_max_row, Some(3));
}

#[test]
fn keep_all_does_not_mark_candidate_when_cell_style_has_apply_protection_without_sheet_protection()
{
    let protected_style = apply_protection_style();

    let mut params = params_with_styles(&[(7, protected_style)]);

    params.advanced_cleanup = Some(Arc::new(AdvancedCleanupConfig {
        remove_unreferenced_whitespace: false,
        remove_unreferenced_hidden_content: true,
    }));
    params.cleanup.inactive_protection_cell_policy = InactiveProtectionCellPolicy::KeepAll;

    let mut metrics = CleanSheetMetrics::default();
    let row_view = hidden_row_3();

    let cell_xml = br#"<c r="A3" s="7"><v>secret</v></c>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="7""#, Some(7));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.hidden_candidates, 1);
    assert_eq!(metrics.candidates.len(), 0);
}

#[test]
fn keeps_empty_font_only_cell_when_row_has_non_default_fill() {
    let row_red_style = fill_style(4, true);

    let cell_font_only_style = apply_font_only_style(4);

    let params = params_with_styles(&[(4, row_red_style.clone()), (10, cell_font_only_style)]);

    let mut metrics = CleanSheetMetrics::default();

    let row_view = RowView {
        promoted_style_id: None,
        raw_attributes: br#" r="3" s="4" customFormat="1""#,
        reference: b"3",
        hidden: false,
        style: Some(&row_red_style),
        has_height: false,
        has_custom_height: false,
        number: Some(3),
        spans: None,
    };

    let cell_xml = br#"<c r="A3" s="10"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="10""#, Some(10));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn removes_empty_cell_with_fill_zero_when_column_has_non_default_fill_without_apply_fill() {
    let column_red_style = fill_style(4, false);

    let cell_default_fill_style = empty_style_without_visible_parts();

    let params =
        params_with_styles(&[(4, column_red_style.clone()), (10, cell_default_fill_style)]);

    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_red_style),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="10"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="10""#, Some(10));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn removes_empty_cell_when_row_and_column_apply_same_fill_as_cell() {
    let inherited_default_fill = fill_style(0, true);
    let cell_default_fill_style = fill_style(0, false);

    let params = params_with_styles(&[
        (4, inherited_default_fill.clone()),
        (10, cell_default_fill_style),
    ]);

    let mut metrics = CleanSheetMetrics::default();

    let row_view = RowView {
        promoted_style_id: None,
        raw_attributes: br#" r="3" s="4" customFormat="1""#,
        reference: b"3",
        hidden: false,
        style: Some(&inherited_default_fill),
        has_height: false,
        has_custom_height: false,
        number: Some(3),
        spans: None,
    };

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(inherited_default_fill.clone()),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="10"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="10""#, Some(10));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn keeps_empty_cell_when_row_and_column_apply_fill_and_only_column_differs() {
    let row_default_fill = fill_style(0, true);
    let column_red_fill = fill_style(4, true);
    let cell_default_fill_style = fill_style(0, false);

    let params = params_with_styles(&[
        (4, row_default_fill.clone()),
        (5, column_red_fill.clone()),
        (10, cell_default_fill_style),
    ]);

    let mut metrics = CleanSheetMetrics::default();

    let row_view = RowView {
        promoted_style_id: None,
        raw_attributes: br#" r="3" s="4" customFormat="1""#,
        reference: b"3",
        hidden: false,
        style: Some(&row_default_fill),
        has_height: false,
        has_custom_height: false,
        number: Some(3),
        spans: None,
    };

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_red_fill),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="10"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="10""#, Some(10));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn removes_empty_cell_when_row_applies_same_visible_fill_as_cell() {
    let row_red_fill = fill_style(4, true);
    let cell_red_fill = fill_style(4, false);

    let params = params_with_styles(&[(4, row_red_fill.clone()), (10, cell_red_fill)]);

    let mut metrics = CleanSheetMetrics::default();

    let row_view = RowView {
        promoted_style_id: None,
        raw_attributes: br#" r="3" s="4" customFormat="1""#,
        reference: b"3",
        hidden: false,
        style: Some(&row_red_fill),
        has_height: false,
        has_custom_height: false,
        number: Some(3),
        spans: None,
    };

    let cell_xml = br#"<c r="A3" s="10"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="10""#, Some(10));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn removes_empty_cell_when_column_applies_same_visible_fill_as_cell() {
    let column_red_fill = fill_style(4, true);
    let cell_red_fill = fill_style(4, false);

    let params = params_with_styles(&[(4, column_red_fill.clone()), (10, cell_red_fill)]);

    let mut metrics = CleanSheetMetrics::default();

    metrics.column_definitions.add(ColumnDefinition {
        min: 1,
        max: 1,
        cell_xf: Some(column_red_fill),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="A3" s="10"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="10""#, Some(10));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
}

#[test]
fn keeps_empty_cell_when_no_row_or_column_apply_fill_and_cell_has_visible_fill() {
    let cell_red_fill = fill_style(4, false);
    let params = params_with_styles(&[(10, cell_red_fill)]);

    let mut metrics = CleanSheetMetrics::default();

    let cell_xml = br#"<c r="A3" s="10"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="10""#, Some(10));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn keeps_empty_cell_when_row_applies_fill_and_cell_overrides_with_default_fill() {
    let row_red_fill = fill_style(4, true);
    let cell_default_fill = fill_style(0, false);

    let params = params_with_styles(&[(4, row_red_fill.clone()), (10, cell_default_fill)]);

    let mut metrics = CleanSheetMetrics::default();

    let row_view = RowView {
        promoted_style_id: None,
        raw_attributes: br#" r="3" s="4" customFormat="1""#,
        reference: b"3",
        hidden: false,
        style: Some(&row_red_fill),
        has_height: false,
        has_custom_height: false,
        number: Some(3),
        spans: None,
    };

    let cell_xml = br#"<c r="A3" s="10"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="10""#, Some(10));
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..cell
    };

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_count, 0);
}

#[test]
fn marks_x4_whitespace_candidate_when_cell_and_column_apply_default_fill() {
    let cell_style_91 = font_and_apply_fill_style(0);
    let column_style_74 = font_and_apply_fill_style(0);
    let mut params = params_with_styles(&[(74, column_style_74.clone()), (91, cell_style_91)]);
    params.cleanup.unreferenced_whitespace_cells = true;
    params.patterns.whitespace_indices = Some(vec![15801]);

    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 15,
        max: 16_384,
        cell_xf: Some(column_style_74),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="X4" s="91" t="s"><v>15801</v></c>"#;
    let cell = parsed_x4_cell(cell_xml, br#" r="X4" s="91" t="s""#, Some(91));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.whitespace_candidates, 1);
    assert_eq!(metrics.candidates.len(), 1);
    assert_eq!(metrics.candidates[0].col, 24);
    assert_eq!(metrics.candidates[0].row, 4);
}

#[test]
fn does_not_mark_whitespace_candidate_when_cell_is_style_representative() {
    let cell_style_91 = font_and_apply_fill_style(0);
    let column_style_74 = font_and_apply_fill_style(0);
    let mut params = params_with_styles(&[(74, column_style_74.clone()), (91, cell_style_91)]);
    params.cleanup.unreferenced_whitespace_cells = true;
    params.patterns.whitespace_indices = Some(vec![15801]);

    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 15,
        max: 16_384,
        cell_xf: Some(column_style_74),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="X4" s="91" t="s"><v>15801</v></c>"#;
    let cell = parsed_x4_cell(cell_xml, br#" r="X4" s="91" t="s""#, Some(91));

    let cleaned = replace_parsed_cell_with_empty_style_policy(
        &cell,
        &params,
        &mut metrics,
        true,
        true,
        false,
        false,
    );

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(metrics.whitespace_candidates, 0);
    assert!(metrics.candidates.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 1);
}

#[test]
fn marks_x4_whitespace_candidate_when_shared_string_text_is_self_closing() {
    let cell_style_91 = font_and_apply_fill_style(0);
    let column_style_74 = font_and_apply_fill_style(0);
    let mut params = params_with_styles(&[(74, column_style_74.clone()), (91, cell_style_91)]);
    params.cleanup.unreferenced_whitespace_cells = true;

    let shared_strings = shared_strings_with_self_closing_text_at(15801);
    params.patterns.whitespace_indices = Some(find_whitespace_sst_indices(&shared_strings));

    let mut metrics = CleanSheetMetrics::default();
    metrics.column_definitions.add(ColumnDefinition {
        min: 15,
        max: 16_384,
        cell_xf: Some(column_style_74),
        hidden: false,
    });
    metrics.column_definitions.finalize();

    let cell_xml = br#"<c r="X4" s="91" t="s"><v>15801</v></c>"#;
    let cell = parsed_x4_cell(cell_xml, br#" r="X4" s="91" t="s""#, Some(91));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned.as_ref(), cell_xml);
    assert_eq!(
        params.patterns.whitespace_indices.as_deref(),
        Some(&[15801][..])
    );
    assert_eq!(metrics.whitespace_candidates, 1);
    assert_eq!(metrics.candidates.len(), 1);
    assert_eq!(metrics.candidates[0].col, 24);
    assert_eq!(metrics.candidates[0].row, 4);
}

#[cfg(feature = "cell-tracing")]
#[test]
fn traces_d1048576_removed_when_empty_cell_style_only_affects_font() {
    fn parsed_d1048576_cell<'a>(
        raw: &'a [u8],
        attr: &'a [u8],
        style_id: Option<u32>,
    ) -> ParsedCell<'a, 'static> {
        ParsedCell {
            column: None,
            row_view: None,
            raw,
            attr,
            reference_attr: br#"r="D1048576""#,
            row: b"1048576",
            col_num: 4,
            style_id,
        }
    }

    let params = params_with_styles(&[(12, font_only_style())]);
    let mut metrics = CleanSheetMetrics::default();
    cleaning_trace::clear();

    let cell_xml = br#"<c r="D1048576" s="12"/>"#;
    let cell = parsed_d1048576_cell(cell_xml, br#" r="D1048576" s="12""#, Some(12));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert!(cleaned.is_empty());
    assert_eq!(metrics.kept_cell_states.len(), 0);
    assert_eq!(metrics.deleted_cell_count, 1);
    assert_eq!(
        cleaning_trace::recorded_cell_decisions(),
        vec![RecordedCellDecision {
            sheet_file: "xl/worksheets/sheet1.xml".to_string(),
            col: 4,
            row: b"1048576".to_vec(),
            style_id: Some(12),
            decision: CellDecision::Removed,
            cause: CleanupCause::EmptyCellsWithoutContent,
        }]
    );
}

#[test]
fn row_context_does_not_apply_to_a_different_cell_reference() {
    let row_view = hidden_row_3();
    let mut params = default_params();
    params.cleanup.unreferenced_hidden_content_cells = true;
    let mut metrics = CleanSheetMetrics::default();
    let raw = br#"<c r="X4"><v>visible</v></c>"#;
    let cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..parsed_x4_cell(raw, br#" r="X4""#, None)
    };
    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);
    assert_eq!(cleaned.as_ref(), raw);
    assert!(metrics.candidates.is_empty());

    let hidden_raw = br#"<c r="A3"><v>hidden</v></c>"#;
    let hidden_cell = ParsedCell {
        column: None,
        row_view: Some(&row_view),
        ..parsed_a3_cell(hidden_raw, br#" r="A3""#, None)
    };
    replace_parsed_cell(&hidden_cell, &params, &mut metrics);
    assert_eq!(metrics.candidates.len(), 1);
}

#[test]
fn fixed_height_still_requires_column_font_override_protection() {
    for keep in [false, true] {
        for fixed in [false, true] {
            for visible_border in [false, true] {
                let column_style = apply_font_only_style(4);
                let mut cell_style = apply_font_only_style(8);
                if visible_border {
                    cell_style.border_id = Some(1);
                    cell_style.has_visible_border = true;
                    cell_style.has_fill_or_border = true;
                    cell_style.has_no_empty_cell_visible_style = false;
                }
                let mut params = params_with_styles(&[(4, column_style.clone()), (8, cell_style)]);
                params.cleanup.keep_textless_column_style_representatives = keep;
                let mut metrics = CleanSheetMetrics::default();
                metrics.column_definitions.add(ColumnDefinition {
                    min: 1,
                    max: 1,
                    cell_xf: Some(column_style),
                    hidden: false,
                });
                metrics.column_definitions.finalize();
                let row = RowView {
                    promoted_style_id: None,
                    raw_attributes: br#" r="3" ht="20.25""#,
                    reference: b"3",
                    hidden: false,
                    style: None,
                    has_height: true,
                    has_custom_height: fixed,
                    number: Some(3),
                    spans: None,
                };
                let raw = br#"<c r="A3" s="8"/>"#;
                let cell = ParsedCell {
                    row_view: Some(&row),
                    ..parsed_a3_cell(raw, br#" r="A3" s="8""#, Some(8))
                };
                let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);
                assert_eq!(
                    !cleaned.is_empty(),
                    visible_border || keep,
                    "fixed={fixed} keep={keep} border={visible_border}"
                );
            }
        }
    }
}

#[test]
fn hidden_column_content_preserves_automatic_row_height() {
    for hidden_row in [false, true] {
        for fixed_height in [false, true] {
            let mut params = default_params();
            params.cleanup.unreferenced_hidden_content_cells = true;
            params.cleanup.unreferenced_whitespace_cells = true;
            let mut metrics = CleanSheetMetrics::default();
            metrics.column_definitions.add(ColumnDefinition {
                min: 1, max: 1, cell_xf: None, hidden: true,
            });
            metrics.column_definitions.finalize();
            let row = RowView {
                promoted_style_id: None, raw_attributes: b"", reference: b"3",
                hidden: hidden_row, style: None, has_height: true,
                has_custom_height: fixed_height, number: Some(3), spans: None,
            };
            let raw = br#"<c r="A3" t="inlineStr"><is><t>Heading
second line</t></is></c>"#;
            let cell = ParsedCell {
                column: None, row_view: Some(&row),
                ..parsed_a3_cell(raw, br#" r="A3" t="inlineStr""#, None)
            };
            let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);
            assert_eq!(cleaned.as_ref(), raw);
            assert_eq!(metrics.candidates.len(), usize::from(hidden_row || fixed_height));
        }
    }
}
