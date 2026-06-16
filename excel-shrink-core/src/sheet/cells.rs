use crate::models::{
    CellAttributes, CleanSheetMetrics, CleanSheetParameters, CleanupCandidate, CleanupCause, Xf,
};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use regex::bytes::Captures;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ParsedCell<'a> {
    pub(crate) raw: &'a [u8],
    pub(crate) attr: &'a [u8],
    pub(crate) row: &'a [u8],
    pub(crate) col_num: u32,
    pub(crate) style_id: Option<u32>,
}

impl<'a> ParsedCell<'a> {
    fn from_captures(caps: &Captures<'a>) -> Option<Self> {
        let raw = caps.get(0)?.as_bytes();
        let attr = caps.get(1)?.as_bytes();
        let column = caps.get(2)?.as_bytes();
        let row = caps.get(3)?.as_bytes();

        let col_num = letters_byte_string_to_integer(column);
        let style_id = caps
            .get(4)
            .map(|m| digits_byte_string_to_integer(m.as_bytes()));

        Some(Self {
            raw,
            attr,
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

    replace_parsed_cell(&cell, params, metrics)
}

pub(crate) fn replace_parsed_cell(
    cell: &ParsedCell<'_>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    replace_parsed_cell_with_font_policy(cell, params, metrics, false, false, false)
}

pub(crate) fn replace_parsed_cell_with_font_policy(
    cell: &ParsedCell<'_>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    keep_font_only_empty_cell: bool,
    keep_other_empty_cell: bool,
    keep_default_empty_cell: bool,
) -> Vec<u8> {
    let style = cell.style(params);
    let is_hidden = cell_is_hidden(cell, metrics);
    let is_excel_protected = cell_has_relevant_protection_style(cell, style, metrics);
    record_visible_cell_bounds(cell, style, metrics);

    if unreferenced_whitespace_cleanup_enabled(params)
        || unreferenced_hidden_content_cleanup_enabled(params)
    {
        let mut candidate_cause = None;

        if unreferenced_whitespace_cleanup_enabled(params) && is_whitespace_cell(cell.raw, params) {
            metrics.whitespace_candidates += 1;
            candidate_cause = Some(CleanupCause::UnreferencedWhitespaceCells);
        } else if unreferenced_hidden_content_cleanup_enabled(params)
            && is_hidden
            && has_content(cell.raw)
            && !has_formula(cell.raw)
        {
            metrics.hidden_candidates += 1;
            candidate_cause = Some(CleanupCause::UnreferencedHiddenContentCells);
        }

        if let Some(cause) = candidate_cause {
            if !is_excel_protected {
                metrics.candidates.push(cell.to_cleanup_candidate(cause));
            }
            metrics
                .kept_cell_states
                .push(cell.to_cell_attributes_with_protection_only(is_excel_protected));
            return cell.raw.to_vec();
        }
    }

    if is_hidden {
        if !has_content(cell.raw) {
            if params.cleanup.empty_hidden_cells {
                metrics
                    .cleanup_stats
                    .record_removed(CleanupCause::EmptyHiddenCells, cell.raw.len());
                metrics.deleted_cell_states.push(cell.to_cell_attributes());
                return Vec::new();
            }

            metrics.kept_cell_states.push(cell.to_cell_attributes());
            return cell.raw.to_vec();
        }

        metrics.kept_cell_states.push(cell.to_cell_attributes());
        return cell.raw.to_vec();
    }

    let Some(style) = style else {
        if !has_content(cell.raw) {
            if keep_default_empty_cell {
                let out = generate_self_closing_cell_xml(cell.attr);
                metrics.kept_cell_states.push(cell.to_cell_attributes());
                return out;
            }

            return remove_or_keep_empty_cell(
                cell,
                params,
                metrics,
                CleanupCause::EmptyCellsWithoutContent,
            );
        }

        metrics.kept_cell_states.push(cell.to_cell_attributes());
        return cell.raw.to_vec();
    };

    if has_content(cell.raw) {
        metrics.kept_cell_states.push(cell.to_cell_attributes());
        return cell.raw.to_vec();
    }

    if style.has_only_font_style {
        if keep_font_only_empty_cell {
            let out = generate_self_closing_cell_xml(cell.attr);
            metrics.kept_cell_states.push(cell.to_cell_attributes());
            return out;
        }

        return remove_or_keep_empty_cell(
            cell,
            params,
            metrics,
            CleanupCause::EmptyCellsWithoutContent,
        );
    }

    if keep_other_empty_cell {
        let out = generate_self_closing_cell_xml(cell.attr);
        metrics.kept_cell_states.push(cell.to_cell_attributes());
        return out;
    }

    let has_relevant_protection = cell_has_relevant_protection_style(cell, Some(style), metrics);
    let has_effective_fill_or_border = cell_has_effective_fill_or_border(style, cell, metrics);

    if has_relevant_protection || has_effective_fill_or_border {
        let out = generate_self_closing_cell_xml(cell.attr);
        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes_with_protection_only(
                has_relevant_protection && !has_effective_fill_or_border,
            ));
        return out;
    }

    remove_or_keep_empty_cell(
        cell,
        params,
        metrics,
        CleanupCause::EmptyCellsWithoutContent,
    )
}

fn remove_or_keep_empty_cell(
    cell: &ParsedCell<'_>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    cause: CleanupCause,
) -> Vec<u8> {
    if params.cleanup.empty_cells_without_content {
        metrics.cleanup_stats.record_removed(cause, cell.raw.len());
        metrics.deleted_cell_states.push(cell.to_cell_attributes());
        Vec::new()
    } else {
        metrics.kept_cell_states.push(cell.to_cell_attributes());
        cell.raw.to_vec()
    }
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

    let is_visible = has_content(cell.raw)
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
        row_style.filter(|style| style.apply_fill),
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
    metrics: &CleanSheetMetrics,
) -> bool {
    if !cell_has_protection_style(cell, cell_style, metrics) {
        return false;
    }

    true
}

#[cfg(test)]
#[path = "cells_regression_tests.rs"]
mod regression_tests;
