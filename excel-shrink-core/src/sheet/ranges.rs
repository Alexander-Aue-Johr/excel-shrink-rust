use crate::models::{
    CleanSheetMetrics, CleanSheetParameters, ColumnRangeMap, VisibleContentBounds, Xf,
};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use crate::xml::{find_bytes, rfind_bytes};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleSheetDataRanges<'a> {
    pub cells_range: &'a [u8],
    pub rows_range: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowEndPosition {
    pub row: u32,
    pub end: usize,
}

pub fn retrieve_sheet_data_range_containing_cells<'a>(
    sheet_data: &'a [u8],
    params: &CleanSheetParameters,
    sheet_protection_enabled: bool,
    preserve_through_row: Option<u32>,
) -> VisibleSheetDataRanges<'a> {
    let last_row_with_cell_end_pos = find_last_row_with_cell_end_pos(sheet_data);

    let last_visible_attribute_position =
        find_last_fill_or_border_or_protection_style_or_hidden_attribute(
            sheet_data,
            params,
            last_row_with_cell_end_pos,
            sheet_protection_enabled,
        );

    let last_visible_row_end =
        find_last_visible_row_end_position(sheet_data, last_visible_attribute_position);
    let mut last_visible_row_end_position = last_visible_row_end.map(|row_end| row_end.end);
    if let Some(preserve_row) = preserve_through_row {
        let needs_preserve_scan = last_visible_row_end
            .map(|row_end| preserve_row > row_end.row)
            .unwrap_or(true);
        if needs_preserve_scan {
            let search_start = last_visible_row_end
                .map(|row_end| row_end.end)
                .or(last_row_with_cell_end_pos)
                .unwrap_or(0);
            if let Some(preserved_end) =
                find_last_row_end_at_or_before_from(sheet_data, search_start, preserve_row)
            {
                last_visible_row_end_position = Some(
                    last_visible_row_end_position
                        .map_or(preserved_end.end, |old| old.max(preserved_end.end)),
                );
            }
        }
    }

    let cells_range = match last_row_with_cell_end_pos {
        Some(pos) => &sheet_data[..pos],
        None => &[],
    };

    let rows_range = match last_visible_row_end_position {
        Some(end) => {
            let start = last_row_with_cell_end_pos.unwrap_or(0);
            &sheet_data[start..end]
        }
        None => &[],
    };

    VisibleSheetDataRanges {
        cells_range,
        rows_range,
    }
}

fn find_last_row_end_at_or_before_from(
    sheet_data: &[u8],
    start: usize,
    max_row: u32,
) -> Option<RowEndPosition> {
    find_row_end_for_reference_from(sheet_data, start, max_row)
        .or_else(|| scan_last_row_end_at_or_before_from(sheet_data, start, max_row))
}

fn find_row_end_for_reference_from(
    sheet_data: &[u8],
    start: usize,
    row: u32,
) -> Option<RowEndPosition> {
    let needle = format!(r#" r="{row}""#);
    let needle = needle.as_bytes();
    let mut search_start = start.min(sheet_data.len());

    while let Some(relative) = find_bytes(&sheet_data[search_start..], needle) {
        let attr_start = search_start + relative;
        if let Some(row_end) = row_end_containing_row_reference(sheet_data, attr_start, row) {
            return Some(row_end);
        }

        search_start = (attr_start + needle.len()).min(sheet_data.len());
    }

    None
}

fn row_end_containing_row_reference(
    sheet_data: &[u8],
    attr_start: usize,
    expected_row: u32,
) -> Option<RowEndPosition> {
    let row_start = rfind_bytes(&sheet_data[..attr_start], b"<row")?;
    let row_open_end = find_tag_open_end(sheet_data, row_start)?;
    if attr_start >= row_open_end {
        return None;
    }

    let row_tag = &sheet_data[row_start..row_open_end];
    let row = extract_attr_value(row_tag, b"r").map(digits_byte_string_to_integer)?;
    if row != expected_row {
        return None;
    }

    let end = find_row_end(sheet_data, row_open_end)?;
    Some(RowEndPosition { row, end })
}

fn scan_last_row_end_at_or_before_from(
    sheet_data: &[u8],
    start: usize,
    max_row: u32,
) -> Option<RowEndPosition> {
    let mut pos = start.min(sheet_data.len());
    let mut last_end = None;

    while let Some(row_start_relative) = find_bytes(&sheet_data[pos..], b"<row") {
        let row_start = pos + row_start_relative;
        let Some(row_open_end) = find_tag_open_end(sheet_data, row_start) else {
            break;
        };
        let row_tag = &sheet_data[row_start..row_open_end];
        let Some(row_number) = extract_attr_value(row_tag, b"r").map(digits_byte_string_to_integer)
        else {
            pos = row_open_end;
            continue;
        };

        if row_number > max_row {
            break;
        }

        let Some(row_end) = find_row_end(sheet_data, row_open_end) else {
            break;
        };
        last_end = Some(RowEndPosition {
            row: row_number,
            end: row_end,
        });
        pos = row_end;
    }

    last_end
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
    _sheet_protection_enabled: bool,
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
            .is_some_and(|xf| xf.has_fill_or_border)
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

    scan_visible_rows_and_cells(sheet_data, params, column_definitions, &mut bounds);

    bounds
}

fn scan_visible_rows_and_cells(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    column_definitions: &ColumnRangeMap,
    bounds: &mut VisibleContentBounds,
) {
    let mut pos = 0;

    while let Some(row_start_relative) = find_bytes(&sheet_data[pos..], b"<row") {
        let row_start = pos + row_start_relative;
        let Some(row_open_end) = find_tag_open_end(sheet_data, row_start) else {
            break;
        };
        let Some(row_end) = find_row_end(sheet_data, row_open_end) else {
            break;
        };

        let row_tag = &sheet_data[row_start..row_open_end];
        let row_num = extract_attr_value(row_tag, b"r").map(digits_byte_string_to_integer);
        let row_style = extract_attr_value(row_tag, b"s")
            .map(digits_byte_string_to_integer)
            .and_then(|id| params.patterns.xfs_by_index.get(&id));

        if row_style.is_some_and(style_has_visible_fill_or_border) {
            if let Some(row_num) = row_num {
                bounds.include_row(row_num);
            }
        }

        if row_open_end < row_end {
            let row_content_end = row_end.saturating_sub(b"</row>".len());
            if row_content_end > row_open_end {
                scan_visible_cells(
                    &sheet_data[row_open_end..row_content_end],
                    params,
                    column_definitions,
                    bounds,
                );
            }
        }

        pos = row_end;
    }
}

fn scan_visible_cells(
    row_content: &[u8],
    params: &CleanSheetParameters,
    column_definitions: &ColumnRangeMap,
    bounds: &mut VisibleContentBounds,
) {
    let mut pos = 0;

    while let Some(cell_start_relative) = find_bytes(&row_content[pos..], b"<c") {
        let cell_start = pos + cell_start_relative;
        let Some(cell_open_end) = find_tag_open_end(row_content, cell_start) else {
            break;
        };
        let Some(cell_end) = find_cell_end(row_content, cell_open_end) else {
            break;
        };

        let cell = &row_content[cell_start..cell_end];
        let cell_tag = &row_content[cell_start..cell_open_end];

        let Some((col, row)) = extract_cell_coordinate(cell_tag) else {
            pos = cell_end;
            continue;
        };

        let cell_style = extract_attr_value(cell_tag, b"s")
            .map(digits_byte_string_to_integer)
            .and_then(|id| params.patterns.xfs_by_index.get(&id));
        let column_style = column_definitions
            .get(col)
            .and_then(|column| column.cell_xf.as_ref());

        if cell_has_visible_content(cell)
            || cell_style.is_some_and(style_has_visible_fill_or_border)
            || column_style.is_some_and(style_has_visible_fill_or_border)
        {
            bounds.include_cell(col, row);
        }

        pos = cell_end;
    }
}

fn find_tag_open_end(data: &[u8], tag_start: usize) -> Option<usize> {
    crate::xml::find_byte_from(data, b'>', tag_start).map(|idx| idx + 1)
}

fn find_row_end(data: &[u8], row_open_end: usize) -> Option<usize> {
    if row_open_end >= 2 && data[row_open_end - 2] == b'/' {
        return Some(row_open_end);
    }

    let closing_relative = find_bytes(&data[row_open_end..], b"</row>")?;
    Some(row_open_end + closing_relative + b"</row>".len())
}

fn find_cell_end(data: &[u8], cell_open_end: usize) -> Option<usize> {
    if cell_open_end >= 2 && data[cell_open_end - 2] == b'/' {
        return Some(cell_open_end);
    }

    let closing_relative = find_bytes(&data[cell_open_end..], b"</c>")?;
    Some(cell_open_end + closing_relative + b"</c>".len())
}

fn extract_cell_coordinate(cell_tag: &[u8]) -> Option<(u32, u32)> {
    let reference = extract_attr_value(cell_tag, b"r")?;
    let split = reference
        .iter()
        .position(|byte| byte.is_ascii_digit())
        .filter(|split| *split > 0 && *split < reference.len())?;

    let col = letters_byte_string_to_integer(&reference[..split]);
    let row = digits_byte_string_to_integer(&reference[split..]);

    Some((col, row))
}

fn extract_attr_value<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let mut needle = Vec::with_capacity(attr_name.len() + 2);
    needle.extend_from_slice(attr_name);
    needle.extend_from_slice(b"=\"");

    let mut search_start = 0;

    while search_start < data.len() {
        let Some(pos_relative) = find_bytes(&data[search_start..], &needle) else {
            return None;
        };
        let pos = search_start + pos_relative;

        if pos > 0 && !data[pos - 1].is_ascii_whitespace() {
            search_start = pos + 1;
            continue;
        }

        let value_start = pos + needle.len();
        let value_end_relative = data[value_start..].iter().position(|byte| *byte == b'"')?;

        return Some(&data[value_start..value_start + value_end_relative]);
    }

    None
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
) -> Option<RowEndPosition> {
    let pos = last_visible_attribute_position?;
    if let Some(row_end) = row_end_at_or_before_position(sheet_data, pos) {
        return Some(row_end);
    }

    let row_start = find_from(sheet_data, b"<row", pos)?;
    row_end_from_start(sheet_data, row_start)
}

fn row_end_at_or_before_position(sheet_data: &[u8], pos: usize) -> Option<RowEndPosition> {
    let row_start = rfind_bytes(&sheet_data[..pos.min(sheet_data.len())], b"<row")?;
    let row_end = row_end_from_start(sheet_data, row_start)?;
    (pos <= row_end.end).then_some(row_end)
}

fn row_end_from_start(sheet_data: &[u8], row_start: usize) -> Option<RowEndPosition> {
    let row_open_end = find_tag_open_end(sheet_data, row_start)?;
    let row_tag = &sheet_data[row_start..row_open_end];
    let row = extract_attr_value(row_tag, b"r").map(digits_byte_string_to_integer)?;
    let end = find_row_end(sheet_data, row_open_end)?;

    Some(RowEndPosition { row, end })
}

fn find_from(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    crate::xml::find_bytes(&haystack[start..], needle).map(|idx| start + idx)
}

pub fn get_max_cell_column(metrics: &CleanSheetMetrics) -> Option<u32> {
    metrics
        .kept_cell_states
        .iter()
        .filter(|cell| !cell.kept_only_for_protection)
        .map(|cell| cell.col)
        .max()
}

pub fn get_max_cell_row(metrics: &CleanSheetMetrics) -> Option<u32> {
    metrics
        .kept_cell_states
        .iter()
        .filter(|cell| !cell.kept_only_for_protection)
        .map(|cell| cell.row)
        .max()
}

pub fn get_max_merge_cells_column_index(metrics: &CleanSheetMetrics) -> Option<u32> {
    metrics
        .max_merge_cells_column_and_row
        .as_ref()
        .map(|coord| crate::utils::letters_byte_string_to_integer(&coord.column))
}

pub fn get_max_merge_cells_row_index(metrics: &CleanSheetMetrics) -> Option<u32> {
    metrics
        .max_merge_cells_column_and_row
        .as_ref()
        .map(|coord| crate::utils::digits_byte_string_to_integer(&coord.row))
}

pub fn get_max_visible_column_index(metrics: &CleanSheetMetrics) -> Option<u32> {
    metrics
        .column_definitions
        .get_last_visible_range(false)
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
