use crate::models::{CleanSheetParameters, ColumnDefinition, ColumnRangeMap};
use crate::utils::digits_byte_string_to_integer;
use regex::bytes::Regex;
use std::sync::OnceLock;

static COL_PATTERN: OnceLock<Regex> = OnceLock::new();
static ATTR_MIN: OnceLock<Regex> = OnceLock::new();
static ATTR_MAX: OnceLock<Regex> = OnceLock::new();
static ATTR_STYLE: OnceLock<Regex> = OnceLock::new();
static ATTR_HIDDEN: OnceLock<Regex> = OnceLock::new();

pub fn process_cols_node(col_data: &[u8], params: &CleanSheetParameters) -> ColumnRangeMap {
    let mut map = ColumnRangeMap::default();

    let col_re = COL_PATTERN.get_or_init(|| Regex::new(r"<col\b[^>]*/>").unwrap());
    let min_re = ATTR_MIN.get_or_init(|| Regex::new(r#"min="([^"]+)""#).unwrap());
    let max_re = ATTR_MAX.get_or_init(|| Regex::new(r#"max="([^"]+)""#).unwrap());
    let style_re = ATTR_STYLE.get_or_init(|| Regex::new(r#"style="([^"]+)""#).unwrap());
    let hidden_re = ATTR_HIDDEN.get_or_init(|| Regex::new(r#"hidden="([^"]+)""#).unwrap());

    for mat in col_re.find_iter(col_data) {
        let tag = mat.as_bytes();

        let min_val = min_re
            .captures(tag)
            .map(|c| digits_byte_string_to_integer(c.get(1).unwrap().as_bytes()))
            .unwrap_or(0);

        let max_val = max_re
            .captures(tag)
            .map(|c| digits_byte_string_to_integer(c.get(1).unwrap().as_bytes()))
            .unwrap_or(0);

        if min_val == 0 || max_val == 0 {
            continue;
        }

        let style_id = style_re
            .captures(tag)
            .map(|c| digits_byte_string_to_integer(c.get(1).unwrap().as_bytes()));

        let style = style_id.and_then(|id| params.patterns.xfs_by_index.get(&id).cloned());

        let hidden = hidden_re
            .captures(tag)
            .map_or(false, |c| c.get(1).unwrap().as_bytes() == b"1");

        map.add(ColumnDefinition {
            min: min_val,
            max: max_val,
            cell_xf: style,
            hidden,
        });
    }

    map.finalize();
    map
}

pub fn merge_extraneous_columns(cols_content: &[u8], max_column: u32) -> Vec<u8> {
    let col_re = COL_PATTERN.get_or_init(|| Regex::new(r"<col\b[^>]*/>").unwrap());
    let min_re = ATTR_MIN.get_or_init(|| Regex::new(r#"min="([^"]+)""#).unwrap());
    let hidden_re = ATTR_HIDDEN.get_or_init(|| Regex::new(r#"hidden="([^"]+)""#).unwrap());

    let mut removable_columns = Vec::new();

    for mat in col_re.find_iter(cols_content) {
        let tag = mat.as_bytes();

        let hidden = hidden_re
            .captures(tag)
            .is_some_and(|caps| caps.get(1).unwrap().as_bytes() == b"1");

        // Analyzer compatibility:
        // find_excess_columns(...) returns no removable columns as soon as any
        // column definition is hidden.
        if hidden {
            return cols_content.to_vec();
        }

        let Some(min_caps) = min_re.captures(tag) else {
            continue;
        };

        let min_value = digits_byte_string_to_integer(min_caps.get(1).unwrap().as_bytes());

        // Analyzer compatibility:
        // only complete <col> definitions whose min is beyond max_column are
        // collapsed. Do not split a column definition where min <= max_column
        // but max > max_column.
        if min_value > max_column {
            removable_columns.push(RemovableColumnTag {
                start: mat.start(),
                end: mat.end(),
                tag,
                min_value,
            });
        }
    }

    if removable_columns.is_empty() {
        return cols_content.to_vec();
    }

    let first = removable_columns.first().unwrap();
    let last = removable_columns.last().unwrap();

    let replacement_min = format!(r#"min="{}""#, first.min_value);
    let collapsed_last_tag = min_re
        .replace(last.tag, replacement_min.as_bytes())
        .into_owned();

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

#[derive(Debug, Clone, Copy)]
struct RemovableColumnTag<'a> {
    start: usize,
    end: usize,
    tag: &'a [u8],
    min_value: u32,
}

#[cfg(test)]
mod tests {
    use super::merge_extraneous_columns;

    const COLS: &[u8] = br#"<col min="1" max="12" width="8.43"/><col min="13" max="17" width="10.0"/><col min="18" max="25" width="10.0"/>"#;

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
}
