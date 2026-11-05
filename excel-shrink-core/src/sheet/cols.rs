use crate::models::{CleanSheetParameters, ColumnDefinition, ColumnRangeMap, Xf};
use crate::utils::digits_byte_string_to_integer;
use crate::xml::{find_byte_from, find_bytes};

pub fn process_cols_node(col_data: &[u8], params: &CleanSheetParameters) -> ColumnRangeMap {
    let mut map = ColumnRangeMap::default();

    for column in parse_column_tags(col_data) {
        map.add(column_definition(column, params));
    }

    map.finalize();
    map
}

fn column_definition(
    column: ParsedColumnTag<'_>,
    params: &CleanSheetParameters,
) -> ColumnDefinition {
    let style = column
        .style_id
        .and_then(|id| params.patterns.xfs_by_index.get(&id).cloned());

    ColumnDefinition {
        min: column.min,
        max: column.max,
        cell_xf: style,
        hidden: column.hidden,
    }
}

pub fn merge_extraneous_columns(cols_content: &[u8], max_column: u32) -> Vec<u8> {
    let Some(removable_columns) = removable_column_tags(cols_content, max_column) else {
        return cols_content.to_vec();
    };

    collapse_removable_columns(cols_content, &removable_columns)
        .unwrap_or_else(|| cols_content.to_vec())
}

fn removable_column_tags(
    cols_content: &[u8],
    max_column: u32,
) -> Option<Vec<RemovableColumnTag<'_>>> {
    let mut removable_columns = Vec::new();

    for column in parse_column_tags(cols_content) {
        // Analyzer compatibility: any hidden definition disables collapsing globally.
        if column.hidden {
            return None;
        }

        // Analyzer compatibility: definitions crossing max_column are not split.
        if column.min > max_column {
            removable_columns.push(RemovableColumnTag {
                start: column.start,
                end: column.end,
                tag: column.tag,
                min_value: column.min,
            });
        }
    }

    Some(removable_columns)
}

fn collapse_removable_columns(
    cols_content: &[u8],
    removable_columns: &[RemovableColumnTag<'_>],
) -> Option<Vec<u8>> {
    let first = removable_columns.first()?;
    let last = removable_columns.last()?;
    let replacement_min = first.min_value.to_string();
    let collapsed_last_tag = replace_attr_value(last.tag, b"min", replacement_min.as_bytes())?;

    let mut out = Vec::with_capacity(cols_content.len());
    let mut cursor = 0;

    for removable in &removable_columns[..removable_columns.len().saturating_sub(1)] {
        out.extend_from_slice(&cols_content[cursor..removable.start]);
        cursor = removable.end;
    }

    out.extend_from_slice(&cols_content[cursor..last.start]);
    out.extend_from_slice(&collapsed_last_tag);
    out.extend_from_slice(&cols_content[last.end..]);
    Some(out)
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
    let mut position = 0;

    while let Some(tag_match) = find_next_col_tag(cols_content, position) {
        position = tag_match.end;

        if let Some(column) = parse_column_tag(tag_match) {
            columns.push(column);
        }
    }

    columns
}

fn parse_column_tag(tag_match: ColumnTagMatch<'_>) -> Option<ParsedColumnTag<'_>> {
    let tag = tag_match.tag;
    let min = extract_attr_value(tag, b"min").map(digits_byte_string_to_integer)?;
    let max = extract_attr_value(tag, b"max").map(digits_byte_string_to_integer)?;

    if min == 0 || max == 0 {
        return None;
    }

    Some(ParsedColumnTag {
        start: tag_match.start,
        end: tag_match.end,
        tag,
        min,
        max,
        hidden: extract_attr_value(tag, b"hidden").is_some_and(|value| value == b"1"),
        style_id: extract_attr_value(tag, b"style").map(digits_byte_string_to_integer),
    })
}
fn find_next_col_tag<'a>(data: &'a [u8], start: usize) -> Option<ColumnTagMatch<'a>> {
    let mut search_start = start;

    while search_start < data.len() {
        let relative = find_bytes(&data[search_start..], b"<col")?;
        let tag_start = search_start + relative;
        let after_name = tag_start + b"<col".len();

        if col_tag_name_is_delimited(data, after_name) {
            return column_tag_match_at(data, tag_start);
        }

        search_start = after_name;
    }

    None
}

#[inline]
fn col_tag_name_is_delimited(data: &[u8], after_name: usize) -> bool {
    data.get(after_name)
        .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
}

fn column_tag_match_at(data: &[u8], tag_start: usize) -> Option<ColumnTagMatch<'_>> {
    let tag_end = find_byte_from(data, b'>', tag_start)? + 1;

    Some(ColumnTagMatch {
        start: tag_start,
        end: tag_end,
        tag: &data[tag_start..tag_end],
    })
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
        let relative = find_bytes(&data[search_start..], attr_name)?;
        let attribute_start = search_start + relative;
        let after_name = attribute_start + attr_name.len();

        if quoted_attribute_starts_at(data, attribute_start, after_name) {
            return quoted_value_range(data, after_name);
        }

        search_start = after_name;
    }

    None
}

#[inline]
fn quoted_attribute_starts_at(data: &[u8], attribute_start: usize, after_name: usize) -> bool {
    attribute_start > 0
        && data[attribute_start - 1].is_ascii_whitespace()
        && data.get(after_name) == Some(&b'=')
        && data.get(after_name + 1) == Some(&b'"')
}

fn quoted_value_range(data: &[u8], after_name: usize) -> Option<std::ops::Range<usize>> {
    let value_start = after_name + 2;
    let value_end = value_start + data[value_start..].iter().position(|byte| *byte == b'"')?;

    Some(value_start..value_end)
}

fn collapse_visible_equal_fill_column_tail(
    cols_content: &[u8],
    columns: &[ParsedColumnTag<'_>],
    max_column: u32,
    params: &CleanSheetParameters,
    collapse_equal_fill_tail: bool,
) -> Option<Vec<u8>> {
    if !collapse_equal_fill_tail {
        return None;
    }

    let last = columns.last()?;
    let target_style = eligible_visible_tail_style(last, max_column, params)?;
    let first_tail_index = equal_fill_tail_start(columns, max_column, params, target_style);
    let first_tail = multiple_column_tail_start(columns, first_tail_index)?;

    collapse_tail_into_first(cols_content, first_tail, last)
}

fn eligible_visible_tail_style(
    last: &ParsedColumnTag<'_>,
    max_column: u32,
    params: &CleanSheetParameters,
) -> Option<ColumnTailStyle> {
    if last.min <= max_column {
        return None;
    }

    column_tail_style(last, params).filter(|style| style.has_visible_fill)
}

fn equal_fill_tail_start(
    columns: &[ParsedColumnTag<'_>],
    max_column: u32,
    params: &CleanSheetParameters,
    target_style: ColumnTailStyle,
) -> usize {
    let mut first_tail_index = columns.len() - 1;

    while first_tail_index > 0 {
        let previous = &columns[first_tail_index - 1];
        if !column_extends_equal_fill_tail(previous, max_column, params, target_style) {
            break;
        }
        first_tail_index -= 1;
    }

    first_tail_index
}

fn column_extends_equal_fill_tail(
    column: &ParsedColumnTag<'_>,
    max_column: u32,
    params: &CleanSheetParameters,
    target_style: ColumnTailStyle,
) -> bool {
    column.min > max_column
        && column_tail_style(column, params)
            .is_some_and(|style| style.fill_key == target_style.fill_key)
}

fn collapse_default_column_tail(
    cols_content: &[u8],
    columns: &[ParsedColumnTag<'_>],
    max_column: u32,
    params: &CleanSheetParameters,
) -> Option<Vec<u8>> {
    let last = columns.last()?;
    eligible_default_tail_column(last, max_column, params)?;

    let first_tail_index = default_tail_start(columns, max_column, params);
    let first_tail = multiple_column_tail_start(columns, first_tail_index)?;

    collapse_tail_into_last(cols_content, first_tail, last)
}

fn eligible_default_tail_column(
    last: &ParsedColumnTag<'_>,
    max_column: u32,
    params: &CleanSheetParameters,
) -> Option<()> {
    if last.min <= max_column {
        return None;
    }

    column_tail_style(last, params)
        .filter(|style| !style.has_visible_fill)
        .map(|_| ())
}

fn default_tail_start(
    columns: &[ParsedColumnTag<'_>],
    max_column: u32,
    params: &CleanSheetParameters,
) -> usize {
    let mut first_tail_index = columns.len() - 1;

    while first_tail_index > 0 {
        let previous = &columns[first_tail_index - 1];
        if !column_extends_default_tail(previous, max_column, params) {
            break;
        }
        first_tail_index -= 1;
    }

    first_tail_index
}

fn column_extends_default_tail(
    column: &ParsedColumnTag<'_>,
    max_column: u32,
    params: &CleanSheetParameters,
) -> bool {
    column.min > max_column
        && column_tail_style(column, params).is_some_and(|style| !style.has_visible_fill)
}

fn multiple_column_tail_start<'slice, 'data>(
    columns: &'slice [ParsedColumnTag<'data>],
    first_tail_index: usize,
) -> Option<&'slice ParsedColumnTag<'data>> {
    (first_tail_index < columns.len() - 1).then(|| &columns[first_tail_index])
}

fn collapse_tail_into_first(
    cols_content: &[u8],
    first_tail: &ParsedColumnTag<'_>,
    last: &ParsedColumnTag<'_>,
) -> Option<Vec<u8>> {
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

fn collapse_tail_into_last(
    cols_content: &[u8],
    first_tail: &ParsedColumnTag<'_>,
    last: &ParsedColumnTag<'_>,
) -> Option<Vec<u8>> {
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
            chart_format_anchors: Default::default(),
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
