use crate::error::{Result, ShrinkError};
use crate::models::{CleanSheetMetrics, CleanSheetParameters, CleanupCause, ColumnRangeMap};
use crate::profiling;
use crate::sheet::rows::{
    replace_empty_row_after_cell_range_bytes, replace_row_bytes, replace_row_bytes_sampled,
};
use crate::xml::{find_byte_from, find_bytes};
use rayon::prelude::*;
use std::borrow::Cow;
use std::collections::HashSet;
use tracing::info;

#[derive(Debug, Clone)]
pub struct SheetDataPreprocessState {
    any_column_has_non_default_fill_or_border: bool,
    active_row_has_non_default_fill_or_border_style: bool,

    /// Style IDs whose XF can apply a visible fill or border to empty cells.
    /// Keys are the raw ASCII bytes of the style index, e.g. b"429".
    fill_or_border_style_ids_by_bytes: HashSet<Vec<u8>>,

    /// Cache for row style IDs we have already checked.
    checked_row_style_ids: HashSet<Vec<u8>>,
}

impl SheetDataPreprocessState {
    pub fn new(column_definitions: &ColumnRangeMap, params: &CleanSheetParameters) -> Self {
        let any_column_has_non_default_fill_or_border =
            column_definitions.has_any_non_default_fill_or_border_style();

        let fill_or_border_style_ids_by_bytes = params
            .patterns
            .xfs_by_index
            .iter()
            .filter(|(_, xf)| xf.applies_visible_fill_or_border())
            .map(|(style_id, _)| style_id.to_string().into_bytes())
            .collect();

        Self {
            any_column_has_non_default_fill_or_border,
            active_row_has_non_default_fill_or_border_style: false,
            fill_or_border_style_ids_by_bytes,
            checked_row_style_ids: HashSet::new(),
        }
    }

    fn row_style_id_has_non_default_fill_or_border(&mut self, style_id: &[u8]) -> bool {
        if !self.checked_row_style_ids.contains(style_id) {
            self.checked_row_style_ids.insert(style_id.to_vec());
        }

        self.fill_or_border_style_ids_by_bytes.contains(style_id)
    }

    fn empty_cell_style_id_is_removable(&self, _style_id: &[u8]) -> bool {
        false
    }
}

pub fn pre_process_sheet_data_chunk<'a>(
    chunk: &'a [u8],
    params: &CleanSheetParameters,
    state: &mut SheetDataPreprocessState,
    metrics: &mut CleanSheetMetrics,
) -> Result<Cow<'a, [u8]>> {
    if find_bytes(chunk, b"<![CDATA[").is_some() {
        return Err(ShrinkError::UnsupportedCData);
    }

    if !params.cleanup.preprocess_empty_self_closing_cells {
        return Ok(Cow::Borrowed(chunk));
    }

    if should_skip_empty_cell_cleanup_for_chunk(chunk, state) {
        return Ok(Cow::Borrowed(chunk));
    }

    Ok(remove_consecutive_empty_cells(chunk, state, metrics))
}

pub fn clean_sheet_rows_and_cells_in_cell_range(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Result<Vec<u8>> {
    if tracing::enabled!(target: "excel_shrink::profile", tracing::Level::INFO) {
        return Ok(apply_row_byte_substitution_impl::<true, _>(
            sheet_data,
            |row, sampled| {
                if sampled {
                    replace_row_bytes_sampled(row.raw, row.attr, row.content, params, metrics)
                } else {
                    replace_row_bytes(row.raw, row.attr, row.content, params, metrics)
                }
            },
        ));
    }
    Ok(apply_row_byte_substitution(sheet_data, |row| {
        replace_row_bytes(row.raw, row.attr, row.content, params, metrics)
    }))
}

#[inline]
pub(crate) fn should_parallel_chunk_range(range_len: usize, max_chunk_size: usize) -> bool {
    max_chunk_size > 0 && range_len > max_chunk_size
}

pub fn clean_sheet_rows_and_cells_in_cell_range_parallel_by_max_chunk_size(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    max_chunk_size: usize,
    range_kind: &'static str,
    sheet_name: &str,
) -> Result<Vec<u8>> {
    if !should_parallel_chunk_range(sheet_data.len(), max_chunk_size) {
        return clean_sheet_rows_and_cells_in_cell_range(sheet_data, params, metrics);
    }

    clean_row_aligned_chunks_by_max_size(
        sheet_data,
        params,
        metrics,
        max_chunk_size,
        range_kind,
        sheet_name,
        clean_sheet_rows_and_cells_in_cell_range,
    )
}

pub fn clean_selected_rows_and_cells_in_cell_range(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_numbers: &HashSet<u32>,
) -> Result<Vec<u8>> {
    if row_numbers.is_empty() {
        return Ok(sheet_data.to_vec());
    }

    Ok(apply_row_byte_substitution(sheet_data, |row| {
        clean_row_if_selected(row, params, metrics, row_numbers)
    }))
}

fn clean_row_if_selected<'a>(
    row: ParsedRowBlock<'a>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_numbers: &HashSet<u32>,
) -> Cow<'a, [u8]> {
    let Some(row_number) =
        extract_xml_attr_value(row.attr, b"r").map(crate::utils::digits_byte_string_to_integer)
    else {
        return Cow::Borrowed(row.raw);
    };

    if !row_numbers.contains(&row_number) {
        return Cow::Borrowed(row.raw);
    }

    replace_row_bytes(row.raw, row.attr, row.content, params, metrics)
}

pub fn clean_empty_rows_from_after_cell_range(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Result<Vec<u8>> {
    Ok(apply_row_byte_substitution(sheet_data, |row| {
        clean_empty_row_after_cell_range(row, params, metrics)
    }))
}

fn clean_empty_row_after_cell_range<'a>(
    row: ParsedRowBlock<'a>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    if row.content.is_some_and(|content| !content.is_empty()) {
        return Cow::Borrowed(row.raw);
    }

    replace_empty_row_after_cell_range_bytes(row.raw, row.attr, params, metrics)
}

pub fn clean_empty_rows_from_after_cell_range_parallel_by_max_chunk_size(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    max_chunk_size: usize,
    range_kind: &'static str,
    sheet_name: &str,
) -> Result<Vec<u8>> {
    if !should_parallel_chunk_range(sheet_data.len(), max_chunk_size) {
        return clean_empty_rows_from_after_cell_range(sheet_data, params, metrics);
    }

    clean_row_aligned_chunks_by_max_size(
        sheet_data,
        params,
        metrics,
        max_chunk_size,
        range_kind,
        sheet_name,
        clean_empty_rows_from_after_cell_range,
    )
}

#[derive(Debug, Clone, Copy)]
struct ParsedRowBlock<'a> {
    raw: &'a [u8],
    attr: &'a [u8],
    content: Option<&'a [u8]>,
}

#[derive(Debug, Clone, Copy)]
struct LocatedRowBlock<'a> {
    start: usize,
    end: usize,
    row: ParsedRowBlock<'a>,
}

type CleanChunkFn = fn(&[u8], &CleanSheetParameters, &mut CleanSheetMetrics) -> Result<Vec<u8>>;

#[derive(Debug, Clone, Copy)]
struct RowAlignedChunk {
    index: usize,
    start: usize,
    end: usize,
}

struct CleanedChunk {
    index: usize,
    xml: Vec<u8>,
    metrics: CleanSheetMetrics,
}

#[derive(Debug, Clone, Copy)]
enum ScannedCellTag<'a> {
    NonCellMatch {
        end: usize,
    },
    PreservedTag {
        end: usize,
    },
    EmptySelfClosing {
        start: usize,
        end: usize,
        attr: &'a [u8],
    },
}

#[derive(Debug, Clone, Copy)]
struct RowStyleRisk {
    tag_end: usize,
    has_non_default_fill_or_border: bool,
    continues_beyond_chunk: bool,
}

struct OptionalRewriteBuffer {
    output: Option<Vec<u8>>,
    position: usize,
}

impl OptionalRewriteBuffer {
    fn new() -> Self {
        Self {
            output: None,
            position: 0,
        }
    }

    fn preserve_through(&mut self, input: &[u8], end: usize) {
        if let Some(output) = self.output.as_mut() {
            extend_without_large_growth(output, &input[self.position..end]);
        }
        self.position = end;
    }

    fn remove_range(&mut self, input: &[u8], start: usize, end: usize) {
        if let Some(output) = self.output.as_mut() {
            extend_without_large_growth(output, &input[self.position..start]);
        } else {
            let mut output = Vec::with_capacity(input.len().min(64 * 1024 * 1024));
            output.extend_from_slice(&input[..start]);
            self.output = Some(output);
        }
        self.position = end;
    }

    fn finish<'a>(mut self, input: &'a [u8]) -> Cow<'a, [u8]> {
        if let Some(output) = self.output.as_mut() {
            extend_without_large_growth(output, &input[self.position..]);
        }
        self.output.map_or(Cow::Borrowed(input), Cow::Owned)
    }
}

#[derive(Debug, Clone, Copy)]
struct ChunkSizeSummary {
    largest: usize,
    smallest: usize,
}

impl ChunkSizeSummary {
    fn from_chunks(chunks: &[RowAlignedChunk]) -> Self {
        let mut sizes = chunks.iter().map(|chunk| chunk.end - chunk.start);
        let Some(first) = sizes.next() else {
            return Self {
                largest: 0,
                smallest: 0,
            };
        };

        sizes.fold(
            Self {
                largest: first,
                smallest: first,
            },
            |summary, size| Self {
                largest: summary.largest.max(size),
                smallest: summary.smallest.min(size),
            },
        )
    }
}

fn plan_row_aligned_chunks_by_max_size(
    sheet_data: &[u8],
    max_chunk_size: usize,
) -> Vec<RowAlignedChunk> {
    if !should_parallel_chunk_range(sheet_data.len(), max_chunk_size) {
        return vec![full_range_chunk(sheet_data.len())];
    }

    let row_ends = collect_row_end_positions(sheet_data);
    if row_ends.is_empty() {
        return vec![full_range_chunk(sheet_data.len())];
    }

    let chunks = build_row_aligned_chunks(sheet_data.len(), max_chunk_size, &row_ends);
    if chunks.is_empty() {
        vec![full_range_chunk(sheet_data.len())]
    } else {
        chunks
    }
}

#[inline]
fn full_range_chunk(data_len: usize) -> RowAlignedChunk {
    RowAlignedChunk {
        index: 0,
        start: 0,
        end: data_len,
    }
}

fn build_row_aligned_chunks(
    data_len: usize,
    max_chunk_size: usize,
    row_ends: &[usize],
) -> Vec<RowAlignedChunk> {
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut row_end_index = 0usize;

    while start < data_len {
        let end = next_row_aligned_chunk_end(
            row_ends,
            &mut row_end_index,
            start,
            max_chunk_size,
            data_len,
        );
        if end <= start {
            break;
        }

        chunks.push(RowAlignedChunk {
            index: chunks.len(),
            start,
            end,
        });
        start = end;
    }

    chunks
}

fn next_row_aligned_chunk_end(
    row_ends: &[usize],
    row_end_index: &mut usize,
    start: usize,
    max_chunk_size: usize,
    data_len: usize,
) -> usize {
    skip_consumed_row_ends(row_ends, row_end_index, start);

    let target_end = start.saturating_add(max_chunk_size).min(data_len);
    let first_candidate = *row_end_index;
    let last_fit = take_row_ends_through_target(row_ends, row_end_index, start, target_end);

    last_fit.unwrap_or_else(|| {
        row_ends
            .get(first_candidate)
            .copied()
            .filter(|end| *end > start)
            .unwrap_or(data_len)
    })
}

fn skip_consumed_row_ends(row_ends: &[usize], row_end_index: &mut usize, start: usize) {
    while *row_end_index < row_ends.len() && row_ends[*row_end_index] <= start {
        *row_end_index += 1;
    }
}

fn take_row_ends_through_target(
    row_ends: &[usize],
    row_end_index: &mut usize,
    start: usize,
    target_end: usize,
) -> Option<usize> {
    let mut last_fit = None;
    while *row_end_index < row_ends.len() && row_ends[*row_end_index] <= target_end {
        if row_ends[*row_end_index] > start {
            last_fit = Some(row_ends[*row_end_index]);
        }
        *row_end_index += 1;
    }
    last_fit
}

fn clean_row_aligned_chunks_by_max_size(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    max_chunk_size: usize,
    range_kind: &'static str,
    sheet_name: &str,
    clean_chunk: CleanChunkFn,
) -> Result<Vec<u8>> {
    let planning = profiling::phase(
        "plan_row_chunks",
        &params.xlsx_file_path,
        &params.zip_info_filename,
    );
    let chunks = plan_row_aligned_chunks_by_max_size(sheet_data, max_chunk_size);
    profiling::drop_value("range.planning_span", planning);
    if chunks.len() <= 1 {
        return clean_chunk(sheet_data, params, metrics);
    }

    log_chunking_plan(
        sheet_data.len(),
        max_chunk_size,
        range_kind,
        sheet_name,
        &chunks,
    );
    let cleaned_chunks =
        clean_chunks_in_parallel(sheet_data, params, metrics, &chunks, clean_chunk)?;

    Ok(assemble_cleaned_chunks(
        cleaned_chunks,
        sheet_data.len(),
        metrics,
    ))
}

fn log_chunking_plan(
    range_bytes: usize,
    max_chunk_size: usize,
    range_kind: &'static str,
    sheet_name: &str,
    chunks: &[RowAlignedChunk],
) {
    let sizes = ChunkSizeSummary::from_chunks(chunks);
    info!(
        event = "range_chunked",
        sheet = %sheet_name,
        range_kind,
        range_bytes,
        max_chunk_size,
        actual_chunks = chunks.len(),
        largest_chunk_bytes = sizes.largest,
        smallest_chunk_bytes = sizes.smallest,
        "Parallel range chunking enabled"
    );
}

fn clean_chunks_in_parallel(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    chunks: &[RowAlignedChunk],
    clean_chunk: CleanChunkFn,
) -> Result<Vec<CleanedChunk>> {
    let _profile = profiling::phase(
        "range_chunks_fork_join",
        &params.xlsx_file_path,
        &params.zip_info_filename,
    );
    let cloning = profiling::phase("clone_base_metrics", "", "");
    let base_metrics = make_chunk_metrics(metrics);
    profiling::drop_value("range.clone_base_metrics_span", cloning);
    if !tracing::enabled!(target: "excel_shrink::profile", tracing::Level::INFO) {
        return chunks
            .par_iter()
            .map(|chunk| {
                clean_chunk_in_isolation(sheet_data, params, &base_metrics, *chunk, clean_chunk)
            })
            .collect();
    }
    let jobs: Vec<_> = chunks
        .iter()
        .map(|chunk| {
            (
                *chunk,
                profiling::Handoff::start(
                    "range_chunk",
                    &params.xlsx_file_path,
                    &params.zip_info_filename,
                ),
            )
        })
        .collect();
    jobs.into_par_iter()
        .map(|(chunk, handoff)| {
            handoff.finish();
            clean_chunk_in_isolation(sheet_data, params, &base_metrics, chunk, clean_chunk)
        })
        .collect()
}

fn clean_chunk_in_isolation(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    base_metrics: &CleanSheetMetrics,
    chunk: RowAlignedChunk,
    clean_chunk: CleanChunkFn,
) -> Result<CleanedChunk> {
    let _profile = tracing::info_span!(target: "excel_shrink::profile", "clean_range_chunk",
        xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, chunk = chunk.index,
        input_bytes = chunk.end - chunk.start)
    .entered();
    let cloning = profiling::phase("clone_chunk_metrics", "", "");
    let mut chunk_metrics = make_chunk_metrics(base_metrics);
    profiling::drop_value("range.clone_chunk_metrics_span", cloning);
    let xml = clean_chunk(
        &sheet_data[chunk.start..chunk.end],
        params,
        &mut chunk_metrics,
    )?;

    Ok(CleanedChunk {
        index: chunk.index,
        xml,
        metrics: chunk_metrics,
    })
}

fn assemble_cleaned_chunks(
    mut cleaned_chunks: Vec<CleanedChunk>,
    input_size: usize,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let _profile = profiling::phase("assemble_range_chunks", "", "");
    cleaned_chunks.sort_by_key(|chunk| chunk.index);

    let mut output = Vec::with_capacity(input_size.min(64 * 1024 * 1024));
    for chunk in cleaned_chunks {
        {
            let _copy = profiling::phase("append_chunk_xml", "", "");
            extend_without_large_growth(&mut output, &chunk.xml);
        }
        {
            let _merge = profiling::phase("merge_chunk_metrics", "", "");
            merge_chunk_metrics(metrics, chunk.metrics);
        }
    }
    output
}

fn collect_row_end_positions(sheet_data: &[u8]) -> Vec<usize> {
    let mut row_ends = Vec::new();
    let mut position = 0;

    while let Some(block) = find_next_row_block(sheet_data, position) {
        row_ends.push(block.end);
        position = block.end;
    }

    row_ends
}

fn find_next_row_block(sheet_data: &[u8], position: usize) -> Option<LocatedRowBlock<'_>> {
    let row_start_rel = find_bytes(&sheet_data[position..], b"<row")?;
    let row_start = position + row_start_rel;
    let tag_end = find_byte_from(sheet_data, b'>', row_start)?;
    let self_closing = tag_end > row_start && sheet_data[tag_end - 1] == b'/';
    let attr_start = row_start + b"<row".len();
    let attr_end = if self_closing { tag_end - 1 } else { tag_end };

    let (row_end, content) = if self_closing {
        (tag_end + 1, None)
    } else {
        let content_start = tag_end + 1;
        let close_rel = find_bytes(&sheet_data[content_start..], b"</row>")?;
        let content_end = content_start + close_rel;
        (
            content_end + b"</row>".len(),
            Some(&sheet_data[content_start..content_end]),
        )
    };

    Some(LocatedRowBlock {
        start: row_start,
        end: row_end,
        row: ParsedRowBlock {
            raw: &sheet_data[row_start..row_end],
            attr: &sheet_data[attr_start..attr_end],
            content,
        },
    })
}

fn make_chunk_metrics(base: &CleanSheetMetrics) -> CleanSheetMetrics {
    CleanSheetMetrics {
        column_definitions: base.column_definitions.clone(),
        max_merge_cells_column_and_row: base.max_merge_cells_column_and_row.clone(),
        merge_cell_ranges: base.merge_cell_ranges.clone(),
        sheet_protection_enabled: base.sheet_protection_enabled,
        visible_data_range_bounds: base.visible_data_range_bounds.clone(),
        has_hints: base.has_hints,
        cleanup_stats: if base.cleanup_stats.enabled {
            crate::models::CleanupStats::enabled()
        } else {
            crate::models::CleanupStats::default()
        },
        ..Default::default()
    }
}

fn merge_chunk_metrics(target: &mut CleanSheetMetrics, chunk: CleanSheetMetrics) {
    target.kept_cell_states.extend(chunk.kept_cell_states);
    target.deleted_cell_states.extend(chunk.deleted_cell_states);

    target.whitespace_candidates += chunk.whitespace_candidates;
    target.hidden_candidates += chunk.hidden_candidates;
    target.candidates.extend(chunk.candidates);

    if let Some(col) = chunk.visible_content_bounds.max_col {
        target.visible_content_bounds.include_col(col);
    }
    if let Some(row) = chunk.visible_content_bounds.max_row {
        target.visible_content_bounds.include_row(row);
    }
    target.merge_tail_anchor_rows(chunk.tail_anchor_rows);
    if let Some(row) = chunk.tail_anchor_max_row {
        target.tail_anchor_max_row =
            Some(target.tail_anchor_max_row.map_or(row, |old| old.max(row)));
    }

    target.cleanup_stats.merge(chunk.cleanup_stats);
    target.has_hints |= chunk.has_hints;
}

fn apply_row_byte_substitution<F>(sheet_data: &[u8], mut replace: F) -> Vec<u8>
where
    F: for<'a> FnMut(ParsedRowBlock<'a>) -> Cow<'a, [u8]>,
{
    apply_row_byte_substitution_impl::<false, _>(sheet_data, |row, _| replace(row))
}

fn apply_row_byte_substitution_impl<const PROFILED: bool, F>(
    sheet_data: &[u8],
    mut replace: F,
) -> Vec<u8>
where
    F: for<'a> FnMut(ParsedRowBlock<'a>, bool) -> Cow<'a, [u8]>,
{
    const MEAN_SAMPLE_INTERVAL: usize = 4096;
    let mut output = Vec::with_capacity(sheet_data.len().min(64 * 1024 * 1024));
    let mut position = 0;
    let mut row_index = 0;
    let mut sampled_rows = 0;
    let mut next_sample_row = 0;
    // Jitter avoids repeatedly landing on power-of-two Vec/HashMap growth points.
    // This is deterministic sampling, not a source of cryptographic randomness.
    let mut sample_state = (sheet_data.len() as u64) ^ 0x9e3779b97f4a7c15;

    while position < sheet_data.len() {
        let sampled = PROFILED && row_index == next_sample_row;
        let sample_span = sampled.then(|| {
            tracing::info_span!(target: "excel_shrink::profile", "sampled_row",
            row_index, byte_offset = position, row = tracing::field::Empty, row_bytes = tracing::field::Empty)
            .entered()
        });
        let scanning = sampled.then(|| profiling::phase("row_find_block", "", ""));
        let Some(block) = find_next_row_block(sheet_data, position) else {
            break;
        };
        profiling::drop_optional("row.scanning_span", scanning);
        if let Some(span) = &sample_span {
            span.record("row_bytes", block.row.raw.len());
            if let Some(row) = extract_xml_attr_value(block.row.attr, b"r") {
                span.record("row", String::from_utf8_lossy(row).as_ref());
            }
            sampled_rows += 1;
            sample_state ^= sample_state << 13;
            sample_state ^= sample_state >> 7;
            sample_state ^= sample_state << 17;
            next_sample_row = row_index
                + MEAN_SAMPLE_INTERVAL / 2
                + (sample_state as usize % MEAN_SAMPLE_INTERVAL);
        }
        let copying = sampled.then(|| profiling::phase("row_copy_prefix", "", ""));
        extend_without_large_growth(&mut output, &sheet_data[position..block.start]);
        profiling::drop_optional("row.copying_span", copying);
        let replacement = replace(block.row, sampled);
        let _appending = sampled.then(|| profiling::phase("row_append_output", "", ""));
        extend_without_large_growth(&mut output, replacement.as_ref());
        position = block.end;
        if PROFILED {
            row_index += 1;
        }
    }

    extend_without_large_growth(&mut output, &sheet_data[position..]);
    if PROFILED {
        tracing::info!(target: "excel_shrink::profile", event = "range_row_samples",
            total_rows = row_index, sampled_rows, mean_sample_interval_rows = MEAN_SAMPLE_INTERVAL,
            input_bytes = sheet_data.len(), output_bytes = output.len());
    }
    output
}

#[inline]
fn extend_without_large_growth(out: &mut Vec<u8>, bytes: &[u8]) {
    let spare = out.capacity().saturating_sub(out.len());
    if spare < bytes.len() {
        out.reserve_exact((bytes.len() - spare).max(16 * 1024 * 1024));
    }
    out.extend_from_slice(bytes);
}

fn should_skip_empty_cell_cleanup_for_chunk(
    chunk: &[u8],
    state: &mut SheetDataPreprocessState,
) -> bool {
    let chunk_has_risky_active_or_declared_row_style =
        scan_row_styles_for_empty_cell_cleanup_risk(chunk, state);

    state.any_column_has_non_default_fill_or_border || chunk_has_risky_active_or_declared_row_style
}

fn scan_row_styles_for_empty_cell_cleanup_risk(
    chunk: &[u8],
    state: &mut SheetDataPreprocessState,
) -> bool {
    let mut chunk_is_risky = state.active_row_has_non_default_fill_or_border_style;
    let mut position = 0;

    while let Some(row_start_rel) = find_bytes(&chunk[position..], b"<row") {
        let row_start = position + row_start_rel;
        let risk = inspect_row_style_risk(chunk, row_start, state);

        chunk_is_risky |= risk.has_non_default_fill_or_border;
        state.active_row_has_non_default_fill_or_border_style =
            risk.continues_beyond_chunk && risk.has_non_default_fill_or_border;

        if risk.tag_end <= row_start {
            break;
        }
        position = risk.tag_end;
    }

    chunk_is_risky
}

fn inspect_row_style_risk(
    chunk: &[u8],
    row_start: usize,
    state: &mut SheetDataPreprocessState,
) -> RowStyleRisk {
    let tag_end = find_byte_from(chunk, b'>', row_start).map_or(chunk.len(), |end| end + 1);
    let row_tag = &chunk[row_start..tag_end];
    let has_non_default_fill_or_border = extract_xml_attr_value(row_tag, b"s")
        .is_some_and(|style_id| state.row_style_id_has_non_default_fill_or_border(style_id));
    let is_self_closing = tag_end >= 2 && chunk[tag_end - 2] == b'/';
    let continues_beyond_chunk =
        !is_self_closing && find_bytes(&chunk[tag_end..], b"</row>").is_none();

    RowStyleRisk {
        tag_end,
        has_non_default_fill_or_border,
        continues_beyond_chunk,
    }
}

fn remove_consecutive_empty_cells<'a>(
    chunk: &'a [u8],
    state: &SheetDataPreprocessState,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    if find_bytes(chunk, b"<c").is_none() {
        return Cow::Borrowed(chunk);
    }

    let mut rewrite = OptionalRewriteBuffer::new();
    while let Some(cell_start_rel) = find_bytes(&chunk[rewrite.position..], b"<c") {
        let cell_start = rewrite.position + cell_start_rel;
        let Some(tag) = scan_cell_tag(chunk, cell_start) else {
            break;
        };

        match tag {
            ScannedCellTag::NonCellMatch { end } | ScannedCellTag::PreservedTag { end } => {
                rewrite.preserve_through(chunk, end);
            }
            ScannedCellTag::EmptySelfClosing { start, end, attr } => {
                process_empty_self_closing_cell(
                    chunk,
                    start,
                    end,
                    attr,
                    state,
                    metrics,
                    &mut rewrite,
                );
            }
        }
    }

    rewrite.finish(chunk)
}

fn scan_cell_tag(chunk: &[u8], cell_start: usize) -> Option<ScannedCellTag<'_>> {
    if !is_cell_tag_start(chunk, cell_start) {
        return Some(ScannedCellTag::NonCellMatch {
            end: cell_start + b"<c".len(),
        });
    }

    let tag_end = find_byte_from(chunk, b'>', cell_start)?;
    if tag_end == cell_start || chunk[tag_end - 1] != b'/' {
        return Some(ScannedCellTag::PreservedTag { end: tag_end + 1 });
    }

    Some(ScannedCellTag::EmptySelfClosing {
        start: cell_start,
        end: tag_end + 1,
        attr: &chunk[cell_start + b"<c".len()..tag_end - 1],
    })
}

fn process_empty_self_closing_cell(
    chunk: &[u8],
    start: usize,
    end: usize,
    attr: &[u8],
    state: &SheetDataPreprocessState,
    metrics: &mut CleanSheetMetrics,
    rewrite: &mut OptionalRewriteBuffer,
) {
    if !empty_self_closing_cell_is_removable(attr, state) {
        rewrite.preserve_through(chunk, end);
        return;
    }

    rewrite.remove_range(chunk, start, end);
    metrics
        .cleanup_stats
        .record_removed(CleanupCause::PreprocessEmptySelfClosingCells, end - start);
}

#[inline]
fn empty_self_closing_cell_is_removable(attr: &[u8], state: &SheetDataPreprocessState) -> bool {
    extract_xml_attr_value(attr, b"s")
        .is_none_or(|style_id| state.empty_cell_style_id_is_removable(style_id))
}

#[inline(always)]
fn is_cell_tag_start(data: &[u8], cell_start: usize) -> bool {
    matches!(
        data.get(cell_start + b"<c".len()),
        Some(byte) if byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>')
    )
}

fn extract_xml_attr_value<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let mut search_start = 0;

    while search_start < data.len() {
        let relative = find_bytes(&data[search_start..], attr_name)?;
        let attribute_start = search_start + relative;
        let value_marker = attribute_start + attr_name.len();
        let has_attribute_boundary =
            attribute_start == 0 || data[attribute_start - 1].is_ascii_whitespace();

        if has_attribute_boundary && data.get(value_marker..value_marker + 2) == Some(b"=\"") {
            let value_start = value_marker + 2;
            let value_length = data[value_start..].iter().position(|byte| *byte == b'"')?;
            return Some(&data[value_start..value_start + value_length]);
        }

        search_start = attribute_start + 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        clean_empty_rows_from_after_cell_range,
        clean_empty_rows_from_after_cell_range_parallel_by_max_chunk_size,
        clean_sheet_rows_and_cells_in_cell_range,
        clean_sheet_rows_and_cells_in_cell_range_parallel_by_max_chunk_size,
        plan_row_aligned_chunks_by_max_size, pre_process_sheet_data_chunk,
        remove_consecutive_empty_cells, SheetDataPreprocessState,
    };
    use crate::models::{
        CleanSheetMetrics, CleanSheetParameters, CleanupStats, CleanupSwitches,
        SheetPreprocessConfig, WorkbookSpecificPatterns, Xf,
    };
    use std::collections::HashMap;
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
            cleanup: CleanupSwitches::default(),
            collect_cleanup_stats: false,
        }
    }

    #[test]
    fn preprocess_empty_cell_cleanup_preserves_hidden_empty_rows() {
        let params = default_params();
        let mut state = SheetDataPreprocessState::new(&Default::default(), &params);
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data = br#"<row r="307905" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307906" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307907" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307908" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row>"#;

        let cleaned =
            pre_process_sheet_data_chunk(sheet_data, &params, &mut state, &mut metrics).unwrap();
        let cleaned = String::from_utf8_lossy(cleaned.as_ref());

        assert_eq!(cleaned.as_bytes(), sheet_data);
    }

    #[test]
    fn range_at_or_below_max_chunk_size_is_not_chunked() {
        let data = br#"<row r="1"><c r="A1"/></row>"#;
        let chunks = plan_row_aligned_chunks_by_max_size(data, data.len());

        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn range_above_max_chunk_size_is_split_on_row_boundaries() {
        let data = br#"<row r="1"><c r="A1"/></row><row r="2"><c r="A2"/></row><row r="3"><c r="A3"/></row>"#;
        let chunks = plan_row_aligned_chunks_by_max_size(data, 40);

        assert!(chunks.len() > 1);

        let rebuilt: Vec<u8> = chunks
            .iter()
            .flat_map(|chunk| data[chunk.start..chunk.end].to_vec())
            .collect();

        assert_eq!(rebuilt, data);
    }

    #[test]
    fn parallel_cell_range_cleaning_matches_sequential_output_and_metrics() {
        let params = default_params();
        let sheet_data = br#"
        <row r="1"><c r="A1"/><c r="B1"><v>42</v></c></row>
        <row r="2"><c r="A2"/><c r="B2"><v>43</v></c></row>
        <row r="3"><c r="A3"/><c r="B3"><v>44</v></c></row>
        <row r="4"><c r="A4"/><c r="B4"><v>45</v></c></row>
    "#;

        let mut sequential_metrics = CleanSheetMetrics::default();
        let sequential =
            clean_sheet_rows_and_cells_in_cell_range(sheet_data, &params, &mut sequential_metrics)
                .unwrap();

        let mut parallel_metrics = CleanSheetMetrics::default();
        let parallel = clean_sheet_rows_and_cells_in_cell_range_parallel_by_max_chunk_size(
            sheet_data,
            &params,
            &mut parallel_metrics,
            80,
            "cells_range",
            &params.zip_info_filename,
        )
        .unwrap();

        assert_eq!(parallel, sequential);
        assert_eq!(
            parallel_metrics.kept_cell_states.len(),
            sequential_metrics.kept_cell_states.len()
        );
        assert_eq!(
            parallel_metrics.deleted_cell_states.len(),
            sequential_metrics.deleted_cell_states.len()
        );
        assert_eq!(
            parallel_metrics.candidates.len(),
            sequential_metrics.candidates.len()
        );
    }

    #[test]
    fn parallel_cell_range_cleaning_preserves_hidden_empty_rows() {
        let params = default_params();
        let sheet_data = br#"<row r="307905" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307906" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307907" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307908" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row>"#;

        let mut metrics = CleanSheetMetrics::default();
        let cleaned = clean_sheet_rows_and_cells_in_cell_range_parallel_by_max_chunk_size(
            sheet_data,
            &params,
            &mut metrics,
            80,
            "cells_range",
            &params.zip_info_filename,
        )
        .unwrap();
        let cleaned = String::from_utf8_lossy(&cleaned);

        assert!(cleaned.contains(r#"r="307905""#));
        assert!(cleaned.contains(r#"r="307906""#));
        assert!(cleaned.contains(r#"r="307907""#));
        assert!(cleaned.contains(r#"r="307908""#));
    }

    #[test]
    fn parallel_rows_range_cleaning_matches_sequential_output() {
        let params = default_params();
        let sheet_data = br#"
        <row r="5"></row>
        <row r="6" ht="15" customHeight="1"></row>
        <row r="7"></row>
        <row r="8" hidden="1"></row>
    "#;

        let mut sequential_metrics = CleanSheetMetrics::default();
        let sequential =
            clean_empty_rows_from_after_cell_range(sheet_data, &params, &mut sequential_metrics)
                .unwrap();

        let mut parallel_metrics = CleanSheetMetrics::default();
        let parallel = clean_empty_rows_from_after_cell_range_parallel_by_max_chunk_size(
            sheet_data,
            &params,
            &mut parallel_metrics,
            70,
            "rows_range",
            &params.zip_info_filename,
        )
        .unwrap();

        assert_eq!(parallel, sequential);
    }

    #[test]
    fn preprocess_empty_cell_scanner_removes_unstyled_but_keeps_font_only_self_closing_cells() {
        let mut params = default_params();
        params.patterns.xfs_by_index.insert(
            7,
            Xf::builder()
                .has_no_empty_cell_visible_style(true)
                .build_xf(),
        );
        let state = SheetDataPreprocessState::new(&Default::default(), &params);
        let mut metrics = CleanSheetMetrics {
            cleanup_stats: CleanupStats::enabled(),
            ..CleanSheetMetrics::default()
        };
        let chunk = br#"<row r="1"><c r="A1"/><c r="B1" s="7"/><c r="C1"><v>1</v></c><custom r="D1"/></row>"#;

        let cleaned = remove_consecutive_empty_cells(chunk, &state, &mut metrics);
        let cleaned = String::from_utf8_lossy(cleaned.as_ref());

        assert!(!cleaned.contains(r#"r="A1""#));
        assert!(cleaned.contains(r#"r="B1""#));
        assert!(cleaned.contains(r#"r="C1""#));
        assert!(cleaned.contains(r#"<custom r="D1"/>"#));
        assert_eq!(
            metrics
                .cleanup_stats
                .removed_items_by_cause
                .get(&crate::models::CleanupCause::PreprocessEmptySelfClosingCells),
            Some(&1)
        );
    }

    #[test]
    fn preprocess_empty_cell_scanner_keeps_non_removable_styled_self_closing_cell() {
        let params = default_params();
        let state = SheetDataPreprocessState::new(&Default::default(), &params);
        let mut metrics = CleanSheetMetrics::default();
        let chunk = br#"<row r="1"><c r="A1" s="9"/></row>"#;

        let cleaned = remove_consecutive_empty_cells(chunk, &state, &mut metrics);

        assert_eq!(cleaned.as_ref(), chunk);
    }
}
