use crate::models::{CellCleanerMode, CleanSheetMetrics, CleanSheetParameters, RowAttributes};
use crate::sheet::cells::{replace_cell, replace_cell_view, CellReplacement, CellView};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
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
                metrics.deleted_row_states.push(row_attributes);
                return Vec::new();
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

        if row_style.as_ref().map_or(true, |style| {
            !style.has_fill_or_border && !style.has_apply_protection
        }) && !has_height
        {
            metrics.deleted_row_states.push(row_attributes);
            return Vec::new();
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
            metrics.deleted_row_states.push(row_attributes);
            return Vec::new();
        }

        if has_height {
            row_attr_str = add_custom_height_if_absent(&row_attr_str);
        }

        return generate_and_log_self_closing_row(&row_attr_str, params, metrics, row_attributes);
    }

    if row_style.as_ref().map_or(true, |style| {
        !style.has_fill_or_border && !style.has_apply_protection
    }) && !has_height
    {
        metrics.deleted_row_states.push(row_attributes);
        return Vec::new();
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

    match params.cell_cleaner_mode {
        CellCleanerMode::Regex => clean_cells_regex(params, metrics, row_content),
        CellCleanerMode::Bytes => clean_cells_bytes(params, metrics, row_content),
    }
}

fn clean_cells_regex(
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_content: &[u8],
) -> Vec<u8> {
    let needs_content_cell_scan = params.advanced_cleanup.as_ref().is_some_and(|advanced| {
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

fn clean_cells_bytes(
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_content: &[u8],
) -> Vec<u8> {
    let needs_content_cell_scan = needs_content_cell_scan(params);

    if find_bytes(row_content, b"<c").is_none() {
        return row_content.to_vec();
    }

    if !needs_content_cell_scan && !contains_self_closing_cell_candidate(row_content) {
        return row_content.to_vec();
    }

    let mut out: Option<Vec<u8>> = None;
    let mut last_copied = 0;
    let mut search_start = 0;

    while let Some(relative_start) = find_bytes(&row_content[search_start..], b"<c") {
        let cell_start = search_start + relative_start;

        if !is_cell_tag_start(row_content, cell_start) {
            search_start = cell_start + 2;
            continue;
        }

        let Some(opening_end) = row_content[cell_start..]
            .iter()
            .position(|byte| *byte == b'>')
            .map(|idx| cell_start + idx)
        else {
            break;
        };

        let opening_tag = &row_content[cell_start..=opening_end];
        let Some(attributes) = cell_attributes_from_opening_tag(opening_tag) else {
            search_start = opening_end + 1;
            continue;
        };
        let is_self_closing = is_self_closing_opening_tag(opening_tag);

        if !needs_content_cell_scan && !is_self_closing {
            search_start = opening_end + 1;
            continue;
        }

        let cell_end = if is_self_closing {
            opening_end + 1
        } else {
            let content_start = opening_end + 1;
            let Some(closing_relative) = find_bytes(&row_content[content_start..], b"</c>") else {
                search_start = opening_end + 1;
                continue;
            };
            content_start + closing_relative + b"</c>".len()
        };

        let raw = &row_content[cell_start..cell_end];
        let (reference, column_ref, row_ref, column, row, style_id) =
            parse_cell_view_attributes(attributes);
        let cell = CellView {
            raw,
            opening_tag,
            attributes,
            reference,
            column_ref,
            row_ref,
            column,
            row,
            style_id,
            is_self_closing,
            has_content: !is_self_closing && cell_has_content(raw),
        };

        match replace_cell_view(&cell, params, metrics) {
            CellReplacement::KeepOriginal => {}
            CellReplacement::Delete => {
                let out = out.get_or_insert_with(|| Vec::with_capacity(row_content.len()));
                out.extend_from_slice(&row_content[last_copied..cell_start]);
                last_copied = cell_end;
            }
            CellReplacement::Replace(replacement) => {
                let out = out.get_or_insert_with(|| Vec::with_capacity(row_content.len()));
                out.extend_from_slice(&row_content[last_copied..cell_start]);
                out.extend_from_slice(&replacement);
                last_copied = cell_end;
            }
            CellReplacement::Borrowed(replacement) => {
                let out = out.get_or_insert_with(|| Vec::with_capacity(row_content.len()));
                out.extend_from_slice(&row_content[last_copied..cell_start]);
                out.extend_from_slice(replacement);
                last_copied = cell_end;
            }
        }

        search_start = cell_end;
    }

    if let Some(mut out) = out {
        out.extend_from_slice(&row_content[last_copied..]);
        out
    } else {
        row_content.to_vec()
    }
}

fn needs_content_cell_scan(params: &CleanSheetParameters) -> bool {
    params.advanced_cleanup.as_ref().is_some_and(|advanced| {
        advanced.remove_unreferenced_whitespace || advanced.remove_unreferenced_hidden_content
    })
}

fn contains_self_closing_cell_candidate(row_content: &[u8]) -> bool {
    let mut search_start = 0;

    while let Some(relative_start) = find_bytes(&row_content[search_start..], b"<c") {
        let cell_start = search_start + relative_start;

        if !is_cell_tag_start(row_content, cell_start) {
            search_start = cell_start + 2;
            continue;
        }

        let Some(opening_end) = row_content[cell_start..]
            .iter()
            .position(|byte| *byte == b'>')
            .map(|idx| cell_start + idx)
        else {
            return false;
        };

        if is_self_closing_opening_tag(&row_content[cell_start..=opening_end]) {
            return true;
        }

        search_start = opening_end + 1;
    }

    false
}

fn is_cell_tag_start(row_content: &[u8], cell_start: usize) -> bool {
    let Some(next) = row_content.get(cell_start + 2) else {
        return false;
    };

    next.is_ascii_whitespace() || *next == b'/' || *next == b'>'
}

fn is_self_closing_opening_tag(opening_tag: &[u8]) -> bool {
    if opening_tag.last() != Some(&b'>') {
        return false;
    }

    let mut idx = opening_tag.len().saturating_sub(1);
    while idx > 0 && opening_tag[idx - 1].is_ascii_whitespace() {
        idx -= 1;
    }

    opening_tag.get(idx - 1) == Some(&b'/')
}

fn cell_attributes_from_opening_tag(opening_tag: &[u8]) -> Option<&[u8]> {
    if !opening_tag.starts_with(b"<c") || opening_tag.last() != Some(&b'>') {
        return None;
    }

    let mut end = opening_tag.len() - 1;
    while end > 2 && opening_tag[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end > 2 && opening_tag[end - 1] == b'/' {
        end -= 1;
    }

    Some(&opening_tag[2..end])
}

type ParsedCellAttributes<'a> = (
    Option<&'a [u8]>,
    Option<&'a [u8]>,
    Option<&'a [u8]>,
    Option<u32>,
    Option<u32>,
    Option<u32>,
);

fn parse_cell_view_attributes(attributes: &[u8]) -> ParsedCellAttributes<'_> {
    let mut reference = None;
    let mut column_ref = None;
    let mut row_ref = None;
    let mut column = None;
    let mut row = None;
    let mut style_id = None;

    let mut pos = 0;
    while pos < attributes.len() {
        while pos < attributes.len() && attributes[pos].is_ascii_whitespace() {
            pos += 1;
        }

        let name_start = pos;
        while pos < attributes.len() && is_xml_attr_name_byte(attributes[pos]) {
            pos += 1;
        }

        let name = &attributes[name_start..pos];
        if name.is_empty() {
            pos += 1;
            continue;
        }

        while pos < attributes.len() && attributes[pos].is_ascii_whitespace() {
            pos += 1;
        }

        if attributes.get(pos) != Some(&b'=') {
            continue;
        }
        pos += 1;

        while pos < attributes.len() && attributes[pos].is_ascii_whitespace() {
            pos += 1;
        }

        if attributes.get(pos) != Some(&b'"') {
            continue;
        }
        pos += 1;

        let value_start = pos;
        while pos < attributes.len() && attributes[pos] != b'"' {
            pos += 1;
        }
        if pos >= attributes.len() {
            break;
        }

        let value = &attributes[value_start..pos];
        pos += 1;

        match name {
            b"r" => {
                reference = Some(value);
                let (col_bytes, row_bytes) = split_cell_reference(value);
                if !col_bytes.is_empty() && !row_bytes.is_empty() {
                    column_ref = Some(col_bytes);
                    row_ref = Some(row_bytes);
                    column = Some(letters_byte_string_to_integer(col_bytes));
                    row = Some(digits_byte_string_to_integer(row_bytes));
                }
            }
            b"s" => {
                style_id = Some(digits_byte_string_to_integer(value));
            }
            _ => {}
        }
    }

    (reference, column_ref, row_ref, column, row, style_id)
}

fn is_xml_attr_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'.' | b'-')
}

fn split_cell_reference(reference: &[u8]) -> (&[u8], &[u8]) {
    let split_at = reference
        .iter()
        .position(|byte| byte.is_ascii_digit())
        .unwrap_or(reference.len());

    (&reference[..split_at], &reference[split_at..])
}

fn cell_has_content(cell: &[u8]) -> bool {
    find_bytes(cell, b"<v>").is_some()
        || find_bytes(cell, b"<f>").is_some()
        || find_bytes(cell, b"<is>").is_some()
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

fn generate_and_log_self_closing_row(
    row_attr: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_attributes: RowAttributes,
) -> Vec<u8> {
    metrics.kept_row_states.push(row_attributes);

    let cleaned_row_attr = remove_irrelevant_attributes_for_empty_rows(row_attr, params);

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
