use crate::models::{CellAttributes, CellCoordinate, CleanSheetMetrics, CleanSheetParameters, Xf};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use regex::bytes::Captures;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct CellView<'a> {
    pub(crate) raw: &'a [u8],
    pub(crate) opening_tag: &'a [u8],
    pub(crate) attributes: &'a [u8],
    pub(crate) reference: Option<&'a [u8]>,
    pub(crate) column_ref: Option<&'a [u8]>,
    pub(crate) row_ref: Option<&'a [u8]>,
    pub(crate) column: Option<u32>,
    pub(crate) row: Option<u32>,
    pub(crate) style_id: Option<u32>,
    pub(crate) is_self_closing: bool,
    pub(crate) has_content: bool,
}

pub(crate) enum CellReplacement<'a> {
    KeepOriginal,
    Delete,
    Replace(Vec<u8>),
    #[allow(dead_code)]
    Borrowed(&'a [u8]),
}

impl<'a> CellView<'a> {
    fn from_captures(caps: &Captures<'a>) -> Option<Self> {
        let raw = caps.get(0)?.as_bytes();
        let attributes = caps.get(1)?.as_bytes();
        let column = caps.get(2)?.as_bytes();
        let row = caps.get(3)?.as_bytes();
        let reference = caps.get(2).map(|m| m.as_bytes());

        let opening_tag = raw
            .iter()
            .position(|byte| *byte == b'>')
            .map(|idx| &raw[..=idx])
            .unwrap_or(raw);
        let style_id = caps
            .get(4)
            .map(|m| digits_byte_string_to_integer(m.as_bytes()));

        Some(Self {
            raw,
            opening_tag,
            attributes,
            reference,
            column_ref: Some(column),
            row_ref: Some(row),
            column: Some(letters_byte_string_to_integer(column)),
            row: Some(digits_byte_string_to_integer(row)),
            style_id,
            is_self_closing: raw.ends_with(b"/>"),
            has_content: has_content(raw),
        })
    }

    fn style<'p>(&self, params: &'p CleanSheetParameters) -> Option<&'p Xf> {
        self.style_id
            .and_then(|style_id| params.patterns.xfs_by_index.get(&style_id))
    }

    fn to_cell_attributes(&self, style: Option<&Xf>) -> CellAttributes {
        CellAttributes {
            cell_attribute_string: self.attributes.to_vec(),
            cell_coordinate: CellCoordinate {
                column: self.column_ref.unwrap_or_default().to_vec(),
                row: self.row_ref.unwrap_or_default().to_vec(),
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
    let Some(cell) = CellView::from_captures(caps) else {
        return caps.get(0).unwrap().as_bytes().to_vec();
    };

    cell_replacement_to_vec(replace_cell_view(&cell, params, metrics), cell.raw)
}

pub(crate) fn replace_cell_view<'a>(
    cell: &CellView<'a>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> CellReplacement<'a> {
    if cell.reference.is_none() || cell.column.is_none() || cell.row.is_none() {
        return CellReplacement::KeepOriginal;
    }

    let style = cell.style(params);
    let is_hidden = cell_is_hidden(cell, metrics);
    let is_excel_protected = cell_has_protection_style(cell, style, metrics);

    if let Some(adv) = &params.advanced_cleanup {
        let mut is_candidate = false;

        if adv.remove_unreferenced_whitespace && is_whitespace_cell(cell.raw, params) {
            metrics.whitespace_candidates += 1;
            is_candidate = true;
        } else if adv.remove_unreferenced_hidden_content && is_hidden && cell.has_content {
            metrics.hidden_candidates += 1;
            is_candidate = true;
        }

        if is_candidate && !is_excel_protected {
            metrics
                .candidates
                .push(cell.to_cell_attributes(style).cell_coordinate);
            metrics
                .kept_cell_states
                .push(cell.to_cell_attributes(style));
            return CellReplacement::KeepOriginal;
        }
    }

    if is_hidden {
        if !cell.has_content {
            metrics
                .deleted_cell_states
                .push(cell.to_cell_attributes(style));
            return CellReplacement::Delete;
        }

        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes(style));
        return CellReplacement::KeepOriginal;
    }

    let Some(style) = style else {
        if !cell.has_content {
            metrics
                .deleted_cell_states
                .push(cell.to_cell_attributes(None));
            return CellReplacement::Delete;
        }

        metrics.kept_cell_states.push(cell.to_cell_attributes(None));
        return CellReplacement::KeepOriginal;
    };

    if style.has_fill_or_border || style.has_apply_protection {
        if cell.has_content {
            metrics
                .kept_cell_states
                .push(cell.to_cell_attributes(Some(style)));
            return CellReplacement::KeepOriginal;
        }

        let out = generate_self_closing_cell_xml(cell.attributes);
        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes(Some(style)));
        return CellReplacement::Replace(out);
    }

    if cell_overrides_fill_or_border(style, cell, metrics) {
        if cell.has_content {
            metrics
                .kept_cell_states
                .push(cell.to_cell_attributes(Some(style)));
            return CellReplacement::KeepOriginal;
        }

        let out = generate_self_closing_cell_xml(cell.attributes);
        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes(Some(style)));
        return CellReplacement::Replace(out);
    }

    if !style.has_apply_fill_or_border {
        if !cell.has_content {
            metrics
                .deleted_cell_states
                .push(cell.to_cell_attributes(Some(style)));
            return CellReplacement::Delete;
        }

        metrics
            .kept_cell_states
            .push(cell.to_cell_attributes(Some(style)));
        return CellReplacement::KeepOriginal;
    }

    if !cell.has_content {
        metrics
            .deleted_cell_states
            .push(cell.to_cell_attributes(Some(style)));
        return CellReplacement::Delete;
    }

    metrics
        .kept_cell_states
        .push(cell.to_cell_attributes(Some(style)));
    CellReplacement::KeepOriginal
}

fn cell_replacement_to_vec(replacement: CellReplacement<'_>, original: &[u8]) -> Vec<u8> {
    match replacement {
        CellReplacement::KeepOriginal => original.to_vec(),
        CellReplacement::Delete => Vec::new(),
        CellReplacement::Replace(out) => out,
        CellReplacement::Borrowed(out) => out.to_vec(),
    }
}

#[cfg(test)]
fn replace_parsed_cell(
    cell: &CellView<'_>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    cell_replacement_to_vec(replace_cell_view(cell, params, metrics), cell.raw)
}

fn cell_is_hidden(cell: &CellView<'_>, metrics: &CleanSheetMetrics) -> bool {
    let Some(row) = cell.row_ref else {
        return false;
    };
    let Some(column) = cell.column else {
        return false;
    };

    if metrics
        .row_attributes
        .get(row)
        .is_some_and(|row| row.hidden)
    {
        return true;
    }

    metrics
        .column_definitions
        .get(column)
        .is_some_and(|column| column.hidden)
}

fn cell_overrides_fill_or_border(
    cell_style: &Xf,
    cell: &CellView<'_>,
    metrics: &CleanSheetMetrics,
) -> bool {
    let Some(column) = cell.column else {
        return false;
    };
    let Some(row) = cell.row_ref else {
        return false;
    };

    let column_style = metrics
        .column_definitions
        .get(column)
        .and_then(|column| column.cell_xf.as_ref());

    let row_style = metrics
        .row_attributes
        .get(row)
        .and_then(|row| row.style.as_ref());

    let overrides_fill = overrides_inherited_non_default_fill(cell_style, row_style)
        || overrides_inherited_non_default_fill(cell_style, column_style);

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
    cell: &CellView<'_>,
    cell_style: Option<&Xf>,
    metrics: &CleanSheetMetrics,
) -> bool {
    if cell_style.is_some_and(|style| style.has_apply_protection) {
        return true;
    }

    let Some(row) = cell.row_ref else {
        return false;
    };
    let Some(column) = cell.column else {
        return false;
    };

    if metrics
        .row_attributes
        .get(row)
        .and_then(|row| row.style.as_ref())
        .is_some_and(|style| style.has_apply_protection)
    {
        return true;
    }

    metrics
        .column_definitions
        .get(column)
        .and_then(|column| column.cell_xf.as_ref())
        .is_some_and(|style| style.has_apply_protection)
}

#[cfg(test)]
#[path = "cells_regression_tests.rs"]
mod regression_tests;
