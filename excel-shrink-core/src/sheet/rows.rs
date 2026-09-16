use crate::models::{CleanSheetMetrics, CleanSheetParameters, CleanupCause};
use crate::profiling;
use crate::sheet::cells::{
    parsed_empty_cell_has_effective_fill_or_border, replace_parsed_cell_with_empty_style_policy,
    ParsedCell,
};
use crate::sheet::row_view::RowView;
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use crate::xml::attributes::Attributes;
use crate::xml::{find_byte_from, find_bytes, find_bytes_from, rfind_bytes};
use regex::bytes::Captures;
use std::borrow::Cow;
use std::time::Instant;

// Only sampled rows read the clock. Accumulate locally instead of emitting events
// inside the cell loop. The normal const-false specialization removes this work.
#[derive(Default)]
struct CellTimings<const SAMPLED: bool> {
    ns: [u64; 7],
    visited: u64,
    changed: u64,
}

#[derive(Clone, Copy)]
enum CellStage {
    Find,
    ParseBlock,
    ParseAttributesAndContent,
    SelectRepresentative,
    ApplyPolicy,
    RecordRepresentative,
    AppendOutput,
}

impl<const SAMPLED: bool> CellTimings<SAMPLED> {
    #[inline]
    fn measure<T>(&mut self, stage: CellStage, work: impl FnOnce() -> T) -> T {
        if SAMPLED {
            let started = Instant::now();
            let result = work();
            self.ns[stage as usize] += started.elapsed().as_nanos() as u64;
            result
        } else {
            work()
        }
    }

    fn report(&self) {
        if SAMPLED {
            tracing::info!(target: "excel_shrink::profile", event = "sampled_cell_work",
                cells = self.visited, changed_cells = self.changed,
                find_cell_ns = self.ns[0], parse_cell_block_ns = self.ns[1],
                parse_attributes_and_content_ns = self.ns[2], select_style_representative_ns = self.ns[3],
                apply_cell_policy_ns = self.ns[4], record_representative_ns = self.ns[5],
                append_cell_output_ns = self.ns[6]);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmptyStyleRepresentativePolicy {
    keep_default_styleless: bool,
    keep_font_styles: bool,
    require_content_in_row: bool,
}

impl EmptyStyleRepresentativePolicy {
    #[inline]
    fn for_row(params: &CleanSheetParameters, row: &RowView<'_, '_>) -> Self {
        Self {
            keep_default_styleless: params.cleanup.keep_textless_default_font_representatives,
            keep_font_styles: params.cleanup.keep_textless_style_representatives,
            require_content_in_row: row.has_custom_height,
        }
    }
}

struct CleanCellsResult<'a> {
    row_content: Cow<'a, [u8]>,
    tentative_style_representatives: Vec<TentativeStyleRepresentative<'a>>,
    cells_removed: bool,
    has_kept_content: bool,
    max_kept_col: Option<u32>,
    kept_cells_start: usize,
}

#[derive(Debug, Clone)]
struct TentativeStyleRepresentative<'a> {
    reference_attr: &'a [u8],
    col: u32,
    row: u32,
}

struct RepresentativeRemovalResult<'a> {
    row_content: Vec<u8>,
    removed: Vec<RemovedStyleRepresentative<'a>>,
}

struct RemovedStyleRepresentative<'a> {
    representative: TentativeStyleRepresentative<'a>,
    bytes_removed: usize,
}

#[derive(Debug, Clone, Copy)]
struct ParsedCellBlock<'a> {
    has_content: bool,
    start: usize,
    end: usize,
    raw: &'a [u8],
    attr: &'a [u8],
}

#[derive(Debug, Clone, Copy)]
struct EmptyStyleRepresentativeDecision {
    keep_font_only: bool,
    keep_merged: bool,
    keep_default: bool,
    has_effective_fill_or_border: bool,
}

impl EmptyStyleRepresentativeDecision {
    #[inline]
    fn is_tentative(self, policy: EmptyStyleRepresentativePolicy) -> bool {
        policy.require_content_in_row
            && !self.has_effective_fill_or_border
            && (self.keep_font_only || self.keep_default)
    }
}

struct RowCellCleanupState<'a> {
    output: Option<Vec<u8>>,
    position: usize,
    cells_removed: bool,
    has_kept_content: bool,
    max_kept_col: Option<u32>,
    kept_cells_start: usize,
    kept_default_style_cell: bool,
    tentative_style_representatives: Vec<TentativeStyleRepresentative<'a>>,
    kept_represented_style_ids: std::collections::HashSet<u32>,
}

impl<'a> RowCellCleanupState<'a> {
    fn new(kept_cells_start: usize) -> Self {
        Self {
            output: None,
            position: 0,
            cells_removed: false,
            has_kept_content: false,
            max_kept_col: None,
            kept_cells_start,
            kept_default_style_cell: false,
            tentative_style_representatives: Vec::new(),
            kept_represented_style_ids: std::collections::HashSet::new(),
        }
    }

    fn advance_past_non_cell_match(&mut self, row_content: &[u8], next_position: usize) {
        if let Some(output) = self.output.as_mut() {
            output.extend_from_slice(&row_content[self.position..next_position]);
        }
        self.position = next_position;
    }

    fn append_cleaned_cell(
        &mut self,
        row_content: &[u8],
        block: ParsedCellBlock<'_>,
        cleaned_cell: Cow<'_, [u8]>,
    ) {
        self.cells_removed |= cleaned_cell.is_empty();

        if let Some(output) = self.output.as_mut() {
            output.extend_from_slice(&row_content[self.position..block.start]);
            output.extend_from_slice(cleaned_cell.as_ref());
        } else if matches!(&cleaned_cell, Cow::Owned(_)) {
            let mut output = Vec::with_capacity(row_content.len().min(64 * 1024));
            output.extend_from_slice(&row_content[..block.start]);
            output.extend_from_slice(cleaned_cell.as_ref());
            self.output = Some(output);
        }

        self.position = block.end;
    }

    fn record_kept_cell(
        &mut self,
        cell: &ParsedCell<'_, '_>,
        row: &RowView<'_, '_>,
        has_content: bool,
    ) {
        let same_reference = cell.row == row.reference;
        self.has_kept_content |= same_reference && has_content;
        // XML keys retain byte identity for inheritance; spans are numeric, so
        // equivalent references such as row="03" and cell="A3" still agree.
        if same_reference || row.number == Some(digits_byte_string_to_integer(cell.row)) {
            self.max_kept_col = Some(
                self.max_kept_col
                    .map_or(cell.col_num, |old| old.max(cell.col_num)),
            );
        }
    }

    fn record_kept_empty_representative(
        &mut self,
        cell: &ParsedCell<'a, '_>,
        decision: EmptyStyleRepresentativeDecision,
        policy: EmptyStyleRepresentativePolicy,
    ) {
        record_represented_cell_style(
            cell,
            &mut self.kept_represented_style_ids,
            &mut self.kept_default_style_cell,
        );

        if decision.is_tentative(policy) {
            self.tentative_style_representatives
                .push(TentativeStyleRepresentative {
                    reference_attr: cell.reference_attr,
                    col: cell.col_num,
                    row: digits_byte_string_to_integer(cell.row),
                });
        }
    }

    fn finish(self, row_content: &'a [u8]) -> CleanCellsResult<'a> {
        let row_content = if let Some(mut output) = self.output {
            output.extend_from_slice(&row_content[self.position..]);
            Cow::Owned(output)
        } else {
            Cow::Borrowed(row_content)
        };

        CleanCellsResult {
            row_content,
            tentative_style_representatives: self.tentative_style_representatives,
            cells_removed: self.cells_removed,
            has_kept_content: self.has_kept_content,
            max_kept_col: self.max_kept_col,
            kept_cells_start: self.kept_cells_start,
        }
    }
}

pub fn replace_row(
    caps: &Captures,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let Some(row_match) = caps.get(0).map(|m| m.as_bytes()) else {
        return Vec::new();
    };
    let Some(row_attr) = caps.get(1).map(|m| m.as_bytes()) else {
        return caps.get(0).unwrap().as_bytes().to_vec();
    };
    let row_content = caps.get(6).map(|m| m.as_bytes());

    replace_row_bytes(row_match, row_attr, row_content, params, metrics).into_owned()
}

pub fn replace_row_bytes<'a>(
    row_raw: &'a [u8],
    row_attr: &'a [u8],
    row_content: Option<&'a [u8]>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    replace_row_bytes_impl::<false>(row_raw, row_attr, row_content, params, metrics)
}

pub(super) fn replace_row_bytes_sampled<'a>(
    row_raw: &'a [u8],
    row_attr: &'a [u8],
    row_content: Option<&'a [u8]>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    replace_row_bytes_impl::<true>(row_raw, row_attr, row_content, params, metrics)
}

fn replace_row_bytes_impl<'a, const SAMPLED: bool>(
    row_raw: &'a [u8],
    row_attr: &'a [u8],
    row_content: Option<&'a [u8]>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    let inspecting = profiling::sampled_phase::<SAMPLED>("row_inspect_header");
    let Some(row) = RowView::parse(row_attr, params) else {
        return Cow::Borrowed(row_raw);
    };
    profiling::drop_optional("row.header_span", inspecting);
    {
        let _summary = profiling::sampled_phase::<SAMPLED>("row_record_summary");
        record_visible_row_bounds(&row, metrics);
    }
    let style_policy = EmptyStyleRepresentativePolicy::for_row(params, &row);
    let clean_result = clean_row_content::<SAMPLED>(
        params,
        metrics,
        row_content.unwrap_or_default(),
        style_policy,
        &row,
    );
    let _render = profiling::sampled_phase::<SAMPLED>("row_render_xml");
    render_cleaned_row::<SAMPLED>(row_raw, clean_result, params, metrics, &row)
}

fn clean_row_content<'a, const SAMPLED: bool>(
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    original_row_content: &'a [u8],
    style_policy: EmptyStyleRepresentativePolicy,
    row_attributes: &RowView<'_, '_>,
) -> CleanCellsResult<'a> {
    if original_row_content.is_empty() {
        return unchanged_row_content(original_row_content);
    }

    let cell_phase = profiling::sampled_phase::<SAMPLED>("row_scan_and_clean_cells");
    let clean_result = clean_cells::<SAMPLED>(
        params,
        metrics,
        original_row_content,
        style_policy,
        row_attributes,
    );
    profiling::drop_optional("row.cell_phase_span", cell_phase);
    let _representatives = profiling::sampled_phase::<SAMPLED>("row_prune_style_representatives");
    remove_unneeded_style_representatives(clean_result, style_policy, row_attributes, metrics)
}

fn unchanged_row_content(row_content: &[u8]) -> CleanCellsResult<'_> {
    CleanCellsResult {
        row_content: Cow::Borrowed(row_content),
        tentative_style_representatives: Vec::new(),
        cells_removed: false,
        has_kept_content: false,
        max_kept_col: None,
        kept_cells_start: 0,
    }
}

fn remove_unneeded_style_representatives<'a>(
    clean_result: CleanCellsResult<'a>,
    style_policy: EmptyStyleRepresentativePolicy,
    row_attributes: &RowView<'_, '_>,
    metrics: &mut CleanSheetMetrics,
) -> CleanCellsResult<'a> {
    if !style_policy.require_content_in_row
        || clean_result.has_kept_content
        || clean_result.tentative_style_representatives.is_empty()
    {
        return clean_result;
    }

    let removal = remove_tentative_style_representatives(
        clean_result.row_content.into_owned(),
        clean_result.tentative_style_representatives,
    );
    record_style_representative_removal(metrics, &removal.removed, clean_result.kept_cells_start);

    CleanCellsResult {
        row_content: Cow::Owned(removal.row_content),
        tentative_style_representatives: Vec::new(),
        cells_removed: clean_result.cells_removed || !removal.removed.is_empty(),
        has_kept_content: clean_result.has_kept_content,
        // Only the uncommon pruning path needs a rescan, limited to this row.
        max_kept_col: metrics.kept_cell_states[clean_result.kept_cells_start..]
            .iter()
            .filter(|cell| Some(cell.row) == row_attributes.number)
            .map(|cell| cell.col)
            .max(),
        kept_cells_start: clean_result.kept_cells_start,
    }
}

fn record_style_representative_removal(
    metrics: &mut CleanSheetMetrics,
    removed: &[RemovedStyleRepresentative<'_>],
    kept_cells_start: usize,
) {
    for removed_cell in removed {
        metrics.cleanup_stats.record_removed(
            CleanupCause::EmptyCellsWithoutContent,
            removed_cell.bytes_removed,
        );
    }
    remove_tentative_representatives_from_metrics(metrics, removed, kept_cells_start);
}

fn render_cleaned_row<'a, const SAMPLED: bool>(
    row_raw: &'a [u8],
    clean_result: CleanCellsResult<'a>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row: &RowView<'a, '_>,
) -> Cow<'a, [u8]> {
    if clean_result.row_content.is_empty() {
        return render_empty_row::<SAMPLED>(row_raw, row.raw_attributes, params, metrics);
    }
    if matches!(&clean_result.row_content, Cow::Borrowed(_)) {
        return Cow::Borrowed(row_raw);
    }
    let spans = profiling::sampled_phase::<SAMPLED>("row_rewrite_spans");
    let attributes = if clean_result.cells_removed {
        rewrite_row_spans_upper_bound(row, clean_result.max_kept_col)
    } else {
        Cow::Borrowed(row.raw_attributes)
    };
    profiling::drop_optional("row.rewrite_spans_span", spans);
    let _build = profiling::sampled_phase::<SAMPLED>("row_build_xml");
    Cow::Owned(generate_row_xml(&attributes, &clean_result.row_content))
}

pub fn replace_empty_row_after_cell_range(
    caps: &Captures,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let Some(row_match) = caps.get(0).map(|m| m.as_bytes()) else {
        return Vec::new();
    };
    let Some(row_attr) = caps.get(1).map(|m| m.as_bytes()) else {
        return caps.get(0).unwrap().as_bytes().to_vec();
    };

    replace_empty_row_after_cell_range_bytes(row_match, row_attr, params, metrics).into_owned()
}

pub fn replace_empty_row_after_cell_range_bytes<'a>(
    row_raw: &'a [u8],
    row_attr: &'a [u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    let Some(row) = RowView::parse(row_attr, params) else {
        return Cow::Borrowed(row_raw);
    };
    record_visible_row_bounds(&row, metrics);
    render_empty_row::<false>(row_raw, row_attr, params, metrics)
}

fn clean_cells<'a, const SAMPLED: bool>(
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_content: &'a [u8],
    style_policy: EmptyStyleRepresentativePolicy,
    row: &RowView<'_, '_>,
) -> CleanCellsResult<'a> {
    if row_content.is_empty() {
        return unchanged_row_content(row_content);
    }

    let mut state = RowCellCleanupState::new(metrics.kept_cell_states.len());
    let mut timings = CellTimings::<SAMPLED>::default();
    while let Some(cell_start_rel) = timings.measure(CellStage::Find, || {
        find_bytes(&row_content[state.position..], b"<c")
    }) {
        let cell_start = state.position + cell_start_rel;

        if !is_cell_tag_start(row_content, cell_start) {
            state.advance_past_non_cell_match(row_content, cell_start + b"<c".len());
            continue;
        }

        let Some(block) = timings.measure(CellStage::ParseBlock, || {
            parse_cell_block(row_content, cell_start)
        }) else {
            break;
        };
        let cleaned_cell = clean_cell_block(
            block,
            params,
            metrics,
            style_policy,
            &mut state,
            &mut timings,
            row,
        );
        if SAMPLED {
            timings.visited += 1;
            timings.changed += u64::from(cleaned_cell.as_ref() != block.raw);
        }
        timings.measure(CellStage::AppendOutput, || {
            state.append_cleaned_cell(row_content, block, cleaned_cell)
        });
    }

    let result = timings.measure(CellStage::AppendOutput, || state.finish(row_content));
    timings.report();
    result
}

fn parse_cell_block(row_content: &[u8], cell_start: usize) -> Option<ParsedCellBlock<'_>> {
    let tag_end = find_byte_from(row_content, b'>', cell_start)?;
    let self_closing = tag_end > cell_start && row_content[tag_end - 1] == b'/';
    let attr_start = cell_start + b"<c".len();
    let attr_end = if self_closing { tag_end - 1 } else { tag_end };

    let (cell_end, mut has_content) = if self_closing {
        (tag_end + 1, false)
    } else {
        scan_cell_content(row_content, tag_end + 1)?
    };
    // Preserve the previous substring behavior even for malformed headers
    // containing literal '<' bytes. Valid XML attribute values escape these.
    if memchr::memchr(b'<', &row_content[attr_start..tag_end]).is_some() {
        has_content = crate::sheet::cells::has_content(&row_content[cell_start..cell_end]);
    }

    Some(ParsedCellBlock {
        has_content,
        start: cell_start,
        end: cell_end,
        raw: &row_content[cell_start..cell_end],
        attr: &row_content[attr_start..attr_end],
    })
}

/// Inspect markup until the closing cell or the first content marker. After a
/// marker is known, only the closing-cell search remains necessary.
fn scan_cell_content(bytes: &[u8], mut position: usize) -> Option<(usize, bool)> {
    while let Some(relative) = memchr::memchr(b'<', &bytes[position..]) {
        position += relative;
        let tail = &bytes[position..];
        if tail.starts_with(b"</c>") {
            return Some((position + 4, false));
        }
        if tail.starts_with(b"<v>")
            || tail.starts_with(b"<is>")
            || (tail.starts_with(b"<f") && matches!(tail.get(2), Some(b' ' | b'/' | b'>')))
        {
            let end = find_bytes(tail, b"</c>")?;
            return Some((position + end + 4, true));
        }
        position += 1;
    }
    None
}

fn clean_cell_block<'a, const SAMPLED: bool>(
    block: ParsedCellBlock<'a>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    style_policy: EmptyStyleRepresentativePolicy,
    state: &mut RowCellCleanupState<'a>,
    timings: &mut CellTimings<SAMPLED>,
    row: &RowView<'_, '_>,
) -> Cow<'a, [u8]> {
    let Some((cell, has_cell_content)) =
        timings.measure(CellStage::ParseAttributesAndContent, || {
            let mut cell = parse_cell_from_parts(block.raw, block.attr)?;
            cell.row_view = Some(row);
            let has_content = block.has_content;
            Some((cell, has_content))
        })
    else {
        return Cow::Borrowed(block.raw);
    };

    record_cell_visit(params, &cell);
    let representative = timings.measure(CellStage::SelectRepresentative, || {
        select_empty_style_representative(
            &cell,
            params,
            metrics,
            has_cell_content,
            style_policy,
            state,
        )
    });
    let cleaned_cell = timings.measure(CellStage::ApplyPolicy, || {
        replace_parsed_cell_with_empty_style_policy(
            &cell,
            params,
            metrics,
            has_cell_content,
            representative.keep_font_only,
            representative.keep_merged,
            representative.keep_default,
        )
    });

    if !cleaned_cell.is_empty() {
        state.record_kept_cell(&cell, row, has_cell_content);
    }
    if !cleaned_cell.is_empty() && !has_cell_content {
        timings.measure(CellStage::RecordRepresentative, || {
            state.record_kept_empty_representative(&cell, representative, style_policy)
        });
    }

    cleaned_cell
}

fn select_empty_style_representative(
    cell: &ParsedCell<'_, '_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    has_cell_content: bool,
    policy: EmptyStyleRepresentativePolicy,
    state: &RowCellCleanupState<'_>,
) -> EmptyStyleRepresentativeDecision {
    let keep_font_only = policy.keep_font_styles
        && empty_cell_has_no_empty_cell_visible_style(cell, params, has_cell_content)
        && cell
            .style_id
            .is_some_and(|style_id| !state.kept_represented_style_ids.contains(&style_id));
    let keep_merged =
        empty_styled_cell_is_inside_merge_range(cell, params, metrics, has_cell_content);
    let keep_default = policy.keep_default_styleless
        && !state.kept_default_style_cell
        && empty_cell_has_default_style(cell, has_cell_content);
    let has_effective_fill_or_border =
        parsed_empty_cell_has_effective_fill_or_border(cell, params, metrics, has_cell_content);

    EmptyStyleRepresentativeDecision {
        keep_font_only,
        keep_merged,
        keep_default,
        has_effective_fill_or_border,
    }
}

#[cfg(feature = "cell-tracing")]
fn record_cell_visit(params: &CleanSheetParameters, cell: &ParsedCell<'_, '_>) {
    crate::cleaning_trace::record_cell_visit_if_target(crate::cleaning_trace::CellVisitEvent {
        sheet_file: &params.zip_info_filename,
        col: cell.col_num,
        row: cell.row,
        style_id: cell.style_id,
    });
}

#[cfg(not(feature = "cell-tracing"))]
#[inline]
fn record_cell_visit(_params: &CleanSheetParameters, _cell: &ParsedCell<'_, '_>) {}

fn record_represented_cell_style(
    cell: &ParsedCell<'_, '_>,
    kept_represented_style_ids: &mut std::collections::HashSet<u32>,
    kept_default_style_cell: &mut bool,
) {
    if let Some(style_id) = cell.style_id {
        kept_represented_style_ids.insert(style_id);
    } else {
        *kept_default_style_cell = true;
    }
}

fn empty_styled_cell_is_inside_merge_range(
    cell: &ParsedCell<'_, '_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    has_cell_content: bool,
) -> bool {
    if has_cell_content || cell.style_id.is_none() {
        return false;
    }

    let row = digits_byte_string_to_integer(cell.row);
    if !metrics
        .merge_cell_ranges
        .iter()
        .any(|range| range.contains(cell.col_num, row))
    {
        return false;
    }

    cell.style_id
        .and_then(|style_id| params.patterns.xfs_by_index.get(&style_id))
        .map_or(true, |style| style.has_no_empty_cell_visible_style)
}

fn remove_tentative_style_representatives<'a>(
    mut row_content: Vec<u8>,
    representatives: Vec<TentativeStyleRepresentative<'a>>,
) -> RepresentativeRemovalResult<'a> {
    let mut removed = Vec::new();
    let mut search_start = 0usize;

    for representative in representatives {
        let Some(reference_start) =
            find_bytes_from(&row_content, representative.reference_attr, search_start)
        else {
            continue;
        };
        let Some(cell_start) = find_cell_start_before_reference(&row_content, reference_start)
        else {
            search_start = reference_start + representative.reference_attr.len();
            continue;
        };
        let Some(cell_end) = find_cell_end_from_start(&row_content, cell_start) else {
            search_start = reference_start + representative.reference_attr.len();
            continue;
        };

        let bytes_removed = cell_end - cell_start;
        row_content.drain(cell_start..cell_end);
        removed.push(RemovedStyleRepresentative {
            representative,
            bytes_removed,
        });
        search_start = cell_start;
    }

    RepresentativeRemovalResult {
        row_content,
        removed,
    }
}

fn find_cell_start_before_reference(row_content: &[u8], reference_start: usize) -> Option<usize> {
    let mut search_end = reference_start;
    while let Some(cell_start) = rfind_bytes(&row_content[..search_end], b"<c") {
        if is_cell_tag_start(row_content, cell_start) {
            return Some(cell_start);
        }
        search_end = cell_start;
    }
    None
}

fn find_cell_end_from_start(row_content: &[u8], cell_start: usize) -> Option<usize> {
    let tag_end = find_byte_from(row_content, b'>', cell_start)?;
    if tag_end > cell_start && row_content[tag_end - 1] == b'/' {
        return Some(tag_end + 1);
    }
    find_bytes_from(row_content, b"</c>", tag_end + 1).map(|end| end + b"</c>".len())
}

fn remove_tentative_representatives_from_metrics(
    metrics: &mut CleanSheetMetrics,
    removed: &[RemovedStyleRepresentative<'_>],
    kept_cells_start: usize,
) {
    for removed in removed {
        if let Some(index) = metrics.kept_cell_states[kept_cells_start..]
            .iter()
            .position(|cell| {
                cell.col == removed.representative.col && cell.row == removed.representative.row
            })
        {
            let removed_cell = metrics.kept_cell_states.remove(kept_cells_start + index);
            if removed_cell.is_tail_anchor {
                metrics.remove_tail_anchor_row(removed_cell.row);
            }
        }
        metrics.deleted_cell_count += 1;
    }
}

fn empty_cell_has_no_empty_cell_visible_style(
    cell: &ParsedCell<'_, '_>,
    params: &CleanSheetParameters,
    has_cell_content: bool,
) -> bool {
    if has_cell_content {
        return false;
    }

    cell.style_id
        .and_then(|style_id| params.patterns.xfs_by_index.get(&style_id))
        .is_some_and(|style| style.has_no_empty_cell_visible_style)
}

#[inline]
fn empty_cell_has_default_style(cell: &ParsedCell<'_, '_>, has_cell_content: bool) -> bool {
    cell.style_id.is_none() && !has_cell_content
}

#[inline(always)]
fn is_cell_tag_start(data: &[u8], cell_start: usize) -> bool {
    matches!(
        data.get(cell_start + b"<c".len()),
        Some(byte) if byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>')
    )
}

fn parse_cell_from_parts<'a, 'r>(raw: &'a [u8], attr: &'a [u8]) -> Option<ParsedCell<'a, 'r>> {
    let mut reference_attr = None;
    let mut style_id = None;
    for attribute in Attributes::new(attr) {
        let attribute = attribute.ok()?;
        match attribute.name {
            b"r" if reference_attr.is_none() => reference_attr = Some(attribute),
            b"s" if style_id.is_none() => {
                style_id = Some(digits_byte_string_to_integer(attribute.value))
            }
            _ => {}
        }
    }
    let reference_attr = reference_attr?;
    let reference = reference_attr.value;
    let split = reference
        .iter()
        .position(u8::is_ascii_digit)
        .filter(|split| *split > 0 && *split < reference.len())?;
    let column = &reference[..split];
    let row = &reference[split..];
    if !column.iter().all(u8::is_ascii_uppercase) || !row.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(ParsedCell {
        row_view: None,
        raw,
        attr,
        reference_attr: reference_attr.assignment,
        row,
        col_num: letters_byte_string_to_integer(column),
        style_id,
    })
}

fn record_visible_row_bounds(row: &RowView<'_, '_>, metrics: &mut CleanSheetMetrics) {
    if !row.style.is_some_and(|style| style.has_fill_or_border) {
        return;
    }
    if let Some(number) = row.number {
        metrics.visible_content_bounds.include_row(number);
        metrics.record_tail_anchor_row(number);
    }
}

fn render_empty_row<'a, const SAMPLED: bool>(
    raw: &'a [u8],
    attributes: &'a [u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    let cleaning = profiling::sampled_phase::<SAMPLED>("row_clean_empty_attributes");
    let cleaned = clean_empty_row_attributes(attributes, params, metrics);
    profiling::drop_optional("row.cleaning_span", cleaning);
    if raw.ends_with(b"/>") && matches!(cleaned, Cow::Borrowed(_)) {
        return Cow::Borrowed(raw);
    }
    let _build = profiling::sampled_phase::<SAMPLED>("row_build_xml");
    Cow::Owned(generate_self_closing_row_xml(&cleaned))
}

fn clean_empty_row_attributes<'a>(
    row_attr: &'a [u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    let cleaned = if params.cleanup.empty_row_attribute_cleanup {
        remove_irrelevant_attributes_for_empty_rows(row_attr, params)
    } else {
        Cow::Borrowed(row_attr)
    };

    metrics.cleanup_stats.record_delta(
        CleanupCause::EmptyRowAttributeCleanup,
        row_attr.len(),
        cleaned.len(),
    );
    cleaned
}

fn generate_self_closing_row_xml(row_attr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(row_attr.len() + 7);
    output.extend_from_slice(b"<row");
    output.extend_from_slice(row_attr);
    output.extend_from_slice(b"/>");
    output
}

fn generate_row_xml(row_attr: &[u8], row_content: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(row_attr.len() + row_content.len() + 11);
    output.extend_from_slice(b"<row");
    output.extend_from_slice(row_attr);
    output.extend_from_slice(b">");
    output.extend_from_slice(row_content);
    output.extend_from_slice(b"</row>");
    output
}

fn rewrite_row_spans_upper_bound<'a>(
    row: &RowView<'a, '_>,
    max_kept_col: Option<u32>,
) -> Cow<'a, [u8]> {
    if row.number.is_none_or(|number| number == 0) {
        return Cow::Borrowed(row.raw_attributes);
    }
    let Some((start, end)) = row.spans else {
        return Cow::Borrowed(row.raw_attributes);
    };
    let Some(maximum) = max_kept_col else {
        return Cow::Borrowed(row.raw_attributes);
    };
    rewrite_spans_value(row.raw_attributes, start, end, maximum)
}

fn rewrite_spans_value(attributes: &[u8], start: usize, end: usize, maximum: u32) -> Cow<'_, [u8]> {
    let value = &attributes[start..end];
    let Some(colon) = value.iter().position(|byte| *byte == b':') else {
        return Cow::Borrowed(attributes);
    };
    let mut buffer = [0u8; 10];
    let upper = decimal_bytes(maximum, &mut buffer);
    if &value[colon + 1..] == upper {
        return Cow::Borrowed(attributes);
    }
    let mut out = Vec::with_capacity(attributes.len() - value.len() + colon + 1 + upper.len());
    out.extend_from_slice(&attributes[..start + colon + 1]);
    out.extend_from_slice(upper);
    out.extend_from_slice(&attributes[end..]);
    Cow::Owned(out)
}

fn decimal_bytes(mut value: u32, buffer: &mut [u8; 10]) -> &[u8] {
    let mut start = buffer.len();
    loop {
        start -= 1;
        buffer[start] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            return &buffer[start..];
        }
    }
}

fn remove_irrelevant_attributes_for_empty_rows<'a>(
    row_attr: &'a [u8],
    _params: &CleanSheetParameters,
) -> Cow<'a, [u8]> {
    Cow::Borrowed(row_attr)
}

#[cfg(test)]
#[path = "rows_regression_tests.rs"]
mod regression_tests;
