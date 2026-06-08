use crate::models::{CleanSheetParameters, CleanSheetMetrics};
use crate::utils::digits_byte_string_to_integer;
use crate::xml::{find_bytes, rfind_bytes};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleSheetDataRanges {
    pub cells_range: Vec<u8>,
    pub rows_range: Vec<u8>,
}

pub fn retrieve_sheet_data_range_containing_cells(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
) -> VisibleSheetDataRanges {
    let last_row_with_cell_end_pos = find_last_row_with_cell_end_pos(sheet_data);

    let last_visible_attribute_position =
        find_last_fill_or_border_or_protection_style_or_hidden_attribute(
            sheet_data,
            params,
            last_row_with_cell_end_pos,
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
) -> Option<usize> {
    let start = last_row_with_cell_end_pos.unwrap_or(0);
    let mut last_pos = last_row_with_cell_end_pos;

    if start >= sheet_data.len() {
        return last_pos;
    }

    // hidden="1" nur einmal suchen.
    // Nach dem Preprocessing sollten davon nur noch sehr wenige übrig sein.
    if let Some(relative) = rfind_bytes(&sheet_data[start..], b"hidden=\"1\"") {
        let absolute = start + relative;
        last_pos = Some(last_pos.map_or(absolute, |old| old.max(absolute)));
    }

    // Styles rückwärts suchen, bis der erste relevante Style gefunden wurde.
    if let Some(style_pos) = rfind_relevant_style_attribute(sheet_data, params, start) {
        last_pos = Some(last_pos.map_or(style_pos, |old| old.max(style_pos)));
    }

    last_pos
}

fn rfind_relevant_style_attribute(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    start: usize,
) -> Option<usize> {
    let mut end = sheet_data.len();

    while end > start {
        let relative = rfind_bytes(&sheet_data[start..end], b"s=\"")?;
        let absolute = start + relative;

        // Wichtig: Nicht "spans" treffen.
        // Ein echtes s="..."-Attribut muss vor dem s durch Whitespace getrennt sein.
        if absolute == 0 || !sheet_data[absolute - 1].is_ascii_whitespace() {
            end = absolute;
            continue;
        }

        let value_start = absolute + b"s=\"".len();

        let Some(value_end_relative) = sheet_data[value_start..]
            .iter()
            .position(|b| *b == b'"')
        else {
            end = absolute;
            continue;
        };

        let value = &sheet_data[value_start..value_start + value_end_relative];
        let style_id = digits_byte_string_to_integer(value);

        if params
            .patterns
            .fill_or_border_or_protection_ids
            .contains(&style_id)
        {
            return Some(absolute);
        }

        end = absolute;
    }

    None
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
        .get_last_visible_range()
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