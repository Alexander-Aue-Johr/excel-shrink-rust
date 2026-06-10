use crate::error::Result;
use crate::models::{CleanSheetParameters, CleanedSheetData, Xf};
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

const SHEET17_DEBUG_ENV: &str = "EXCEL_SHRINK_DEBUG_SHEET17_COLS";
const SHEET17_DEBUG_WORKBOOK_HINT: &str = "statistischer-bericht-binnenschifffahrt-2080400231015";
const SHEET17_DEBUG_SHEET_PATH: &str = "xl/worksheets/sheet17.xml";

const SHEET92_DEBUG_ENV: &str = "EXCEL_SHRINK_DEBUG_SHEET92_COLS";
const SHEET92_DEBUG_WORKBOOK_HINT: &str =
    "statistischer-bericht-rechnungsergebnis-kernhaushalt-gemeinden-2140331217005";
const SHEET92_DEBUG_SHEET_PATH: &str = "xl/worksheets/sheet92.xml";
const DEBUG_WATCHED_CELL_LIMIT: usize = 300;

pub fn build_clean_sheet_xml(
    preprocessed: PreprocessedSheetXml,
    params: &CleanSheetParameters,
    disable_sheetdata_splitting: bool,
) -> Result<CleanedSheetData> {
    let mut metrics = preprocessed.clean_sheet_metrics;

    let final_sheet_data = if disable_sheetdata_splitting {
        clean_sheet_rows_and_cells_in_cell_range(&preprocessed.sheet_data, params, &mut metrics)?
    } else {
        let ranges = retrieve_sheet_data_range_containing_cells(&preprocessed.sheet_data, params);

        let cleaned_cells_range =
            clean_sheet_rows_and_cells_in_cell_range(&ranges.cells_range, params, &mut metrics)?;

        let cleaned_visible_rows_range =
            clean_empty_rows_from_after_cell_range(&ranges.rows_range, params, &mut metrics)?;

        [cleaned_cells_range, cleaned_visible_rows_range].concat()
    };

    let final_sheet_data = trim_trailing_layout_only_rows(&final_sheet_data, params);

    let max_cell_column = get_max_cell_column(&metrics);
    let max_merge_column = get_max_merge_cells_column_index(&metrics);
    let max_visible_column = get_max_visible_column_index(&metrics);

    let max_column = [max_cell_column, max_merge_column, max_visible_column]
        .into_iter()
        .flatten()
        .max();

    let debug_config = debug_config(params);

    if let Some(debug_config) = debug_config {
        emit_column_parity_debug(
            debug_config,
            params,
            &metrics,
            max_cell_column,
            max_merge_column,
            max_visible_column,
            max_column,
            &preprocessed.in_cols_node_output_chunks,
            None,
            "before_merge",
        );
    }

    let in_cols_node_output_chunks =
        merge_extraneous_columns(&preprocessed.in_cols_node_output_chunks, max_column);

    if let Some(debug_config) = debug_config {
        emit_column_parity_debug(
            debug_config,
            params,
            &metrics,
            max_cell_column,
            max_merge_column,
            max_visible_column,
            max_column,
            &preprocessed.in_cols_node_output_chunks,
            Some(&in_cols_node_output_chunks),
            "after_merge",
        );
    }

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

fn trim_trailing_layout_only_rows(sheet_data: &[u8], params: &CleanSheetParameters) -> Vec<u8> {
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

#[derive(Debug, Clone)]
struct ColumnParityDebugConfig {
    label: &'static str,
    env: &'static str,
    workbook_hint: &'static str,
    sheet_path: &'static str,
    watched_columns: std::ops::RangeInclusive<u32>,
    watched_label: &'static str,
}

const COLUMN_PARITY_DEBUG_CONFIGS: &[ColumnParityDebugConfig] = &[
    ColumnParityDebugConfig {
        label: "sheet17-cols-debug",
        env: SHEET17_DEBUG_ENV,
        workbook_hint: SHEET17_DEBUG_WORKBOOK_HINT,
        sheet_path: SHEET17_DEBUG_SHEET_PATH,
        watched_columns: 13..=18,
        watched_label: "M..R",
    },
    ColumnParityDebugConfig {
        label: "sheet92-cols-debug",
        env: SHEET92_DEBUG_ENV,
        workbook_hint: SHEET92_DEBUG_WORKBOOK_HINT,
        sheet_path: SHEET92_DEBUG_SHEET_PATH,
        watched_columns: 7..=9,
        watched_label: "G..I",
    },
];

fn debug_config(params: &CleanSheetParameters) -> Option<&'static ColumnParityDebugConfig> {
    COLUMN_PARITY_DEBUG_CONFIGS.iter().find(|config| {
        matches!(std::env::var(config.env).ok().as_deref(), Some("1"))
            && params.xlsx_file_path.contains(config.workbook_hint)
            && params.zip_info_filename == config.sheet_path
    })
}

fn emit_column_parity_debug(
    config: &ColumnParityDebugConfig,
    params: &CleanSheetParameters,
    metrics: &crate::models::CleanSheetMetrics,
    max_cell_column: Option<u32>,
    max_merge_column: Option<u32>,
    max_visible_column: Option<u32>,
    max_column: Option<u32>,
    cols_before: &[u8],
    cols_after: Option<&[u8]>,
    phase: &str,
) {
    let watched_kept = collect_watched_cells_from_states(
        &metrics.kept_cell_states,
        config.watched_columns.clone(),
    );
    let watched_deleted = collect_watched_cells_from_states(
        &metrics.deleted_cell_states,
        config.watched_columns.clone(),
    );
    let watched_candidates =
        collect_watched_cells_from_candidates(&metrics.candidates, config.watched_columns.clone());

    eprintln!(
        "[{}] phase={phase} workbook={} sheet={} max_cell_column={:?} max_merge_cells_column={:?} max_visible_column={:?} max_column={:?}",
        config.label,
        params.xlsx_file_path,
        params.zip_info_filename,
        max_cell_column,
        max_merge_column,
        max_visible_column,
        max_column,
    );
    eprintln!(
        "[{}] watched kept({})={:?} deleted({})={:?} candidates({})={:?}",
        config.label,
        config.watched_label,
        watched_kept,
        config.watched_label,
        watched_deleted,
        config.watched_label,
        watched_candidates
    );
    eprintln!(
        "[{}] totals kept={} deleted={} candidates={} hints={}",
        config.label,
        metrics.kept_cell_states.len(),
        metrics.deleted_cell_states.len(),
        metrics.candidates.len(),
        metrics.has_hints,
    );
    eprintln!(
        "[{}] cols_before={}{}",
        config.label,
        String::from_utf8_lossy(cols_before),
        cols_after
            .map(|after| format!(
                "\n[{}] cols_after={}",
                config.label,
                String::from_utf8_lossy(after)
            ))
            .unwrap_or_default()
    );
}

fn collect_watched_cells_from_states(
    states: &[crate::models::CellAttributes],
    watched_columns: std::ops::RangeInclusive<u32>,
) -> Vec<String> {
    let mut out = Vec::new();

    for state in states {
        let col = crate::utils::letters_byte_string_to_integer(&state.cell_coordinate.column);
        if watched_columns.contains(&col) {
            out.push(format!(
                "{}{}",
                String::from_utf8_lossy(&state.cell_coordinate.column),
                String::from_utf8_lossy(&state.cell_coordinate.row)
            ));

            if out.len() >= DEBUG_WATCHED_CELL_LIMIT {
                break;
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

fn collect_watched_cells_from_candidates(
    candidates: &[crate::models::CellCoordinate],
    watched_columns: std::ops::RangeInclusive<u32>,
) -> Vec<String> {
    let mut out = Vec::new();

    for coord in candidates {
        let col = crate::utils::letters_byte_string_to_integer(&coord.column);
        if watched_columns.contains(&col) {
            out.push(format!(
                "{}{}",
                String::from_utf8_lossy(&coord.column),
                String::from_utf8_lossy(&coord.row)
            ));

            if out.len() >= DEBUG_WATCHED_CELL_LIMIT {
                break;
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::build_clean_sheet_xml;
    use crate::models::{
        CellCleanerMode, CleanSheetMetrics, CleanSheetParameters, SheetPreprocessConfig,
        WorkbookSpecificPatterns,
    };
    use crate::sheet::streaming::PreprocessedSheetXml;
    use std::collections::HashMap;
    use zip::CompressionMethod;

    fn default_params() -> CleanSheetParameters {
        CleanSheetParameters {
            xlsx_file_path: "test.xlsx".to_string(),
            zip_info_filename: "xl/worksheets/sheet17.xml".to_string(),
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
