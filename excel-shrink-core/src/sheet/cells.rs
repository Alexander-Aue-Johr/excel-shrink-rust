use crate::models::{CellAttributes, CellCoordinate, CleanSheetMetrics, CleanSheetParameters, Xf};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use regex::bytes::Captures;

#[derive(Debug, Clone, Copy)]
struct ParsedCell<'a> {
    raw: &'a [u8],
    attr: &'a [u8],
    column: &'a [u8],
    row: &'a [u8],
    col_num: u32,
    row_num: u32,
    style_id: Option<u32>,
}

impl<'a> ParsedCell<'a> {
    fn from_captures(caps: &Captures<'a>) -> Option<Self> {
        let raw = caps.get(0)?.as_bytes();
        let attr = caps.get(1)?.as_bytes();
        let column = caps.get(2)?.as_bytes();
        let row = caps.get(3)?.as_bytes();

        let col_num = letters_byte_string_to_integer(column);
        let row_num = digits_byte_string_to_integer(row);

        let style_id = caps
            .get(4)
            .map(|m| digits_byte_string_to_integer(m.as_bytes()));

        Some(Self {
            raw,
            attr,
            column,
            row,
            col_num,
            row_num,
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
    let is_excel_protected = cell_has_protection_style(cell, style, metrics);

    let mut force_delete = false;

    if let Some(adv) = &params.advanced_cleanup {
        if adv.safe_to_clean {
            let is_deletion_protected =
                adv.protected_cells.contains(cell.col_num, cell.row_num) || is_excel_protected;

            if adv.remove_unreferenced_whitespace && is_whitespace_cell(cell.raw, params) {
                metrics.whitespace_candidates += 1;

                if !is_deletion_protected {
                    metrics.whitespace_removed += 1;
                    force_delete = true;
                } else {
                    metrics.whitespace_protected_by_reference += 1;
                }
            }

            if adv.remove_unreferenced_hidden_content && is_hidden && has_content(cell.raw) {
                metrics.hidden_candidates += 1;

                if !is_deletion_protected {
                    metrics.hidden_removed += 1;
                    force_delete = true;
                } else {
                    metrics.hidden_protected_by_reference += 1;
                }
            }
        } else {
            if adv.remove_unreferenced_whitespace && is_whitespace_cell(cell.raw, params) {
                metrics.whitespace_candidates += 1;
            }

            if adv.remove_unreferenced_hidden_content && is_hidden && has_content(cell.raw) {
                metrics.hidden_candidates += 1;
            }
        }
    }

    if force_delete {
        metrics
            .deleted_cell_states
            .push(cell.to_cell_attributes(style));
        return Vec::new();
    }

    if is_hidden {
        if !has_content(cell.raw) {
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

    let Some(style) = style else {
        if !has_content(cell.raw) {
            metrics
                .deleted_cell_states
                .push(cell.to_cell_attributes(None));
            return Vec::new();
        }

        metrics.kept_cell_states.push(cell.to_cell_attributes(None));
        return cell.raw.to_vec();
    };

    if style.has_fill_or_border || style.has_apply_protection {
        if has_content(cell.raw) {
            metrics
                .kept_cell_states
                .push(cell.to_cell_attributes(Some(style)));
            return cell.raw.to_vec();
        }

        let out = generate_self_closing_cell_xml(cell.attr);
        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes(Some(style)));
        return out;
    }

    if cell_overrides_fill_or_border(style, cell, metrics) {
        if has_content(cell.raw) {
            metrics
                .kept_cell_states
                .push(cell.to_cell_attributes(Some(style)));
            return cell.raw.to_vec();
        }

        let out = generate_self_closing_cell_xml(cell.attr);
        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes(Some(style)));
        return out;
    }

    if !style.has_apply_fill_or_border {
        if !has_content(cell.raw) {
            metrics
                .deleted_cell_states
                .push(cell.to_cell_attributes(Some(style)));
            return Vec::new();
        }

        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes(Some(style)));
        return cell.raw.to_vec();
    }

    if !has_content(cell.raw) {
        metrics
            .deleted_cell_states
            .push(cell.to_cell_attributes(Some(style)));
        return Vec::new();
    }

    metrics
        .kept_cell_states
        .push(cell.to_cell_attributes(Some(style)));
    cell.raw.to_vec()
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

fn cell_overrides_fill_or_border(
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

    let overrides_fill = overrides_inherited_non_default_fill(cell_style, row_style)
        || overrides_inherited_non_default_fill(cell_style, column_style);

    // Border-Logik erst einmal nah an deiner bisherigen Semantik lassen.
    // Der konkrete Bug betrifft Fill; hier weiterhin nur dann als Override zählen,
    // wenn Row/Column applyBorder explizit setzen.
    let overrides_border = row_style
        .is_some_and(|row| row.apply_border && cell_style.border_id != row.border_id)
        || column_style
            .is_some_and(|column| column.apply_border && cell_style.border_id != column.border_id);

    overrides_fill || overrides_border
}

fn overrides_inherited_non_default_fill(cell_style: &Xf, inherited_style: Option<&Xf>) -> bool {
    let Some(inherited_style) = inherited_style else {
        return false;
    };

    let inherited_fill_id = inherited_style.fill_id.unwrap_or(0);
    let cell_fill_id = cell_style.fill_id.unwrap_or(0);

    inherited_fill_id != 0 && cell_fill_id != inherited_fill_id
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

#[cfg(test)]
#[path = "cells_regression_tests.rs"]
mod regression_tests;
