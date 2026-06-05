use crate::models::{
    CellAttributes, CellCoordinate, CleanSheetMetrics, CleanSheetParameters, Xf,
};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use regex::bytes::{Captures, Regex};
use std::sync::OnceLock;

static CELL_REF_RE: OnceLock<Regex> = OnceLock::new();
static STYLE_RE: OnceLock<Regex> = OnceLock::new();

pub fn replace_cell(
    caps: &Captures,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let Some(cell_attributes) = extract_cell_attributes(caps, params) else {
        return caps.get(0).unwrap().as_bytes().to_vec();
    };

    let coord = &cell_attributes.cell_coordinate;
    let style = cell_attributes.style.clone();

    if cell_is_hidden(coord, metrics) {
        metrics.deleted_cell_states.push(cell_attributes);
        return Vec::new();
    }

    let Some(style) = style else {
        metrics.deleted_cell_states.push(cell_attributes);
        return Vec::new();
    };

    if style.has_fill_or_border || style.has_apply_protection {
        let out = generate_self_closing_cell_xml(&cell_attributes.cell_attribute_string);
        metrics.kept_cell_states.push(cell_attributes);
        return out;
    }

    if !style.has_apply_fill_or_border {
        metrics.deleted_cell_states.push(cell_attributes);
        return Vec::new();
    }

    if cell_overrides_fill_or_border(&style, coord, metrics) {
        let out = generate_self_closing_cell_xml(&cell_attributes.cell_attribute_string);
        metrics.kept_cell_states.push(cell_attributes);
        return out;
    }

    metrics.deleted_cell_states.push(cell_attributes);
    Vec::new()
}

fn extract_cell_attributes(
    caps: &Captures,
    params: &CleanSheetParameters,
) -> Option<CellAttributes> {
    let cell_attr = caps.get(1)?.as_bytes().to_vec();

    let cell_ref_re =
        CELL_REF_RE.get_or_init(|| Regex::new(r#"r="([A-Z]+)(\d+)""#).unwrap());
    let style_re = STYLE_RE.get_or_init(|| Regex::new(r#"s="(\d+?)""#).unwrap());

    let ref_caps = cell_ref_re.captures(&cell_attr)?;
    let col_ref = ref_caps.get(1)?.as_bytes().to_vec();
    let row_ref = ref_caps.get(2)?.as_bytes().to_vec();

    let style = style_re
        .captures(&cell_attr)
        .and_then(|c| c.get(1))
        .map(|m| digits_byte_string_to_integer(m.as_bytes()))
        .and_then(|style_id| params.patterns.xfs_by_index.get(&style_id).cloned());

    Some(CellAttributes {
        cell_attribute_string: cell_attr,
        cell_coordinate: CellCoordinate {
            column: col_ref,
            row: row_ref,
        },
        style,
    })
}

fn cell_is_hidden(cell_coordinate: &CellCoordinate, metrics: &CleanSheetMetrics) -> bool {
    if metrics
        .row_attributes
        .get(&cell_coordinate.row)
        .is_some_and(|row| row.hidden)
    {
        return true;
    }

    let column_index = letters_byte_string_to_integer(&cell_coordinate.column);

    metrics
        .column_definitions
        .get(column_index)
        .is_some_and(|column| column.hidden)
}

fn cell_overrides_fill_or_border(
    cell_style: &Xf,
    cell_coordinate: &CellCoordinate,
    metrics: &CleanSheetMetrics,
) -> bool {
    let column_index = letters_byte_string_to_integer(&cell_coordinate.column);

    let column_style = metrics
        .column_definitions
        .get(column_index)
        .and_then(|column| column.cell_xf.as_ref());

    let row_style = metrics
        .row_attributes
        .get(&cell_coordinate.row)
        .and_then(|row| row.style.as_ref());

    let overrides_fill =
        row_style.is_some_and(|row| row.apply_fill && cell_style.fill_id != row.fill_id)
            || column_style
                .is_some_and(|column| column.apply_fill && cell_style.fill_id != column.fill_id);

    let overrides_border =
        row_style.is_some_and(|row| row.apply_border && cell_style.border_id != row.border_id)
            || column_style.is_some_and(|column| {
                column.apply_border && cell_style.border_id != column.border_id
            });

    overrides_fill || overrides_border
}

fn generate_self_closing_cell_xml(cell_attr: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cell_attr.len() + 4);
    out.extend_from_slice(b"<c");
    out.extend_from_slice(cell_attr);
    out.extend_from_slice(b"/>");
    out
}