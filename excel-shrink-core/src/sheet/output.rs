use crate::error::Result;
use crate::models::{CleanSheetParameters, CleanedSheetData, CleanupCause, Xf};
use crate::sheet::cols::merge_extraneous_columns;
use crate::sheet::ranges::{
    get_max_cell_column, get_max_merge_cells_column_index, get_max_visible_column_index,
    retrieve_sheet_data_range_containing_cells,
};
use crate::sheet::sheet_data::{
    clean_empty_rows_from_after_cell_range, clean_sheet_rows_and_cells_in_cell_range,
};
use crate::sheet::streaming::PreprocessedSheetXml;
use crate::xml::{find_byte_from, find_bytes, rfind_bytes};

pub fn build_clean_sheet_xml(
    preprocessed: PreprocessedSheetXml,
    params: &CleanSheetParameters,
    disable_sheetdata_splitting: bool,
) -> Result<CleanedSheetData> {
    let mut metrics = preprocessed.clean_sheet_metrics;
    metrics.cleanup_stats.enabled |= params.collect_cleanup_stats;

    let use_range_tail_pruning =
        !disable_sheetdata_splitting && params.cleanup.sheetdata_range_tail_pruning;

    let final_sheet_data = if !use_range_tail_pruning {
        clean_sheet_rows_and_cells_in_cell_range(&preprocessed.sheet_data, params, &mut metrics)?
    } else {
        let original_len = preprocessed.sheet_data.len();
        let ranges = retrieve_sheet_data_range_containing_cells(&preprocessed.sheet_data, params);
        let kept_len = ranges.cells_range.len() + ranges.rows_range.len();

        if original_len > kept_len {
            metrics.cleanup_stats.record_removed(
                CleanupCause::SheetDataRangeTailPruning,
                original_len - kept_len,
            );
        }

        let cleaned_cells_range =
            clean_sheet_rows_and_cells_in_cell_range(&ranges.cells_range, params, &mut metrics)?;

        let cleaned_visible_rows_range =
            clean_empty_rows_from_after_cell_range(&ranges.rows_range, params, &mut metrics)?;

        [cleaned_cells_range, cleaned_visible_rows_range].concat()
    };

    let final_sheet_data = if params.cleanup.trailing_layout_only_rows {
        trim_trailing_layout_only_rows(&final_sheet_data, params, &mut metrics)
    } else {
        final_sheet_data
    };

    let max_cell_column = get_max_cell_column(&metrics);
    let max_merge_column = get_max_merge_cells_column_index(&metrics);
    let max_visible_column = get_max_visible_column_index(&metrics);

    let max_column = [max_cell_column, max_merge_column, max_visible_column]
        .into_iter()
        .flatten()
        .max();

    let in_cols_node_output_chunks = if params.cleanup.extraneous_column_range_merge {
        merge_extraneous_columns(&preprocessed.in_cols_node_output_chunks, max_column)
    } else {
        preprocessed.in_cols_node_output_chunks.clone()
    };

    metrics.cleanup_stats.record_delta(
        CleanupCause::ExtraneousColumnRangeMerge,
        preprocessed.in_cols_node_output_chunks.len(),
        in_cols_node_output_chunks.len(),
    );

    let mut out = Vec::new();
    out.extend_from_slice(&preprocessed.until_worksheet_start_output_chunks);
    out.extend_from_slice(&preprocessed.before_cols_node_output_chunks);
    out.extend_from_slice(&in_cols_node_output_chunks);
    out.extend_from_slice(&preprocessed.before_sheet_data_node_output_chunks);
    out.extend_from_slice(&final_sheet_data);
    out.extend_from_slice(&preprocessed.before_merge_cells_node_output_chunks);
    out.extend_from_slice(&preprocessed.in_merge_cells_node_output_chunks);
    out.extend_from_slice(&preprocessed.until_worksheet_end_output_chunks);

    metrics.has_hints = crate::reference_scanner::check_for_reference_hints(&out);
    Ok(CleanedSheetData { xml: out, metrics })
}

fn trim_trailing_layout_only_rows(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut crate::models::CleanSheetMetrics,
) -> Vec<u8> {
    let mut end = sheet_data.len();

    loop {
        let Some(row_start) = rfind_bytes(&sheet_data[..end], b"<row") else {
            break;
        };

        let Some(row_end) = find_row_end(sheet_data, row_start) else {
            break;
        };

        if sheet_data[row_end..end]
            .iter()
            .any(|byte| !byte.is_ascii_whitespace())
        {
            break;
        }

        let row = &sheet_data[row_start..row_end];

        if trailing_row_is_layout_only(row, params) {
            metrics
                .cleanup_stats
                .record_removed(CleanupCause::TrailingLayoutOnlyRows, row.len());
            end = row_start;
            continue;
        }

        break;
    }

    if end == sheet_data.len() {
        sheet_data.to_vec()
    } else {
        sheet_data[..end].to_vec()
    }
}

fn find_row_end(data: &[u8], row_start: usize) -> Option<usize> {
    let open_end = find_byte_from(data, b'>', row_start)?;

    if open_end > row_start && data[open_end - 1] == b'/' {
        return Some(open_end + 1);
    }

    let content_start = open_end + 1;
    let closing_relative = find_bytes(&data[content_start..], b"</row>")?;

    Some(content_start + closing_relative + b"</row>".len())
}

fn trailing_row_is_layout_only(row: &[u8], params: &CleanSheetParameters) -> bool {
    if find_bytes(row, b"<c").is_some() {
        return false;
    }

    if row_is_hidden(row) {
        return true;
    }

    if row_has_visible_style(row, params) {
        return false;
    }

    true
}

fn row_is_hidden(row: &[u8]) -> bool {
    find_bytes(row, b" hidden=\"1\"").is_some() || find_bytes(row, b" hidden=\"true\"").is_some()
}

fn row_has_visible_style(row: &[u8], params: &CleanSheetParameters) -> bool {
    let Some(style_id) = extract_style_id(row) else {
        return false;
    };

    match params.patterns.xfs_by_index.get(&style_id) {
        Some(style) => style_is_visible_anchor(style),
        None => true, // conservative: unknown row style may be visible
    }
}

fn style_is_visible_anchor(style: &Xf) -> bool {
    style.has_fill_or_border || style.has_apply_protection
}

fn extract_style_id(tag: &[u8]) -> Option<u32> {
    let needle = b" s=\"";
    let pos = find_bytes(tag, needle)?;
    let start = pos + needle.len();
    let end_rel = tag[start..].iter().position(|byte| *byte == b'"')?;
    let value = &tag[start..start + end_rel];

    Some(crate::utils::digits_byte_string_to_integer(value))
}

#[cfg(test)]
mod tests {
    use super::build_clean_sheet_xml;
    use crate::models::{
        CleanSheetMetrics, CleanSheetParameters, SheetPreprocessConfig, WorkbookSpecificPatterns,
    };
    use crate::sheet::streaming::PreprocessedSheetXml;
    use std::collections::HashMap;
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
            cleanup: Default::default(),
            collect_cleanup_stats: false,
        }
    }

    #[test]
    fn reduced_fixture_q_cell_keeps_tail_cols_and_points_upstream_if_max_drops() {
        let params = default_params();

        let preprocessed = PreprocessedSheetXml {
            until_worksheet_start_output_chunks: b"<worksheet>".to_vec(),
            before_cols_node_output_chunks: b"<cols>".to_vec(),
            in_cols_node_output_chunks: br#"<col min="1" max="12" width="8.43"/><col min="13" max="17" width="10.0"/><col min="18" max="25" width="9.5"/>"#.to_vec(),
            before_sheet_data_node_output_chunks: b"</cols><sheetData>".to_vec(),
            sheet_data: b"<row r=\"1\"><c r=\"Q1\"><v>1</v></c></row>".to_vec(),
            before_merge_cells_node_output_chunks: b"".to_vec(),
            in_merge_cells_node_output_chunks: b"".to_vec(),
            until_worksheet_end_output_chunks: b"</sheetData></worksheet>".to_vec(),
            clean_sheet_metrics: CleanSheetMetrics::default(),
        };

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<col min="13" max="17" width="10.0"/>"#));
        assert!(xml.contains(r#"<col min="18" max="25" width="9.5"/>"#));
        assert!(!xml.contains(r#"<col min="13" max="25" width="10.0"/>"#));
    }
}
