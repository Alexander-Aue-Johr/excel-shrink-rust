use crate::error::Result;
use crate::models::{CleanSheetParameters, CleanedSheetData, CleanupCause, Xf};
use crate::sheet::cols::merge_extraneous_columns;
use crate::sheet::drawings::find_worksheet_drawing_anchor_bounds;
use crate::sheet::ranges::{
    get_max_cell_column, get_max_cell_row, get_max_merge_cells_column_index,
    get_max_merge_cells_row_index, retrieve_sheet_data_range_containing_cells,
};
use crate::sheet::sheet_data::{
    clean_empty_rows_from_after_cell_range,
    clean_empty_rows_from_after_cell_range_parallel_by_max_chunk_size,
    clean_sheet_rows_and_cells_in_cell_range,
    clean_sheet_rows_and_cells_in_cell_range_parallel_by_max_chunk_size,
    should_parallel_chunk_range,
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
    build_clean_sheet_xml_with_progress(
        preprocessed,
        params,
        disable_sheetdata_splitting,
        0,
        &mut |_| {},
    )
}

pub fn build_clean_sheet_xml_with_progress(
    preprocessed: PreprocessedSheetXml,
    params: &CleanSheetParameters,
    disable_sheetdata_splitting: bool,
    parallel_range_chunk_size: usize,
    progress: &mut impl FnMut(&'static str),
) -> Result<CleanedSheetData> {
    progress("build_prepare_sheet_metrics");
    let sheet_protection_enabled = worksheet_has_enabled_sheet_protection(&preprocessed);
    let drawing_anchor_bounds = find_worksheet_drawing_anchor_bounds(&preprocessed, params);
    let mut metrics = preprocessed.clean_sheet_metrics;
    metrics.cleanup_stats.enabled |= params.collect_cleanup_stats;
    metrics.sheet_protection_enabled = sheet_protection_enabled;
    if let Some(column) = metrics.column_definitions.get_last_visible_range(false) {
        metrics.visible_content_bounds.include_col(column.max);
    }
    if let Some(row) = drawing_anchor_bounds.max_row {
        metrics.visible_content_bounds.include_row(row);
        metrics.tail_anchor_max_row =
            Some(metrics.tail_anchor_max_row.map_or(row, |old| old.max(row)));
    }
    if let Some(column) = drawing_anchor_bounds.max_col {
        metrics.visible_content_bounds.include_col(column);
    }
    let max_merge_row = get_max_merge_cells_row_index(&metrics);
    let preserve_through_row = tail_boundary_row(&metrics, max_merge_row);

    let use_range_tail_pruning =
        !disable_sheetdata_splitting && params.cleanup.sheetdata_range_tail_pruning;

    let final_sheet_data = if !use_range_tail_pruning {
        progress("clean_sheet_rows_and_cells");
        clean_sheet_rows_and_cells_in_cell_range(&preprocessed.sheet_data, params, &mut metrics)?
    } else {
        progress("retrieve_sheet_data_ranges");
        let original_len = preprocessed.sheet_data.len();
        let ranges = retrieve_sheet_data_range_containing_cells(
            &preprocessed.sheet_data,
            params,
            metrics.sheet_protection_enabled,
            preserve_through_row,
        );
        let kept_len = ranges.cells_range.len() + ranges.rows_range.len();

        if original_len > kept_len {
            metrics.cleanup_stats.record_removed(
                CleanupCause::SheetDataRangeTailPruning,
                original_len - kept_len,
            );
        }

        let cleaned_cells_range =
            if should_parallel_chunk_range(ranges.cells_range.len(), parallel_range_chunk_size) {
                progress("clean_cells_range_parallel");
                clean_sheet_rows_and_cells_in_cell_range_parallel_by_max_chunk_size(
                    ranges.cells_range,
                    params,
                    &mut metrics,
                    parallel_range_chunk_size,
                    "cells_range",
                    &params.zip_info_filename,
                )?
            } else {
                progress("clean_cells_range");
                clean_sheet_rows_and_cells_in_cell_range(ranges.cells_range, params, &mut metrics)?
            };

        let cleaned_visible_rows_range =
            if should_parallel_chunk_range(ranges.rows_range.len(), parallel_range_chunk_size) {
                progress("clean_visible_rows_range_parallel");
                clean_empty_rows_from_after_cell_range_parallel_by_max_chunk_size(
                    ranges.rows_range,
                    params,
                    &mut metrics,
                    parallel_range_chunk_size,
                    "rows_range",
                    &params.zip_info_filename,
                )?
            } else {
                progress("clean_visible_rows_range");
                clean_empty_rows_from_after_cell_range(ranges.rows_range, params, &mut metrics)?
            };

        [cleaned_cells_range, cleaned_visible_rows_range].concat()
    };

    let mut final_sheet_data = if params.cleanup.trailing_layout_only_rows {
        progress("trim_trailing_layout_only_rows");
        trim_trailing_layout_only_rows_vec_preserving_rows_through(
            final_sheet_data,
            params,
            &mut metrics,
            max_merge_row,
        )
    } else {
        final_sheet_data
    };

    progress("compute_visible_bounds");
    let max_cell_column = get_max_cell_column(&metrics);
    let max_cell_row = get_max_cell_row(&metrics);
    let max_merge_column = get_max_merge_cells_column_index(&metrics);
    let max_merge_row = get_max_merge_cells_row_index(&metrics);

    let max_column = [
        max_cell_column,
        max_merge_column,
        metrics.visible_content_bounds.max_col,
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);
    let max_row = [
        max_cell_row,
        max_merge_row,
        metrics.visible_content_bounds.max_row,
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);

    progress("remove_protection_only_cells_outside_bounds");
    final_sheet_data = remove_protection_only_cells_outside_bounds(
        final_sheet_data,
        max_column,
        max_row,
        &mut metrics,
    );

    if params.cleanup.trailing_layout_only_rows {
        progress("trim_trailing_layout_only_rows_after_bounds");
        final_sheet_data = trim_trailing_layout_only_rows_vec_preserving_rows_through(
            final_sheet_data,
            params,
            &mut metrics,
            max_merge_row,
        );
    }

    progress("merge_extraneous_columns");
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

    progress("assemble_clean_sheet_xml");
    let mut out = Vec::new();
    out.extend_from_slice(&preprocessed.until_worksheet_start_output_chunks);
    out.extend_from_slice(&preprocessed.before_cols_node_output_chunks);
    out.extend_from_slice(&in_cols_node_output_chunks);
    out.extend_from_slice(&preprocessed.before_sheet_data_node_output_chunks);
    let sheet_data_start = out.len();
    out.extend_from_slice(&final_sheet_data);
    let sheet_data_end = out.len();
    out.extend_from_slice(&preprocessed.before_merge_cells_node_output_chunks);
    out.extend_from_slice(&preprocessed.in_merge_cells_node_output_chunks);
    out.extend_from_slice(&preprocessed.until_worksheet_end_output_chunks);

    progress("scan_reference_hints");
    metrics.has_hints = crate::reference_scanner::check_for_reference_hints(&out);
    Ok(CleanedSheetData {
        xml: out,
        sheet_data_range: sheet_data_start..sheet_data_end,
        metrics,
    })
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
    trim_trailing_layout_only_rows_preserving_rows_through(sheet_data, params, metrics, None)
}

fn trim_trailing_layout_only_rows_preserving_rows_through(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut crate::models::CleanSheetMetrics,
    preserve_through_row: Option<u32>,
) -> Vec<u8> {
    let tail_boundary_row = tail_boundary_row(metrics, preserve_through_row);

    if tail_boundary_row.is_none() {
        if let Some((remove_start, preserve_start)) =
            trailing_hidden_tail_removal_range(sheet_data, None)
        {
            let mut out = Vec::with_capacity(sheet_data.len() - (preserve_start - remove_start));
            out.extend_from_slice(&sheet_data[..remove_start]);
            out.extend_from_slice(&sheet_data[preserve_start..]);
            metrics.cleanup_stats.record_removed(
                CleanupCause::TrailingLayoutOnlyRows,
                preserve_start - remove_start,
            );
            return out;
        }
    }

    let end = trailing_layout_only_rows_cutoff(sheet_data, params, metrics, tail_boundary_row);

    if end == sheet_data.len() {
        sheet_data.to_vec()
    } else {
        sheet_data[..end].to_vec()
    }
}

pub fn trim_trailing_layout_only_rows_vec(
    sheet_data: Vec<u8>,
    params: &CleanSheetParameters,
    metrics: &mut crate::models::CleanSheetMetrics,
) -> Vec<u8> {
    trim_trailing_layout_only_rows_vec_preserving_rows_through(sheet_data, params, metrics, None)
}

fn trim_trailing_layout_only_rows_vec_preserving_rows_through(
    mut sheet_data: Vec<u8>,
    params: &CleanSheetParameters,
    metrics: &mut crate::models::CleanSheetMetrics,
    preserve_through_row: Option<u32>,
) -> Vec<u8> {
    let tail_boundary_row = tail_boundary_row(metrics, preserve_through_row);

    if tail_boundary_row.is_none() {
        if let Some((remove_start, preserve_start)) =
            trailing_hidden_tail_removal_range(&sheet_data, None)
        {
            sheet_data.drain(remove_start..preserve_start);
            metrics.cleanup_stats.record_removed(
                CleanupCause::TrailingLayoutOnlyRows,
                preserve_start - remove_start,
            );
            return sheet_data;
        }
    }

    let end = trailing_layout_only_rows_cutoff(&sheet_data, params, metrics, tail_boundary_row);

    if end < sheet_data.len() {
        sheet_data.truncate(end);
    }

    sheet_data
}

fn tail_boundary_row(
    metrics: &crate::models::CleanSheetMetrics,
    preserve_through_row: Option<u32>,
) -> Option<u32> {
    [
        preserve_through_row,
        metrics.tail_anchor_max_row,
        get_max_merge_cells_row_index(metrics),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn trailing_hidden_tail_removal_range(
    sheet_data: &[u8],
    preserve_through_row: Option<u32>,
) -> Option<(usize, usize)> {
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
    if row_is_protected_tail_boundary(last_row, preserve_through_row) {
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
        if row_is_protected_tail_boundary(row, preserve_through_row) {
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
    preserve_through_row: Option<u32>,
) -> usize {
    if let Some(max_row) = preserve_through_row {
        return cutoff_after_row_boundary(sheet_data, max_row, params, metrics);
    }

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

        if row_is_protected_tail_boundary(row, preserve_through_row) {
            break;
        }

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

fn cutoff_after_row_boundary(
    sheet_data: &[u8],
    max_row: u32,
    params: &CleanSheetParameters,
    metrics: &mut crate::models::CleanSheetMetrics,
) -> usize {
    if let Some(end) = cutoff_after_exact_row_reference(sheet_data, max_row) {
        let end = extend_cutoff_to_visible_row_before_hidden_tail(sheet_data, end, params);
        if end < sheet_data.len() {
            metrics
                .cleanup_stats
                .record_removed(CleanupCause::TrailingLayoutOnlyRows, sheet_data.len() - end);
        }

        return end;
    }

    let mut end = sheet_data.len();

    loop {
        let mut scan_end = end;
        while scan_end > 0 && sheet_data[scan_end - 1].is_ascii_whitespace() {
            scan_end -= 1;
        }

        let Some(row_start) = rfind_bytes(&sheet_data[..scan_end], b"<row") else {
            end = 0;
            break;
        };

        let Some(row_end) = find_row_end(sheet_data, row_start) else {
            return sheet_data.len();
        };

        if row_end != scan_end {
            return sheet_data.len();
        }

        let row = &sheet_data[row_start..row_end];
        let Some(row_number) = row_number(row) else {
            return sheet_data.len();
        };

        if row_number <= max_row {
            end = if end == sheet_data.len() {
                sheet_data.len()
            } else {
                row_end
            };
            end = extend_cutoff_to_visible_row_before_hidden_tail(sheet_data, end, params);
            break;
        }

        end = row_start;
    }

    if end < sheet_data.len() {
        metrics
            .cleanup_stats
            .record_removed(CleanupCause::TrailingLayoutOnlyRows, sheet_data.len() - end);
    }

    end
}

fn extend_cutoff_to_visible_row_before_hidden_tail(
    sheet_data: &[u8],
    cutoff: usize,
    params: &CleanSheetParameters,
) -> usize {
    let Some((layout_row, layout_row_end)) = next_row_after_cutoff(sheet_data, cutoff) else {
        return cutoff;
    };

    if row_is_hidden(layout_row) || !trailing_row_is_layout_only(layout_row, params) {
        return cutoff;
    }

    if tail_after_position_is_hidden_rows_only(sheet_data, layout_row_end) {
        layout_row_end
    } else {
        cutoff
    }
}

fn next_row_after_cutoff(sheet_data: &[u8], cutoff: usize) -> Option<(&[u8], usize)> {
    let mut pos = cutoff;
    while pos < sheet_data.len() && sheet_data[pos].is_ascii_whitespace() {
        pos += 1;
    }

    if pos >= sheet_data.len() || !row_tag_starts_at(sheet_data, pos) {
        return None;
    }

    let row_end = find_row_end(sheet_data, pos)?;
    Some((&sheet_data[pos..row_end], row_end))
}

fn tail_after_position_is_hidden_rows_only(sheet_data: &[u8], mut pos: usize) -> bool {
    let mut found_hidden_row = false;

    loop {
        while pos < sheet_data.len() && sheet_data[pos].is_ascii_whitespace() {
            pos += 1;
        }

        if pos >= sheet_data.len() {
            return found_hidden_row;
        }

        if !row_tag_starts_at(sheet_data, pos) {
            return false;
        }

        let Some(row_end) = find_row_end(sheet_data, pos) else {
            return false;
        };
        let row = &sheet_data[pos..row_end];

        if !row_is_hidden(row) {
            return false;
        }

        found_hidden_row = true;
        pos = row_end;
    }
}

fn cutoff_after_exact_row_reference(sheet_data: &[u8], max_row: u32) -> Option<usize> {
    let needle = format!(r#"r="{max_row}""#);
    let needle = needle.as_bytes();
    let mut scan_end = sheet_data.len();

    while let Some(attr_start) = rfind_bytes(&sheet_data[..scan_end], needle) {
        if let Some(row_start) =
            row_start_containing_row_reference(sheet_data, attr_start, needle.len())
        {
            let row_end = find_row_end(sheet_data, row_start)?;
            let row = &sheet_data[row_start..row_end];

            if row_number(row) == Some(max_row) {
                return Some(
                    if sheet_data[row_end..].iter().all(u8::is_ascii_whitespace) {
                        sheet_data.len()
                    } else {
                        row_end
                    },
                );
            }
        }

        scan_end = attr_start;
    }

    None
}

fn row_start_containing_row_reference(
    sheet_data: &[u8],
    attr_start: usize,
    attr_len: usize,
) -> Option<usize> {
    let row_start = rfind_bytes(&sheet_data[..attr_start], b"<row")?;
    if !row_tag_starts_at(sheet_data, row_start) {
        return None;
    }

    let open_end = find_byte_from(sheet_data, b'>', row_start)?;
    if attr_start >= open_end || attr_start + attr_len > open_end {
        return None;
    }

    if !sheet_data
        .get(attr_start.wrapping_sub(1))
        .is_some_and(u8::is_ascii_whitespace)
    {
        return None;
    }

    if !sheet_data
        .get(attr_start + attr_len)
        .is_none_or(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
    {
        return None;
    }

    Some(row_start)
}

fn row_tag_starts_at(sheet_data: &[u8], row_start: usize) -> bool {
    sheet_data
        .get(row_start..row_start + b"<row".len())
        .is_some_and(|tag| tag == b"<row")
        && sheet_data
            .get(row_start + b"<row".len())
            .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
}

fn row_is_protected_tail_boundary(row: &[u8], preserve_through_row: Option<u32>) -> bool {
    preserve_through_row.is_some_and(|max_row| {
        row_number(row)
            .map(|row_number| row_number <= max_row)
            .unwrap_or(true)
    })
}

fn row_number(row: &[u8]) -> Option<u32> {
    let open_end = find_byte_from(row, b'>', 0)?;
    extract_attribute_value(&row[..open_end], b"r").map(crate::utils::digits_byte_string_to_integer)
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
    if row_has_visible_cell_anchor(row, params) {
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

fn row_has_visible_cell_anchor(row: &[u8], params: &CleanSheetParameters) -> bool {
    let mut pos = 0;

    while let Some(cell_start_rel) = find_bytes(&row[pos..], b"<c") {
        let cell_start = pos + cell_start_rel;
        let after_name = cell_start + b"<c".len();

        if !matches!(
            row.get(after_name),
            Some(byte) if byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>')
        ) {
            pos = after_name;
            continue;
        }

        let Some(tag_end) = find_byte_from(row, b'>', cell_start) else {
            return true;
        };

        let attr_end = if tag_end > cell_start && row[tag_end - 1] == b'/' {
            tag_end - 1
        } else {
            return true;
        };

        let attr = &row[after_name..attr_end];
        let is_anchor = match extract_style_id(attr) {
            Some(style_id) => params
                .patterns
                .xfs_by_index
                .get(&style_id)
                .map_or(true, style_is_visible_anchor),
            None => params.cleanup.empty_style_representative_cells,
        };
        if is_anchor {
            return true;
        }

        pos = tag_end + 1;
    }

    false
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
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::FileOptions;
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
        let mut params = default_params();

        for (style_id, style) in styles {
            if style.has_no_empty_cell_visible_style {
                params
                    .patterns
                    .no_empty_cell_visible_style_ids
                    .push(*style_id);
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
        Xf::builder()
            .font_id(Some(0))
            .fill_id(Some(0))
            .border_id(Some(0))
            .has_apply_protection(true)
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

    fn visible_fill_style() -> Xf {
        Xf::builder()
            .font_id(Some(0))
            .fill_id(Some(4))
            .border_id(Some(0))
            .has_visible_fill(true)
            .has_fill_or_border(true)
            .build_xf()
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

    fn temp_xlsx_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{}-{nanos}.xlsx", std::process::id()))
    }

    fn write_drawing_anchor_xlsx(path: &PathBuf) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = FileOptions::default().compression_method(CompressionMethod::Stored);

        zip.start_file("xl/worksheets/_rels/sheet1.xml.rels", options)
            .unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#)
            .unwrap();

        zip.start_file("xl/drawings/drawing1.xml", options).unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>109</xdr:row><xdr:rowOff>47625</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>66675</xdr:colOff><xdr:row>110</xdr:row><xdr:rowOff>49790</xdr:rowOff></xdr:to><xdr:sp><xdr:txBody><a:bodyPr/><a:p><a:r><a:t>Statistisches Bundesamt, Zeitreihen, 2021</a:t></a:r></a:p></xdr:txBody></xdr:sp><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#)
            .unwrap();

        zip.finish().unwrap();
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
    fn drawing_text_box_anchor_preserves_layout_row_after_last_cell() {
        let xlsx_path = temp_xlsx_path("drawing-anchor-row");
        write_drawing_anchor_xlsx(&xlsx_path);

        let mut params = default_params();
        params.xlsx_file_path = xlsx_path.to_string_lossy().to_string();
        let mut preprocessed = preprocessed_with_sheet_data(
            br#"<row r="110"><c r="I110"><v>27</v></c></row><row r="111" spans="1:9" x14ac:dyDescent="0.2"/><row r="112" spans="1:9" x14ac:dyDescent="0.2"/>"#,
            None,
        );
        preprocessed.until_worksheet_end_output_chunks =
            br#"</sheetData><drawing r:id="rId2"/></worksheet>"#.to_vec();

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"r="110""#));
        assert!(xml.contains(r#"r="111""#));
        assert!(!xml.contains(r#"r="112""#));
        assert!(xml.contains(r#"<drawing r:id="rId2"/>"#));

        let _ = std::fs::remove_file(xlsx_path);
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
    fn tail_anchor_metric_trims_after_matching_row_index() {
        let params = default_params();
        let mut metrics = CleanSheetMetrics {
            tail_anchor_max_row: Some(10),
            ..CleanSheetMetrics::default()
        };
        let sheet_data = br#"<row r="1"/><row r="10"/><row r="20"/>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="1""#));
        assert!(cleaned.contains(r#"r="10""#));
        assert!(!cleaned.contains(r#"r="20""#));
    }

    #[test]
    fn tail_anchor_preserves_visible_layout_row_before_hidden_tail() {
        let params = default_params();
        let mut metrics = CleanSheetMetrics {
            tail_anchor_max_row: Some(110),
            ..CleanSheetMetrics::default()
        };
        let sheet_data = br#"<row r="110"><c r="I110"><v>27</v></c></row><row r="111" spans="1:9" x14ac:dyDescent="0.2"/><row r="112" hidden="1" customHeight="1"/><row r="113" hidden="1" customHeight="1"/>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="110""#));
        assert!(cleaned.contains(r#"r="111""#));
        assert!(!cleaned.contains(r#"r="112""#));
        assert!(!cleaned.contains(r#"r="113""#));
    }

    #[test]
    fn tail_anchor_does_not_preserve_visible_layout_row_without_hidden_tail() {
        let params = default_params();
        let mut metrics = CleanSheetMetrics {
            tail_anchor_max_row: Some(110),
            ..CleanSheetMetrics::default()
        };
        let sheet_data = br#"<row r="110"><c r="I110"><v>27</v></c></row><row r="111" spans="1:9" x14ac:dyDescent="0.2"/><row r="112" spans="1:9" x14ac:dyDescent="0.2"/>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="110""#));
        assert!(!cleaned.contains(r#"r="111""#));
        assert!(!cleaned.contains(r#"r="112""#));
    }

    #[test]
    fn tail_anchor_metric_uses_previous_existing_row_when_target_row_is_missing() {
        let params = default_params();
        let mut metrics = CleanSheetMetrics {
            tail_anchor_max_row: Some(95),
            ..CleanSheetMetrics::default()
        };
        let sheet_data = br#"
            <row r="90"/>
            <row r="93"/>
            <row r="96"/>
            <row r="100"/>
        "#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="90""#));
        assert!(cleaned.contains(r#"r="93""#));
        assert!(!cleaned.contains(r#"r="96""#));
        assert!(!cleaned.contains(r#"r="100""#));
    }

    #[test]
    fn merge_range_preserves_layout_tail_rows_through_merge_end() {
        let params = params_with_styles(&[(1029, apply_protection_only_style())]);
        let merge_cells = br#"<mergeCells count="1"><mergeCell ref="A93:M95"/></mergeCells>"#;
        let mut preprocessed = preprocessed_with_sheet_data(
            br#"<row r="93" spans="1:14" ht="5.45" customHeight="1"><c r="A93" s="1029" t="s"><v>1740</v></c><c r="B93" s="1029"/><c r="M93" s="1029"/></row><row r="94" spans="1:14" ht="13.15" customHeight="1"><c r="A94" s="1029"/></row><row r="95" spans="1:14" ht="7.9" customHeight="1"><c r="A95" s="1029"/></row><row r="96" spans="1:14"/><row r="97"/><row r="98"/>"#,
            None,
        );
        preprocessed.in_merge_cells_node_output_chunks = merge_cells.to_vec();
        preprocessed
            .clean_sheet_metrics
            .max_merge_cells_column_and_row =
            crate::sheet::merge_cells::find_max_merge_cells(merge_cells);

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"r="93""#));
        assert!(xml.contains(r#"r="94""#));
        assert!(xml.contains(r#"r="95""#));
        assert!(!xml.contains(r#"r="96""#));
        assert!(!xml.contains(r#"r="97""#));
        assert!(!xml.contains(r#"r="98""#));
        assert!(xml.contains(r#"r="B93""#));
        assert!(xml.contains(r#"<mergeCell ref="A93:M95"/>"#));
    }

    #[test]
    fn opt_in_removes_unprotected_apply_protection_cells_but_preserves_merge_tail_rows() {
        let mut params = params_with_styles(&[(1029, apply_protection_only_style())]);
        params
            .cleanup
            .remove_apply_protection_without_sheet_protection = true;
        let merge_cells = br#"<mergeCells count="1"><mergeCell ref="A93:M95"/></mergeCells>"#;
        let mut preprocessed = preprocessed_with_sheet_data(
            br#"<row r="93" spans="1:14" ht="5.45" customHeight="1"><c r="A93" s="1029" t="s"><v>1740</v></c><c r="B93" s="1029"/><c r="M93" s="1029"/></row><row r="94" spans="1:14" ht="13.15" customHeight="1"><c r="A94" s="1029"/></row><row r="95" spans="1:14" ht="7.9" customHeight="1"><c r="A95" s="1029"/></row><row r="96" spans="1:14"/>"#,
            None,
        );
        preprocessed.in_merge_cells_node_output_chunks = merge_cells.to_vec();
        preprocessed
            .clean_sheet_metrics
            .max_merge_cells_column_and_row =
            crate::sheet::merge_cells::find_max_merge_cells(merge_cells);

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"r="93""#));
        assert!(xml.contains(r#"r="94""#));
        assert!(xml.contains(r#"r="95""#));
        assert!(!xml.contains(r#"r="B93""#));
        assert!(!xml.contains(r#"r="A94""#));
        assert!(!xml.contains(r#"r="A95""#));
        assert!(!xml.contains(r#"r="96""#));
        assert!(xml.contains(r#"<mergeCell ref="A93:M95"/>"#));
    }

    #[test]
    fn trailing_self_closing_default_cell_keeps_tail_when_representatives_are_enabled() {
        let params = default_params();
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data =
            br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="999"><c r="A999"/></row>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="1""#));
        assert!(cleaned.contains(r#"r="999""#));
    }

    #[test]
    fn trailing_self_closing_font_only_cell_is_layout_only_when_representatives_are_enabled() {
        let params = params_with_styles(&[(7, font_only_style())]);
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data =
            br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="999"><c r="A999" s="7"/></row>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="1""#));
        assert!(!cleaned.contains(r#"r="999""#));
    }

    #[test]
    fn build_clean_sheet_xml_trims_trailing_font_only_representatives() {
        let params = params_with_styles(&[(7, font_only_style())]);
        let preprocessed = preprocessed_with_sheet_data(
            br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="999"><c r="A999" s="7"/></row>"#,
            None,
        );

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"r="1""#));
        assert!(!xml.contains(r#"r="999""#));
    }

    #[test]
    fn trailing_self_closing_apply_protection_cell_is_layout_only() {
        let params = params_with_styles(&[(7, apply_protection_only_style())]);
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data =
            br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="999"><c r="A999" s="7"/></row>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="1""#));
        assert!(!cleaned.contains(r#"r="999""#));
    }

    #[test]
    fn trailing_self_closing_default_cell_is_layout_only_when_default_representatives_are_disabled()
    {
        let mut params = default_params();
        params.cleanup.empty_style_representative_cells = false;
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data =
            br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="999"><c r="A999"/></row>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="1""#));
        assert!(!cleaned.contains(r#"r="999""#));
    }

    #[test]
    fn trailing_self_closing_font_only_cell_is_layout_only_when_font_representatives_are_disabled()
    {
        let mut params = params_with_styles(&[(7, font_only_style())]);
        params.cleanup.empty_font_style_representative_cells = false;
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data =
            br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="999"><c r="A999" s="7"/></row>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="1""#));
        assert!(!cleaned.contains(r#"r="999""#));
    }

    #[test]
    fn trailing_self_closing_visible_fill_cell_keeps_tail() {
        let params = params_with_styles(&[(7, visible_fill_style())]);
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data =
            br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="999"><c r="A999" s="7"/></row>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="1""#));
        assert!(cleaned.contains(r#"r="999""#));
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
