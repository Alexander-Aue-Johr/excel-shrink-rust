use crate::models::{CleanSheetMetrics, CleanSheetParameters, CleanupCause, RowAttributes};
use crate::sheet::cells::{
    parsed_cell_has_content, parsed_empty_cell_has_effective_fill_or_border,
    replace_parsed_cell_with_empty_style_policy, ParsedCell,
};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use crate::xml::{find_byte_from, find_bytes, find_bytes_from, rfind_bytes};
use regex::bytes::Captures;
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmptyStyleRepresentativePolicy {
    keep_default_styleless: bool,
    keep_font_styles: bool,
    require_content_in_row: bool,
}

struct CleanCellsResult<'a> {
    row_content: Cow<'a, [u8]>,
    tentative_style_representatives: Vec<TentativeStyleRepresentative>,
    cells_removed: bool,
}

#[derive(Debug, Clone)]
struct TentativeStyleRepresentative {
    reference_attr: Vec<u8>,
    col: u32,
    row: u32,
}

struct RepresentativeRemovalResult {
    row_content: Vec<u8>,
    removed: Vec<RemovedStyleRepresentative>,
}

struct RemovedStyleRepresentative {
    representative: TentativeStyleRepresentative,
    bytes_removed: usize,
}

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

    replace_row_bytes(row_match, row_attr, row_content, params, metrics).into_owned()
}

pub fn replace_row_bytes<'a>(
    row_raw: &'a [u8],
    row_attr: &'a [u8],
    row_content: Option<&'a [u8]>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    let Some(row_attributes) = extract_row_attributes_from_parts(row_attr, row_content, params)
    else {
        return Cow::Borrowed(row_raw);
    };

    let is_hidden = row_attributes.hidden;
    let original_row_content = row_content.unwrap_or_default();

    metrics
        .row_attributes
        .insert(row_attributes.ref_.clone(), row_attributes.clone());
    record_visible_row_bounds(&row_attributes, metrics);

    let empty_style_representative_policy = EmptyStyleRepresentativePolicy {
        keep_default_styleless: params.cleanup.keep_textless_default_font_representatives,
        keep_font_styles: params.cleanup.keep_textless_style_representatives,
        require_content_in_row: row_attributes.has_custom_height,
    };
    let was_already_empty = original_row_content.is_empty();
    let mut cells_removed = false;

    let row_content = if !was_already_empty {
        let clean_result = clean_cells(
            params,
            metrics,
            original_row_content,
            empty_style_representative_policy,
        );
        let CleanCellsResult {
            row_content,
            tentative_style_representatives,
            cells_removed: clean_cells_removed,
        } = clean_result;
        cells_removed = clean_cells_removed;

        if empty_style_representative_policy.require_content_in_row
            && !metrics
                .rows_with_kept_content_cells
                .contains(row_attributes.ref_.as_slice())
        {
            if tentative_style_representatives.is_empty() {
                row_content
            } else {
                let removal = remove_tentative_style_representatives(
                    row_content.into_owned(),
                    tentative_style_representatives,
                );
                for removed in &removal.removed {
                    metrics.cleanup_stats.record_removed(
                        CleanupCause::EmptyCellsWithoutContent,
                        removed.bytes_removed,
                    );
                }
                remove_tentative_representatives_from_metrics(metrics, &removal.removed);
                cells_removed = cells_removed || !removal.removed.is_empty();
                Cow::Owned(removal.row_content)
            }
        } else {
            row_content
        }
    } else {
        Cow::Borrowed(original_row_content)
    };

    let is_empty = row_content.is_empty();

    if is_empty {
        if is_hidden {
            return generate_and_log_self_closing_row(row_attr, params, metrics, row_attributes)
                .into();
        }

        return generate_and_log_self_closing_row(row_attr, params, metrics, row_attributes).into();
    }

    if matches!(&row_content, Cow::Borrowed(_)) {
        metrics.kept_row_states.push(row_attributes);
        Cow::Borrowed(row_raw)
    } else {
        generate_and_log_row(
            row_attr,
            row_content.as_ref(),
            metrics,
            row_attributes,
            cells_removed,
        )
        .into()
    }
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

    replace_empty_row_after_cell_range_bytes(row_match, row_attr, params, metrics).into_owned()
}

pub fn replace_empty_row_after_cell_range_bytes<'a>(
    row_raw: &'a [u8],
    row_attr: &'a [u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    let Some(row_attributes) = extract_empty_row_attributes_from_attr(row_attr, params) else {
        return Cow::Borrowed(row_raw);
    };

    let is_hidden = row_attributes.hidden;

    metrics
        .row_attributes
        .insert(row_attributes.ref_.clone(), row_attributes.clone());
    record_visible_row_bounds(&row_attributes, metrics);

    if is_hidden {
        return generate_and_log_self_closing_row(row_attr, params, metrics, row_attributes).into();
    }

    generate_and_log_self_closing_row(row_attr, params, metrics, row_attributes).into()
}

fn clean_cells<'a>(
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    row_content: &'a [u8],
    empty_style_representative_policy: EmptyStyleRepresentativePolicy,
) -> CleanCellsResult<'a> {
    if row_content.is_empty() {
        return CleanCellsResult {
            row_content: Cow::Borrowed(row_content),
            tentative_style_representatives: Vec::new(),
            cells_removed: false,
        };
    }

    let mut out: Option<Vec<u8>> = None;
    let mut pos = 0;
    let mut cells_removed = false;
    let mut kept_default_style_cell = false;
    let mut tentative_style_representatives = Vec::new();
    let mut kept_represented_style_ids = std::collections::HashSet::<u32>::new();

    while let Some(cell_start_rel) = find_bytes(&row_content[pos..], b"<c") {
        let cell_start = pos + cell_start_rel;

        if !is_cell_tag_start(row_content, cell_start) {
            if let Some(out) = out.as_mut() {
                out.extend_from_slice(&row_content[pos..cell_start + b"<c".len()]);
            }
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

        let attr = &row_content[attr_start..attr_end];
        let cleaned_cell = if let Some(cell) = parse_cell_from_parts(raw, attr) {
            let has_cell_content = parsed_cell_has_content(&cell);
            #[cfg(feature = "cell-tracing")]
            crate::cleaning_trace::record_cell_visit_if_target(
                crate::cleaning_trace::CellVisitEvent {
                    sheet_file: &params.zip_info_filename,
                    col: cell.col_num,
                    row: cell.row,
                    style_id: cell.style_id,
                },
            );

            let keep_empty_font_only_style_cell = empty_style_representative_policy
                .keep_font_styles
                && empty_cell_has_no_empty_cell_visible_style(&cell, params, has_cell_content)
                && cell
                    .style_id
                    .is_some_and(|style_id| !kept_represented_style_ids.contains(&style_id));
            let keep_default_empty_cell = empty_style_representative_policy.keep_default_styleless
                && !kept_default_style_cell
                && empty_cell_has_default_style(&cell, has_cell_content);
            let has_effective_fill_or_border = parsed_empty_cell_has_effective_fill_or_border(
                &cell,
                params,
                metrics,
                has_cell_content,
            );
            let cleaned_cell = replace_parsed_cell_with_empty_style_policy(
                &cell,
                params,
                metrics,
                has_cell_content,
                keep_empty_font_only_style_cell,
                false,
                keep_default_empty_cell,
            );

            if !cleaned_cell.as_ref().is_empty() && !has_cell_content {
                record_represented_cell_style(
                    &cell,
                    &mut kept_represented_style_ids,
                    &mut kept_default_style_cell,
                );
                if keep_empty_font_only_style_cell || keep_default_empty_cell {
                    if empty_style_representative_policy.require_content_in_row
                        && !has_effective_fill_or_border
                    {
                        tentative_style_representatives.push(TentativeStyleRepresentative {
                            reference_attr: cell.reference_attr.to_vec(),
                            col: cell.col_num,
                            row: digits_byte_string_to_integer(cell.row),
                        });
                    }
                }
            }

            cleaned_cell
        } else {
            Cow::Borrowed(raw)
        };

        if cleaned_cell.as_ref().is_empty() {
            cells_removed = true;
        }

        if let Some(out) = out.as_mut() {
            out.extend_from_slice(&row_content[pos..cell_start]);
            out.extend_from_slice(cleaned_cell.as_ref());
        } else if matches!(&cleaned_cell, Cow::Owned(_)) {
            let mut new_out = Vec::with_capacity(row_content.len().min(64 * 1024));
            new_out.extend_from_slice(&row_content[..cell_start]);
            new_out.extend_from_slice(cleaned_cell.as_ref());
            out = Some(new_out);
        }

        pos = cell_end;
    }

    let row_content = if let Some(mut out) = out {
        out.extend_from_slice(&row_content[pos..]);
        Cow::Owned(out)
    } else {
        Cow::Borrowed(row_content)
    };

    CleanCellsResult {
        row_content,
        tentative_style_representatives,
        cells_removed,
    }
}

fn record_represented_cell_style(
    cell: &ParsedCell<'_>,
    kept_represented_style_ids: &mut std::collections::HashSet<u32>,
    kept_default_style_cell: &mut bool,
) {
    if let Some(style_id) = cell.style_id {
        kept_represented_style_ids.insert(style_id);
    } else {
        *kept_default_style_cell = true;
    }
}

fn remove_tentative_style_representatives(
    mut row_content: Vec<u8>,
    representatives: Vec<TentativeStyleRepresentative>,
) -> RepresentativeRemovalResult {
    let mut removed = Vec::new();
    let mut search_start = 0usize;

    for representative in representatives {
        let Some(reference_start) =
            find_bytes_from(&row_content, &representative.reference_attr, search_start)
        else {
            continue;
        };
        let Some(cell_start) = find_cell_start_before_reference(&row_content, reference_start)
        else {
            search_start = reference_start + representative.reference_attr.len();
            continue;
        };
        let Some(cell_end) = find_cell_end_from_start(&row_content, cell_start) else {
            search_start = reference_start + representative.reference_attr.len();
            continue;
        };

        let bytes_removed = cell_end - cell_start;
        row_content.drain(cell_start..cell_end);
        removed.push(RemovedStyleRepresentative {
            representative,
            bytes_removed,
        });
        search_start = cell_start;
    }

    RepresentativeRemovalResult {
        row_content,
        removed,
    }
}

fn find_cell_start_before_reference(row_content: &[u8], reference_start: usize) -> Option<usize> {
    let mut search_end = reference_start;
    while let Some(cell_start) = rfind_bytes(&row_content[..search_end], b"<c") {
        if is_cell_tag_start(row_content, cell_start) {
            return Some(cell_start);
        }
        search_end = cell_start;
    }
    None
}

fn find_cell_end_from_start(row_content: &[u8], cell_start: usize) -> Option<usize> {
    let tag_end = find_byte_from(row_content, b'>', cell_start)?;
    if tag_end > cell_start && row_content[tag_end - 1] == b'/' {
        return Some(tag_end + 1);
    }
    find_bytes_from(row_content, b"</c>", tag_end + 1).map(|end| end + b"</c>".len())
}

fn remove_tentative_representatives_from_metrics(
    metrics: &mut CleanSheetMetrics,
    removed: &[RemovedStyleRepresentative],
) {
    let removed_tail_anchor = metrics.tail_anchor_max_row.is_some_and(|tail_row| {
        removed
            .iter()
            .any(|removed| removed.representative.row == tail_row)
    });

    for removed in removed {
        if let Some(index) = metrics.kept_cell_states.iter().position(|cell| {
            cell.col == removed.representative.col && cell.row == removed.representative.row
        }) {
            metrics.kept_cell_states.remove(index);
        }
        metrics
            .deleted_cell_states
            .push(crate::models::CellAttributes {
                col: removed.representative.col,
                row: removed.representative.row,
                kept_only_for_protection: false,
            });
    }

    if removed_tail_anchor {
        metrics.tail_anchor_max_row = None;
    }
}

fn empty_cell_has_no_empty_cell_visible_style(
    cell: &ParsedCell<'_>,
    params: &CleanSheetParameters,
    has_cell_content: bool,
) -> bool {
    if has_cell_content {
        return false;
    }

    cell.style_id
        .and_then(|style_id| params.patterns.xfs_by_index.get(&style_id))
        .is_some_and(|style| style.has_no_empty_cell_visible_style)
}

fn empty_cell_has_default_style(cell: &ParsedCell<'_>, has_cell_content: bool) -> bool {
    cell.style_id.is_none() && !has_cell_content
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
    let reference_attr = extract_attr_assignment(attr, b"r")?;

    if !column.iter().all(|byte| byte.is_ascii_uppercase())
        || !row.iter().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let style_id = extract_attr_value(attr, b"s").map(digits_byte_string_to_integer);

    Some(ParsedCell {
        raw,
        attr,
        reference_attr,
        row,
        col_num: letters_byte_string_to_integer(column),
        style_id,
    })
}

fn extract_attr_assignment<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
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
            return Some(&data[attr_start..value_start + value_end + 1]);
        }

        search_start = attr_start + attr_name.len();
    }

    None
}

fn extract_row_attributes_from_parts(
    row_attr: &[u8],
    row_content: Option<&[u8]>,
    params: &CleanSheetParameters,
) -> Option<RowAttributes> {
    let row_attr_vec = row_attr.to_vec();
    let ref_ = extract_attr_value(row_attr, b"r")
        .map(|value| value.to_vec())
        .unwrap_or_default();
    let hidden = extract_attr_value(row_attr, b"hidden").is_some_and(|value| value == b"1");
    let style_id = extract_attr_value(row_attr, b"s").map(digits_byte_string_to_integer);
    let ht = extract_attr_value(row_attr, b"ht").is_some();
    let custom_height =
        extract_attr_value(row_attr, b"customHeight").is_some_and(|value| value == b"1");
    let style = style_id.and_then(|id| params.patterns.xfs_by_index.get(&id).cloned());

    Some(RowAttributes {
        row_attr: row_attr_vec,
        ref_,
        hidden,
        style,
        has_height: ht,
        has_custom_height: custom_height,
        is_empty: row_content.is_none_or(|content| content.is_empty()),
    })
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
    let custom_height =
        extract_attr_value(row_attr, b"customHeight").is_some_and(|value| value == b"1");
    let style = style_id.and_then(|id| params.patterns.xfs_by_index.get(&id).cloned());

    Some(RowAttributes {
        row_attr: row_attr_vec,
        ref_,
        hidden,
        style,
        has_height: ht,
        has_custom_height: custom_height,
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
    metrics.tail_anchor_max_row = Some(
        metrics
            .tail_anchor_max_row
            .map_or(row_num, |old| old.max(row_num)),
    );
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
        Cow::Borrowed(row_attr)
    };

    metrics.cleanup_stats.record_delta(
        CleanupCause::EmptyRowAttributeCleanup,
        row_attr.len(),
        cleaned_row_attr.as_ref().len(),
    );

    let mut out = Vec::with_capacity(cleaned_row_attr.as_ref().len() + 7);
    out.extend_from_slice(b"<row");
    out.extend_from_slice(cleaned_row_attr.as_ref());
    out.extend_from_slice(b"/>");
    out
}

fn generate_and_log_row(
    row_attr: &[u8],
    row_content: &[u8],
    metrics: &mut CleanSheetMetrics,
    row_attributes: RowAttributes,
    rewrite_spans: bool,
) -> Vec<u8> {
    let row_attr = if rewrite_spans {
        rewrite_row_spans_upper_bound(row_attr, metrics, &row_attributes)
    } else {
        Cow::Borrowed(row_attr)
    };
    metrics.kept_row_states.push(row_attributes);

    let mut out = Vec::with_capacity(row_attr.as_ref().len() + row_content.len() + 11);
    out.extend_from_slice(b"<row");
    out.extend_from_slice(row_attr.as_ref());
    out.extend_from_slice(b">");
    out.extend_from_slice(row_content);
    out.extend_from_slice(b"</row>");
    out
}

fn rewrite_row_spans_upper_bound<'a>(
    row_attr: &'a [u8],
    metrics: &CleanSheetMetrics,
    row_attributes: &RowAttributes,
) -> Cow<'a, [u8]> {
    let row_number = digits_byte_string_to_integer(&row_attributes.ref_);
    if row_number == 0 {
        return Cow::Borrowed(row_attr);
    }

    let Some(max_kept_col) = metrics
        .kept_cell_states
        .iter()
        .filter(|cell| cell.row == row_number)
        .map(|cell| cell.col)
        .max()
    else {
        return Cow::Borrowed(row_attr);
    };

    rewrite_spans_attr_upper_bound(row_attr, max_kept_col)
}

fn rewrite_spans_attr_upper_bound(row_attr: &[u8], max_kept_col: u32) -> Cow<'_, [u8]> {
    let Some(value_range) = find_attr_value_range(row_attr, b"spans") else {
        return Cow::Borrowed(row_attr);
    };

    let value = &row_attr[value_range.clone()];
    let Some(colon) = value.iter().position(|byte| *byte == b':') else {
        return Cow::Borrowed(row_attr);
    };

    let upper = max_kept_col.to_string();
    if &value[colon + 1..] == upper.as_bytes() {
        return Cow::Borrowed(row_attr);
    }

    let mut out =
        Vec::with_capacity(row_attr.len() - value.len() + colon + 1 + upper.as_bytes().len());
    out.extend_from_slice(&row_attr[..value_range.start]);
    out.extend_from_slice(&value[..colon + 1]);
    out.extend_from_slice(upper.as_bytes());
    out.extend_from_slice(&row_attr[value_range.end..]);
    Cow::Owned(out)
}

fn find_attr_value_range(data: &[u8], attr_name: &[u8]) -> Option<std::ops::Range<usize>> {
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
            return Some(value_start..value_start + value_end);
        }

        search_start = attr_start + attr_name.len();
    }

    None
}

fn remove_irrelevant_attributes_for_empty_rows<'a>(
    row_attr: &'a [u8],
    _params: &CleanSheetParameters,
) -> Cow<'a, [u8]> {
    Cow::Borrowed(row_attr)
}

#[cfg(test)]
#[path = "rows_regression_tests.rs"]
mod regression_tests;
