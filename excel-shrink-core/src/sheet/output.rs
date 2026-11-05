use crate::error::Result;
use crate::models::{
    CleanSheetMetrics, CleanSheetParameters, CleanedSheetData, CleanupCause, ColumnRangeMap,
    VisibleContentBounds, Xf,
};
use crate::sheet::cols::merge_extraneous_columns_with_styles;
use crate::sheet::drawings::find_worksheet_drawing_anchor_bounds;
use crate::sheet::ranges::{
    find_visible_content_bounds, get_max_cell_column, get_max_cell_row,
    get_max_merge_cells_column_index, get_max_merge_cells_row_index,
    retrieve_sheet_data_range_containing_cells,
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
use std::ops::Range;
use std::sync::OnceLock;

static SHEET_PROTECTION_RE: OnceLock<Regex> = OnceLock::new();
static CELL_RE: OnceLock<Regex> = OnceLock::new();

#[cfg(test)]
#[path = "tail_fill_regression_tests.rs"]
mod tail_fill_regression_tests;

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
    let mut preprocessed = preprocessed;
    let drawing_anchor_bounds = prepare_sheet_metrics(&mut preprocessed, params, progress);
    let initial_max_merge_row = get_max_merge_cells_row_index(&preprocessed.clean_sheet_metrics);
    let preserve_through_row =
        tail_boundary_row(&preprocessed.clean_sheet_metrics, initial_max_merge_row);

    let mut final_sheet_data = clean_sheet_data(
        &preprocessed.sheet_data,
        params,
        &mut preprocessed.clean_sheet_metrics,
        disable_sheetdata_splitting,
        parallel_range_chunk_size,
        preserve_through_row,
        progress,
    )?;
    final_sheet_data = trim_layout_rows_if_enabled(
        final_sheet_data,
        params,
        &mut preprocessed.clean_sheet_metrics,
        initial_max_merge_row,
        "trim_trailing_layout_only_rows",
        progress,
    );

    progress("compute_visible_bounds");
    let bounds = compute_sheet_bounds(&preprocessed.clean_sheet_metrics, drawing_anchor_bounds);

    progress("remove_protection_only_cells_outside_bounds");
    final_sheet_data = remove_protection_only_cells_outside_bounds(
        final_sheet_data,
        bounds.max_column,
        bounds.max_row,
        params,
        &mut preprocessed.clean_sheet_metrics,
    );
    final_sheet_data = trim_layout_rows_if_enabled(
        final_sheet_data,
        params,
        &mut preprocessed.clean_sheet_metrics,
        bounds.max_merge_row,
        "trim_trailing_layout_only_rows_after_bounds",
        progress,
    );

    let columns = clean_columns(
        &preprocessed.in_cols_node_output_chunks,
        bounds,
        params,
        &mut preprocessed.clean_sheet_metrics,
        progress,
    );

    progress("assemble_clean_sheet_xml");
    let assembled = assemble_clean_sheet_xml(&preprocessed, &columns, &final_sheet_data);

    progress("scan_reference_hints");
    let mut metrics = preprocessed.clean_sheet_metrics;
    metrics.has_hints = crate::reference_scanner::check_for_reference_hints(&assembled.xml);
    metrics.source_content_cell_count_complete = true;

    Ok(CleanedSheetData {
        xml: assembled.xml,
        sheet_data_range: assembled.sheet_data_range,
        metrics,
    })
}

fn prepare_sheet_metrics(
    preprocessed: &mut PreprocessedSheetXml,
    params: &CleanSheetParameters,
    progress: &mut impl FnMut(&'static str),
) -> VisibleContentBounds {
    progress("build_prepare_sheet_metrics");
    let sheet_protection_enabled = worksheet_has_enabled_sheet_protection(preprocessed);
    let drawing_anchor_bounds = find_worksheet_drawing_anchor_bounds(preprocessed, params);
    // Only this policy consults the original visible data rectangle. Other
    // policies can avoid a complete preliminary scan of every row and cell.
    let visible_data_range_bounds = if params.cleanup.inactive_protection_cell_policy
        == crate::models::InactiveProtectionCellPolicy::KeepInDataRange
    {
        find_visible_content_bounds(
            &preprocessed.sheet_data,
            params,
            &preprocessed.clean_sheet_metrics.column_definitions,
        )
    } else {
        VisibleContentBounds::default()
    };

    let metrics = &mut preprocessed.clean_sheet_metrics;
    metrics.cleanup_stats.enabled |= params.collect_cleanup_stats;
    metrics.sheet_protection_enabled = sheet_protection_enabled;
    metrics.visible_data_range_bounds = visible_data_range_bounds;
    include_drawing_anchor_bounds(metrics, drawing_anchor_bounds);
    include_last_visible_column(metrics);

    drawing_anchor_bounds
}

fn include_drawing_anchor_bounds(
    metrics: &mut CleanSheetMetrics,
    drawing_anchor_bounds: VisibleContentBounds,
) {
    if let Some(row) = drawing_anchor_bounds.max_row {
        metrics.visible_content_bounds.include_row(row);
        metrics.record_tail_anchor_row(row);
    }
    if let Some(column) = drawing_anchor_bounds.max_col {
        metrics.visible_content_bounds.include_col(column);
    }
}

fn include_last_visible_column(metrics: &mut CleanSheetMetrics) {
    if let Some(column) = metrics.column_definitions.get_last_visible_range(false) {
        metrics.visible_content_bounds.include_col(column.max);
    }
}

fn clean_sheet_data(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    disable_sheetdata_splitting: bool,
    parallel_range_chunk_size: usize,
    preserve_through_row: Option<u32>,
    progress: &mut impl FnMut(&'static str),
) -> Result<Vec<u8>> {
    let use_range_tail_pruning =
        !disable_sheetdata_splitting && params.cleanup.sheetdata_range_tail_pruning;

    if use_range_tail_pruning {
        clean_pruned_sheet_data(
            sheet_data,
            params,
            metrics,
            parallel_range_chunk_size,
            preserve_through_row,
            progress,
        )
    } else {
        progress("clean_sheet_rows_and_cells");
        clean_sheet_rows_and_cells_in_cell_range(sheet_data, params, metrics)
    }
}

fn clean_pruned_sheet_data(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    parallel_range_chunk_size: usize,
    preserve_through_row: Option<u32>,
    progress: &mut impl FnMut(&'static str),
) -> Result<Vec<u8>> {
    progress("retrieve_sheet_data_ranges");
    let ranges = retrieve_sheet_data_range_containing_cells(
        sheet_data,
        params,
        metrics.sheet_protection_enabled,
        preserve_through_row,
    );
    if tail_contains_content_cell(&sheet_data[ranges.cells_range.len()..]) {
        metrics.tail_content_guard_triggered = true;
        progress("clean_content_cell_tail");
        return clean_cells_range(
            sheet_data,
            params,
            metrics,
            parallel_range_chunk_size,
            progress,
        );
    }
    record_pruned_sheet_data_tail(sheet_data.len(), &ranges, metrics);

    let cleaned_cells = clean_cells_range(
        ranges.cells_range,
        params,
        metrics,
        parallel_range_chunk_size,
        progress,
    )?;
    let cleaned_rows = clean_visible_rows_range(
        ranges.rows_range,
        params,
        metrics,
        parallel_range_chunk_size,
        progress,
    )?;

    Ok([cleaned_cells, cleaned_rows].concat())
}

fn record_pruned_sheet_data_tail(
    original_len: usize,
    ranges: &crate::sheet::ranges::VisibleSheetDataRanges<'_>,
    metrics: &mut CleanSheetMetrics,
) {
    let kept_len = ranges.cells_range.len() + ranges.rows_range.len();
    if original_len > kept_len {
        metrics.cleanup_stats.record_removed(
            CleanupCause::SheetDataRangeTailPruning,
            original_len - kept_len,
        );
    }
}

fn clean_cells_range(
    cells_range: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    parallel_range_chunk_size: usize,
    progress: &mut impl FnMut(&'static str),
) -> Result<Vec<u8>> {
    if should_parallel_chunk_range(cells_range.len(), parallel_range_chunk_size) {
        progress("clean_cells_range_parallel");
        clean_sheet_rows_and_cells_in_cell_range_parallel_by_max_chunk_size(
            cells_range,
            params,
            metrics,
            parallel_range_chunk_size,
            "cells_range",
            &params.zip_info_filename,
        )
    } else {
        progress("clean_cells_range");
        clean_sheet_rows_and_cells_in_cell_range(cells_range, params, metrics)
    }
}

fn clean_visible_rows_range(
    rows_range: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    parallel_range_chunk_size: usize,
    progress: &mut impl FnMut(&'static str),
) -> Result<Vec<u8>> {
    if should_parallel_chunk_range(rows_range.len(), parallel_range_chunk_size) {
        progress("clean_visible_rows_range_parallel");
        clean_empty_rows_from_after_cell_range_parallel_by_max_chunk_size(
            rows_range,
            params,
            metrics,
            parallel_range_chunk_size,
            "rows_range",
            &params.zip_info_filename,
        )
    } else {
        progress("clean_visible_rows_range");
        clean_empty_rows_from_after_cell_range(rows_range, params, metrics)
    }
}

fn trim_layout_rows_if_enabled(
    sheet_data: Vec<u8>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    preserve_through_row: Option<u32>,
    progress_stage: &'static str,
    progress: &mut impl FnMut(&'static str),
) -> Vec<u8> {
    if !params.cleanup.trailing_layout_only_rows {
        return sheet_data;
    }

    progress(progress_stage);
    trim_trailing_layout_only_rows_vec_preserving_rows_through(
        sheet_data,
        params,
        metrics,
        preserve_through_row,
    )
}

fn compute_sheet_bounds(
    metrics: &CleanSheetMetrics,
    drawing_anchor_bounds: VisibleContentBounds,
) -> SheetBounds {
    let max_cell_column = get_max_cell_column(metrics);
    let max_cell_row = get_max_cell_row(metrics);
    let max_merge_column = get_max_merge_cells_column_index(metrics);
    let max_merge_row = get_max_merge_cells_row_index(metrics);

    SheetBounds {
        column_tail_merge_boundary: maximum_bound([
            max_cell_column,
            max_merge_column,
            drawing_anchor_bounds.max_col,
        ]),
        max_column: maximum_bound([
            max_cell_column,
            max_merge_column,
            metrics.visible_content_bounds.max_col,
        ]),
        max_row: maximum_bound([
            max_cell_row,
            max_merge_row,
            metrics.visible_content_bounds.max_row,
        ]),
        max_merge_row,
    }
}

#[inline]
fn maximum_bound(bounds: [Option<u32>; 3]) -> u32 {
    bounds.into_iter().flatten().max().unwrap_or(0)
}

/// Merge invisible tail definitions while retaining column boundaries exposed by row borders.
fn clean_columns(
    columns: &[u8],
    bounds: SheetBounds,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    progress: &mut impl FnMut(&'static str),
) -> Vec<u8> {
    progress("merge_extraneous_columns");
    let cleaned_columns = if params.cleanup.extraneous_column_range_merge
        && !metrics.has_visible_row_border
    {
        merge_extraneous_columns_with_styles(
            columns,
            column_merge_boundary(bounds, params),
            params,
            params.cleanup.column_tail_fill_merge,
        )
    } else {
        columns.to_vec()
    };

    metrics.cleanup_stats.record_delta(
        CleanupCause::ExtraneousColumnRangeMerge,
        columns.len(),
        cleaned_columns.len(),
    );
    cleaned_columns
}

#[inline]
fn column_merge_boundary(bounds: SheetBounds, params: &CleanSheetParameters) -> u32 {
    if params.cleanup.column_tail_fill_merge {
        bounds.column_tail_merge_boundary
    } else {
        bounds.max_column
    }
}

fn assemble_clean_sheet_xml(
    preprocessed: &PreprocessedSheetXml,
    columns: &[u8],
    sheet_data: &[u8],
) -> AssembledSheetXml {
    let mut xml = Vec::new();
    xml.extend_from_slice(&preprocessed.until_worksheet_start_output_chunks);
    xml.extend_from_slice(&preprocessed.before_cols_node_output_chunks);
    xml.extend_from_slice(columns);
    xml.extend_from_slice(&preprocessed.before_sheet_data_node_output_chunks);
    let sheet_data_start = xml.len();
    xml.extend_from_slice(sheet_data);
    let sheet_data_end = xml.len();
    xml.extend_from_slice(&preprocessed.before_merge_cells_node_output_chunks);
    xml.extend_from_slice(&preprocessed.in_merge_cells_node_output_chunks);
    xml.extend_from_slice(&preprocessed.until_worksheet_end_output_chunks);

    AssembledSheetXml {
        xml,
        sheet_data_range: sheet_data_start..sheet_data_end,
    }
}

#[derive(Debug)]
struct AssembledSheetXml {
    xml: Vec<u8>,
    sheet_data_range: Range<usize>,
}

#[derive(Debug, Clone, Copy)]
struct SheetBounds {
    max_column: u32,
    max_row: u32,
    max_merge_row: Option<u32>,
    column_tail_merge_boundary: u32,
}

fn remove_protection_only_cells_outside_bounds(
    sheet_data: Vec<u8>,
    max_column: u32,
    max_row: u32,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    if keep_all_inactive_protection_cells(params, metrics) {
        return sheet_data;
    }

    let protected_outside_refs =
        protection_only_references_outside_bounds(metrics, max_column, max_row);
    if protected_outside_refs.is_empty() {
        return sheet_data;
    }

    remove_cells_with_references(sheet_data, &protected_outside_refs, metrics)
}

#[inline]
fn keep_all_inactive_protection_cells(
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
) -> bool {
    !metrics.sheet_protection_enabled
        && params.cleanup.inactive_protection_cell_policy
            == crate::models::InactiveProtectionCellPolicy::KeepAll
}

fn protection_only_references_outside_bounds(
    metrics: &CleanSheetMetrics,
    max_column: u32,
    max_row: u32,
) -> HashSet<(u32, u32)> {
    metrics
        .kept_cell_states
        .iter()
        .filter(|cell| cell.kept_only_for_protection)
        .filter(|cell| cell.col > max_column || cell.row > max_row)
        .map(|cell| (cell.col, cell.row))
        .collect()
}

fn remove_cells_with_references(
    sheet_data: Vec<u8>,
    references: &HashSet<(u32, u32)>,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let cell_re =
        CELL_RE.get_or_init(|| Regex::new(r#"(?s)<c\b([^>]*)/>|<c\b([^>]*)>.*?</c>"#).unwrap());
    let mut out = Vec::with_capacity(sheet_data.len().min(64 * 1024 * 1024));
    let mut last_end = 0;

    for captures in cell_re.captures_iter(&sheet_data) {
        let Some(cell_match) = captures.get(0) else {
            continue;
        };
        out.extend_from_slice(&sheet_data[last_end..cell_match.start()]);

        if captured_cell_has_reference(&captures, references) {
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

fn captured_cell_has_reference(
    captures: &regex::bytes::Captures<'_>,
    references: &HashSet<(u32, u32)>,
) -> bool {
    let attributes = captures
        .get(1)
        .or_else(|| captures.get(2))
        .map_or(b"".as_slice(), |capture| capture.as_bytes());

    extract_attribute_value(attributes, b"r=\"")
        .and_then(parse_cell_reference)
        .is_some_and(|reference| references.contains(&reference))
}
pub fn trim_trailing_layout_only_rows(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    trim_trailing_layout_only_rows_preserving_rows_through(sheet_data, params, metrics, None)
}

fn trim_trailing_layout_only_rows_preserving_rows_through(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    preserve_through_row: Option<u32>,
) -> Vec<u8> {
    let boundary_row = tail_boundary_row(metrics, preserve_through_row);

    if let Some(removal_range) = unbounded_hidden_tail_removal_range(sheet_data, boundary_row) {
        record_trailing_row_removal(metrics, removal_range);
        return copy_without_range(sheet_data, removal_range);
    }

    let end = trailing_layout_only_rows_cutoff(sheet_data, params, metrics, boundary_row);
    sheet_data[..end].to_vec()
}

pub fn trim_trailing_layout_only_rows_vec(
    sheet_data: Vec<u8>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    trim_trailing_layout_only_rows_vec_preserving_rows_through(sheet_data, params, metrics, None)
}

fn trim_trailing_layout_only_rows_vec_preserving_rows_through(
    mut sheet_data: Vec<u8>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    preserve_through_row: Option<u32>,
) -> Vec<u8> {
    let boundary_row = tail_boundary_row(metrics, preserve_through_row);

    if let Some(removal_range) = unbounded_hidden_tail_removal_range(&sheet_data, boundary_row) {
        sheet_data.drain(removal_range.0..removal_range.1);
        record_trailing_row_removal(metrics, removal_range);
        return sheet_data;
    }

    let end = trailing_layout_only_rows_cutoff(&sheet_data, params, metrics, boundary_row);
    sheet_data.truncate(end);
    sheet_data
}

fn unbounded_hidden_tail_removal_range(
    sheet_data: &[u8],
    boundary_row: Option<u32>,
) -> Option<(usize, usize)> {
    if boundary_row.is_some() {
        return None;
    }

    trailing_hidden_tail_removal_range(sheet_data, None)
}

fn copy_without_range(sheet_data: &[u8], range: (usize, usize)) -> Vec<u8> {
    let mut out = Vec::with_capacity(sheet_data.len() - (range.1 - range.0));
    out.extend_from_slice(&sheet_data[..range.0]);
    out.extend_from_slice(&sheet_data[range.1..]);
    out
}

fn record_trailing_row_removal(metrics: &mut CleanSheetMetrics, range: (usize, usize)) {
    metrics
        .cleanup_stats
        .record_removed(CleanupCause::TrailingLayoutOnlyRows, range.1 - range.0);
}
fn tail_boundary_row(
    metrics: &CleanSheetMetrics,
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
    let preserved_row = contiguous_trailing_row(sheet_data, sheet_data.len())?;
    if !is_removable_hidden_tail_row(preserved_row.raw, preserve_through_row) {
        return None;
    }

    let preserve_start = preserved_row.start;
    let mut remove_start = preserve_start;
    let mut scan_end = preserve_start;

    while let Some(row) = contiguous_trailing_row(sheet_data, scan_end) {
        if !is_removable_hidden_tail_row(row.raw, preserve_through_row) {
            break;
        }
        remove_start = row.start;
        scan_end = row.start;
    }

    (remove_start < preserve_start).then_some((remove_start, preserve_start))
}

#[inline]
fn is_removable_hidden_tail_row(row: &[u8], preserve_through_row: Option<u32>) -> bool {
    hidden_row_without_cells(row) && !row_is_protected_tail_boundary(row, preserve_through_row)
}

#[inline]
fn hidden_row_without_cells(row: &[u8]) -> bool {
    row_is_hidden(row) && find_bytes(row, b"<c").is_none()
}

fn contiguous_trailing_row(sheet_data: &[u8], end: usize) -> Option<TrailingRow<'_>> {
    let row_end_boundary = trim_trailing_whitespace_end(sheet_data, end);
    let row_start = rfind_bytes(&sheet_data[..row_end_boundary], b"<row")?;
    let row_end = find_row_end(sheet_data, row_start)?;

    (row_end == row_end_boundary).then_some(TrailingRow {
        start: row_start,
        raw: &sheet_data[row_start..row_end],
    })
}

#[inline]
fn trim_trailing_whitespace_end(data: &[u8], mut end: usize) -> usize {
    while end > 0 && data[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end
}

fn trailing_layout_only_rows_cutoff(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    preserve_through_row: Option<u32>,
) -> usize {
    if let Some(max_row) = preserve_through_row {
        return cutoff_after_row_boundary(sheet_data, max_row, params, metrics);
    }

    remove_trailing_layout_rows(sheet_data, params, metrics)
}

fn remove_trailing_layout_rows(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> usize {
    let mut end = sheet_data.len();

    while let Some(row) = contiguous_trailing_row(sheet_data, end) {
        if row_is_protected_tail_boundary(row.raw, None)
            || !trailing_row_is_layout_only(row.raw, params, &metrics.column_definitions)
        {
            break;
        }

        metrics
            .cleanup_stats
            .record_removed(CleanupCause::TrailingLayoutOnlyRows, row.raw.len());
        end = row.start;
    }

    end
}

fn cutoff_after_row_boundary(
    sheet_data: &[u8],
    max_row: u32,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> usize {
    let cutoff = cutoff_after_exact_row_reference(sheet_data, max_row)
        .map(|end| {
            extend_cutoff_to_visible_row_before_hidden_tail(
                sheet_data,
                end,
                params,
                &metrics.column_definitions,
            )
        })
        .unwrap_or_else(|| {
            scan_cutoff_after_row_boundary(sheet_data, max_row, params, &metrics.column_definitions)
        });

    if tail_contains_content_cell(&sheet_data[cutoff..]) {
        metrics.tail_content_guard_triggered = true;
        return sheet_data.len();
    }

    record_removed_sheet_data_tail(metrics, sheet_data.len(), cutoff);
    cutoff
}

/// A range heuristic may miss cells whose opening tag has no literal space
/// after `<c`. Scan only the suffix outside its recognized cell range.
fn tail_contains_content_cell(tail: &[u8]) -> bool {
    let mut position = 0;

    while let Some(relative) = find_bytes(&tail[position..], b"<c") {
        let cell_start = position + relative;
        let after_name = cell_start + b"<c".len();
        if !xml_name_is_delimited(tail, after_name) {
            position = after_name;
            continue;
        }

        let Some(tag_end) = find_byte_from(tail, b'>', cell_start) else {
            return true;
        };
        if tag_end > cell_start && tail[tag_end - 1] == b'/' {
            position = tag_end + 1;
            continue;
        }

        let content_start = tag_end + 1;
        let Some(close_relative) = find_bytes(&tail[content_start..], b"</c>") else {
            return true;
        };
        let content_end = content_start + close_relative;
        if cell_has_content_markup(&tail[content_start..content_end]) {
            return true;
        }
        position = content_end + b"</c>".len();
    }

    false
}

fn cell_has_content_markup(cell_content: &[u8]) -> bool {
    let mut position = 0;
    while let Some(tag_start) = find_byte_from(cell_content, b'<', position) {
        let tag = &cell_content[tag_start..];
        if tag.starts_with(b"<![CDATA[")
            || (tag.starts_with(b"<v") && xml_name_is_delimited(cell_content, tag_start + 2))
            || (tag.starts_with(b"<f") && xml_name_is_delimited(cell_content, tag_start + 2))
            || (tag.starts_with(b"<is") && xml_name_is_delimited(cell_content, tag_start + 3))
        {
            return true;
        }
        position = tag_start + 1;
    }
    false
}

fn scan_cutoff_after_row_boundary(
    sheet_data: &[u8],
    max_row: u32,
    params: &CleanSheetParameters,
    columns: &ColumnRangeMap,
) -> usize {
    let mut end = sheet_data.len();

    loop {
        let scan_end = trim_trailing_whitespace_end(sheet_data, end);
        let Some(row_start) = rfind_bytes(&sheet_data[..scan_end], b"<row") else {
            return 0;
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
            let cutoff = if end == sheet_data.len() {
                sheet_data.len()
            } else {
                row_end
            };
            return extend_cutoff_to_visible_row_before_hidden_tail(
                sheet_data, cutoff, params, columns,
            );
        }

        end = row_start;
    }
}

fn record_removed_sheet_data_tail(
    metrics: &mut CleanSheetMetrics,
    sheet_data_len: usize,
    cutoff: usize,
) {
    if cutoff < sheet_data_len {
        metrics.cleanup_stats.record_removed(
            CleanupCause::TrailingLayoutOnlyRows,
            sheet_data_len - cutoff,
        );
    }
}

#[derive(Debug, Clone, Copy)]
struct TrailingRow<'a> {
    start: usize,
    raw: &'a [u8],
}
fn extend_cutoff_to_visible_row_before_hidden_tail(
    sheet_data: &[u8],
    cutoff: usize,
    params: &CleanSheetParameters,
    columns: &ColumnRangeMap,
) -> usize {
    let Some((layout_row, layout_row_end)) = next_row_after_cutoff(sheet_data, cutoff) else {
        return cutoff;
    };

    if row_is_hidden(layout_row) || !trailing_row_is_layout_only(layout_row, params, columns) {
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
    if !attribute_is_inside_open_tag(attr_start, attr_len, open_end)
        || !attribute_has_xml_boundaries(sheet_data, attr_start, attr_len)
    {
        return None;
    }

    Some(row_start)
}

#[inline]
fn attribute_is_inside_open_tag(attr_start: usize, attr_len: usize, open_end: usize) -> bool {
    attr_start < open_end && attr_start + attr_len <= open_end
}

#[inline]
fn attribute_has_xml_boundaries(sheet_data: &[u8], attr_start: usize, attr_len: usize) -> bool {
    sheet_data
        .get(attr_start.wrapping_sub(1))
        .is_some_and(u8::is_ascii_whitespace)
        && sheet_data
            .get(attr_start + attr_len)
            .is_none_or(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
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
    extract_attribute_value(&row[..open_end], b"r=\"")
        .map(crate::utils::digits_byte_string_to_integer)
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

fn trailing_row_is_layout_only(
    row: &[u8],
    params: &CleanSheetParameters,
    columns: &ColumnRangeMap,
) -> bool {
    !row_has_visible_cell_anchor(row, params, columns)
        && (row_is_hidden(row) || !row_has_visible_style(row, params, columns))
}

fn row_has_visible_cell_anchor(
    row: &[u8],
    params: &CleanSheetParameters,
    columns: &ColumnRangeMap,
) -> bool {
    let mut search_start = 0;

    while let Some(cell_tag) = next_cell_tag(row, search_start) {
        match cell_tag {
            ScannedCellTag::Anchoring => return true,
            ScannedCellTag::SelfClosing { attributes, end } => {
                if self_closing_cell_is_anchor(attributes, params, columns) {
                    return true;
                }
                search_start = end;
            }
        }
    }

    false
}

fn next_cell_tag(row: &[u8], mut search_start: usize) -> Option<ScannedCellTag<'_>> {
    while let Some(relative) = find_bytes(&row[search_start..], b"<c") {
        let cell_start = search_start + relative;
        let after_name = cell_start + b"<c".len();

        if !xml_name_is_delimited(row, after_name) {
            search_start = after_name;
            continue;
        }

        let Some(tag_end) = find_byte_from(row, b'>', cell_start) else {
            return Some(ScannedCellTag::Anchoring);
        };
        if tag_end <= cell_start || row[tag_end - 1] != b'/' {
            return Some(ScannedCellTag::Anchoring);
        }

        return Some(ScannedCellTag::SelfClosing {
            attributes: &row[after_name..tag_end - 1],
            end: tag_end + 1,
        });
    }

    None
}

#[inline]
fn xml_name_is_delimited(xml: &[u8], after_name: usize) -> bool {
    xml.get(after_name)
        .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
}

fn self_closing_cell_is_anchor(
    attributes: &[u8],
    params: &CleanSheetParameters,
    columns: &ColumnRangeMap,
) -> bool {
    match extract_style_id(attributes) {
        Some(style_id) => params
            .patterns
            .xfs_by_index
            .get(&style_id)
            .map_or(true, |style| {
                style_is_trailing_cell_anchor(style, params, columns)
            }),
        // Row fills/borders are checked by trailing_row_is_layout_only. A
        // styleless cell can still mask a visible column format. Its font alone
        // cannot anchor an otherwise empty tail, even when representatives stay
        // enabled for rows before content. Malformed coordinates stay guarded.
        None => extract_attribute_value(attributes, b"r=\"")
            .and_then(parse_cell_reference)
            .is_none_or(|(column, _)| {
                columns
                    .get(column)
                    .and_then(|column| column.cell_xf.as_ref())
                    .is_some_and(|style| style.has_visible_fill || style.has_visible_border)
            }),
    }
}

#[derive(Debug, Clone, Copy)]
enum ScannedCellTag<'a> {
    Anchoring,
    SelfClosing { attributes: &'a [u8], end: usize },
}
#[inline]
fn row_is_hidden(row: &[u8]) -> bool {
    find_bytes(row, b" hidden=\"1\"").is_some() || find_bytes(row, b" hidden=\"true\"").is_some()
}

fn row_has_visible_style(
    row: &[u8],
    params: &CleanSheetParameters,
    columns: &ColumnRangeMap,
) -> bool {
    let Some(style_id) = extract_style_id(row) else {
        return false;
    };

    let Some(end) = find_byte_from(row, b'>', 0) else {
        return true;
    };
    let attributes = &row[4..end - usize::from(row.get(end.wrapping_sub(1)) == Some(&b'/'))];
    params
        .patterns
        .xfs_by_index
        .get(&style_id)
        .is_none_or(|style| columns.row_style_has_tail_visual_effect(style, attributes))
}

#[inline]
fn style_is_trailing_cell_anchor(
    style: &Xf,
    params: &CleanSheetParameters,
    columns: &ColumnRangeMap,
) -> bool {
    columns.style_has_tail_visual_effect(style)
        || (style.has_apply_protection
            && params.cleanup.inactive_protection_cell_policy
                == crate::models::InactiveProtectionCellPolicy::KeepAll)
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
    match extract_attribute_value(tag, b"sheet=\"") {
        Some(value) if value == b"0" || value.eq_ignore_ascii_case(b"false") => false,
        _ => true,
    }
}

fn extract_attribute_value<'a>(tag: &'a [u8], needle: &[u8]) -> Option<&'a [u8]> {
    let mut search_start = 0;
    while search_start < tag.len() {
        let relative = find_bytes(&tag[search_start..], needle)?;
        let attribute_start = search_start + relative;

        if attribute_name_has_left_boundary(tag, attribute_start) {
            return quoted_value_after(tag, attribute_start + needle.len());
        }

        search_start = attribute_start + 1;
    }

    None
}

#[inline]
fn attribute_name_has_left_boundary(tag: &[u8], attribute_start: usize) -> bool {
    attribute_start == 0 || tag[attribute_start - 1].is_ascii_whitespace()
}

fn quoted_value_after(tag: &[u8], value_start: usize) -> Option<&[u8]> {
    let value_end = value_start + tag[value_start..].iter().position(|byte| *byte == b'"')?;
    Some(&tag[value_start..value_end])
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
        CleanSheetMetrics, CleanSheetParameters, CleanupCause, InactiveProtectionCellPolicy,
        SheetPreprocessConfig, WorkbookSpecificPatterns, Xf,
    };
    use crate::sheet::streaming::PreprocessedSheetXml;
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::FileOptions;
    use zip::CompressionMethod;

    pub(super) fn default_params() -> CleanSheetParameters {
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

    pub(super) fn preprocessed_with_sheet_data(
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
    fn range_tail_pruning_preserves_content_cells_with_other_tag_delimiters() {
        let params = default_params();
        let sheet_data = br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c
r="A2"><v>42</v></c></row><row r="3"><c	r="A3"><f>1+1</f></c></row><row r="4"><c> <is><t>text</t></is></c></row>"#;
        let cleaned = build_clean_sheet_xml(
            preprocessed_with_sheet_data(sheet_data, None),
            &params,
            false,
        )
        .unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains("<v>42</v>"));
        assert!(xml.contains("<f>1+1</f>"));
        assert!(xml.contains("<is><t>text</t></is>"));
        assert_eq!(cleaned.metrics.source_content_cell_count, 4);
        assert!(cleaned.metrics.tail_content_guard_triggered);

        let mut metrics = cleaned.metrics;
        let touched_rows = std::collections::HashSet::from([2]);
        crate::sheet::sheet_data::clean_selected_rows_and_cells_in_cell_range(
            sheet_data,
            &params,
            &mut metrics,
            &touched_rows,
        )
        .unwrap();
        assert_eq!(metrics.source_content_cell_count, 4);
    }

    #[test]
    fn content_cell_in_visible_rows_range_is_counted() {
        let params = default_params();
        let sheet_data = b"<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"2\" hidden=\"1\"><c\nr=\"A2\"><v>42</v></c></row><row r=\"3\"/>";
        let ranges = crate::sheet::ranges::retrieve_sheet_data_range_containing_cells(
            sheet_data, &params, false, None,
        );
        assert!(ranges
            .rows_range
            .windows(b"<v>42</v>".len())
            .any(|window| window == b"<v>42</v>"));

        let cleaned = build_clean_sheet_xml(
            preprocessed_with_sheet_data(sheet_data, None),
            &params,
            false,
        )
        .unwrap();

        assert!(cleaned
            .xml
            .windows(b"<v>42</v>".len())
            .any(|window| window == b"<v>42</v>"));
        assert_eq!(cleaned.metrics.source_content_cell_count, 2);
        assert!(cleaned.metrics.tail_content_guard_triggered);
    }

    #[test]
    fn row_boundary_cutoff_preserves_later_content_cell() {
        let params = default_params();
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data = b"<row r=\"1\"/><row r=\"2\"><c\nr=\"A2\"><v>42</v></c></row>";

        let cutoff = super::cutoff_after_row_boundary(sheet_data, 1, &params, &mut metrics);

        assert_eq!(cutoff, sheet_data.len());
        assert!(metrics.tail_content_guard_triggered);
    }

    #[test]
    fn candidate_tail_scan_preserves_cdata_and_skips_empty_cells() {
        assert!(super::tail_contains_content_cell(
            b"<row r=\"2\"><c r=\"A2\"><is><t><![CDATA[text]]></t></is></c></row>"
        ));
        assert!(!super::tail_contains_content_cell(
            b"<row r=\"2\"><c r=\"A2\"/><c r=\"B2\"></c></row>"
        ));
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
    fn visible_row_borders_preserve_empty_column_widths() {
        let border = Xf::builder().border_id(Some(1)).apply_border(true)
            .has_visible_border(true).has_fill_or_border(true)
            .has_apply_fill_or_border(true).build_xf();
        let params = params_with_styles(&[(9, border)]);
        let cols = br#"<col min="1" max="1" width="10"/><col min="2" max="2" width="40"/><col min="3" max="16384" width="9"/>"#;
        let preprocessed = PreprocessedSheetXml {
            until_worksheet_start_output_chunks: b"<worksheet>".to_vec(),
            before_cols_node_output_chunks: b"<cols>".to_vec(),
            in_cols_node_output_chunks: cols.to_vec(),
            before_sheet_data_node_output_chunks: b"</cols><sheetData>".to_vec(),
            sheet_data: br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" s="9" customFormat="1"/>"#.to_vec(),
            before_merge_cells_node_output_chunks: Vec::new(),
            in_merge_cells_node_output_chunks: Vec::new(),
            until_worksheet_end_output_chunks: b"</sheetData></worksheet>".to_vec(),
            clean_sheet_metrics: CleanSheetMetrics::default(),
        };
        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        assert!(cleaned.metrics.has_visible_row_border);
        assert!(cleaned.xml.windows(cols.len()).any(|part| part == cols));
    }

    #[test]
    fn visible_column_styles_after_last_cell_do_not_block_tail_column_merge() {
        let visible_column_style = Xf::builder()
            .font_id(Some(3))
            .fill_id(Some(2))
            .border_id(Some(0))
            .apply_fill(true)
            .has_visible_fill(true)
            .has_fill_or_border(true)
            .has_apply_fill_or_border(true)
            .build_xf();
        let params = params_with_styles(&[(39, visible_column_style)]);
        let cols = br#"<col min="1" max="20" width="5.21875" style="39" customWidth="1"/><col min="21" max="239" width="8.77734375" style="39" customWidth="1"/><col min="240" max="16148" width="11.5546875" style="39" customWidth="1"/><col min="16149" max="16384" width="8.77734375" style="39" customWidth="1"/>"#;
        let mut metrics = CleanSheetMetrics::default();
        metrics.column_definitions = crate::sheet::cols::process_cols_node(cols, &params);

        let preprocessed = PreprocessedSheetXml {
            until_worksheet_start_output_chunks: b"<worksheet>".to_vec(),
            before_cols_node_output_chunks: b"<cols>".to_vec(),
            in_cols_node_output_chunks: cols.to_vec(),
            before_sheet_data_node_output_chunks: b"</cols><sheetData>".to_vec(),
            sheet_data: br#"<row r="5"><c r="T5" s="39"><v>1</v></c></row>"#.to_vec(),
            before_merge_cells_node_output_chunks: b"".to_vec(),
            in_merge_cells_node_output_chunks: b"".to_vec(),
            until_worksheet_end_output_chunks: b"</sheetData></worksheet>".to_vec(),
            clean_sheet_metrics: metrics,
        };

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<c r="T5" s="39"><v>1</v></c>"#));
        assert!(xml.contains(
            r#"<col min="21" max="16384" width="8.77734375" style="39" customWidth="1"/>"#
        ));
        assert!(!xml.contains(r#"<col min="21" max="239""#));
        assert!(!xml.contains(r#"<col min="240" max="16148""#));
        assert!(cleaned
            .metrics
            .cleanup_stats
            .removed_bytes_by_cause
            .is_empty());
    }

    #[test]
    fn column_tail_fill_merge_can_be_disabled_at_sheet_level() {
        let visible_column_style = Xf::builder()
            .font_id(Some(3))
            .fill_id(Some(2))
            .border_id(Some(0))
            .apply_fill(true)
            .has_visible_fill(true)
            .has_fill_or_border(true)
            .has_apply_fill_or_border(true)
            .build_xf();
        let mut params = params_with_styles(&[(39, visible_column_style)]);
        params.cleanup.column_tail_fill_merge = false;
        let cols = br#"<col min="1" max="20" width="5.21875" style="39" customWidth="1"/><col min="21" max="239" width="8.77734375" style="39" customWidth="1"/><col min="240" max="16148" width="11.5546875" style="39" customWidth="1"/><col min="16149" max="16384" width="8.77734375" style="39" customWidth="1"/>"#;
        let mut metrics = CleanSheetMetrics::default();
        metrics.column_definitions = crate::sheet::cols::process_cols_node(cols, &params);

        let preprocessed = PreprocessedSheetXml {
            until_worksheet_start_output_chunks: b"<worksheet>".to_vec(),
            before_cols_node_output_chunks: b"<cols>".to_vec(),
            in_cols_node_output_chunks: cols.to_vec(),
            before_sheet_data_node_output_chunks: b"</cols><sheetData>".to_vec(),
            sheet_data: br#"<row r="5"><c r="T5" s="39"><v>1</v></c></row>"#.to_vec(),
            before_merge_cells_node_output_chunks: b"".to_vec(),
            in_merge_cells_node_output_chunks: b"".to_vec(),
            until_worksheet_end_output_chunks: b"</sheetData></worksheet>".to_vec(),
            clean_sheet_metrics: metrics,
        };

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<col min="21" max="239""#));
        assert!(xml.contains(r#"<col min="240" max="16148""#));
        assert!(xml.contains(r#"<col min="16149" max="16384""#));
    }

    #[test]
    fn column_tail_fill_merge_records_saved_bytes_when_cleanup_stats_are_enabled() {
        let visible_column_style = Xf::builder()
            .font_id(Some(3))
            .fill_id(Some(2))
            .border_id(Some(0))
            .apply_fill(true)
            .has_visible_fill(true)
            .has_fill_or_border(true)
            .has_apply_fill_or_border(true)
            .build_xf();
        let mut params = params_with_styles(&[(39, visible_column_style)]);
        params.collect_cleanup_stats = true;
        let cols = br#"<col min="1" max="20" width="5.21875" style="39" customWidth="1"/><col min="21" max="239" width="8.77734375" style="39" customWidth="1"/><col min="240" max="16148" width="11.5546875" style="39" customWidth="1"/><col min="16149" max="16384" width="8.77734375" style="39" customWidth="1"/>"#;
        let collapsed_cols = br#"<col min="1" max="20" width="5.21875" style="39" customWidth="1"/><col min="21" max="16384" width="8.77734375" style="39" customWidth="1"/>"#;
        let mut metrics = CleanSheetMetrics::default();
        metrics.column_definitions = crate::sheet::cols::process_cols_node(cols, &params);

        let preprocessed = PreprocessedSheetXml {
            until_worksheet_start_output_chunks: b"<worksheet>".to_vec(),
            before_cols_node_output_chunks: b"<cols>".to_vec(),
            in_cols_node_output_chunks: cols.to_vec(),
            before_sheet_data_node_output_chunks: b"</cols><sheetData>".to_vec(),
            sheet_data: br#"<row r="5"><c r="T5" s="39"><v>1</v></c></row>"#.to_vec(),
            before_merge_cells_node_output_chunks: b"".to_vec(),
            in_merge_cells_node_output_chunks: b"".to_vec(),
            until_worksheet_end_output_chunks: b"</sheetData></worksheet>".to_vec(),
            clean_sheet_metrics: metrics,
        };

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let removed_bytes = cleaned
            .metrics
            .cleanup_stats
            .removed_bytes_by_cause
            .get(&CleanupCause::ExtraneousColumnRangeMerge)
            .copied();

        assert_eq!(removed_bytes, Some(cols.len() - collapsed_cols.len()));
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
    fn unprotected_sheet_default_keeps_inactive_protection_cell_inside_visible_data_range() {
        let params = params_with_styles(&[(7, apply_protection_only_style())]);
        let preprocessed = preprocessed_with_sheet_data(
            br#"<row r="1"><c r="B1" s="7"/></row><row r="2"><c r="C2"><v>1</v></c></row>"#,
            None,
        );

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"r="B1""#));
        assert!(xml.contains(r#"<c r="C2"><v>1</v></c>"#));
    }

    #[test]
    fn centered_text_keeps_only_its_contiguous_alignment_span() {
        let mut params = params_with_styles(&[(7, Xf {
            centers_across_selection: true,
            has_no_empty_cell_visible_style: true,
            ..Default::default()
        })]);
        params.cleanup.keep_textless_default_font_representatives = false;
        params.cleanup.keep_textless_style_representatives = false;
        params.cleanup.keep_textless_column_style_representatives = false;
        let preprocessed = preprocessed_with_sheet_data(
            br#"<row r="2"><c r="D2" s="7"><v>1</v></c><c r="E2" s="7"/><c r="F2" s="7"/><c r="G2" s="7"/></row><row r="3"><c r="G3" s="7"/></row><row r="4"><c r="A4" s="7"><v>1</v></c><c r="C4" s="7"/></row><row r="5"><c r="A5"><v>1</v></c><c r="B5" s="7"/></row>"#,
            None,
        );
        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);
        for address in ["D2", "E2", "F2", "G2"] {
            assert!(xml.contains(&format!("r=\"{address}\"")), "lost {address}");
        }
        for address in ["G3", "C4", "B5"] {
            assert!(!xml.contains(&format!("r=\"{address}\"")), "kept unanchored {address}");
        }
    }

    #[test]
    fn keep_in_data_range_keeps_inactive_protection_cell_inside_visible_data_range_only() {
        let mut params = params_with_styles(&[(7, apply_protection_only_style())]);
        params.cleanup.inactive_protection_cell_policy =
            InactiveProtectionCellPolicy::KeepInDataRange;
        let preprocessed = preprocessed_with_sheet_data(
            br#"<row r="1"><c r="B1" s="7"/></row><row r="2"><c r="C2"><v>1</v></c></row><row r="3"><c r="D3" s="7"/></row>"#,
            None,
        );

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<c r="B1" s="7"/>"#));
        assert!(!xml.contains(r#"r="D3""#));
        assert!(xml.contains(r#"<c r="C2"><v>1</v></c>"#));
    }

    #[test]
    fn keep_in_data_range_is_not_order_dependent_on_later_visible_content() {
        let mut params = params_with_styles(&[(7, apply_protection_only_style())]);
        params.cleanup.inactive_protection_cell_policy =
            InactiveProtectionCellPolicy::KeepInDataRange;
        let preprocessed = preprocessed_with_sheet_data(
            br#"<row r="1"><c r="B1" s="7"/></row><row r="5"><c r="C5"><v>1</v></c></row>"#,
            None,
        );

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<c r="B1" s="7"/>"#));
    }

    #[test]
    fn keep_all_keeps_inactive_protection_cell_outside_visible_data_range() {
        let mut params = params_with_styles(&[(7, apply_protection_only_style())]);
        params.cleanup.inactive_protection_cell_policy = InactiveProtectionCellPolicy::KeepAll;
        let preprocessed = preprocessed_with_sheet_data(
            br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="9"><c r="J9" s="7"/></row>"#,
            None,
        );

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<c r="J9" s="7"/>"#));
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
        preprocessed.clean_sheet_metrics.merge_cell_ranges =
            crate::sheet::merge_cells::find_merge_cell_ranges(merge_cells);

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"r="93""#));
        assert!(xml.contains(r#"r="94""#));
        assert!(xml.contains(r#"r="95""#));
        assert!(!xml.contains(r#"r="96""#));
        assert!(!xml.contains(r#"r="97""#));
        assert!(!xml.contains(r#"r="98""#));
        assert!(!xml.contains(r#"r="B93""#));
        assert!(!xml.contains(r#"r="A94""#));
        assert!(!xml.contains(r#"r="A95""#));
        assert!(xml.contains(r#"<mergeCell ref="A93:M95"/>"#));
    }

    #[test]
    fn default_removes_unprotected_apply_protection_cells_but_preserves_merge_tail_rows() {
        let mut params = params_with_styles(&[(1029, apply_protection_only_style())]);
        params.cleanup.inactive_protection_cell_policy = InactiveProtectionCellPolicy::Remove;
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
        preprocessed.clean_sheet_metrics.merge_cell_ranges =
            crate::sheet::merge_cells::find_merge_cell_ranges(merge_cells);

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
    fn keeps_empty_styled_cells_inside_merged_range_under_aggressive_textless_cleanup() {
        let mut params = params_with_styles(&[(2170, font_only_style())]);
        params.cleanup.keep_textless_style_representatives = false;

        let merge_cells = br#"<mergeCells count="1"><mergeCell ref="A1:C1"/></mergeCells>"#;
        let mut preprocessed = preprocessed_with_sheet_data(
            br#"<row r="1" spans="1:3" s="499" customFormat="1" ht="11.1" customHeight="1"><c r="A1" s="2170" t="s"><v>48</v></c><c r="B1" s="2170"/><c r="C1" s="2170"/></row>"#,
            None,
        );
        preprocessed.in_merge_cells_node_output_chunks = merge_cells.to_vec();
        preprocessed
            .clean_sheet_metrics
            .max_merge_cells_column_and_row =
            crate::sheet::merge_cells::find_max_merge_cells(merge_cells);
        preprocessed.clean_sheet_metrics.merge_cell_ranges =
            crate::sheet::merge_cells::find_merge_cell_ranges(merge_cells);

        let cleaned = build_clean_sheet_xml(preprocessed, &params, false).unwrap();
        let xml = String::from_utf8_lossy(&cleaned.xml);

        assert!(xml.contains(r#"<c r="B1" s="2170"/>"#), "{xml}");
        assert!(xml.contains(r#"<c r="C1" s="2170"/>"#), "{xml}");
    }

    #[test]
    /// Default-font representatives protect layout before subsequent content;
    /// enabling them must not turn an empty trailing cell into a content anchor.
    fn trailing_self_closing_default_cell_is_pruned_when_representatives_are_enabled() {
        let params = default_params();
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data =
            br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="999"><c r="A999"/></row>"#;

        let cleaned = super::trim_trailing_layout_only_rows(sheet_data, &params, &mut metrics);
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="1""#));
        assert!(!cleaned.contains(r#"r="999""#));
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
        params.cleanup.keep_textless_default_font_representatives = false;
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
        params.cleanup.keep_textless_style_representatives = false;
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
