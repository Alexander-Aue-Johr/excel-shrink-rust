use super::{replace_parsed_cell, CellView};
use crate::models::{
    AdvancedCleanupConfig, CellCleanerMode, CleanSheetMetrics, CleanSheetParameters,
    ColumnDefinition, RowAttributes, SheetPreprocessConfig, WorkbookSpecificPatterns, Xf,
};

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
            only_font_style_ids: Vec::new(),
            fill_or_border_or_protection_ids: Vec::new(),
            whitespace_indices: None,
        },
        advanced_cleanup: None,
        preprocess: SheetPreprocessConfig::default(),
        cell_cleaner_mode: CellCleanerMode::Regex,
    }
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

fn hidden_row_3() -> RowAttributes {
    RowAttributes {
        row_attr: br#" r="3" hidden="1""#.to_vec(),
        ref_: b"3".to_vec(),
        hidden: true,
        style: None,
        has_height: false,
        is_empty: false,
    }
}

fn parsed_a3_cell<'a>(raw: &'a [u8], attr: &'a [u8], style_id: Option<u32>) -> CellView<'a> {
    CellView {
        raw,
        opening_tag: raw,
        attributes: attr,
        reference: Some(b"A3"),
        column_ref: Some(b"A"),
        row_ref: Some(b"3"),
        column: Some(1),
        row: Some(3),
        style_id,
        is_self_closing: raw.ends_with(b"/>"),
        has_content: raw.contains(&b'>') && !raw.ends_with(b"/>"),
    }
}

#[test]
fn keeps_hidden_cell_with_content_without_advanced_cleanup() {
    let params = default_params();
    let mut metrics = CleanSheetMetrics::default();

    metrics.row_attributes.insert(b"3".to_vec(), hidden_row_3());

    let cell_xml = br#"<c r="A3"><v>secret</v></c>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3""#, None);

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned, cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_states.len(), 0);
}

#[test]
fn marks_hidden_content_as_candidate_with_advanced_cleanup() {
    let mut params = default_params();

    params.advanced_cleanup = Some(Arc::new(AdvancedCleanupConfig {
        remove_unreferenced_whitespace: false,
        remove_unreferenced_hidden_content: true,
    }));

    let mut metrics = CleanSheetMetrics::default();

    metrics.row_attributes.insert(b"3".to_vec(), hidden_row_3());

    let cell_xml = br#"<c r="A3"><v>secret</v></c>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3""#, None);

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    // Zelle wird in Phase 1 noch nicht geloescht, sondern nur als Kandidat gespeichert.
    assert_eq!(cleaned, cell_xml);
    assert_eq!(metrics.hidden_candidates, 1);
    assert_eq!(metrics.candidates.len(), 1);
    assert_eq!(metrics.candidates[0].column, b"A");
    assert_eq!(metrics.candidates[0].row, b"3");
}

#[test]
fn does_not_mark_candidate_when_cell_style_has_apply_protection() {
    let protected_style = Xf::new(Some(0), Some(0), false, false, false, false, false, true);

    let mut params = params_with_styles(&[(7, protected_style)]);

    params.advanced_cleanup = Some(Arc::new(AdvancedCleanupConfig {
        remove_unreferenced_whitespace: false,
        remove_unreferenced_hidden_content: true,
    }));

    let mut metrics = CleanSheetMetrics::default();

    metrics.row_attributes.insert(b"3".to_vec(), hidden_row_3());

    let cell_xml = br#"<c r="A3" s="7"><v>secret</v></c>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="7""#, Some(7));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    // Zelle bleibt erhalten und ist KEIN Loesch-Kandidat, da sie direkt durch den Excel-Style geschuetzt ist.
    assert_eq!(cleaned, cell_xml);
    assert_eq!(metrics.hidden_candidates, 1);
    assert_eq!(metrics.candidates.len(), 0);
}

#[test]
fn keeps_empty_cell_with_fill_zero_when_row_has_non_default_fill_without_apply_fill() {
    let row_red_style = Xf::new(Some(4), Some(0), false, false, true, false, false, false);

    let cell_default_fill_style =
        Xf::new(Some(0), Some(0), false, false, false, false, true, false);

    let params = params_with_styles(&[(4, row_red_style.clone()), (10, cell_default_fill_style)]);

    let mut metrics = CleanSheetMetrics::default();

    metrics.row_attributes.insert(
        b"3".to_vec(),
        RowAttributes {
            row_attr: br#" r="3" s="4" customFormat="1""#.to_vec(),
            ref_: b"3".to_vec(),
            hidden: false,
            style: Some(row_red_style),
            has_height: false,
            is_empty: false,
        },
    );

    let cell_xml = br#"<c r="A3" s="10"/>"#;
    let cell = parsed_a3_cell(cell_xml, br#" r="A3" s="10""#, Some(10));

    let cleaned = replace_parsed_cell(&cell, &params, &mut metrics);

    assert_eq!(cleaned, cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_states.len(), 0);
}

#[test]
fn keeps_empty_cell_with_fill_zero_when_column_has_non_default_fill_without_apply_fill() {
    let column_red_style = Xf::new(Some(4), Some(0), false, false, true, false, false, false);

    let cell_default_fill_style =
        Xf::new(Some(0), Some(0), false, false, false, false, true, false);

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

    assert_eq!(cleaned, cell_xml);
    assert_eq!(metrics.kept_cell_states.len(), 1);
    assert_eq!(metrics.deleted_cell_states.len(), 0);
}
