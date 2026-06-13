use crate::models::{CleanSheetMetrics, CleanSheetParameters, CleanupCause, RowAttributes, Xf};
use crate::sheet::cells::{replace_parsed_cell, ParsedCell};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use crate::xml::{find_byte_from, find_bytes};
use regex::bytes::Captures;

pub fn replace_row(
    caps: &Captures,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let Some(row_match) = caps.get(0).map(|m| m.as_bytes()) else {
        return Vec::new();
    };
    let Some(row_attr) = caps.get(1).map(|m| m.as_bytes()) else {
        return caps.get(0).unwrap().as_bytes().to_vec();
    };
    let row_content = caps.get(6).map(|m| m.as_bytes());

    replace_row_bytes(row_match, row_attr, row_content, params, metrics)
}

pub fn replace_row_bytes(
    row_raw: &[u8],
    row_attr: &[u8],
    row_content: Option<&[u8]>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let Some((row_attributes, original_row_content)) =
        extract_row_attributes_from_parts(row_attr, row_content, params)
    else {
        return row_raw.to_vec();
    };

    let mut row_attr_str = row_attributes.row_attr.clone();
    let is_hidden = row_attributes.hidden;
    let row_style = row_attributes.style.clone();
    let has_height = row_attributes.has_height;

    metrics
        .row_attributes
        .insert(row_attributes.ref_.clone(), row_attributes.clone());
    record_visible_row_bounds(&row_attributes, metrics);

    let was_already_empty = original_row_content.is_empty();

    let row_content = if !was_already_empty {
        clean_cells(params, metrics, &original_row_content)
    } else {
        original_row_content.clone()
    };

    let is_empty = row_content.is_empty();

    if is_empty {
        if is_hidden {
            if previous_row_was_also_hidden(metrics) {
                if params.cleanup.consecutive_hidden_empty_rows {
                    metrics
                        .cleanup_stats
                        .record_removed(CleanupCause::ConsecutiveHiddenEmptyRows, row_raw.len());
                    metrics.deleted_row_states.push(row_attributes);
                    return Vec::new();
                }
            }

            if has_height {
                row_attr_str = add_custom_height_if_absent(&row_attr_str);
            }

            return generate_and_log_self_closing_row(
                &row_attr_str,
                params,
                metrics,
                row_attributes,
            );
        }

        if !empty_row_has_relevant_visible_or_protected_style(
            row_style.as_ref(),
            &row_attributes.ref_,
            metrics,
        ) && !has_height
        {
            if params.cleanup.empty_rows_without_visible_style_or_height {
                metrics.cleanup_stats.record_removed(
                    CleanupCause::EmptyRowsWithoutVisibleStyleOrHeight,
                    row_raw.len(),
                );
                metrics.deleted_row_states.push(row_attributes);
                return Vec::new();
            }
        }

        if has_height {
            row_attr_str = add_custom_height_if_absent(&row_attr_str);
        }

        return generate_and_log_self_closing_row(&row_attr_str, params, metrics, row_attributes);
    }

    generate_and_log_row(&row_attr_str, &row_content, metrics, row_attributes)
}

pub fn replace_empty_row_after_cell_range(
    caps: &Captures,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let Some(row_match) = caps.get(0).map(|m| m.as_bytes()) else {
        return Vec::new();
    };
    let Some(row_attr) = caps.get(1).map(|m| m.as_bytes()) else {
        return caps.get(0).unwrap().as_bytes().to_vec();
    };

    replace_empty_row_after_cell_range_bytes(row_match, row_attr, params, metrics)
}

pub fn replace_empty_row_after_cell_range_bytes(
    row_raw: &[u8],
    row_attr: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let Some(row_attributes) = extract_empty_row_attributes_from_attr(row_attr, params) else {
        return row_raw.to_vec();
    };

    let mut row_attr_str = row_attributes.row_attr.clone();
    let is_hidden = row_attributes.hidden;
    let row_style = row_attributes.style.clone();
    let has_height = row_attributes.has_height;

    metrics
        .row_attributes
        .insert(row_attributes.ref_.clone(), row_attributes.clone());
    record_visible_row_bounds(&row_attributes, metrics);

    if is_hidden {
        if previous_row_was_also_hidden(metrics) {
            if params.cleanup.consecutive_hidden_empty_rows {
                metrics
                    .cleanup_stats
                    .record_removed(CleanupCause::ConsecutiveHiddenEmptyRows, row_raw.len());
                metrics.deleted_row_states.push(row_attributes);
                return Vec::new();
            }
        }

        if has_height {
            row_attr_str = add_custom_height_if_absent(&row_attr_str);
        }

        return generate_and_log_self_closing_row(&row_attr_str, params, metrics, row_attributes);
    }

    if !empty_row_has_relevant_visible_or_protected_style(
        row_style.as_ref(),
        &row_attributes.ref_,
        metrics,
    ) && !has_height
    {
        if params.cleanup.empty_rows_without_visible_style_or_height {
            metrics.cleanup_stats.record_removed(
                CleanupCause::EmptyRowsWithoutVisibleStyleOrHeight,
                row_raw.len(),
            );
            metrics.deleted_row_states.push(row_attributes);
            return Vec::new();
        }
    }

    if has_height {
        row_attr_str = add_custom_height_if_absent(&row_attr_str);
    }

    generate_and_log_self_closing_row(&row_attr_str, params, metrics, row_attributes)
}

fn clean_cells(
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_content: &[u8],
) -> Vec<u8> {
    if row_content.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(row_content.len().min(64 * 1024));
    let mut pos = 0;

    while let Some(cell_start_rel) = find_bytes(&row_content[pos..], b"<c") {
        let cell_start = pos + cell_start_rel;

        if !is_cell_tag_start(row_content, cell_start) {
            out.extend_from_slice(&row_content[pos..cell_start + b"<c".len()]);
            pos = cell_start + b"<c".len();
            continue;
        }

        let Some(tag_end) = find_byte_from(row_content, b'>', cell_start) else {
            break;
        };

        let attr_start = cell_start + b"<c".len();
        let self_closing = tag_end > cell_start && row_content[tag_end - 1] == b'/';
        let attr_end = if self_closing { tag_end - 1 } else { tag_end };

        let (cell_end, raw) = if self_closing {
            (tag_end + 1, &row_content[cell_start..tag_end + 1])
        } else {
            let content_start = tag_end + 1;
            let Some(close_rel) = find_bytes(&row_content[content_start..], b"</c>") else {
                break;
            };
            let cell_end = content_start + close_rel + b"</c>".len();
            (cell_end, &row_content[cell_start..cell_end])
        };

        out.extend_from_slice(&row_content[pos..cell_start]);

        let attr = &row_content[attr_start..attr_end];
        if let Some(cell) = parse_cell_from_parts(raw, attr) {
            out.extend_from_slice(&replace_parsed_cell(&cell, params, metrics));
        } else {
            out.extend_from_slice(raw);
        }

        pos = cell_end;
    }

    out.extend_from_slice(&row_content[pos..]);
    out
}

fn is_cell_tag_start(data: &[u8], cell_start: usize) -> bool {
    matches!(
        data.get(cell_start + b"<c".len()),
        Some(byte) if byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>')
    )
}

fn parse_cell_from_parts<'a>(raw: &'a [u8], attr: &'a [u8]) -> Option<ParsedCell<'a>> {
    let reference = extract_attr_value(attr, b"r")?;
    let split = reference
        .iter()
        .position(|byte| byte.is_ascii_digit())
        .filter(|split| *split > 0 && *split < reference.len())?;

    let column = &reference[..split];
    let row = &reference[split..];

    if !column.iter().all(|byte| byte.is_ascii_uppercase())
        || !row.iter().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let style_id = extract_attr_value(attr, b"s").map(digits_byte_string_to_integer);

    Some(ParsedCell {
        raw,
        attr,
        row,
        col_num: letters_byte_string_to_integer(column),
        style_id,
    })
}

fn extract_row_attributes_from_parts(
    row_attr: &[u8],
    row_content: Option<&[u8]>,
    params: &CleanSheetParameters,
) -> Option<(RowAttributes, Vec<u8>)> {
    let row_attr_vec = row_attr.to_vec();
    let ref_ = extract_attr_value(row_attr, b"r")
        .map(|value| value.to_vec())
        .unwrap_or_default();
    let hidden = extract_attr_value(row_attr, b"hidden").is_some_and(|value| value == b"1");
    let style_id = extract_attr_value(row_attr, b"s").map(digits_byte_string_to_integer);
    let ht = extract_attr_value(row_attr, b"ht").is_some();
    let original_row_content = row_content
        .map(|content| content.to_vec())
        .unwrap_or_default();
    let style = style_id.and_then(|id| params.patterns.xfs_by_index.get(&id).cloned());

    let row_attributes = RowAttributes {
        row_attr: row_attr_vec,
        ref_,
        hidden,
        style,
        has_height: ht,
        is_empty: original_row_content.is_empty(),
    };

    Some((row_attributes, original_row_content))
}

fn extract_empty_row_attributes_from_attr(
    row_attr: &[u8],
    params: &CleanSheetParameters,
) -> Option<RowAttributes> {
    let row_attr_vec = row_attr.to_vec();
    let ref_ = extract_attr_value(row_attr, b"r")
        .map(|value| value.to_vec())
        .unwrap_or_default();
    let hidden = extract_attr_value(row_attr, b"hidden").is_some_and(|value| value == b"1");
    let style_id = extract_attr_value(row_attr, b"s").map(digits_byte_string_to_integer);
    let ht = extract_attr_value(row_attr, b"ht").is_some();
    let style = style_id.and_then(|id| params.patterns.xfs_by_index.get(&id).cloned());

    Some(RowAttributes {
        row_attr: row_attr_vec,
        ref_,
        hidden,
        style,
        has_height: ht,
        is_empty: true,
    })
}

fn extract_attr_value<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let mut search_start = 0;

    while search_start < data.len() {
        let relative = find_bytes(&data[search_start..], attr_name)?;
        let attr_start = search_start + relative;
        let value_marker = attr_start + attr_name.len();

        if attr_start > 0
            && data[attr_start - 1].is_ascii_whitespace()
            && data.get(value_marker..value_marker + 2) == Some(b"=\"")
        {
            let value_start = value_marker + 2;
            let value_end = data[value_start..].iter().position(|byte| *byte == b'"')?;
            return Some(&data[value_start..value_start + value_end]);
        }

        search_start = attr_start + attr_name.len();
    }

    None
}

fn previous_row_was_also_hidden(metrics: &CleanSheetMetrics) -> bool {
    metrics
        .kept_row_states
        .last()
        .is_some_and(|previous| previous.hidden)
}

fn record_visible_row_bounds(row_attributes: &RowAttributes, metrics: &mut CleanSheetMetrics) {
    if !row_attributes
        .style
        .as_ref()
        .is_some_and(|style| style.has_fill_or_border)
    {
        return;
    }

    let Ok(row_str) = std::str::from_utf8(&row_attributes.ref_) else {
        return;
    };
    let Ok(row_num) = row_str.parse::<u32>() else {
        return;
    };

    metrics.visible_content_bounds.include_row(row_num);
}

fn empty_row_has_relevant_visible_or_protected_style(
    style: Option<&Xf>,
    row_ref: &[u8],
    metrics: &CleanSheetMetrics,
) -> bool {
    let Some(style) = style else {
        return false;
    };

    if style.has_fill_or_border {
        return true;
    }

    if !style.has_apply_protection {
        return false;
    }

    if metrics.sheet_protection_enabled {
        return true;
    }

    let Ok(row_str) = std::str::from_utf8(row_ref) else {
        return true;
    };

    let Ok(row_num) = row_str.parse::<u32>() else {
        return true;
    };

    !metrics.visible_content_bounds.row_is_below(row_num)
}

fn generate_and_log_self_closing_row(
    row_attr: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_attributes: RowAttributes,
) -> Vec<u8> {
    metrics.kept_row_states.push(row_attributes);

    let cleaned_row_attr = if params.cleanup.empty_row_attribute_cleanup {
        remove_irrelevant_attributes_for_empty_rows(row_attr, params)
    } else {
        row_attr.to_vec()
    };

    metrics.cleanup_stats.record_delta(
        CleanupCause::EmptyRowAttributeCleanup,
        row_attr.len(),
        cleaned_row_attr.len(),
    );

    let mut out = Vec::with_capacity(cleaned_row_attr.len() + 7);
    out.extend_from_slice(b"<row");
    out.extend_from_slice(&cleaned_row_attr);
    out.extend_from_slice(b"/>");
    out
}

fn generate_and_log_row(
    row_attr: &[u8],
    row_content: &[u8],
    metrics: &mut CleanSheetMetrics,
    row_attributes: RowAttributes,
) -> Vec<u8> {
    metrics.kept_row_states.push(row_attributes);

    let mut out = Vec::with_capacity(row_attr.len() + row_content.len() + 11);
    out.extend_from_slice(b"<row");
    out.extend_from_slice(row_attr);
    out.extend_from_slice(b">");
    out.extend_from_slice(row_content);
    out.extend_from_slice(b"</row>");
    out
}

fn add_custom_height_if_absent(row_attr: &[u8]) -> Vec<u8> {
    if find_bytes(row_attr, b" customHeight=\"1\"").is_some() {
        return row_attr.to_vec();
    }

    if find_bytes(row_attr, b" ht=\"").is_none() {
        return row_attr.to_vec();
    }

    let mut out = row_attr.to_vec();
    out.extend_from_slice(b" customHeight=\"1\"");
    out
}

fn remove_irrelevant_attributes_for_empty_rows(
    row_attr: &[u8],
    params: &CleanSheetParameters,
) -> Vec<u8> {
    let remove_font_only_style = extract_attr_value(row_attr, b"s")
        .map(digits_byte_string_to_integer)
        .and_then(|style_id| params.patterns.xfs_by_index.get(&style_id))
        .is_some_and(|xf| xf.has_only_font_style);

    remove_attribute_matching(row_attr, |attr| {
        attr.windows(b":dyDescent=".len())
            .any(|window| window == b":dyDescent=")
            || (remove_font_only_style && attribute_has_name(attr, b"s"))
    })
}

fn attribute_has_name(attr: &[u8], attr_name: &[u8]) -> bool {
    attr.starts_with(attr_name) && attr.get(attr_name.len()) == Some(&b'=')
}

fn remove_attribute_matching<F>(data: &[u8], predicate: F) -> Vec<u8>
where
    F: Fn(&[u8]) -> bool,
{
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;

    while i < data.len() {
        if data[i].is_ascii_whitespace() {
            let ws_start = i;

            while i < data.len() && data[i].is_ascii_whitespace() {
                i += 1;
            }

            let attr_start = i;

            while i < data.len() && !data[i].is_ascii_whitespace() {
                i += 1;
            }

            let attr = &data[attr_start..i];

            if predicate(attr) {
                continue;
            }

            out.extend_from_slice(&data[ws_start..i]);
        } else {
            out.push(data[i]);
            i += 1;
        }
    }

    out
}

#[cfg(test)]
#[path = "rows_regression_tests.rs"]
mod regression_tests;
