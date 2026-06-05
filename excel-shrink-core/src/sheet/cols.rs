use crate::models::{ColumnDefinition, ColumnRangeMap, CleanSheetParameters};
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

        let min_val = min_re.captures(tag)
            .map(|c| digits_byte_string_to_integer(c.get(1).unwrap().as_bytes()))
            .unwrap_or(0);
        
        let max_val = max_re.captures(tag)
            .map(|c| digits_byte_string_to_integer(c.get(1).unwrap().as_bytes()))
            .unwrap_or(0);

        if min_val == 0 || max_val == 0 { continue; }

        let style_id = style_re.captures(tag)
            .map(|c| digits_byte_string_to_integer(c.get(1).unwrap().as_bytes()));

        let style = style_id.and_then(|id| params.patterns.xfs_by_index.get(&id).cloned());

        let hidden = hidden_re.captures(tag).map_or(false, |c| c.get(1).unwrap().as_bytes() == b"1");

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

pub fn merge_extraneous_columns(cols_content: &[u8], last_max_col: u32) -> Vec<u8> {
    if last_max_col == 0 {
        return cols_content.to_vec();
    }

    let col_re = COL_PATTERN.get_or_init(|| Regex::new(r"<col\b[^>]*/>").unwrap());
    let max_re = ATTR_MAX.get_or_init(|| Regex::new(r#"max="([^"]+)""#).unwrap());

    let mut col_chunks = Vec::new();
    let mut first_extraneous_match = None;
    let mut last_max_attr = None;

    let mut last_end = 0;

    for mat in col_re.find_iter(cols_content) {
        let tag = mat.as_bytes();
        
        if let Some(max_caps) = max_re.captures(tag) {
            let max_val = digits_byte_string_to_integer(max_caps.get(1).unwrap().as_bytes());
            
            last_max_attr = Some(max_caps.get(0).unwrap().as_bytes().to_vec());

            if max_val <= last_max_col {
                col_chunks.extend_from_slice(&cols_content[last_end..mat.end()]);
                last_end = mat.end();
            } else if first_extraneous_match.is_none() {
                // Wir haben die erste Spalte nach last_max_col gefunden
                col_chunks.extend_from_slice(&cols_content[last_end..mat.start()]); // Text vor dem Tag
                first_extraneous_match = Some(tag.to_vec());
                last_end = mat.end(); // Ab hier ignorieren wir alle weiteren Tags
            }
        }
    }

    if let (Some(first_ext_tag), Some(last_max)) = (first_extraneous_match, last_max_attr) {
        // Ersetze das max="..." Attribut des ersten überflüssigen Tags durch das max="..." des letzten Tags
        let replaced = max_re.replace(&first_ext_tag, last_max.as_slice());
        col_chunks.extend_from_slice(&replaced);
        return col_chunks;
    }

    cols_content.to_vec()
}