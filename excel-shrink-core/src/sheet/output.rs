use crate::error::Result;
use crate::models::{CleanSheetParameters, CleanedSheetData, CleanupCause, Xf};
use crate::sheet::cols::merge_extraneous_columns;
use crate::sheet::ranges::{
    get_max_cell_column, get_max_cell_row, get_max_merge_cells_column_index,
    retrieve_sheet_data_range_containing_cells,
};
use crate::sheet::sheet_data::{
    clean_empty_rows_from_after_cell_range, clean_sheet_rows_and_cells_in_cell_range,
};
use crate::sheet::streaming::PreprocessedSheetXml;
use crate::xml::{find_byte_from, find_bytes, rfind_bytes};
use regex::bytes::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;

static SHEET_PROTECTION_RE: OnceLock<Regex> = OnceLock::new();
static CELL_RE: OnceLock<Regex> = OnceLock::new();

pub fn build_clean_sheet_xml(
    preprocessed: PreprocessedSheetXml,
    params: &CleanSheetParameters,
    disable_sheetdata_splitting: bool,
) -> Result<CleanedSheetData> {
    let sheet_protection_enabled = worksheet_has_enabled_sheet_protection(&preprocessed);
    let mut metrics = preprocessed.clean_sheet_metrics;
    metrics.cleanup_stats.enabled |= params.collect_cleanup_stats;
    metrics.sheet_protection_enabled = sheet_protection_enabled;
    if let Some(column) = metrics.column_definitions.get_last_visible_range(false) {
        metrics.visible_content_bounds.include_col(column.max);
    }

    let use_range_tail_pruning =
        !disable_sheetdata_splitting && params.cleanup.sheetdata_range_tail_pruning;

    let final_sheet_data = if !use_range_tail_pruning {
        clean_sheet_rows_and_cells_in_cell_range(&preprocessed.sheet_data, params, &mut metrics)?
    } else {
        let original_len = preprocessed.sheet_data.len();
        let ranges = retrieve_sheet_data_range_containing_cells(
            &preprocessed.sheet_data,
            params,
            metrics.sheet_protection_enabled,
        );
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

    let mut final_sheet_data = if params.cleanup.trailing_layout_only_rows {
        trim_trailing_layout_only_rows_vec(final_sheet_data, params, &mut metrics)
    } else {
        final_sheet_data
    };

    let max_cell_column = get_max_cell_column(&metrics);
    let max_cell_row = get_max_cell_row(&metrics);
    let max_merge_column = get_max_merge_cells_column_index(&metrics);

    let max_column = [
        max_cell_column,
        max_merge_column,
        metrics.visible_content_bounds.max_col,
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);
    let max_row = [max_cell_row, metrics.visible_content_bounds.max_row]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(0);

    final_sheet_data = remove_protection_only_cells_outside_bounds(
        final_sheet_data,
        max_column,
        max_row,
        &mut metrics,
    );

    if params.cleanup.trailing_layout_only_rows {
        final_sheet_data =
            trim_trailing_layout_only_rows_vec(final_sheet_data, params, &mut metrics);
    }

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

fn remove_protection_only_cells_outside_bounds(
    sheet_data: Vec<u8>,
    max_column: u32,
    max_row: u32,
    metrics: &mut crate::models::CleanSheetMetrics,
) -> Vec<u8> {
    let protected_outside_refs: HashSet<(u32, u32)> = metrics
        .kept_cell_states
        .iter()
        .filter(|cell| cell.kept_only_for_protection)
        .filter_map(|cell| {
            if cell.col > max_column || cell.row > max_row {
                Some((cell.col, cell.row))
            } else {
                None
            }
        })
        .collect();

    if protected_outside_refs.is_empty() {
        return sheet_data;
    }

    let cell_re =
        CELL_RE.get_or_init(|| Regex::new(r#"(?s)<c\b([^>]*)/>|<c\b([^>]*)>.*?</c>"#).unwrap());

    let mut out = Vec::with_capacity(sheet_data.len().min(64 * 1024 * 1024));
    let mut last_end = 0;

    for caps in cell_re.captures_iter(&sheet_data) {
        let Some(cell_match) = caps.get(0) else {
            continue;
        };
        out.extend_from_slice(&sheet_data[last_end..cell_match.start()]);

        let attr = caps
            .get(1)
            .or_else(|| caps.get(2))
            .map_or(b"".as_slice(), |m| m.as_bytes());
        let remove = extract_attribute_value(attr, b"r")
            .and_then(parse_cell_reference)
            .is_some_and(|reference| protected_outside_refs.contains(&reference));

        if remove {
            metrics.cleanup_stats.record_removed(
                CleanupCause::EmptyCellsWithoutContent,
                cell_match.as_bytes().len(),
            );
        } else {
            out.extend_from_slice(cell_match.as_bytes());
        }

        last_end = cell_match.end();
    }

    out.extend_from_slice(&sheet_data[last_end..]);
    out
}

pub fn trim_trailing_layout_only_rows(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut crate::models::CleanSheetMetrics,
) -> Vec<u8> {
    if let Some((remove_start, preserve_start)) = trailing_hidden_tail_removal_range(sheet_data) {
        let mut out = Vec::with_capacity(sheet_data.len() - (preserve_start - remove_start));
        out.extend_from_slice(&sheet_data[..remove_start]);
        out.extend_from_slice(&sheet_data[preserve_start..]);
        metrics.cleanup_stats.record_removed(
            CleanupCause::TrailingLayoutOnlyRows,
            preserve_start - remove_start,
        );
        return out;
    }

    let end = trailing_layout_only_rows_cutoff(sheet_data, params, metrics);

    if end == sheet_data.len() {
        sheet_data.to_vec()
    } else {
        sheet_data[..end].to_vec()
    }
}

pub fn trim_trailing_layout_only_rows_vec(
    mut sheet_data: Vec<u8>,
    params: &CleanSheetParameters,
    metrics: &mut crate::models::CleanSheetMetrics,
) -> Vec<u8> {
    if let Some((remove_start, preserve_start)) = trailing_hidden_tail_removal_range(&sheet_data) {
        sheet_data.drain(remove_start..preserve_start);
        metrics.cleanup_stats.record_removed(
            CleanupCause::TrailingLayoutOnlyRows,
            preserve_start - remove_start,
        );
        return sheet_data;
    }

    let end = trailing_layout_only_rows_cutoff(&sheet_data, params, metrics);

    if end < sheet_data.len() {
        sheet_data.truncate(end);
    }

    sheet_data
}

fn trailing_hidden_tail_removal_range(sheet_data: &[u8]) -> Option<(usize, usize)> {
    let mut tail_end = sheet_data.len();

    while tail_end > 0 && sheet_data[tail_end - 1].is_ascii_whitespace() {
        tail_end -= 1;
    }

    let last_row_start = rfind_bytes(&sheet_data[..tail_end], b"<row")?;
    let last_row_end = find_row_end(sheet_data, last_row_start)?;
    if last_row_end != tail_end {
        return None;
    }

    let last_row = &sheet_data[last_row_start..last_row_end];
    if !hidden_row_without_cells(last_row) {
        return None;
    }

    let preserve_start = last_row_start;
    let mut remove_start = preserve_start;
    let mut scan_end = preserve_start;

    loop {
        while scan_end > 0 && sheet_data[scan_end - 1].is_ascii_whitespace() {
            scan_end -= 1;
        }

        let Some(row_start) = rfind_bytes(&sheet_data[..scan_end], b"<row") else {
            break;
        };
        let Some(row_end) = find_row_end(sheet_data, row_start) else {
            break;
        };
        if row_end != scan_end {
            break;
        }

        let row = &sheet_data[row_start..row_end];
        if !hidden_row_without_cells(row) {
            break;
        }

        remove_start = row_start;
        scan_end = row_start;
    }

    (remove_start < preserve_start).then_some((remove_start, preserve_start))
}

fn hidden_row_without_cells(row: &[u8]) -> bool {
    row_is_hidden(row) && find_bytes(row, b"<c").is_none()
}

fn trailing_layout_only_rows_cutoff(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut crate::models::CleanSheetMetrics,
) -> usize {
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

    end
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
    style.has_fill_or_border
}

fn extract_style_id(tag: &[u8]) -> Option<u32> {
    let needle = b" s=\"";
    let pos = find_bytes(tag, needle)?;
    let start = pos + needle.len();
    let end_rel = tag[start..].iter().position(|byte| *byte == b'"')?;
    let value = &tag[start..start + end_rel];

    Some(crate::utils::digits_byte_string_to_integer(value))
}

fn worksheet_has_enabled_sheet_protection(preprocessed: &PreprocessedSheetXml) -> bool {
    let chunks: [&[u8]; 6] = [
        &preprocessed.before_cols_node_output_chunks,
        &preprocessed.in_cols_node_output_chunks,
        &preprocessed.before_sheet_data_node_output_chunks,
        &preprocessed.before_merge_cells_node_output_chunks,
        &preprocessed.in_merge_cells_node_output_chunks,
        &preprocessed.until_worksheet_end_output_chunks,
    ];

    chunks
        .iter()
        .any(|chunk| sheet_protection_chunk_is_enabled(chunk))
}

fn sheet_protection_chunk_is_enabled(chunk: &[u8]) -> bool {
    let re = SHEET_PROTECTION_RE.get_or_init(|| {
        Regex::new(r#"(?s)<(?:[A-Za-z_][\w.-]*:)?sheetProtection\b[^>]*(?:/>|>)"#).unwrap()
    });

    re.find_iter(chunk)
        .any(|tag| sheet_protection_tag_is_enabled(tag.as_bytes()))
}

fn sheet_protection_tag_is_enabled(tag: &[u8]) -> bool {
    match extract_attribute_value(tag, b"sheet") {
        Some(value) if value == b"0" || value.eq_ignore_ascii_case(b"false") => false,
        _ => true,
    }
}

fn extract_attribute_value<'a>(tag: &'a [u8], attr: &[u8]) -> Option<&'a [u8]> {
    let mut needle = Vec::with_capacity(attr.len() + 2);
    needle.extend_from_slice(attr);
    needle.extend_from_slice(b"=\"");

    let mut search_start = 0;
    while search_start < tag.len() {
        let Some(pos_rel) = find_bytes(&tag[search_start..], &needle) else {
            return None;
        };
        let pos = search_start + pos_rel;

        if pos > 0 && !tag[pos - 1].is_ascii_whitespace() {
            search_start = pos + 1;
            continue;
        }

        let value_start = pos + needle.len();
        let value_end_rel = tag[value_start..].iter().position(|byte| *byte == b'"')?;
        return Some(&tag[value_start..value_start + value_end_rel]);
    }

    None
}

fn parse_cell_reference(reference: &[u8]) -> Option<(u32, u32)> {
    let split = reference
        .iter()
        .position(|byte| byte.is_ascii_digit())
        .filter(|split| *split > 0 && *split < reference.len())?;

    let column = &reference[..split];
    let row = &reference[split..];

    if !column.iter().all(|byte| byte.is_ascii_uppercase())
        || !row.iter().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    Some((
        crate::utils::letters_byte_string_to_integer(column),
        crate::utils::digits_byte_string_to_integer(row),
    ))
}

#[cfg(test)]
mod tests {
    use super::build_clean_sheet_xml;
    use crate::models::{
        CleanSheetMetrics, CleanSheetParameters, SheetPreprocessConfig, WorkbookSpecificPatterns,
        Xf,
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

    fn params_with_styles(styles: &[(u32, Xf)]) -> CleanSheetParameters {
        let mut params = default_params();

        for (style_id, style) in styles {
            if style.has_only_font_style {
                params.patterns.only_font_style_ids.push(*style_id);
            }
            if style.has_fill_or_border || style.has_apply_protection {
                params
                    .patterns
                    .fill_or_border_or_protection_ids
                    .push(*style_id);
            }
            params
                .patterns
                .xfs_by_index
                .insert(*style_id, style.clone());
        }

        params
    }

    fn apply_protection_only_style() -> Xf {
        Xf::new(Some(0), Some(0), false, false, false, false, false, true)
    }

    fn visible_fill_style() -> Xf {
        Xf::new(Some(4), Some(0), false, false, true, false, false, false)
    }

    fn preprocessed_with_sheet_data(
        sheet_data: &[u8],
        sheet_protection: Option<&[u8]>,
    ) -> PreprocessedSheetXml {
        let mut before_sheet_data = Vec::new();
        if let Some(sheet_protection) = sheet_protection {
            before_sheet_data.extend_from_slice(sheet_protection);
        }
        before_sheet_data.extend_from_slice(b"<sheetData>");

        PreprocessedSheetXml {
            until_worksheet_start_output_chunks: b"<worksheet>".to_vec(),
            before_cols_node_output_chunks: Vec::new(),
            in_cols_node_output_chunks: Vec::new(),
            before_sheet_data_node_output_chunks: before_sheet_data,
            sheet_data: sheet_data.to_vec(),
            before_merge_cells_node_output_chunks: Vec::new(),
            in_merge_cells_node_output_chunks: Vec::new(),
            until_worksheet_end_output_chunks: b"</sheetData></worksheet>".to_vec(),
            clean_sheet_metrics: CleanSheetMetrics::default(),
        }
    }

    #[test]
    fn col_definitions_do_not_extend_max_column_when_sheet_has_no_cells_or_merges() {
        let params = default_params();

        let preprocessed = PreprocessedSheetXml {
            until_worksheet_start_output_chunks: b"<worksheet>".to_vec(),
            before_cols_node_output_chunks: b"<cols>".to_vec(),
            in_cols_node_output_chunks: br#"<col min="1" max="12" width="8.43"/><col min="13" max="17" width="10.0"/><col min="18" max="25" width="9.5"/>"#.to_vec(),
            before_sheet_data_node_output_chunks: b"</cols><sheetData>".to_vec(),
            sheet_data: Vec::new(),
            before_merge_cells_node_output_chunks: b"".to_vec(),
            in_merge_cells_node_output_chunks: b"".to_vec(),
            until_worksheet_end_output_chunks: b"</sheetData></worksheet>".to_vec(),
            clean_sheet_metrics: CleanSheetMetrics::default(),
        };

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<col min="1" max="25" width="9.5"/>"#));
        assert!(!xml.contains(r#"<col min="1" max="12" width="8.43"/>"#));
        assert!(!xml.contains(r#"<col min="13" max="17" width="10.0"/>"#));
    }

    #[test]
    fn unprotected_sheet_removes_empty_apply_protection_cell_outside_visible_bounds() {
        let params = params_with_styles(&[(7, apply_protection_only_style())]);
        let preprocessed = preprocessed_with_sheet_data(
            br#"<row r="1"><c r="A1"><v>1</v></c><c r="C1" s="7"/></row>"#,
            None,
        );

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<c r="A1"><v>1</v></c>"#));
        assert!(!xml.contains(r#"r="C1""#));
    }

    #[test]
    fn protected_sheet_removes_empty_apply_protection_cell_outside_visible_bounds() {
        let params = params_with_styles(&[(7, apply_protection_only_style())]);
        let preprocessed = preprocessed_with_sheet_data(
            br#"<row r="1"><c r="A1"><v>1</v></c><c r="C1" s="7"/></row>"#,
            Some(br#"<sheetProtection sheet="1"/>"#),
        );

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<c r="A1"><v>1</v></c>"#));
        assert!(!xml.contains(r#"r="C1""#));
    }

    #[test]
    fn protected_sheet_keeps_empty_apply_protection_cell_inside_visible_bounds() {
        let params = params_with_styles(&[(7, apply_protection_only_style())]);
        let preprocessed = preprocessed_with_sheet_data(
            br#"<row r="1"><c r="A1" s="7"/></row><row r="2"><c r="C2"><v>1</v></c></row>"#,
            Some(br#"<sheetProtection sheet="1"/>"#),
        );

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<c r="A1" s="7"/>"#));
    }

    #[test]
    fn apply_protection_does_not_extend_visible_tail_when_sheet_is_unprotected() {
        let params = params_with_styles(&[(7, apply_protection_only_style())]);
        let preprocessed = preprocessed_with_sheet_data(
            br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="999" s="7"/>"#,
            None,
        );

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"r="1""#));
        assert!(!xml.contains(r#"r="999""#));
    }

    #[test]
    fn visible_fill_still_keeps_cell_even_when_sheet_is_unprotected() {
        let params = params_with_styles(&[(7, visible_fill_style())]);
        let preprocessed = preprocessed_with_sheet_data(
            br#"<row r="1"><c r="A1"><v>1</v></c><c r="C1" s="7"/></row>"#,
            None,
        );

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<c r="C1" s="7"/>"#));
    }

    #[test]
    fn trailing_hidden_tail_keeps_only_last_hidden_row() {
        let params = default_params();
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data = br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" hidden="1"/><row r="3" hidden="1"/><row r="4" hidden="1"/>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="1""#));
        assert!(!cleaned.contains(r#"r="2""#));
        assert!(!cleaned.contains(r#"r="3""#));
        assert!(cleaned.contains(r#"r="4""#));
    }

    #[test]
    fn non_trailing_hidden_rows_are_not_collapsed_by_tail_cleanup() {
        let params = default_params();
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data = br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" hidden="1"/><row r="3" hidden="1"/><row r="4"><c r="A4"><v>1</v></c></row>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="2""#));
        assert!(cleaned.contains(r#"r="3""#));
        assert!(cleaned.contains(r#"r="4""#));
    }
}
