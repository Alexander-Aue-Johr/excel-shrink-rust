use crate::models::{
    CellAttributes, CellCoordinate, CleanSheetMetrics, CleanSheetParameters, CleanupCandidate,
    CleanupCause, Xf,
};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use regex::bytes::Captures;

#[derive(Debug, Clone, Copy)]
struct ParsedCell<'a> {
    raw: &'a [u8],
    attr: &'a [u8],
    column: &'a [u8],
    row: &'a [u8],
    col_num: u32,
    style_id: Option<u32>,
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
            column,
            row,
            col_num,
            style_id,
        })
    }

    fn style<'p>(&self, params: &'p CleanSheetParameters) -> Option<&'p Xf> {
        self.style_id
            .and_then(|style_id| params.patterns.xfs_by_index.get(&style_id))
    }

    fn to_cell_attributes(&self, style: Option<&Xf>) -> CellAttributes {
        CellAttributes {
            cell_attribute_string: self.attr.to_vec(),
            cell_coordinate: CellCoordinate {
                column: self.column.to_vec(),
                row: self.row.to_vec(),
            },
            style: style.cloned(),
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

fn replace_parsed_cell(
    cell: &ParsedCell<'_>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let style = cell.style(params);
    let is_hidden = cell_is_hidden(cell, metrics);
    let is_excel_protected = cell_has_relevant_protection_style(cell, style, metrics);

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
        {
            metrics.hidden_candidates += 1;
            candidate_cause = Some(CleanupCause::UnreferencedHiddenContentCells);
        }

        if let Some(cause) = candidate_cause {
            let attrs = cell.to_cell_attributes(style);
            if !is_excel_protected {
                metrics.candidates.push(CleanupCandidate {
                    coordinate: attrs.cell_coordinate.clone(),
                    cause,
                });
            }
            metrics.kept_cell_states.push(attrs);
            return cell.raw.to_vec();
        }
    }

    if is_hidden {
        if !has_content(cell.raw) {
            if params.cleanup.empty_hidden_cells {
                metrics
                    .cleanup_stats
                    .record_removed(CleanupCause::EmptyHiddenCells, cell.raw.len());
                metrics
                    .deleted_cell_states
                    .push(cell.to_cell_attributes(style));
                return Vec::new();
            }

            metrics
                .kept_cell_states
                .push(cell.to_cell_attributes(style));
            return cell.raw.to_vec();
        }

        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes(style));
        return cell.raw.to_vec();
    }

    let Some(style) = style else {
        if !has_content(cell.raw) {
            return remove_or_keep_empty_cell(
                cell,
                None,
                params,
                metrics,
                CleanupCause::EmptyCellsWithoutContent,
            );
        }

        metrics.kept_cell_states.push(cell.to_cell_attributes(None));
        return cell.raw.to_vec();
    };

    if has_content(cell.raw) {
        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes(Some(style)));
        return cell.raw.to_vec();
    }

    let has_relevant_protection = cell_has_relevant_protection_style(cell, Some(style), metrics);
    let has_effective_fill_or_border = cell_has_effective_fill_or_border(style, cell, metrics);

    if has_relevant_protection || has_effective_fill_or_border {
        let out = generate_self_closing_cell_xml(cell.attr);
        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes(Some(style)));
        return out;
    }

    remove_or_keep_empty_cell(
        cell,
        Some(style),
        params,
        metrics,
        CleanupCause::EmptyCellsWithoutContent,
    )
}

fn remove_or_keep_empty_cell(
    cell: &ParsedCell<'_>,
    style: Option<&Xf>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    cause: CleanupCause,
) -> Vec<u8> {
    if params.cleanup.empty_cells_without_content {
        metrics.cleanup_stats.record_removed(cause, cell.raw.len());
        metrics
            .deleted_cell_states
            .push(cell.to_cell_attributes(style));
        Vec::new()
    } else {
        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes(style));
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
        || crate::xml::find_bytes(cell, b"<f>").is_some()
        || crate::xml::find_bytes(cell, b"<is>").is_some()
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

    if metrics.sheet_protection_enabled {
        return true;
    }

    let Ok(row_str) = std::str::from_utf8(cell.row) else {
        return true;
    };

    let Ok(row_num) = row_str.parse::<u32>() else {
        return true;
    };

    !metrics
        .visible_content_bounds
        .cell_is_outside(cell.col_num, row_num)
}

#[cfg(test)]
#[path = "cells_regression_tests.rs"]
mod regression_tests;
