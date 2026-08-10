use crate::models::{CleanSheetMetrics, CleanSheetParameters, CleanupCause, RowAttributes};
use crate::sheet::cells::{
    parsed_cell_has_content, parsed_empty_cell_has_effective_fill_or_border,
    replace_parsed_cell_with_empty_style_policy, ParsedCell,
};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use crate::xml::{find_byte_from, find_bytes, find_bytes_from, rfind_bytes};
use regex::bytes::Captures;
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmptyStyleRepresentativePolicy {
    keep_default_styleless: bool,
    keep_font_styles: bool,
    require_content_in_row: bool,
}

impl EmptyStyleRepresentativePolicy {
    #[inline]
    fn for_row(params: &CleanSheetParameters, row: &RowAttributes) -> Self {
        Self {
            keep_default_styleless: params.cleanup.keep_textless_default_font_representatives,
            keep_font_styles: params.cleanup.keep_textless_style_representatives,
            require_content_in_row: row.has_custom_height,
        }
    }
}

struct CleanCellsResult<'a> {
    row_content: Cow<'a, [u8]>,
    tentative_style_representatives: Vec<TentativeStyleRepresentative>,
    cells_removed: bool,
}

#[derive(Debug, Clone)]
struct TentativeStyleRepresentative {
    reference_attr: Vec<u8>,
    col: u32,
    row: u32,
}

struct RepresentativeRemovalResult {
    row_content: Vec<u8>,
    removed: Vec<RemovedStyleRepresentative>,
}

struct RemovedStyleRepresentative {
    representative: TentativeStyleRepresentative,
    bytes_removed: usize,
}

#[derive(Debug, Clone, Copy)]
struct XmlAttributeMatch {
    assignment_start: usize,
    value_start: usize,
    value_end: usize,
}

#[derive(Debug, Clone, Copy)]
struct ParsedCellBlock<'a> {
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

struct RowCellCleanupState {
    output: Option<Vec<u8>>,
    position: usize,
    cells_removed: bool,
    kept_default_style_cell: bool,
    tentative_style_representatives: Vec<TentativeStyleRepresentative>,
    kept_represented_style_ids: std::collections::HashSet<u32>,
}

impl RowCellCleanupState {
    fn new() -> Self {
        Self {
            output: None,
            position: 0,
            cells_removed: false,
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

    fn record_kept_empty_representative(
        &mut self,
        cell: &ParsedCell<'_>,
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
                    reference_attr: cell.reference_attr.to_vec(),
                    col: cell.col_num,
                    row: digits_byte_string_to_integer(cell.row),
                });
        }
    }

    fn finish<'a>(self, row_content: &'a [u8]) -> CleanCellsResult<'a> {
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
    let Some(row_attributes) = extract_row_attributes_from_parts(row_attr, row_content, params)
    else {
        return Cow::Borrowed(row_raw);
    };

    let original_row_content = row_content.unwrap_or_default();
    register_row_attributes(&row_attributes, metrics);

    let style_policy = EmptyStyleRepresentativePolicy::for_row(params, &row_attributes);
    let clean_result = clean_row_content(
        params,
        metrics,
        original_row_content,
        style_policy,
        &row_attributes,
    );

    render_cleaned_row(
        row_raw,
        row_attr,
        clean_result,
        params,
        metrics,
        row_attributes,
    )
}

fn clean_row_content<'a>(
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    original_row_content: &'a [u8],
    style_policy: EmptyStyleRepresentativePolicy,
    row_attributes: &RowAttributes,
) -> CleanCellsResult<'a> {
    if original_row_content.is_empty() {
        return unchanged_row_content(original_row_content);
    }

    let clean_result = clean_cells(params, metrics, original_row_content, style_policy);
    remove_unneeded_style_representatives(clean_result, style_policy, row_attributes, metrics)
}

fn unchanged_row_content(row_content: &[u8]) -> CleanCellsResult<'_> {
    CleanCellsResult {
        row_content: Cow::Borrowed(row_content),
        tentative_style_representatives: Vec::new(),
        cells_removed: false,
    }
}

fn remove_unneeded_style_representatives<'a>(
    clean_result: CleanCellsResult<'a>,
    style_policy: EmptyStyleRepresentativePolicy,
    row_attributes: &RowAttributes,
    metrics: &mut CleanSheetMetrics,
) -> CleanCellsResult<'a> {
    if !style_policy.require_content_in_row
        || metrics
            .rows_with_kept_content_cells
            .contains(row_attributes.ref_.as_slice())
        || clean_result.tentative_style_representatives.is_empty()
    {
        return clean_result;
    }

    let removal = remove_tentative_style_representatives(
        clean_result.row_content.into_owned(),
        clean_result.tentative_style_representatives,
    );
    record_style_representative_removal(metrics, &removal.removed);

    CleanCellsResult {
        row_content: Cow::Owned(removal.row_content),
        tentative_style_representatives: Vec::new(),
        cells_removed: clean_result.cells_removed || !removal.removed.is_empty(),
    }
}

fn record_style_representative_removal(
    metrics: &mut CleanSheetMetrics,
    removed: &[RemovedStyleRepresentative],
) {
    for removed_cell in removed {
        metrics.cleanup_stats.record_removed(
            CleanupCause::EmptyCellsWithoutContent,
            removed_cell.bytes_removed,
        );
    }
    remove_tentative_representatives_from_metrics(metrics, removed);
}

fn render_cleaned_row<'a>(
    row_raw: &'a [u8],
    row_attr: &[u8],
    clean_result: CleanCellsResult<'a>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_attributes: RowAttributes,
) -> Cow<'a, [u8]> {
    if clean_result.row_content.is_empty() {
        return Cow::Owned(generate_and_log_self_closing_row(
            row_attr,
            params,
            metrics,
            row_attributes,
        ));
    }

    if matches!(&clean_result.row_content, Cow::Borrowed(_)) {
        metrics.kept_row_states.push(row_attributes);
        return Cow::Borrowed(row_raw);
    }

    Cow::Owned(generate_and_log_row(
        row_attr,
        clean_result.row_content.as_ref(),
        metrics,
        row_attributes,
        clean_result.cells_removed,
    ))
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
    let Some(row_attributes) = extract_empty_row_attributes_from_attr(row_attr, params) else {
        return Cow::Borrowed(row_raw);
    };

    register_row_attributes(&row_attributes, metrics);

    generate_and_log_self_closing_row(row_attr, params, metrics, row_attributes).into()
}

fn clean_cells<'a>(
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_content: &'a [u8],
    style_policy: EmptyStyleRepresentativePolicy,
) -> CleanCellsResult<'a> {
    if row_content.is_empty() {
        return unchanged_row_content(row_content);
    }

    let mut state = RowCellCleanupState::new();
    while let Some(cell_start_rel) = find_bytes(&row_content[state.position..], b"<c") {
        let cell_start = state.position + cell_start_rel;

        if !is_cell_tag_start(row_content, cell_start) {
            state.advance_past_non_cell_match(row_content, cell_start + b"<c".len());
            continue;
        }

        let Some(block) = parse_cell_block(row_content, cell_start) else {
            break;
        };
        let cleaned_cell = clean_cell_block(block, params, metrics, style_policy, &mut state);
        state.append_cleaned_cell(row_content, block, cleaned_cell);
    }

    state.finish(row_content)
}

fn parse_cell_block(row_content: &[u8], cell_start: usize) -> Option<ParsedCellBlock<'_>> {
    let tag_end = find_byte_from(row_content, b'>', cell_start)?;
    let self_closing = tag_end > cell_start && row_content[tag_end - 1] == b'/';
    let attr_start = cell_start + b"<c".len();
    let attr_end = if self_closing { tag_end - 1 } else { tag_end };

    let cell_end = if self_closing {
        tag_end + 1
    } else {
        let content_start = tag_end + 1;
        let close_rel = find_bytes(&row_content[content_start..], b"</c>")?;
        content_start + close_rel + b"</c>".len()
    };

    Some(ParsedCellBlock {
        start: cell_start,
        end: cell_end,
        raw: &row_content[cell_start..cell_end],
        attr: &row_content[attr_start..attr_end],
    })
}

fn clean_cell_block<'a>(
    block: ParsedCellBlock<'a>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    style_policy: EmptyStyleRepresentativePolicy,
    state: &mut RowCellCleanupState,
) -> Cow<'a, [u8]> {
    let Some(cell) = parse_cell_from_parts(block.raw, block.attr) else {
        return Cow::Borrowed(block.raw);
    };

    let has_cell_content = parsed_cell_has_content(&cell);
    record_cell_visit(params, &cell);
    let representative = select_empty_style_representative(
        &cell,
        params,
        metrics,
        has_cell_content,
        style_policy,
        state,
    );
    let cleaned_cell = replace_parsed_cell_with_empty_style_policy(
        &cell,
        params,
        metrics,
        has_cell_content,
        representative.keep_font_only,
        representative.keep_merged,
        representative.keep_default,
    );

    if !cleaned_cell.is_empty() && !has_cell_content {
        state.record_kept_empty_representative(&cell, representative, style_policy);
    }

    cleaned_cell
}

fn select_empty_style_representative(
    cell: &ParsedCell<'_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    has_cell_content: bool,
    policy: EmptyStyleRepresentativePolicy,
    state: &RowCellCleanupState,
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
fn record_cell_visit(params: &CleanSheetParameters, cell: &ParsedCell<'_>) {
    crate::cleaning_trace::record_cell_visit_if_target(crate::cleaning_trace::CellVisitEvent {
        sheet_file: &params.zip_info_filename,
        col: cell.col_num,
        row: cell.row,
        style_id: cell.style_id,
    });
}

#[cfg(not(feature = "cell-tracing"))]
#[inline]
fn record_cell_visit(_params: &CleanSheetParameters, _cell: &ParsedCell<'_>) {}

fn record_represented_cell_style(
    cell: &ParsedCell<'_>,
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
    cell: &ParsedCell<'_>,
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

fn remove_tentative_style_representatives(
    mut row_content: Vec<u8>,
    representatives: Vec<TentativeStyleRepresentative>,
) -> RepresentativeRemovalResult {
    let mut removed = Vec::new();
    let mut search_start = 0usize;

    for representative in representatives {
        let Some(reference_start) =
            find_bytes_from(&row_content, &representative.reference_attr, search_start)
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
    removed: &[RemovedStyleRepresentative],
) {
    for removed in removed {
        if let Some(index) = metrics.kept_cell_states.iter().position(|cell| {
            cell.col == removed.representative.col && cell.row == removed.representative.row
        }) {
            let removed_cell = metrics.kept_cell_states.remove(index);
            if removed_cell.is_tail_anchor {
                metrics.remove_tail_anchor_row(removed_cell.row);
            }
        }
        metrics
            .deleted_cell_states
            .push(crate::models::CellAttributes {
                col: removed.representative.col,
                row: removed.representative.row,
                kept_only_for_protection: false,
                is_tail_anchor: false,
            });
    }
}

fn empty_cell_has_no_empty_cell_visible_style(
    cell: &ParsedCell<'_>,
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
fn empty_cell_has_default_style(cell: &ParsedCell<'_>, has_cell_content: bool) -> bool {
    cell.style_id.is_none() && !has_cell_content
}

#[inline(always)]
fn is_cell_tag_start(data: &[u8], cell_start: usize) -> bool {
    matches!(
        data.get(cell_start + b"<c".len()),
        Some(byte) if byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>')
    )
}

fn parse_cell_from_parts<'a>(raw: &'a [u8], attr: &'a [u8]) -> Option<ParsedCell<'a>> {
    let reference = extract_attr_value(attr, b"r")?;
    let split = reference
        .iter()
        .position(|byte| byte.is_ascii_digit())
        .filter(|split| *split > 0 && *split < reference.len())?;

    let column = &reference[..split];
    let row = &reference[split..];
    let reference_attr = extract_attr_assignment(attr, b"r")?;

    if !column.iter().all(|byte| byte.is_ascii_uppercase())
        || !row.iter().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let style_id = extract_attr_value(attr, b"s").map(digits_byte_string_to_integer);

    Some(ParsedCell {
        raw,
        attr,
        reference_attr,
        row,
        col_num: letters_byte_string_to_integer(column),
        style_id,
    })
}

#[inline]
fn extract_attr_assignment<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let attribute = find_xml_attribute(data, attr_name)?;
    Some(&data[attribute.assignment_start..attribute.value_end + 1])
}

fn extract_row_attributes_from_parts(
    row_attr: &[u8],
    row_content: Option<&[u8]>,
    params: &CleanSheetParameters,
) -> Option<RowAttributes> {
    Some(parse_row_attributes(
        row_attr,
        row_content.is_none_or(|content| content.is_empty()),
        params,
    ))
}

fn extract_empty_row_attributes_from_attr(
    row_attr: &[u8],
    params: &CleanSheetParameters,
) -> Option<RowAttributes> {
    Some(parse_row_attributes(row_attr, true, params))
}

fn parse_row_attributes(
    row_attr: &[u8],
    is_empty: bool,
    params: &CleanSheetParameters,
) -> RowAttributes {
    let ref_ = extract_attr_value(row_attr, b"r")
        .map(|value| value.to_vec())
        .unwrap_or_default();
    let hidden = extract_attr_value(row_attr, b"hidden").is_some_and(|value| value == b"1");
    let style_id = extract_attr_value(row_attr, b"s").map(digits_byte_string_to_integer);
    let has_height = extract_attr_value(row_attr, b"ht").is_some();
    let has_custom_height =
        extract_attr_value(row_attr, b"customHeight").is_some_and(|value| value == b"1");
    let style = style_id.and_then(|id| params.patterns.xfs_by_index.get(&id).cloned());

    RowAttributes {
        row_attr: row_attr.to_vec(),
        ref_,
        hidden,
        style,
        has_height,
        has_custom_height,
        is_empty,
    }
}

#[inline]
fn extract_attr_value<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let attribute = find_xml_attribute(data, attr_name)?;
    Some(&data[attribute.value_start..attribute.value_end])
}

fn find_xml_attribute(data: &[u8], attr_name: &[u8]) -> Option<XmlAttributeMatch> {
    let mut search_start = 0;

    while search_start < data.len() {
        let relative = find_bytes(&data[search_start..], attr_name)?;
        let assignment_start = search_start + relative;
        let value_marker = assignment_start + attr_name.len();

        if assignment_start > 0
            && data[assignment_start - 1].is_ascii_whitespace()
            && data.get(value_marker..value_marker + 2) == Some(b"=\"")
        {
            let value_start = value_marker + 2;
            let value_length = data[value_start..].iter().position(|byte| *byte == b'"')?;
            return Some(XmlAttributeMatch {
                assignment_start,
                value_start,
                value_end: value_start + value_length,
            });
        }

        search_start = assignment_start + attr_name.len();
    }

    None
}

fn register_row_attributes(row_attributes: &RowAttributes, metrics: &mut CleanSheetMetrics) {
    metrics
        .row_attributes
        .insert(row_attributes.ref_.clone(), row_attributes.clone());
    record_visible_row_bounds(row_attributes, metrics);
}

fn record_visible_row_bounds(row_attributes: &RowAttributes, metrics: &mut CleanSheetMetrics) {
    if !row_attributes
        .style
        .as_ref()
        .is_some_and(|style| style.has_fill_or_border)
    {
        return;
    }

    let Ok(row_str) = std::str::from_utf8(&row_attributes.ref_) else {
        return;
    };
    let Ok(row_num) = row_str.parse::<u32>() else {
        return;
    };

    metrics.visible_content_bounds.include_row(row_num);
    metrics.record_tail_anchor_row(row_num);
}

fn generate_and_log_self_closing_row(
    row_attr: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_attributes: RowAttributes,
) -> Vec<u8> {
    metrics.kept_row_states.push(row_attributes);
    let cleaned_row_attr = clean_empty_row_attributes(row_attr, params, metrics);
    generate_self_closing_row_xml(cleaned_row_attr.as_ref())
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

fn generate_and_log_row(
    row_attr: &[u8],
    row_content: &[u8],
    metrics: &mut CleanSheetMetrics,
    row_attributes: RowAttributes,
    rewrite_spans: bool,
) -> Vec<u8> {
    let row_attr = if rewrite_spans {
        rewrite_row_spans_upper_bound(row_attr, metrics, &row_attributes)
    } else {
        Cow::Borrowed(row_attr)
    };
    metrics.kept_row_states.push(row_attributes);
    generate_row_xml(row_attr.as_ref(), row_content)
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
    row_attr: &'a [u8],
    metrics: &CleanSheetMetrics,
    row_attributes: &RowAttributes,
) -> Cow<'a, [u8]> {
    let row_number = digits_byte_string_to_integer(&row_attributes.ref_);
    if row_number == 0 {
        return Cow::Borrowed(row_attr);
    }

    let Some(max_kept_col) = metrics
        .kept_cell_states
        .iter()
        .filter(|cell| cell.row == row_number)
        .map(|cell| cell.col)
        .max()
    else {
        return Cow::Borrowed(row_attr);
    };

    rewrite_spans_attr_upper_bound(row_attr, max_kept_col)
}

fn rewrite_spans_attr_upper_bound(row_attr: &[u8], max_kept_col: u32) -> Cow<'_, [u8]> {
    let Some(value_range) = find_attr_value_range(row_attr, b"spans") else {
        return Cow::Borrowed(row_attr);
    };

    let value = &row_attr[value_range.clone()];
    let Some(colon) = value.iter().position(|byte| *byte == b':') else {
        return Cow::Borrowed(row_attr);
    };

    let upper = max_kept_col.to_string();
    if &value[colon + 1..] == upper.as_bytes() {
        return Cow::Borrowed(row_attr);
    }

    let mut out =
        Vec::with_capacity(row_attr.len() - value.len() + colon + 1 + upper.as_bytes().len());
    out.extend_from_slice(&row_attr[..value_range.start]);
    out.extend_from_slice(&value[..colon + 1]);
    out.extend_from_slice(upper.as_bytes());
    out.extend_from_slice(&row_attr[value_range.end..]);
    Cow::Owned(out)
}

#[inline]
fn find_attr_value_range(data: &[u8], attr_name: &[u8]) -> Option<std::ops::Range<usize>> {
    let attribute = find_xml_attribute(data, attr_name)?;
    Some(attribute.value_start..attribute.value_end)
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
