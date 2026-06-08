use crate::models::{CleanSheetParameters, ColumnDefinition, ColumnRangeMap};
use crate::utils::digits_byte_string_to_integer;
use regex::bytes::Regex;
use std::sync::OnceLock;

static COL_PATTERN: OnceLock<Regex> = OnceLock::new();
static ATTR_MIN: OnceLock<Regex> = OnceLock::new();
static ATTR_MAX: OnceLock<Regex> = OnceLock::new();
static ATTR_STYLE: OnceLock<Regex> = OnceLock::new();
static ATTR_HIDDEN: OnceLock<Regex> = OnceLock::new();
static ATTR_PATTERN: OnceLock<Regex> = OnceLock::new();

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

pub fn merge_extraneous_columns(cols_content: &[u8], last_max_col: Option<u32>) -> Vec<u8> {
    let Some(last_max_col) = last_max_col else {
        return cols_content.to_vec();
    };

    if last_max_col == 0 {
        return cols_content.to_vec();
    }

    let col_re = COL_PATTERN.get_or_init(|| Regex::new(r"<col\b[^>]*/>").unwrap());
    let max_re = ATTR_MAX.get_or_init(|| Regex::new(r#"max="([^"]+)""#).unwrap());
    let attr_re =
        ATTR_PATTERN.get_or_init(|| Regex::new(r#"\s([A-Za-z_:][\w:.-]*)="([^"]*)""#).unwrap());

    let mut col_chunks = Vec::new();
    let mut current_run: Option<ExtraneousColumnRun<'_>> = None;

    let mut last_end = 0;

    for mat in col_re.find_iter(cols_content) {
        let tag = mat.as_bytes();

        if let Some(max_caps) = max_re.captures(tag) {
            let max_val = digits_byte_string_to_integer(max_caps.get(1).unwrap().as_bytes());
            let max_attr = max_caps.get(0).unwrap().as_bytes();

            if max_val <= last_max_col {
                if let Some(run) = current_run.take() {
                    flush_extraneous_column_run(&mut col_chunks, max_re, run);
                }
                col_chunks.extend_from_slice(&cols_content[last_end..mat.end()]);
                last_end = mat.end();
            } else if let Some(run) = current_run.as_mut() {
                let tag_attrs = comparable_column_attributes(attr_re, tag);
                let run_attrs = run
                    .attrs
                    .get_or_insert_with(|| comparable_column_attributes(attr_re, run.first_tag));

                if *run_attrs == tag_attrs {
                    run.last_max_attr = max_attr;
                } else {
                    let completed_run = current_run.take().unwrap();
                    flush_extraneous_column_run(&mut col_chunks, max_re, completed_run);
                    col_chunks.extend_from_slice(&cols_content[last_end..mat.start()]);
                    current_run = Some(ExtraneousColumnRun {
                        first_tag: tag,
                        attrs: Some(tag_attrs),
                        last_max_attr: max_attr,
                    });
                }

                last_end = mat.end();
            } else {
                col_chunks.extend_from_slice(&cols_content[last_end..mat.start()]);
                current_run = Some(ExtraneousColumnRun {
                    first_tag: tag,
                    attrs: None,
                    last_max_attr: max_attr,
                });
                last_end = mat.end();
            }
        }
    }

    if let Some(run) = current_run {
        flush_extraneous_column_run(&mut col_chunks, max_re, run);
        col_chunks.extend_from_slice(&cols_content[last_end..]);
        return col_chunks;
    }

    cols_content.to_vec()
}

struct ExtraneousColumnRun<'a> {
    first_tag: &'a [u8],
    attrs: Option<Vec<(&'a [u8], &'a [u8])>>,
    last_max_attr: &'a [u8],
}

fn flush_extraneous_column_run(
    col_chunks: &mut Vec<u8>,
    max_re: &Regex,
    run: ExtraneousColumnRun<'_>,
) {
    let replaced = max_re.replace(run.first_tag, run.last_max_attr);
    col_chunks.extend_from_slice(&replaced);
}

fn comparable_column_attributes<'a>(attr_re: &Regex, tag: &'a [u8]) -> Vec<(&'a [u8], &'a [u8])> {
    let mut attrs: Vec<(&'a [u8], &'a [u8])> = attr_re
        .captures_iter(tag)
        .filter_map(|caps| {
            let name = caps.get(1)?.as_bytes();

            if name == b"min" || name == b"max" {
                return None;
            }

            let value = caps.get(2)?.as_bytes();
            Some((name, value))
        })
        .collect();

    attrs.sort_unstable();
    attrs
}

#[cfg(test)]
mod tests {
    use super::merge_extraneous_columns;

    const COLS: &[u8] = br#"<col min="1" max="12" width="8.43"/><col min="13" max="17" width="10.0"/><col min="18" max="25" width="10.0"/>"#;

    #[test]
    fn merge_extraneous_columns_preserves_boundary_when_last_max_col_is_17() {
        let merged = merge_extraneous_columns(COLS, Some(17));

        assert_eq!(merged, COLS);
    }

    #[test]
    fn merge_extraneous_columns_extends_first_extraneous_range_when_last_max_col_is_14() {
        let merged = merge_extraneous_columns(COLS, Some(14));

        assert_eq!(
            merged,
            br#"<col min="1" max="12" width="8.43"/><col min="13" max="25" width="10.0"/>"#
        );
    }

    #[test]
    fn merge_extraneous_columns_leaves_cols_unchanged_when_last_max_col_is_none() {
        let cols = br#"<col min="8" max="8" width="11.5703125" style="587"/><col min="9" max="16384" width="11.5703125" style="559"/>"#;

        let merged = merge_extraneous_columns(cols, None);

        assert_eq!(merged, cols);
    }

    #[test]
    fn merge_extraneous_columns_preserves_extraneous_style_boundary() {
        let cols = br#"<col min="8" max="8" width="11.5703125" style="587"/><col min="9" max="16384" width="11.5703125" style="559"/>"#;

        let merged = merge_extraneous_columns(cols, Some(7));

        assert_eq!(merged, cols);
    }

    #[test]
    fn merge_extraneous_columns_preserves_custom_width_boundary() {
        let cols = br#"<col min="9" max="11" style="92" width="11.453125"/><col min="12" max="12" customWidth="true" style="92" width="10.90625"/><col min="13" max="16384" style="92" width="11.453125"/>"#;

        let merged = merge_extraneous_columns(cols, Some(8));

        assert_eq!(merged, cols);
    }

    #[test]
    fn merge_extraneous_columns_merges_equivalent_runs_without_global_bailout() {
        let cols = br#"<col min="1" max="5" width="8.43"/><col min="6" max="10" width="10.0"/><col min="11" max="20" width="10.0"/><col min="21" max="30" width="12.0"/><col min="31" max="40" width="12.0"/>"#;

        let merged = merge_extraneous_columns(cols, Some(5));

        assert_eq!(
            merged,
            br#"<col min="1" max="5" width="8.43"/><col min="6" max="20" width="10.0"/><col min="21" max="40" width="12.0"/>"#
        );
    }
}
