use crate::models::{CleanSheetMetrics, CleanSheetParameters, CleanupCause, RowAttributes, Xf};
use crate::sheet::cells::replace_cell;
use crate::utils::digits_byte_string_to_integer;
use crate::xml::find_bytes;
use regex::bytes::{Captures, Regex};
use std::sync::OnceLock;

static PROCESS_SELF_CLOSING_CELLS_RE: OnceLock<Regex> = OnceLock::new();
static PROCESS_ALL_CELLS_RE: OnceLock<Regex> = OnceLock::new();

pub fn replace_row(
    caps: &Captures,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let Some((row_attributes, original_row_content)) = extract_row_attributes(caps, params) else {
        return caps.get(0).unwrap().as_bytes().to_vec();
    };

    let mut row_attr_str = row_attributes.row_attr.clone();
    let is_hidden = row_attributes.hidden;
    let row_style = row_attributes.style.clone();
    let has_height = row_attributes.has_height;

    metrics
        .row_attributes
        .insert(row_attributes.ref_.clone(), row_attributes.clone());

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
                    metrics.cleanup_stats.record_removed(
                        CleanupCause::ConsecutiveHiddenEmptyRows,
                        caps.get(0).unwrap().as_bytes().len(),
                    );
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
                    caps.get(0).unwrap().as_bytes().len(),
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
    let Some(row_attributes) = extract_empty_row_attributes(caps, params) else {
        return caps.get(0).unwrap().as_bytes().to_vec();
    };

    let mut row_attr_str = row_attributes.row_attr.clone();
    let is_hidden = row_attributes.hidden;
    let row_style = row_attributes.style.clone();
    let has_height = row_attributes.has_height;

    metrics
        .row_attributes
        .insert(row_attributes.ref_.clone(), row_attributes.clone());

    if is_hidden {
        if previous_row_was_also_hidden(metrics) {
            if params.cleanup.consecutive_hidden_empty_rows {
                metrics.cleanup_stats.record_removed(
                    CleanupCause::ConsecutiveHiddenEmptyRows,
                    caps.get(0).unwrap().as_bytes().len(),
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

    if !empty_row_has_relevant_visible_or_protected_style(
        row_style.as_ref(),
        &row_attributes.ref_,
        metrics,
    ) && !has_height
    {
        if params.cleanup.empty_rows_without_visible_style_or_height {
            metrics.cleanup_stats.record_removed(
                CleanupCause::EmptyRowsWithoutVisibleStyleOrHeight,
                caps.get(0).unwrap().as_bytes().len(),
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

    let needs_content_cell_scan = params.cleanup.unreferenced_whitespace_cells
        || params.cleanup.unreferenced_hidden_content_cells
        || params.advanced_cleanup.as_ref().is_some_and(|advanced| {
            advanced.remove_unreferenced_whitespace || advanced.remove_unreferenced_hidden_content
        });

    let cell_re = if needs_content_cell_scan {
        PROCESS_ALL_CELLS_RE.get_or_init(|| {
            Regex::new(
                r#"(?s)<c\b((?:[^>]*\br="([A-Z]+)(\d+)")?(?:[^>]*\bs="(\d+?)")?[^>]*?)(?:/>|>.*?</c>)"#,
            )
            .unwrap()
        })
    } else {
        PROCESS_SELF_CLOSING_CELLS_RE.get_or_init(|| {
            Regex::new(r#"<c\b((?:[^>]*\br="([A-Z]+)(\d+)")?(?:[^>]*\bs="(\d+?)")?[^>]*?)/>"#)
                .unwrap()
        })
    };

    cell_re
        .replace_all(row_content, |caps: &Captures| {
            replace_cell(caps, params, metrics)
        })
        .into_owned()
}

fn extract_row_attributes(
    caps: &Captures,
    params: &CleanSheetParameters,
) -> Option<(RowAttributes, Vec<u8>)> {
    let row_attr = caps.get(1)?.as_bytes().to_vec();
    let ref_ = caps
        .get(2)
        .map(|m| m.as_bytes().to_vec())
        .unwrap_or_default();
    let hidden = caps.get(3).is_some();
    let style_id = caps
        .get(4)
        .map(|m| digits_byte_string_to_integer(m.as_bytes()));
    let ht = caps.get(5).is_some();
    let original_row_content = caps
        .get(6)
        .map(|m| m.as_bytes().to_vec())
        .unwrap_or_default();

    let style = style_id.and_then(|id| params.patterns.xfs_by_index.get(&id).cloned());

    let row_attributes = RowAttributes {
        row_attr,
        ref_,
        hidden,
        style,
        has_height: ht,
        is_empty: original_row_content.is_empty(),
    };

    Some((row_attributes, original_row_content))
}

fn extract_empty_row_attributes(
    caps: &Captures,
    params: &CleanSheetParameters,
) -> Option<RowAttributes> {
    let row_attr = caps.get(1)?.as_bytes().to_vec();
    let ref_ = caps
        .get(2)
        .map(|m| m.as_bytes().to_vec())
        .unwrap_or_default();
    let hidden = caps.get(3).is_some();
    let style_id = caps
        .get(4)
        .map(|m| digits_byte_string_to_integer(m.as_bytes()));
    let ht = caps.get(5).is_some();

    let style = style_id.and_then(|id| params.patterns.xfs_by_index.get(&id).cloned());

    Some(RowAttributes {
        row_attr,
        ref_,
        hidden,
        style,
        has_height: ht,
        is_empty: true,
    })
}

fn previous_row_was_also_hidden(metrics: &CleanSheetMetrics) -> bool {
    metrics
        .kept_row_states
        .last()
        .is_some_and(|previous| previous.hidden)
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
    let mut out = row_attr.to_vec();

    out = remove_attribute_by_name(&out, b"spans");
    out = remove_dy_descent_attributes(&out);

    for style_id in params.patterns.xfs_by_index.keys() {
        if params
            .patterns
            .xfs_by_index
            .get(style_id)
            .is_some_and(|xf| xf.has_only_font_style)
        {
            let needle = format!(r#"s="{}""#, style_id).into_bytes();
            out = remove_exact_attribute(&out, &needle);
        }
    }

    out
}

fn remove_attribute_by_name(data: &[u8], attr_name: &[u8]) -> Vec<u8> {
    remove_attribute_matching(data, |attr| {
        attr.starts_with(attr_name) && attr.get(attr_name.len()) == Some(&b'=')
    })
}

fn remove_dy_descent_attributes(data: &[u8]) -> Vec<u8> {
    remove_attribute_matching(data, |attr| {
        attr.windows(b":dyDescent=".len())
            .any(|w| w == b":dyDescent=")
    })
}

fn remove_exact_attribute(data: &[u8], exact_attr: &[u8]) -> Vec<u8> {
    remove_attribute_matching(data, |attr| attr == exact_attr)
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
