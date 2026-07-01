use crate::models::{CleanSheetParameters, ColumnDefinition, ColumnRangeMap, Xf};
use crate::utils::digits_byte_string_to_integer;
use crate::xml::{find_byte_from, find_bytes};

pub fn process_cols_node(col_data: &[u8], params: &CleanSheetParameters) -> ColumnRangeMap {
    let mut map = ColumnRangeMap::default();

    for column in parse_column_tags(col_data) {
        let style = column
            .style_id
            .and_then(|id| params.patterns.xfs_by_index.get(&id).cloned());

        map.add(ColumnDefinition {
            min: column.min,
            max: column.max,
            cell_xf: style,
            hidden: column.hidden,
        });
    }

    map.finalize();
    map
}

pub fn merge_extraneous_columns(cols_content: &[u8], max_column: u32) -> Vec<u8> {
    let mut removable_columns = Vec::new();

    for column in parse_column_tags(cols_content) {
        // Analyzer compatibility:
        // find_excess_columns(...) returns no removable columns as soon as any
        // column definition is hidden.
        if column.hidden {
            return cols_content.to_vec();
        }

        // Analyzer compatibility:
        // only complete <col> definitions whose min is beyond max_column are
        // collapsed. Do not split a column definition where min <= max_column
        // but max > max_column.
        if column.min > max_column {
            removable_columns.push(RemovableColumnTag {
                start: column.start,
                end: column.end,
                tag: column.tag,
                min_value: column.min,
            });
        }
    }

    if removable_columns.is_empty() {
        return cols_content.to_vec();
    }

    let first = removable_columns.first().unwrap();
    let last = removable_columns.last().unwrap();

    let replacement_min = first.min_value.to_string();
    let Some(collapsed_last_tag) = replace_attr_value(last.tag, b"min", replacement_min.as_bytes())
    else {
        return cols_content.to_vec();
    };

    let mut out = Vec::with_capacity(cols_content.len());
    let mut cursor = 0;

    for removable in &removable_columns[..removable_columns.len().saturating_sub(1)] {
        out.extend_from_slice(&cols_content[cursor..removable.start]);
        cursor = removable.end;
    }

    out.extend_from_slice(&cols_content[cursor..last.start]);
    out.extend_from_slice(&collapsed_last_tag);
    out.extend_from_slice(&cols_content[last.end..]);

    out
}

pub fn merge_extraneous_columns_with_styles(
    cols_content: &[u8],
    max_column: u32,
    params: &CleanSheetParameters,
    collapse_equal_fill_tail: bool,
) -> Vec<u8> {
    let columns = parse_column_tags(cols_content);

    if columns.iter().any(|column| column.hidden) {
        return cols_content.to_vec();
    }

    collapse_visible_equal_fill_column_tail(
        cols_content,
        &columns,
        max_column,
        params,
        collapse_equal_fill_tail,
    )
    .or_else(|| collapse_default_column_tail(cols_content, &columns, max_column, params))
    .unwrap_or_else(|| cols_content.to_vec())
}

fn parse_column_tags(cols_content: &[u8]) -> Vec<ParsedColumnTag<'_>> {
    let mut columns = Vec::new();
    let mut pos = 0;

    while let Some(tag_match) = find_next_col_tag(cols_content, pos) {
        let tag = tag_match.tag;
        let min = extract_attr_value(tag, b"min").map(digits_byte_string_to_integer);
        let max = extract_attr_value(tag, b"max").map(digits_byte_string_to_integer);
        let style_id = extract_attr_value(tag, b"style").map(digits_byte_string_to_integer);
        let hidden = extract_attr_value(tag, b"hidden").is_some_and(|value| value == b"1");

        let (Some(min), Some(max)) = (min, max) else {
            pos = tag_match.end;
            continue;
        };

        if min == 0 || max == 0 {
            pos = tag_match.end;
            continue;
        }

        columns.push(ParsedColumnTag {
            start: tag_match.start,
            end: tag_match.end,
            tag,
            min,
            max,
            hidden,
            style_id,
        });

        pos = tag_match.end;
    }

    columns
}

fn find_next_col_tag<'a>(data: &'a [u8], start: usize) -> Option<ColumnTagMatch<'a>> {
    let mut pos = start;

    while pos < data.len() {
        let tag_start_relative = find_bytes(&data[pos..], b"<col")?;
        let tag_start = pos + tag_start_relative;
        let after_name = tag_start + b"<col".len();

        if !data
            .get(after_name)
            .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'/' || *byte == b'>')
        {
            pos = after_name;
            continue;
        }

        let tag_end = find_byte_from(data, b'>', tag_start)? + 1;

        return Some(ColumnTagMatch {
            start: tag_start,
            end: tag_end,
            tag: &data[tag_start..tag_end],
        });
    }

    None
}

fn extract_attr_value<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let range = attr_value_range(data, attr_name)?;
    Some(&data[range])
}

fn replace_attr_value(tag: &[u8], attr_name: &[u8], new_value: &[u8]) -> Option<Vec<u8>> {
    let range = attr_value_range(tag, attr_name)?;
    let mut out = Vec::with_capacity(tag.len() - (range.end - range.start) + new_value.len());

    out.extend_from_slice(&tag[..range.start]);
    out.extend_from_slice(new_value);
    out.extend_from_slice(&tag[range.end..]);

    Some(out)
}

fn attr_value_range(data: &[u8], attr_name: &[u8]) -> Option<std::ops::Range<usize>> {
    let mut search_start = 0;

    while search_start < data.len() {
        let attr_start_relative = find_bytes(&data[search_start..], attr_name)?;
        let attr_start = search_start + attr_start_relative;
        let after_name = attr_start + attr_name.len();

        let before_ok = attr_start > 0 && data[attr_start - 1].is_ascii_whitespace();
        let assignment_ok =
            after_name + 1 < data.len() && data[after_name] == b'=' && data[after_name + 1] == b'"';

        if before_ok && assignment_ok {
            let value_start = after_name + 2;
            let value_end_relative = data[value_start..].iter().position(|byte| *byte == b'"')?;
            let value_end = value_start + value_end_relative;

            return Some(value_start..value_end);
        }

        search_start = after_name;
    }

    None
}

fn collapse_visible_equal_fill_column_tail(
    cols_content: &[u8],
    columns: &[ParsedColumnTag<'_>],
    max_column: u32,
    params: &CleanSheetParameters,
    collapse_equal_fill_tail: bool,
) -> Option<Vec<u8>> {
    let last = columns.last()?;
    if last.min <= max_column {
        return None;
    }

    let target_style = column_tail_style(last, params)?;
    if !target_style.has_visible_fill || !collapse_equal_fill_tail {
        return None;
    }

    let mut first_tail_index = columns.len() - 1;

    while first_tail_index > 0 {
        let previous = &columns[first_tail_index - 1];

        if previous.min <= max_column {
            break;
        }

        let Some(previous_style) = column_tail_style(previous, params) else {
            break;
        };

        if previous_style.fill_key != target_style.fill_key {
            break;
        }

        first_tail_index -= 1;
    }

    if first_tail_index == columns.len() - 1 {
        return None;
    }

    let first_tail = &columns[first_tail_index];
    let replacement_max = last.max.to_string();
    let collapsed_first_tag =
        replace_attr_value(first_tail.tag, b"max", replacement_max.as_bytes())?;

    let mut out = Vec::with_capacity(
        cols_content.len() - (last.end - first_tail.start) + collapsed_first_tag.len(),
    );
    out.extend_from_slice(&cols_content[..first_tail.start]);
    out.extend_from_slice(&collapsed_first_tag);
    out.extend_from_slice(&cols_content[last.end..]);

    Some(out)
}

fn collapse_default_column_tail(
    cols_content: &[u8],
    columns: &[ParsedColumnTag<'_>],
    max_column: u32,
    params: &CleanSheetParameters,
) -> Option<Vec<u8>> {
    let last = columns.last()?;
    if last.min <= max_column {
        return None;
    }

    let target_style = column_tail_style(last, params)?;
    if target_style.has_visible_fill {
        return None;
    }

    let mut first_tail_index = columns.len() - 1;

    while first_tail_index > 0 {
        let previous = &columns[first_tail_index - 1];

        if previous.min <= max_column {
            break;
        }

        let Some(previous_style) = column_tail_style(previous, params) else {
            break;
        };

        if previous_style.has_visible_fill {
            break;
        }

        first_tail_index -= 1;
    }

    if first_tail_index == columns.len() - 1 {
        return None;
    }

    let first_tail = &columns[first_tail_index];
    let replacement_min = first_tail.min.to_string();
    let collapsed_last_tag = replace_attr_value(last.tag, b"min", replacement_min.as_bytes())?;

    let mut out = Vec::with_capacity(
        cols_content.len() - (last.end - first_tail.start) + collapsed_last_tag.len(),
    );
    out.extend_from_slice(&cols_content[..first_tail.start]);
    out.extend_from_slice(&collapsed_last_tag);
    out.extend_from_slice(&cols_content[last.end..]);

    Some(out)
}

fn column_tail_style(
    column: &ParsedColumnTag<'_>,
    params: &CleanSheetParameters,
) -> Option<ColumnTailStyle> {
    let style = column
        .style_id
        .and_then(|style_id| params.patterns.xfs_by_index.get(&style_id));

    if style.is_some_and(column_has_visible_border) {
        return None;
    }

    Some(ColumnTailStyle {
        fill_key: column_fill_key(style),
        has_visible_fill: style.is_some_and(|xf| xf.has_visible_fill),
    })
}

fn column_has_visible_border(style: &Xf) -> bool {
    style.has_visible_border
}

fn column_fill_key(style: Option<&Xf>) -> ColumnFillKey {
    match style {
        Some(style) if style.has_visible_fill => ColumnFillKey::Fill(style.fill_id.unwrap_or(0)),
        _ => ColumnFillKey::Default,
    }
}

#[derive(Debug, Clone, Copy)]
struct ParsedColumnTag<'a> {
    start: usize,
    end: usize,
    tag: &'a [u8],
    min: u32,
    max: u32,
    hidden: bool,
    style_id: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
struct ColumnTagMatch<'a> {
    start: usize,
    end: usize,
    tag: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ColumnTailStyle {
    fill_key: ColumnFillKey,
    has_visible_fill: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnFillKey {
    Default,
    Fill(u32),
}

#[derive(Debug, Clone, Copy)]
struct RemovableColumnTag<'a> {
    start: usize,
    end: usize,
    tag: &'a [u8],
    min_value: u32,
}

#[cfg(test)]
mod tests {
    use super::{merge_extraneous_columns, merge_extraneous_columns_with_styles};
    use crate::models::{
        CleanSheetParameters, SheetPreprocessConfig, WorkbookSpecificPatterns, Xf,
    };
    use std::collections::HashMap;
    use zip::CompressionMethod;

    const COLS: &[u8] = br#"<col min="1" max="12" width="8.43"/><col min="13" max="17" width="10.0"/><col min="18" max="25" width="10.0"/>"#;

    fn default_params() -> CleanSheetParameters {
        CleanSheetParameters {
            xlsx_file_path: "test.xlsx".to_string(),
            zip_info_filename: "xl/worksheets/sheet1.xml".to_string(),
            compress_method: CompressionMethod::Deflated,
            patterns: WorkbookSpecificPatterns {
                xfs_by_index: HashMap::new(),
                no_empty_cell_visible_style_ids: Vec::new(),
                fill_or_border_or_protection_ids: Vec::new(),
                whitespace_indices: None,
            },
            advanced_cleanup: None,
            preprocess: SheetPreprocessConfig::default(),
            cleanup: Default::default(),
            collect_cleanup_stats: false,
        }
    }

    fn params_with_styles(styles: &[(u32, Xf)]) -> CleanSheetParameters {
        let mut params = default_params();

        for (style_id, style) in styles {
            params
                .patterns
                .xfs_by_index
                .insert(*style_id, style.clone());
        }

        params
    }

    fn visible_fill_style(fill_id: u32) -> Xf {
        Xf::builder()
            .fill_id(Some(fill_id))
            .border_id(Some(0))
            .apply_fill(true)
            .has_visible_fill(true)
            .has_fill_or_border(true)
            .has_apply_fill_or_border(true)
            .build_xf()
    }

    fn visible_fill_and_border_style(fill_id: u32, border_id: u32) -> Xf {
        Xf::builder()
            .fill_id(Some(fill_id))
            .border_id(Some(border_id))
            .apply_fill(true)
            .apply_border(true)
            .has_visible_fill(true)
            .has_visible_border(true)
            .has_fill_or_border(true)
            .has_apply_fill_or_border(true)
            .build_xf()
    }

    #[test]
    fn preserves_columns_when_no_col_min_is_beyond_max_column() {
        let merged = merge_extraneous_columns(COLS, 25);

        assert_eq!(merged, COLS);
    }

    #[test]
    fn does_not_split_column_range_that_crosses_max_column_boundary() {
        let merged = merge_extraneous_columns(COLS, 14);

        // Analyzer compatibility:
        // <col min="13" max="17".../> is not removed because min <= max_column.
        // <col min="18" max="25".../> is the only removable column, so changing
        // its min to its own min is a no-op.
        assert_eq!(merged, COLS);
    }

    #[test]
    fn collapses_all_columns_with_min_beyond_max_column_into_last_column() {
        let cols = br#"<col min="1" max="5" width="8.43"/><col min="6" max="10" style="1"/><col min="11" max="20" style="2"/><col min="21" max="30" style="3"/>"#;

        let merged = merge_extraneous_columns(cols, 5);

        assert_eq!(
            merged,
            br#"<col min="1" max="5" width="8.43"/><col min="6" max="30" style="3"/>"#
        );
    }

    #[test]
    fn max_column_zero_collapses_all_columns_into_last_column() {
        let cols = br#"<col min="1" max="10" style="1"/><col min="11" max="16384" style="2"/>"#;

        let merged = merge_extraneous_columns(cols, 0);

        assert_eq!(merged, br#"<col min="1" max="16384" style="2"/>"#);
    }

    #[test]
    fn hidden_column_disables_column_collapse_globally() {
        let cols = br#"<col min="1" max="5" width="8.43"/><col min="6" max="10" hidden="1"/><col min="11" max="20" style="2"/><col min="21" max="30" style="3"/>"#;

        let merged = merge_extraneous_columns(cols, 5);

        assert_eq!(merged, cols);
    }

    #[test]
    fn ignores_style_and_custom_width_boundaries_beyond_max_column_like_analyzer() {
        let cols = br#"<col min="1" max="5" width="8.43"/><col min="6" max="10" style="1" customWidth="true"/><col min="11" max="20" style="2" width="10.0"/><col min="21" max="30" style="3" width="12.0"/>"#;

        let merged = merge_extraneous_columns(cols, 5);

        assert_eq!(
            merged,
            br#"<col min="1" max="5" width="8.43"/><col min="6" max="30" style="3" width="12.0"/>"#
        );
    }

    #[test]
    fn collapses_visible_equal_fill_column_tail_with_no_borders() {
        let params = params_with_styles(&[(39, visible_fill_style(2))]);
        let cols = br#"<col min="1" max="20" width="5.21875" style="39" customWidth="1"/><col min="21" max="239" width="8.77734375" style="39" customWidth="1"/><col min="240" max="16148" width="11.5546875" style="39" customWidth="1"/><col min="16149" max="16384" width="8.77734375" style="39" customWidth="1"/>"#;

        let merged = merge_extraneous_columns_with_styles(cols, 20, &params, true);

        assert_eq!(
            merged,
            br#"<col min="1" max="20" width="5.21875" style="39" customWidth="1"/><col min="21" max="16384" width="8.77734375" style="39" customWidth="1"/>"#
        );
    }

    #[test]
    fn equal_fill_column_tail_merge_can_be_disabled() {
        let params = params_with_styles(&[(39, visible_fill_style(2))]);
        let cols = br#"<col min="1" max="20" style="39"/><col min="21" max="239" style="39"/><col min="240" max="16384" style="39"/>"#;

        let merged = merge_extraneous_columns_with_styles(cols, 20, &params, false);

        assert_eq!(merged, cols);
    }

    #[test]
    fn different_fill_stops_column_tail_merge_boundary() {
        let params =
            params_with_styles(&[(10, visible_fill_style(2)), (11, visible_fill_style(3))]);
        let cols = br#"<col min="1" max="20" style="10"/><col min="21" max="30" style="10"/><col min="31" max="40" style="11"/><col min="41" max="16384" style="11"/>"#;

        let merged = merge_extraneous_columns_with_styles(cols, 20, &params, true);

        assert_eq!(
            merged,
            br#"<col min="1" max="20" style="10"/><col min="21" max="30" style="10"/><col min="31" max="16384" style="11"/>"#
        );
    }

    #[test]
    fn visible_border_stops_equal_fill_column_tail_merge() {
        let params = params_with_styles(&[(39, visible_fill_and_border_style(2, 4))]);
        let cols = br#"<col min="1" max="20" style="39"/><col min="21" max="239" style="39"/><col min="240" max="16384" style="39"/>"#;

        let merged = merge_extraneous_columns_with_styles(cols, 20, &params, true);

        assert_eq!(merged, cols);
    }
}
