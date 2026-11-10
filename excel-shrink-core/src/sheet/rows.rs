use crate::models::{CleanSheetMetrics, CleanSheetParameters, CleanupCause, Xf};
use crate::profiling;
use crate::sheet::cells::{
    empty_cell_overrides_row_font, parsed_empty_cell_has_effective_fill_or_border,
    replace_parsed_cell_with_empty_style_policy, ParsedCell,
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
            // Empty fonts can also affect text baselines at a fixed height.
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
    has_required_style: bool,
}

impl EmptyStyleRepresentativeDecision {
    #[inline]
    fn is_tentative(self, policy: EmptyStyleRepresentativePolicy) -> bool {
        policy.require_content_in_row
            && !self.has_required_style
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
    source_default_font_present: Option<bool>,
    source_other_font_presence: std::collections::BTreeMap<u32, bool>,
    kept_default_style_cell: bool,
    tentative_style_representatives: Vec<TentativeStyleRepresentative<'a>>,
    // A single represented style needs no per-row hash table allocation.
    first_kept_represented_style_id: Option<u32>,
    kept_represented_style_ids: std::collections::HashSet<u32>,
    // Most rows use one font; its ID needs no per-row hash table allocation.
    first_kept_font_id: Option<u32>,
    kept_font_ids: std::collections::HashSet<u32>,
    kept_merged_fonts: std::collections::HashSet<(usize, u32)>,
    hidden_tail: Option<super::hidden_content::HiddenTail>,
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
            source_default_font_present: None,
            source_other_font_presence: std::collections::BTreeMap::new(),
            kept_default_style_cell: false,
            tentative_style_representatives: Vec::new(),
            first_kept_represented_style_id: None,
            kept_represented_style_ids: std::collections::HashSet::new(),
            first_kept_font_id: None,
            kept_font_ids: std::collections::HashSet::new(),
            kept_merged_fonts: std::collections::HashSet::new(),
            hidden_tail: None,
        }
    }

    /// Include the inline representative when checking already covered styles.
    fn has_kept_represented_style_id(&self, style_id: u32) -> bool {
        self.first_kept_represented_style_id == Some(style_id)
            || self.kept_represented_style_ids.contains(&style_id)
    }

    /// Keep the first style inline and allocate only for other distinct styles.
    fn record_kept_represented_style_id(&mut self, style_id: u32) {
        if let Some(first) = self.first_kept_represented_style_id {
            if first != style_id {
                self.kept_represented_style_ids.insert(style_id);
            }
        } else {
            self.first_kept_represented_style_id = Some(style_id);
        }
    }

    fn has_kept_font_id(&self, font: u32) -> bool {
        self.first_kept_font_id == Some(font) || self.kept_font_ids.contains(&font)
    }

    fn record_kept_font_id(&mut self, font: u32) {
        if let Some(first) = self.first_kept_font_id {
            if first != font {
                self.kept_font_ids.insert(font);
            }
        } else {
            self.first_kept_font_id = Some(font);
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
        if let Some(style_id) = cell.style_id {
            self.record_kept_represented_style_id(style_id);
        } else {
            self.kept_default_style_cell = true;
        }

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
    if let Some(promotion) =
        choose_promoted_row_style(&row, row_content.unwrap_or_default(), params, metrics)
    {
        let style_id = promotion.style_id;
        let original_content = row_content.unwrap_or_default();
        let content = promotion.restore_missing_cells(original_content, row.reference);
        let mut attributes = row_attr.to_vec();
        attributes.extend_from_slice(format!(" s=\"{style_id}\" customFormat=\"1\"").as_bytes());
        let mut promoted = RowView::parse(&attributes, params).expect("validated row attributes");
        promoted.promoted_style_id = Some(style_id);
        record_visible_row_bounds(&promoted, metrics);
        metrics.cleanup_stats.record_delta(
            CleanupCause::EmptyCellsWithoutContent,
            row_attr.len() + original_content.len(),
            attributes.len() + content.len(),
        );
        let raw = generate_row_xml(&attributes, &content);
        let clean = clean_row_content::<SAMPLED>(
            params,
            metrics,
            &content,
            EmptyStyleRepresentativePolicy::for_row(params, &promoted),
            &promoted,
        );
        tracing::debug!(target: "excel_shrink::profile", event = "row_font_promoted",
            sheet = %params.zip_info_filename, row = ?row.number, style_id);
        return Cow::Owned(
            render_cleaned_row::<SAMPLED>(&raw, clean, params, metrics, &promoted).into_owned(),
        );
    }
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

struct RowStylePromotion {
    style_id: u32,
    missing_cells: Vec<(usize, u32, u32)>, // XML offset, column, original inherited XF
}

impl RowStylePromotion {
    fn restore_missing_cells<'a>(&self, content: &'a [u8], row: &[u8]) -> Cow<'a, [u8]> {
        if self.missing_cells.is_empty() {
            return Cow::Borrowed(content);
        }
        let mut output = Vec::with_capacity(content.len() + self.missing_cells.len() * 32);
        let mut previous = 0;
        for &(offset, mut column, style_id) in &self.missing_cells {
            output.extend_from_slice(&content[previous..offset]);
            previous = offset;
            let mut letters = [0u8; 3];
            let mut first = letters.len();
            while column > 0 {
                column -= 1;
                first -= 1;
                letters[first] = b'A' + (column % 26) as u8;
                column /= 26;
            }
            output.extend_from_slice(b"<c r=\"");
            output.extend_from_slice(&letters[first..]);
            output.extend_from_slice(row);
            output.extend_from_slice(format!("\" s=\"{style_id}\"/>").as_bytes());
        }
        output.extend_from_slice(&content[previous..]);
        Cow::Owned(output)
    }
}

/// Promote only existing XFs; unknown styles, existing row formats and
/// malformed/out-of-order cells are a fallback. Small gaps in dense rows retain
/// their original formatting as explicit cells before the row style changes.
/// This bounded look-ahead scans the current row, never the worksheet again.
fn choose_promoted_row_style(
    row: &RowView<'_, '_>,
    content: &[u8],
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
) -> Option<RowStylePromotion> {
    if !params.cleanup.empty_cells_without_content
        || row.hidden
        || row.style.is_some()
        // A new row font can shrink Excel's inherited automatic height even
        // when every cell already uses that font. Only fixed heights are safe.
        || !(row.has_height && row.has_custom_height)
        || content.len() < 64
        || metrics.sheet_protection_enabled
    {
        return None;
    }
    for attr in Attributes::new(row.raw_attributes) {
        if matches!(attr.ok()?.name, b"s" | b"customFormat") {
            return None;
        }
    }
    let row_num = row.number?;
    let row_has_merge = metrics
        .merge_cell_ranges
        .iter()
        .any(|r| r.start_row <= row_num && row_num <= r.end_row);
    let mut counts = std::collections::BTreeMap::<u32, usize>::new();
    let mut gaps = Vec::new();
    let mut pos = 0;
    let mut previous_col = 0;
    let mut uniform_applied_fill = None;
    while let Some(offset) = find_bytes(&content[pos..], b"<c") {
        let start = pos + offset;
        if !is_cell_tag_start(content, start) {
            return None;
        }
        let block = parse_cell_block(content, start)?;
        // Reject extension elements outside cells as well as nonstandard cell
        // metadata: preserve them through the ordinary cleanup path.
        if !content[pos..start].iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        let cell = parse_cell_from_parts(block.raw, block.attr)?;
        if cell.row != row.reference || cell.col_num <= previous_col || cell.col_num > 16384 {
            return None;
        }
        for a in Attributes::new(block.attr) {
            if !matches!(a.ok()?.name, b"r" | b"s" | b"t") {
                return None;
            }
        }
        let id = cell.style_id?; // An unstyled value would acquire the new row style.
        let xf = params.patterns.xfs_by_index.get(&id)?;
        let applied_fill = xf.fill_id.filter(|_| xf.apply_fill);
        if previous_col == 0 {
            uniform_applied_fill = applied_fill;
        } else if applied_fill != uniform_applied_fill {
            uniform_applied_fill = None;
        }
        if cell.col_num > previous_col + 1 {
            gaps.push((previous_col + 1, cell.col_num - 1, start));
        }
        previous_col = cell.col_num;
        if !block.has_content && !xf.has_apply_protection {
            *counts.entry(id).or_default() += 1;
        }
        pos = block.end;
    }
    if !content[pos..].iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    if previous_col < 16384 {
        gaps.push((previous_col + 1, 16384, content.len()));
    }
    let (&dominant, &count) = counts
        .iter()
        .max_by_key(|(id, count)| (**count, std::cmp::Reverse(**id)))?;
    if count < 2 {
        return None;
    }
    let xf = params.patterns.xfs_by_index.get(&dominant)?;
    // Dense promotion needs applyFont on this XF. Sparse promotion can also
    // use its existing font-only representative, whose applyFont is checked below.
    if !xf.apply_font && xf.row_font_style_id.is_none() {
        return None;
    }
    // The entire row was explicit: moving an identical full XF changes no
    // previously implicit position. Restrict this path to real dense bloat.
    // Filled rows must apply the same fill to every cell, including content.
    let preserves_uniform_fill = xf.has_visible_fill
        && uniform_applied_fill == xf.fill_id
        && !xf.has_visible_border
        && !xf.apply_border
        && !xf.has_apply_protection;
    if gaps.is_empty()
        && count >= 1024
        && xf.apply_font
        && (xf.has_no_empty_cell_visible_style || preserves_uniform_fill)
    {
        return Some(RowStylePromotion { style_id: dominant, missing_cells: Vec::new() });
    }
    if count >= 1024 && xf.apply_font && xf.has_no_empty_cell_visible_style
        && params.cleanup.keep_textless_column_style_representatives
        && gaps.iter().map(|&(a, b, _)| b - a + 1).sum::<u32>() <= 64
    {
        if let Some(missing_cells) = restore_dense_row_gaps(&gaps, xf, params, metrics) {
            return Some(RowStylePromotion { style_id: dominant, missing_cells });
        }
    }
    // A dense row retains its explicit merged cells through the normal merge
    // policy; sparse merged rows need a stronger inheritance proof.
    if row_has_merge {
        return None;
    }
    // Every previously absent cell in a sparse row must already inherit the
    // proposed format, including hidden/trailing columns and non-font parts.
    let default = params.patterns.xfs_by_index.get(&0)?;
    // Prefer the font-only XF, but preserve other inherited components too.
    // If needed, the original empty-cell XF can serve as the row default.
    [xf.row_font_style_id, Some(dominant)]
        .into_iter()
        .flatten()
        .find(|id| {
            let target = &params.patterns.xfs_by_index[id];
            target.apply_font
                && gaps.iter().all(|&(a, b, _)| {
                    metrics
                        .column_definitions
                        .inherits_row_style(a, b, target, default)
                })
        })
        .map(|style_id| RowStylePromotion { style_id, missing_cells: Vec::new() })
}

fn restore_dense_row_gaps(
    gaps: &[(u32, u32, usize)],
    row_style: &Xf,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
) -> Option<Vec<(usize, u32, u32)>> {
    let mut cells = Vec::new();
    for &(start, end, offset) in gaps {
        for column in start..=end {
            let previous = match metrics.column_definitions.get(column) {
                Some(definition) if !definition.hidden => definition.cell_xf.as_ref()?,
                Some(_) => return None,
                None => params.patterns.xfs_by_index.get(&0)?,
            };
            previous.non_font_style_id?;
            if previous == row_style {
                continue;
            }
            // A font override is retained on subsequent cleanup passes too.
            // Other differences alone need a stronger inheritance proof.
            if previous.font_identity() == row_style.font_identity() {
                return None;
            }
            let style_id = params.patterns.xfs_by_index.iter()
                .filter_map(|(&id, xf)| (xf == previous).then_some(id))
                .min()?;
            cells.push((offset, column, style_id));
        }
    }
    Some(cells)
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
    if clean_result.row_content.is_empty() && row.promoted_style_id.is_some() {
        return Cow::Owned(generate_self_closing_row_xml(row.raw_attributes));
    }
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
    let candidate_start = metrics.candidates.len();
    if !metrics.source_content_cell_count_complete
        && (params.cleanup.unreferenced_hidden_content_cells
            || params.advanced_cleanup.as_ref()
                .is_some_and(|c| c.remove_unreferenced_hidden_content))
    {
        state.hidden_tail = Some(super::hidden_content::HiddenTail::default());
    }
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
        if block.has_content && !metrics.source_content_cell_count_complete {
            metrics.source_content_cell_count += 1;
        }
        let cleaned_cell = clean_cell_block(
            block,
            row_content,
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

    if let Some(tail) = state.hidden_tail.take() {
        tail.finish(
            metrics, candidate_start, digits_byte_string_to_integer(row.reference),
        );
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
        if [b"<v".as_slice(), b"<is", b"<f"].iter().any(|tag| {
            tail.starts_with(tag)
                && matches!(tail.get(tag.len()), Some(b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>'))
        })
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
    source_row: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    style_policy: EmptyStyleRepresentativePolicy,
    state: &mut RowCellCleanupState<'a>,
    timings: &mut CellTimings<SAMPLED>,
    row: &RowView<'_, '_>,
) -> Cow<'a, [u8]> {
    let Some((mut cell, has_cell_content)) =
        timings.measure(CellStage::ParseAttributesAndContent, || {
            let mut cell = parse_cell_from_parts(block.raw, block.attr)?;
            cell.row_view = Some(row);
            let has_content = block.has_content;
            Some((cell, has_content))
        })
    else {
        return Cow::Borrowed(block.raw);
    };

    // ColumnDefinition/Xf contain only scalar values: this stack snapshot has
    // no heap allocation and lets policy code mutate the remaining metrics.
    let column = metrics.column_definitions.get(cell.col_num).cloned();
    cell.column = Some(column.as_ref());
    record_cell_visit(params, &cell);
    if let Some(tail) = state.hidden_tail.as_mut() {
        tail.observe(
            &cell,
            row.hidden || column.as_ref().is_some_and(|c| c.hidden),
            has_cell_content,
        );
    }
    let merged_font = merged_font_key(&cell, params, metrics);
    let mut representative = timings.measure(CellStage::SelectRepresentative, || {
        select_empty_style_representative(
            &cell,
            params,
            metrics,
            has_cell_content,
            style_policy,
            state,
            merged_font,
        )
    });
    let preserve_local_mask = protects_unrepresented_column_font(
        &cell, has_cell_content, source_row, row, params, metrics, state,
    );
    representative.keep_font_only |= preserve_local_mask;
    representative.keep_default |= preserve_local_mask && cell.style_id.is_none();
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
        // Both visible and hidden kept cells can already represent this font.
        if let Some(font) = representative_font_id(&cell, params) {
            state.record_kept_font_id(font);
        }
        if let Some(key) = merged_font {
            state.kept_merged_fonts.insert(key);
        }
    }
    if !cleaned_cell.is_empty() && !has_cell_content {
        timings.measure(CellStage::RecordRepresentative, || {
            state.record_kept_empty_representative(&cell, representative, style_policy)
        });
    }

    cleaned_cell
}

/// Keep local masks when removal would introduce a previously absent inherited
/// font into an automatic-height row, including fonts of hidden columns.
fn protects_unrepresented_column_font(
    cell: &ParsedCell<'_, '_>,
    has_content: bool,
    source: &[u8],
    row: &RowView<'_, '_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    state: &mut RowCellCleanupState<'_>,
) -> bool {
    if has_content
        || !params.cleanup.keep_textless_column_style_representatives
        || (row.has_height && row.has_custom_height)
        || row.style.is_some()
    {
        return false;
    }
    let column = cell.column.flatten();
    // Visible explicit columns already have a separate local-override policy.
    // Hidden rows cannot contribute a new visible automatic height.
    if column.is_some_and(|column| column.cell_xf.is_some() && (!column.hidden || row.hidden)) {
        return false;
    }
    let Some(default) = params.patterns.xfs_by_index.get(&0) else {
        return false;
    };
    let inherited = column.and_then(|column| column.cell_xf.as_ref()).unwrap_or(default);
    let Some(style) = params.patterns.xfs_by_index.get(&cell.style_id.unwrap_or(0)) else {
        return false;
    };
    let exposed_font = inherited.font_identity();
    if style.font_identity() == exposed_font {
        return false;
    }
    let present = if exposed_font == default.font_identity() {
        state.source_default_font_present.get_or_insert_with(|| {
            source_row_contains_font(source, row, exposed_font, params, metrics)
        })
    } else {
        state.source_other_font_presence.entry(exposed_font).or_insert_with(|| {
            source_row_contains_font(source, row, exposed_font, params, metrics)
        })
    };
    !*present
}

/// Inspect the original row once on demand. Unknown or merged layouts cannot
/// prove that an unmasked inherited font already contributes to height.
fn source_row_contains_font(
    content: &[u8],
    row: &RowView<'_, '_>,
    wanted_font: u32,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
) -> bool {
    if metrics.merge_cell_ranges.iter().any(|range| {
        row.number.is_none_or(|number| range.start_row <= number && number <= range.end_row)
    }) {
        return false;
    }
    let Some(default_font) = params.patterns.xfs_by_index.get(&0).map(Xf::font_identity) else {
        return false;
    };
    let mut position = 0;
    let mut previous_column = 0;
    while let Some(offset) = find_bytes(&content[position..], b"<c") {
        let start = position + offset;
        if !is_cell_tag_start(content, start) {
            return false;
        }
        let Some(block) = parse_cell_block(content, start) else {
            return false;
        };
        let Some(cell) = parse_cell_from_parts(block.raw, block.attr) else {
            return false;
        };
        if cell.row != row.reference || cell.col_num <= previous_column || cell.col_num > 16384 {
            return false;
        }
        if metrics.column_definitions.contains_font(
            previous_column + 1, cell.col_num - 1, wanted_font, default_font,
        ) {
            return true;
        }
        let id = cell.style_id.unwrap_or(0);
        if params.patterns.xfs_by_index.get(&id)
            .is_some_and(|style| style.font_identity() == wanted_font)
        {
            return true;
        }
        previous_column = cell.col_num;
        position = block.end;
    }
    metrics.column_definitions.contains_font(previous_column + 1, 16384, wanted_font, default_font)
}

// Unknown styles do not establish a representative: retaining an extra cell
// is safer than claiming a font was already supplied by malformed metadata.
fn representative_font_id(cell: &ParsedCell<'_, '_>, params: &CleanSheetParameters) -> Option<u32> {
    if let Some(id) = cell.style_id {
        return params.patterns.xfs_by_index.get(&id).map(Xf::font_identity);
    }
    if let Some(style) = cell.row_view.and_then(|row| row.style) {
        return Some(style.font_identity());
    }
    Some(cell.column.flatten().and_then(|column| column.cell_xf.as_ref())
        .map(Xf::font_identity).unwrap_or(0))
}

/// Merged text can use the font of an otherwise empty covered cell for spacing.
/// A representative in a different merge does not preserve that local effect.
fn merged_font_key(
    cell: &ParsedCell<'_, '_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
) -> Option<(usize, u32)> {
    let row = digits_byte_string_to_integer(cell.row);
    let merge = metrics
        .merge_cell_ranges
        .iter()
        .position(|range| range.contains(cell.col_num, row))?;
    Some((merge, representative_font_id(cell, params)?))
}

fn select_empty_style_representative(
    cell: &ParsedCell<'_, '_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    has_cell_content: bool,
    policy: EmptyStyleRepresentativePolicy,
    state: &RowCellCleanupState<'_>,
    merged_font: Option<(usize, u32)>,
) -> EmptyStyleRepresentativeDecision {
    let row_represents_font = cell.row_view.is_some_and(|r| {
        r.promoted_style_id.is_some()
            && r.style.map(Xf::font_identity)
                == cell
                    .style_id
                    .and_then(|id| params.patterns.xfs_by_index.get(&id))
                    .map(Xf::font_identity)
    });
    let hidden_column = cell.column.flatten().is_some_and(|column| column.hidden);
    let hidden_auto_height = hidden_column && cell.row_view.is_some_and(|row| {
        !row.hidden && !(row.has_height && row.has_custom_height)
    });
    // Hidden font representatives share the row's represented-font set. Keep
    // only the first needed one, including when textless cleanup is aggressive:
    // these cells affect automatic row height rather than hidden appearance.
    let hidden_font_representative = hidden_auto_height && !has_cell_content
        && !row_represents_font
        && representative_font_id(cell, params)
            .is_some_and(|font| !state.has_kept_font_id(font));
    let merged_font_representative = !has_cell_content
        && policy.keep_font_styles
        && merged_font.is_some_and(|key| !state.kept_merged_fonts.contains(&key));
    let keep_font_only = merged_font_representative
        || hidden_font_representative
        || (!hidden_auto_height
            && policy.keep_font_styles
            && !row_represents_font
            && empty_cell_can_supply_font_representative(cell, params, metrics, has_cell_content)
            && cell
                .style_id
                .is_some_and(|style_id| !state.has_kept_represented_style_id(style_id)));
    let keep_merged =
        empty_styled_cell_is_inside_merge_range(cell, params, metrics, has_cell_content);
    let keep_default = policy.keep_default_styleless
        && !state.kept_default_style_cell
        && empty_cell_has_default_style(cell, has_cell_content);
    let has_required_style =
        parsed_empty_cell_has_effective_fill_or_border(cell, params, metrics, has_cell_content)
        || empty_cell_overrides_row_font(cell, params, has_cell_content)
        || params.chart_format_anchors.contains(
            &params.zip_info_filename,
            cell.col_num,
            digits_byte_string_to_integer(cell.row),
        );

    EmptyStyleRepresentativeDecision {
        keep_font_only,
        keep_merged,
        keep_default,
        has_required_style,
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
        .map_or(true, |style| {
            style.has_no_empty_cell_visible_style
                || (style.has_visible_fill
                    && cell
                        .row_view
                        .and_then(|row| row.style)
                        .is_some_and(|row_style| {
                            row_style.apply_fill && row_style.fill_id == style.fill_id
                        }))
        })
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

/// Explicit no-fill/no-border attributes still allow a cell to supply the font
/// that determines an otherwise empty row's automatic height.
/// Protection metadata does not cancel that font's layout contribution.
fn empty_cell_can_supply_font_representative(
    cell: &ParsedCell<'_, '_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    has_cell_content: bool,
) -> bool {
    if has_cell_content {
        return false;
    }

    cell.style_id
        .and_then(|id| params.patterns.xfs_by_index.get(&id))
        .is_some_and(|style| {
            // Explicit no-fill metadata does not cancel the font's baseline role.
            let explicit_no_fill_font =
                style.font_id.is_some() && style.apply_fill && style.fill_id == Some(0);
            // Excel can use a cell's font for automatic height even when its XF
            // omits applyFont and the row has no inherited style.
            let automatic_height_font = style.font_id.is_some_and(|font| font != 0)
                && cell.row_view.is_some_and(|row| {
                    !(row.has_height && row.has_custom_height)
                });
            style.has_no_empty_cell_visible_style
                || ((automatic_height_font || explicit_no_fill_font || style.apply_font || cell.row_view.is_some_and(|row| {
                        !(row.has_height && row.has_custom_height)
                            && row.style.is_some_and(|inherited| {
                                inherited.font_identity() != style.font_identity()
                            })
                    }))
                    && !parsed_empty_cell_has_effective_fill_or_border(cell, params, metrics, false))
        })
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
        column: None,
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
    metrics.has_visible_row_border |= !row.hidden
        && row.style.is_some_and(|style| style.has_visible_border);
    if !row.style.is_some_and(|style| {
        metrics.column_definitions.row_style_has_tail_visual_effect(style, row.raw_attributes)
    }) {
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
