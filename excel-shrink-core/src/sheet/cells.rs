#[cfg(feature = "cell-tracing")]
use crate::cleaning_trace::{CellDecision, CellDecisionEvent};
use crate::models::{
    CellAttributes, CleanSheetMetrics, CleanSheetParameters, CleanupCandidate, CleanupCause, Xf,
};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use regex::bytes::Captures;
use std::borrow::Cow;

#[cfg(feature = "cell-tracing")]
macro_rules! trace_cell_decision {
    ($event:expr) => {
        crate::cleaning_trace::record($event);
    };
}

#[cfg(not(feature = "cell-tracing"))]
macro_rules! trace_cell_decision {
    ($event:expr) => {};
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ParsedCell<'a> {
    pub(crate) raw: &'a [u8],
    pub(crate) attr: &'a [u8],
    pub(crate) reference_attr: &'a [u8],
    pub(crate) row: &'a [u8],
    pub(crate) col_num: u32,
    pub(crate) style_id: Option<u32>,
}

impl<'a> ParsedCell<'a> {
    fn from_captures(caps: &Captures<'a>) -> Option<Self> {
        let raw = caps.get(0)?.as_bytes();
        let attr = caps.get(1)?.as_bytes();
        let reference_attr = extract_attr_assignment(attr, b"r")?;
        let column = caps.get(2)?.as_bytes();
        let row = caps.get(3)?.as_bytes();

        let col_num = letters_byte_string_to_integer(column);
        let style_id = caps
            .get(4)
            .map(|m| digits_byte_string_to_integer(m.as_bytes()));

        Some(Self {
            raw,
            attr,
            reference_attr,
            row,
            col_num,
            style_id,
        })
    }

    fn style<'p>(&self, params: &'p CleanSheetParameters) -> Option<&'p Xf> {
        self.style_id
            .and_then(|style_id| params.patterns.xfs_by_index.get(&style_id))
    }

    fn to_cell_attributes(&self) -> CellAttributes {
        self.to_cell_attributes_with_protection_only(false)
    }

    fn to_cell_attributes_with_protection_only(
        &self,
        kept_only_for_protection: bool,
    ) -> CellAttributes {
        CellAttributes {
            col: self.col_num,
            row: digits_byte_string_to_integer(self.row),
            kept_only_for_protection,
        }
    }

    fn to_cleanup_candidate(&self, cause: CleanupCause) -> CleanupCandidate {
        CleanupCandidate {
            col: self.col_num,
            row: digits_byte_string_to_integer(self.row),
            cause,
        }
    }
}

pub fn replace_cell(
    caps: &Captures,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let Some(cell) = ParsedCell::from_captures(caps) else {
        return caps.get(0).unwrap().as_bytes().to_vec();
    };

    replace_parsed_cell(&cell, params, metrics).into_owned()
}

pub(crate) fn replace_parsed_cell<'a>(
    cell: &ParsedCell<'a>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    replace_parsed_cell_with_empty_style_policy(
        cell,
        params,
        metrics,
        parsed_cell_has_content(cell),
        false,
        false,
        false,
    )
}

pub(crate) fn replace_parsed_cell_with_empty_style_policy<'a>(
    cell: &ParsedCell<'a>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    has_cell_content: bool,
    keep_no_empty_cell_visible_style_cell: bool,
    keep_other_empty_cell: bool,
    keep_default_empty_cell: bool,
) -> Cow<'a, [u8]> {
    let style = cell.style(params);
    let is_hidden = cell_is_hidden(cell, metrics);
    let unreferenced_whitespace_cleanup = unreferenced_whitespace_cleanup_enabled(params);
    let unreferenced_hidden_content_cleanup = unreferenced_hidden_content_cleanup_enabled(params);
    record_visible_cell_bounds(cell, style, metrics, has_cell_content);
    let keep_column_font_style_cell =
        parsed_empty_cell_has_column_font_style_override(cell, params, metrics, has_cell_content);

    if unreferenced_whitespace_cleanup || unreferenced_hidden_content_cleanup {
        let mut candidate_cause = None;
        let keep_style_representative = keep_no_empty_cell_visible_style_cell
            || keep_other_empty_cell
            || keep_default_empty_cell
            || keep_column_font_style_cell;

        if !keep_style_representative
            && unreferenced_whitespace_cleanup
            && is_whitespace_cell(cell.raw, params)
        {
            metrics.whitespace_candidates += 1;
            candidate_cause = Some(CleanupCause::UnreferencedWhitespaceCells);
        } else if unreferenced_hidden_content_cleanup
            && !keep_style_representative
            && is_hidden
            && has_cell_content
            && !has_formula(cell.raw)
        {
            metrics.hidden_candidates += 1;
            candidate_cause = Some(CleanupCause::UnreferencedHiddenContentCells);
        }

        if let Some(cause) = candidate_cause {
            let is_excel_protected =
                cell_has_relevant_protection_style(cell, style, params, metrics);
            if !is_excel_protected {
                metrics.candidates.push(cell.to_cleanup_candidate(cause));
            }
            record_kept_cell(
                params,
                metrics,
                cell,
                style,
                has_cell_content,
                is_excel_protected,
            );
            record_kept_content_cell_row(metrics, cell, has_cell_content);
            return Cow::Borrowed(cell.raw);
        }
    }

    if is_hidden {
        if !has_cell_content {
            if empty_hidden_cell_is_style_representative(
                cell,
                style,
                params,
                metrics,
                keep_no_empty_cell_visible_style_cell,
                keep_other_empty_cell,
                keep_default_empty_cell,
            ) {
                let out = generate_self_closing_cell_xml(cell.attr);
                let kept_only_for_protection = style.is_some_and(|style| {
                    cell_has_relevant_protection_style(cell, Some(style), params, metrics)
                        && !cell_has_effective_fill_or_border(style, cell, metrics)
                        && !cell_applies_effective_default_fill_override(style, cell, metrics)
                        && !cell_applies_effective_default_border_override(style, cell, metrics)
                        && !empty_cell_has_column_font_style_override(
                            cell,
                            style,
                            params,
                            metrics,
                            has_cell_content,
                        )
                });
                record_kept_cell(
                    params,
                    metrics,
                    cell,
                    style,
                    has_cell_content,
                    kept_only_for_protection,
                );
                return Cow::Owned(out);
            }

            if params.cleanup.empty_hidden_cells {
                metrics
                    .cleanup_stats
                    .record_removed(CleanupCause::EmptyHiddenCells, cell.raw.len());
                metrics.deleted_cell_states.push(cell.to_cell_attributes());
                return Cow::Owned(Vec::new());
            }

            record_kept_cell(params, metrics, cell, style, has_cell_content, false);
            return Cow::Borrowed(cell.raw);
        }

        record_kept_cell(params, metrics, cell, style, has_cell_content, false);
        record_kept_content_cell_row(metrics, cell, has_cell_content);
        return Cow::Borrowed(cell.raw);
    }

    let Some(style) = style else {
        if !has_cell_content {
            if keep_default_empty_cell {
                let out = generate_self_closing_cell_xml(cell.attr);
                record_kept_cell(params, metrics, cell, None, has_cell_content, false);
                return Cow::Owned(out);
            }

            return remove_or_keep_empty_cell(
                cell,
                params,
                metrics,
                CleanupCause::EmptyCellsWithoutContent,
                None,
            );
        }

        record_kept_cell(params, metrics, cell, None, has_cell_content, false);
        record_kept_content_cell_row(metrics, cell, has_cell_content);
        return Cow::Borrowed(cell.raw);
    };

    if has_cell_content {
        record_kept_cell(params, metrics, cell, Some(style), has_cell_content, false);
        record_kept_content_cell_row(metrics, cell, has_cell_content);
        return Cow::Borrowed(cell.raw);
    }

    let has_effective_fill_or_border = cell_has_effective_fill_or_border(style, cell, metrics)
        || cell_applies_effective_default_fill_override(style, cell, metrics)
        || cell_applies_effective_default_border_override(style, cell, metrics);

    if style.has_no_empty_cell_visible_style {
        if keep_no_empty_cell_visible_style_cell
            || keep_column_font_style_cell
            || has_effective_fill_or_border
        {
            let out = generate_self_closing_cell_xml(cell.attr);
            record_kept_cell(params, metrics, cell, Some(style), has_cell_content, false);
            return Cow::Owned(out);
        }

        return remove_or_keep_empty_cell(
            cell,
            params,
            metrics,
            CleanupCause::EmptyCellsWithoutContent,
            Some(style),
        );
    }

    if keep_other_empty_cell || keep_column_font_style_cell {
        let out = generate_self_closing_cell_xml(cell.attr);
        record_kept_cell(params, metrics, cell, Some(style), has_cell_content, false);
        return Cow::Owned(out);
    }

    let has_relevant_protection =
        cell_has_relevant_protection_style(cell, Some(style), params, metrics);

    if has_relevant_protection || has_effective_fill_or_border {
        let out = generate_self_closing_cell_xml(cell.attr);
        record_kept_cell(
            params,
            metrics,
            cell,
            Some(style),
            has_cell_content,
            has_relevant_protection && !has_effective_fill_or_border,
        );
        return Cow::Owned(out);
    }

    remove_or_keep_empty_cell(
        cell,
        params,
        metrics,
        CleanupCause::EmptyCellsWithoutContent,
        Some(style),
    )
}

pub(crate) fn parsed_empty_cell_has_effective_fill_or_border(
    cell: &ParsedCell<'_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    has_cell_content: bool,
) -> bool {
    if has_cell_content {
        return false;
    }

    cell.style(params).is_some_and(|style| {
        cell_has_effective_fill_or_border(style, cell, metrics)
            || cell_applies_effective_default_fill_override(style, cell, metrics)
            || cell_applies_effective_default_border_override(style, cell, metrics)
    })
}

fn record_kept_cell(
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    cell: &ParsedCell<'_>,
    cell_style: Option<&Xf>,
    has_cell_content: bool,
    kept_only_for_protection: bool,
) {
    let row = digits_byte_string_to_integer(cell.row);
    let is_tail_anchor = kept_cell_is_tail_anchor(
        params,
        metrics,
        cell,
        cell_style,
        has_cell_content,
        kept_only_for_protection,
    );

    metrics
        .kept_cell_states
        .push(cell.to_cell_attributes_with_protection_only(kept_only_for_protection));

    if is_tail_anchor {
        metrics.tail_anchor_max_row =
            Some(metrics.tail_anchor_max_row.map_or(row, |old| old.max(row)));
    }
}

fn kept_cell_is_tail_anchor(
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    cell: &ParsedCell<'_>,
    cell_style: Option<&Xf>,
    has_cell_content: bool,
    kept_only_for_protection: bool,
) -> bool {
    if has_cell_content {
        return true;
    }

    if cell_style.is_none()
        && cell.style_id.is_none()
        && params.cleanup.empty_style_representative_cells
    {
        return true;
    }

    if cell_style.is_none() && cell.style_id.is_some() {
        return true;
    }

    let column_style = metrics
        .column_definitions
        .get(cell.col_num)
        .and_then(|column| column.cell_xf.as_ref());
    if column_style.is_some_and(|style| style.has_fill_or_border) {
        return true;
    }

    let has_visible_style_anchor = cell_style.is_some_and(|style| {
        cell_has_effective_fill_or_border(style, cell, metrics)
            || cell_applies_effective_default_fill_override(style, cell, metrics)
            || cell_applies_effective_default_border_override(style, cell, metrics)
    });
    if has_visible_style_anchor {
        return true;
    }

    if kept_only_for_protection {
        return false;
    }

    if parsed_empty_cell_has_column_font_style_override(cell, params, metrics, false) {
        return false;
    }

    false
}

fn record_kept_content_cell_row(
    metrics: &mut CleanSheetMetrics,
    cell: &ParsedCell<'_>,
    has_cell_content: bool,
) {
    if has_cell_content {
        metrics
            .rows_with_kept_content_cells
            .insert(cell.row.to_vec());
    }
}

pub(crate) fn parsed_cell_has_content(cell: &ParsedCell<'_>) -> bool {
    has_content(cell.raw)
}

fn remove_or_keep_empty_cell<'a>(
    cell: &ParsedCell<'a>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    cause: CleanupCause,
    cell_style: Option<&Xf>,
) -> Cow<'a, [u8]> {
    if params.cleanup.empty_cells_without_content {
        metrics.cleanup_stats.record_removed(cause, cell.raw.len());
        metrics.deleted_cell_states.push(cell.to_cell_attributes());
        trace_cell_decision!(CellDecisionEvent {
            sheet_file: &params.zip_info_filename,
            col: cell.col_num,
            row: cell.row,
            style_id: cell.style_id,
            decision: CellDecision::Removed,
            cause,
        });
        Cow::Owned(Vec::new())
    } else {
        record_kept_cell(params, metrics, cell, cell_style, false, false);
        trace_cell_decision!(CellDecisionEvent {
            sheet_file: &params.zip_info_filename,
            col: cell.col_num,
            row: cell.row,
            style_id: cell.style_id,
            decision: CellDecision::Kept,
            cause,
        });
        Cow::Borrowed(cell.raw)
    }
}

fn extract_attr_assignment<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let mut search_start = 0;

    while search_start < data.len() {
        let relative = memchr::memmem::find(&data[search_start..], attr_name)?;
        let attr_start = search_start + relative;
        let value_marker = attr_start + attr_name.len();

        if attr_start > 0
            && data[attr_start - 1].is_ascii_whitespace()
            && data.get(value_marker..value_marker + 2) == Some(b"=\"")
        {
            let value_start = value_marker + 2;
            let value_end = data[value_start..].iter().position(|byte| *byte == b'"')?;
            return Some(&data[attr_start..value_start + value_end + 1]);
        }

        search_start = attr_start + attr_name.len();
    }

    None
}

fn empty_hidden_cell_is_style_representative(
    cell: &ParsedCell<'_>,
    cell_style: Option<&Xf>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    keep_no_empty_cell_visible_style_cell: bool,
    keep_other_empty_cell: bool,
    keep_default_empty_cell: bool,
) -> bool {
    if keep_default_empty_cell && cell.style_id.is_none() {
        return true;
    }

    let Some(style) = cell_style else {
        return false;
    };

    if keep_other_empty_cell {
        return true;
    }

    if keep_no_empty_cell_visible_style_cell && style.has_no_empty_cell_visible_style {
        return true;
    }

    cell_has_relevant_protection_style(cell, Some(style), params, metrics)
        || cell_has_effective_fill_or_border(style, cell, metrics)
        || cell_applies_effective_default_fill_override(style, cell, metrics)
        || cell_applies_effective_default_border_override(style, cell, metrics)
}

fn unreferenced_whitespace_cleanup_enabled(params: &CleanSheetParameters) -> bool {
    params.cleanup.unreferenced_whitespace_cells
        || params
            .advanced_cleanup
            .as_ref()
            .is_some_and(|advanced| advanced.remove_unreferenced_whitespace)
}

fn unreferenced_hidden_content_cleanup_enabled(params: &CleanSheetParameters) -> bool {
    params.cleanup.unreferenced_hidden_content_cells
        || params
            .advanced_cleanup
            .as_ref()
            .is_some_and(|advanced| advanced.remove_unreferenced_hidden_content)
}

fn cell_is_hidden(cell: &ParsedCell<'_>, metrics: &CleanSheetMetrics) -> bool {
    if metrics
        .row_attributes
        .get(cell.row)
        .is_some_and(|row| row.hidden)
    {
        return true;
    }

    metrics
        .column_definitions
        .get(cell.col_num)
        .is_some_and(|column| column.hidden)
}

fn cell_has_effective_fill_or_border(
    cell_style: &Xf,
    cell: &ParsedCell<'_>,
    metrics: &CleanSheetMetrics,
) -> bool {
    let column_style = metrics
        .column_definitions
        .get(cell.col_num)
        .and_then(|column| column.cell_xf.as_ref());

    let row_style = metrics
        .row_attributes
        .get(cell.row)
        .and_then(|row| row.style.as_ref());

    cell_has_effective_fill(cell_style, row_style, column_style)
        || cell_has_effective_border(cell_style, row_style, column_style)
}

fn record_visible_cell_bounds(
    cell: &ParsedCell<'_>,
    cell_style: Option<&Xf>,
    metrics: &mut CleanSheetMetrics,
    has_cell_content: bool,
) {
    let Ok(row_str) = std::str::from_utf8(cell.row) else {
        return;
    };
    let Ok(row_num) = row_str.parse::<u32>() else {
        return;
    };

    let column_style = metrics
        .column_definitions
        .get(cell.col_num)
        .and_then(|column| column.cell_xf.as_ref());

    let is_visible = has_cell_content
        || cell_style.is_some_and(|style| cell_has_effective_fill_or_border(style, cell, metrics))
        || column_style.is_some_and(|style| style.has_fill_or_border);

    if is_visible {
        metrics
            .visible_content_bounds
            .include_cell(cell.col_num, row_num);
    }
}

fn cell_has_effective_fill(
    cell_style: &Xf,
    row_style: Option<&Xf>,
    column_style: Option<&Xf>,
) -> bool {
    let cell_fill_id = cell_style.fill_id.unwrap_or(0);

    match (
        row_style.filter(|style| style.apply_fill || style.has_visible_fill),
        column_style.filter(|style| style.apply_fill),
    ) {
        (Some(row), Some(column)) => {
            cell_fill_id != row.fill_id.unwrap_or(0) || cell_fill_id != column.fill_id.unwrap_or(0)
        }
        (Some(row), None) => cell_fill_id != row.fill_id.unwrap_or(0),
        (None, Some(column)) => cell_fill_id != column.fill_id.unwrap_or(0),
        (None, None) => cell_style.has_visible_fill,
    }
}

fn cell_has_effective_border(
    cell_style: &Xf,
    row_style: Option<&Xf>,
    column_style: Option<&Xf>,
) -> bool {
    let cell_border_id = cell_style.border_id.unwrap_or(0);

    match (
        row_style.filter(|style| style.apply_border),
        column_style.filter(|style| style.apply_border),
    ) {
        (Some(row), Some(column)) => {
            cell_border_id != row.border_id.unwrap_or(0)
                || cell_border_id != column.border_id.unwrap_or(0)
        }
        (Some(row), None) => cell_border_id != row.border_id.unwrap_or(0),
        (None, Some(column)) => cell_border_id != column.border_id.unwrap_or(0),
        (None, None) => cell_style.has_visible_border,
    }
}

fn empty_cell_has_column_font_style_override(
    cell: &ParsedCell<'_>,
    cell_style: &Xf,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    has_cell_content: bool,
) -> bool {
    if has_cell_content || !params.cleanup.column_font_style_representative_cells {
        return false;
    }

    let cell_font_id = cell_style.font_id.unwrap_or(0);
    let row_attributes = metrics.row_attributes.get(cell.row);
    let row_uses_auto_height =
        row_attributes.is_some_and(|row| row.has_height && !row.has_custom_height);

    if cell_font_id == 0 && !cell_style.apply_font && !row_uses_auto_height {
        return false;
    }

    let Some(column_style) = metrics
        .column_definitions
        .get(cell.col_num)
        .and_then(|column| column.cell_xf.as_ref())
    else {
        return false;
    };

    let column_font_id = column_style.font_id.unwrap_or(0);

    if row_uses_auto_height
        && row_attributes
            .and_then(|row| row.style.as_ref())
            .is_some_and(|row_style| {
                cell_style.font_id.unwrap_or(column_font_id) != row_style.font_id.unwrap_or(0)
            })
    {
        return true;
    }

    cell_font_id != column_font_id
}

fn parsed_empty_cell_has_column_font_style_override(
    cell: &ParsedCell<'_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    has_cell_content: bool,
) -> bool {
    cell.style(params).is_some_and(|style| {
        empty_cell_has_column_font_style_override(cell, style, params, metrics, has_cell_content)
    })
}

fn cell_applies_effective_default_fill_override(
    cell_style: &Xf,
    cell: &ParsedCell<'_>,
    metrics: &CleanSheetMetrics,
) -> bool {
    if !cell_style.applies_default_fill_override() {
        return false;
    }

    let column_style = metrics
        .column_definitions
        .get(cell.col_num)
        .and_then(|column| column.cell_xf.as_ref());

    let row_style = metrics
        .row_attributes
        .get(cell.row)
        .and_then(|row| row.style.as_ref());

    match (row_style, column_style) {
        (None, None) => true,
        (row_style, column_style) => {
            row_style.is_some_and(|style| style.apply_fill && style.has_visible_fill)
                || column_style.is_some_and(|style| style.apply_fill && style.has_visible_fill)
        }
    }
}

fn cell_applies_effective_default_border_override(
    cell_style: &Xf,
    cell: &ParsedCell<'_>,
    metrics: &CleanSheetMetrics,
) -> bool {
    if !cell_style.applies_default_border_override() {
        return false;
    }

    let column_style = metrics
        .column_definitions
        .get(cell.col_num)
        .and_then(|column| column.cell_xf.as_ref());

    let row_style = metrics
        .row_attributes
        .get(cell.row)
        .and_then(|row| row.style.as_ref());

    match (row_style, column_style) {
        (None, None) => true,
        (row_style, column_style) => {
            row_style.is_some_and(|style| style.apply_border && style.has_visible_border)
                || column_style.is_some_and(|style| style.apply_border && style.has_visible_border)
        }
    }
}

fn generate_self_closing_cell_xml(cell_attr: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cell_attr.len() + 4);
    out.extend_from_slice(b"<c");
    out.extend_from_slice(cell_attr);
    out.extend_from_slice(b"/>");
    out
}

fn has_content(cell: &[u8]) -> bool {
    crate::xml::find_bytes(cell, b"<v>").is_some()
        || has_formula(cell)
        || crate::xml::find_bytes(cell, b"<is>").is_some()
}

fn has_formula(cell: &[u8]) -> bool {
    has_child_tag(cell, b"f")
}

fn has_child_tag(cell: &[u8], tag_name: &[u8]) -> bool {
    let mut needle = Vec::with_capacity(tag_name.len() + 1);
    needle.push(b'<');
    needle.extend_from_slice(tag_name);

    let mut pos = 0;
    while let Some(rel) = crate::xml::find_bytes(&cell[pos..], &needle) {
        let start = pos + rel;
        let next = cell.get(start + needle.len()).copied();
        if matches!(next, Some(b' ' | b'/' | b'>')) {
            return true;
        }
        pos = start + needle.len();
    }

    false
}

fn is_whitespace_cell(cell: &[u8], params: &CleanSheetParameters) -> bool {
    let Some(indices) = &params.patterns.whitespace_indices else {
        return false;
    };

    // Check if it's a shared string
    if !cell.windows(b"t=\"s\"".len()).any(|w| w == b"t=\"s\"") {
        return false;
    }

    let Some(v_start_rel) = crate::xml::find_bytes(cell, b"<v>") else {
        return false;
    };
    let value_start = v_start_rel + b"<v>".len();

    let Some(v_end_rel) = crate::xml::find_bytes(&cell[value_start..], b"</v>") else {
        return false;
    };

    let value = &cell[value_start..value_start + v_end_rel];

    if let Ok(value_str) = std::str::from_utf8(value) {
        if let Ok(index) = value_str.parse::<usize>() {
            return indices.contains(&index);
        }
    }

    false
}

fn cell_has_protection_style(
    cell: &ParsedCell<'_>,
    cell_style: Option<&Xf>,
    metrics: &CleanSheetMetrics,
) -> bool {
    if cell_style.is_some_and(|style| style.has_apply_protection) {
        return true;
    }

    if metrics
        .row_attributes
        .get(cell.row)
        .and_then(|row| row.style.as_ref())
        .is_some_and(|style| style.has_apply_protection)
    {
        return true;
    }

    metrics
        .column_definitions
        .get(cell.col_num)
        .and_then(|column| column.cell_xf.as_ref())
        .is_some_and(|style| style.has_apply_protection)
}

fn cell_has_relevant_protection_style(
    cell: &ParsedCell<'_>,
    cell_style: Option<&Xf>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
) -> bool {
    if params
        .cleanup
        .remove_apply_protection_without_sheet_protection
        && !metrics.sheet_protection_enabled
    {
        return false;
    }

    if !cell_has_protection_style(cell, cell_style, metrics) {
        return false;
    }

    true
}

#[cfg(test)]
#[path = "cells_regression_tests.rs"]
mod regression_tests;
