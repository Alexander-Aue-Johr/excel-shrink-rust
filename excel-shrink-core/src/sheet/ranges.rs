use crate::models::{
    CleanSheetMetrics, CleanSheetParameters, ColumnRangeMap, VisibleContentBounds, Xf,
};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use crate::xml::{find_bytes, rfind_bytes};
use regex::bytes::Regex;
use std::sync::OnceLock;

static ROW_RE: OnceLock<Regex> = OnceLock::new();
static CELL_RE: OnceLock<Regex> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleSheetDataRanges {
    pub cells_range: Vec<u8>,
    pub rows_range: Vec<u8>,
}

pub fn retrieve_sheet_data_range_containing_cells(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    sheet_protection_enabled: bool,
) -> VisibleSheetDataRanges {
    let last_row_with_cell_end_pos = find_last_row_with_cell_end_pos(sheet_data);

    let last_visible_attribute_position =
        find_last_fill_or_border_or_protection_style_or_hidden_attribute(
            sheet_data,
            params,
            last_row_with_cell_end_pos,
            sheet_protection_enabled,
        );

    let last_visible_row_end_position =
        find_last_visible_row_end_position(sheet_data, last_visible_attribute_position);

    let cells_range = match last_row_with_cell_end_pos {
        Some(pos) => sheet_data[..pos].to_vec(),
        None => Vec::new(),
    };

    let rows_range = match last_visible_row_end_position {
        Some(end) => {
            let start = last_row_with_cell_end_pos.unwrap_or(0);
            sheet_data[start..end].to_vec()
        }
        None => Vec::new(),
    };

    VisibleSheetDataRanges {
        cells_range,
        rows_range,
    }
}

pub fn find_last_row_with_cell_end_pos(sheet_data: &[u8]) -> Option<usize> {
    let last_cell_pos = rfind_bytes(sheet_data, b"<c ")?;
    let row_end_relative = find_bytes(&sheet_data[last_cell_pos..], b"</row>")?;
    Some(last_cell_pos + row_end_relative + b"</row>".len())
}

pub fn find_last_fill_or_border_or_protection_style_or_hidden_attribute(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    last_row_with_cell_end_pos: Option<usize>,
    sheet_protection_enabled: bool,
) -> Option<usize> {
    let start = last_row_with_cell_end_pos.unwrap_or(0);
    let mut last_pos = last_row_with_cell_end_pos;

    if start >= sheet_data.len() {
        return last_pos;
    }

    if let Some(relative) = rfind_bytes(&sheet_data[start..], b"hidden=\"1\"") {
        let absolute = start + relative;
        last_pos = Some(last_pos.map_or(absolute, |old| old.max(absolute)));
    }

    if let Some(style_pos) =
        rfind_relevant_style_attribute(sheet_data, params, start, sheet_protection_enabled)
    {
        last_pos = Some(last_pos.map_or(style_pos, |old| old.max(style_pos)));
    }

    last_pos
}

fn rfind_relevant_style_attribute(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    start: usize,
    sheet_protection_enabled: bool,
) -> Option<usize> {
    let mut end = sheet_data.len();

    while end > start {
        let relative = rfind_bytes(&sheet_data[start..end], b"s=\"")?;
        let absolute = start + relative;

        if absolute == 0 || !sheet_data[absolute - 1].is_ascii_whitespace() {
            end = absolute;
            continue;
        }

        let value_start = absolute + b"s=\"".len();

        let Some(value_end_relative) = sheet_data[value_start..].iter().position(|b| *b == b'"')
        else {
            end = absolute;
            continue;
        };

        let value = &sheet_data[value_start..value_start + value_end_relative];
        let style_id = digits_byte_string_to_integer(value);

        if params
            .patterns
            .xfs_by_index
            .get(&style_id)
            .is_some_and(|xf| {
                xf.has_fill_or_border || (sheet_protection_enabled && xf.has_apply_protection)
            })
        {
            return Some(absolute);
        }

        end = absolute;
    }

    None
}

pub fn find_visible_content_bounds(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    column_definitions: &ColumnRangeMap,
) -> VisibleContentBounds {
    let mut bounds = VisibleContentBounds::default();

    if let Some(column) = column_definitions.get_last_visible_range(false) {
        bounds.include_col(column.max);
    }

    let row_re = ROW_RE.get_or_init(|| {
        Regex::new(
            r#"(?s)<row\b((?:[^>]*\br="([^"]+?)")?(?:[^>]*\bs="([^"]+?)")?[^/>]*?)(?:/>|>(.*?)</row>)"#,
        )
        .unwrap()
    });

    let cell_re = CELL_RE.get_or_init(|| {
        Regex::new(
            r#"(?s)<c\b((?:[^>]*\br="([A-Z]+)(\d+)")?(?:[^>]*\bs="(\d+?)")?[^>]*?)(?:/>|>.*?</c>)"#,
        )
        .unwrap()
    });

    for row_caps in row_re.captures_iter(sheet_data) {
        let row_num = row_caps
            .get(2)
            .map(|m| digits_byte_string_to_integer(m.as_bytes()));

        let row_style = row_caps
            .get(3)
            .map(|m| digits_byte_string_to_integer(m.as_bytes()))
            .and_then(|id| params.patterns.xfs_by_index.get(&id));

        if row_style.is_some_and(style_has_visible_fill_or_border) {
            if let Some(row_num) = row_num {
                bounds.include_row(row_num);
            }
        }

        let row_content = row_caps.get(4).map_or(b"".as_slice(), |m| m.as_bytes());

        for cell_caps in cell_re.captures_iter(row_content) {
            let Some(column) = cell_caps.get(2).map(|m| m.as_bytes()) else {
                continue;
            };
            let Some(row) = cell_caps
                .get(3)
                .map(|m| digits_byte_string_to_integer(m.as_bytes()))
            else {
                continue;
            };

            let col = letters_byte_string_to_integer(column);
            let raw = cell_caps.get(0).unwrap().as_bytes();
            let cell_style = cell_caps
                .get(4)
                .map(|m| digits_byte_string_to_integer(m.as_bytes()))
                .and_then(|id| params.patterns.xfs_by_index.get(&id));
            let column_style = column_definitions
                .get(col)
                .and_then(|column| column.cell_xf.as_ref());

            if cell_has_visible_content(raw)
                || cell_style.is_some_and(style_has_visible_fill_or_border)
                || column_style.is_some_and(style_has_visible_fill_or_border)
            {
                bounds.include_cell(col, row);
            }
        }
    }

    bounds
}

fn cell_has_visible_content(cell: &[u8]) -> bool {
    find_bytes(cell, b"<v").is_some()
        || find_bytes(cell, b"<f").is_some()
        || find_bytes(cell, b"<is").is_some()
}

fn style_has_visible_fill_or_border(style: &Xf) -> bool {
    style.has_fill_or_border
}

pub fn find_last_visible_row_end_position(
    sheet_data: &[u8],
    last_visible_attribute_position: Option<usize>,
) -> Option<usize> {
    let pos = last_visible_attribute_position?;

    let row_end_tag_pos = find_from(sheet_data, b"</row>", pos);
    let self_closing_tag_pos = find_from(sheet_data, b"/>", pos);

    match (row_end_tag_pos, self_closing_tag_pos) {
        (Some(row_end), Some(self_closing)) => {
            if row_end < self_closing {
                Some(row_end + b"</row>".len())
            } else {
                Some(self_closing + b"/>".len())
            }
        }
        (Some(row_end), None) => Some(row_end + b"</row>".len()),
        (None, Some(self_closing)) => Some(self_closing + b"/>".len()),
        (None, None) => None,
    }
}

fn find_from(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    crate::xml::find_bytes(&haystack[start..], needle).map(|idx| start + idx)
}

pub fn get_max_cell_column(metrics: &CleanSheetMetrics) -> Option<u32> {
    let max_col = metrics
        .kept_cell_states
        .iter()
        .map(|cell| cell.cell_coordinate.column.as_slice())
        .max_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)))?;

    Some(crate::utils::letters_byte_string_to_integer(max_col))
}

pub fn get_max_merge_cells_column_index(metrics: &CleanSheetMetrics) -> Option<u32> {
    metrics
        .max_merge_cells_column_and_row
        .as_ref()
        .map(|coord| crate::utils::letters_byte_string_to_integer(&coord.column))
}

pub fn get_max_visible_column_index(metrics: &CleanSheetMetrics) -> Option<u32> {
    metrics
        .column_definitions
        .get_last_visible_range(metrics.sheet_protection_enabled)
        .map(|cd| cd.max)
}

pub fn get_max_column(metrics: &CleanSheetMetrics) -> Option<u32> {
    [
        get_max_cell_column(metrics),
        get_max_merge_cells_column_index(metrics),
        get_max_visible_column_index(metrics),
    ]
    .into_iter()
    .flatten()
    .max()
}

#[cfg(test)]
#[path = "ranges_regression_tests.rs"]
mod regression_tests;
