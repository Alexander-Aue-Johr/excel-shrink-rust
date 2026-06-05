use crate::error::{Result, ShrinkError};
use crate::models::{CleanSheetMetrics, CleanSheetParameters};
use crate::sheet::hidden_rows::remove_hidden_rows_if_present;
use crate::sheet::rows::{replace_empty_row_after_cell_range, replace_row};
use regex::bytes::{Captures, Regex};
use std::sync::OnceLock;

static PROCESS_ROWS_RE: OnceLock<Regex> = OnceLock::new();
static PROCESS_EMPTY_ROWS_IGNORING_HEIGHT_RE: OnceLock<Regex> = OnceLock::new();

pub fn pre_process_sheet_data_chunk(
    chunk: &[u8],
    params: &CleanSheetParameters,
) -> Result<Vec<u8>> {
    if chunk.windows(b"<![CDATA[".len()).any(|w| w == b"<![CDATA[") {
        return Err(ShrinkError::UnsupportedCData);
    }

    let chunk = remove_hidden_rows_if_present(chunk, params);
    let chunk = replace_whitespace_cells(&chunk, params);
    let chunk = remove_consecutive_empty_cells(&chunk, params);

    Ok(chunk)
}

pub fn clean_sheet_rows_and_cells_in_cell_range(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Result<Vec<u8>> {
    let sheet_data = pre_process_sheet_data_chunk(sheet_data, params)?;

    let row_re = PROCESS_ROWS_RE.get_or_init(|| {
        Regex::new(
            r#"(?s)<row\b((?:[^>]*\br="([^"]+?)")?(?:[^>]*\bhidden="(1)")?(?:[^>]*\bs="([^"]+?)")?(?:[^>]*\bht="([^"]+?)")?[^/>]*?)(?:/>|>(.*?)</row>)"#,
        )
        .unwrap()
    });

    Ok(row_re
        .replace_all(&sheet_data, |caps: &Captures| replace_row(caps, params, metrics))
        .into_owned())
}

pub fn clean_empty_rows_from_after_cell_range(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Result<Vec<u8>> {
    let sheet_data = pre_process_sheet_data_chunk(sheet_data, params)?;

    let row_re = PROCESS_EMPTY_ROWS_IGNORING_HEIGHT_RE.get_or_init(|| {
        Regex::new(
            r#"(?s)<row\b((?:[^>]*\br="([^"]+?)")?(?:[^>]*\bhidden="(1)")?(?:[^>]*\bs="([^"]+?)")?[^/>]*?)(?:/>|></row>)"#,
        )
        .unwrap()
    });

    Ok(row_re
        .replace_all(&sheet_data, |caps: &Captures| {
            replace_empty_row_after_cell_range(caps, params, metrics)
        })
        .into_owned())
}

fn replace_whitespace_cells(chunk: &[u8], params: &CleanSheetParameters) -> Vec<u8> {
    let Some(indices) = &params.patterns.whitespace_indices else {
        return chunk.to_vec();
    };

    if indices.is_empty() {
        return chunk.to_vec();
    }

    let mut out = Vec::with_capacity(chunk.len());
    let mut pos = 0;

    while let Some(cell_start_relative) = crate::xml::find_bytes(&chunk[pos..], b"<c") {
        let cell_start = pos + cell_start_relative;
        out.extend_from_slice(&chunk[pos..cell_start]);

        let Some(cell_end_relative) = crate::xml::find_bytes(&chunk[cell_start..], b"</c>") else {
            out.extend_from_slice(&chunk[cell_start..]);
            return out;
        };

        let cell_end = cell_start + cell_end_relative + b"</c>".len();
        let cell = &chunk[cell_start..cell_end];

        if is_whitespace_shared_string_cell(cell, indices) {
            if let Some(open_end) = cell.iter().position(|b| *b == b'>') {
                out.extend_from_slice(&cell[..open_end]);
                out.extend_from_slice(b"/>");
            } else {
                out.extend_from_slice(cell);
            }
        } else {
            out.extend_from_slice(cell);
        }

        pos = cell_end;
    }

    out.extend_from_slice(&chunk[pos..]);
    out
}

fn is_whitespace_shared_string_cell(cell: &[u8], indices: &[usize]) -> bool {
    if !cell.windows(b"t=\"s\"".len()).any(|w| w == b"t=\"s\"") {
        return false;
    }

    let Some(v_start_rel) = crate::xml::find_bytes(cell, b"<v>") else {
        return false;
    };
    let value_start = v_start_rel + b"<v>".len();

    let Some(v_end_rel) = crate::xml::find_bytes(&cell[value_start..], b"</v>") else {
        return false;
    };

    let value = &cell[value_start..value_start + v_end_rel];

    let Ok(value_str) = std::str::from_utf8(value) else {
        return false;
    };

    let Ok(index) = value_str.parse::<usize>() else {
        return false;
    };

    indices.contains(&index)
}

fn remove_consecutive_empty_cells(chunk: &[u8], params: &CleanSheetParameters) -> Vec<u8> {
    // Erst einmal konservativ: einzelne empty cells entfernen, wenn sie keinen relevanten Style haben.
    // Das ist noch nicht ganz so aggressiv wie das Python-Pattern für "consecutive", aber sicherer als falsches Löschen.
    let cell_re = Regex::new(r#"<c\b([^>]*)/>"#).unwrap();

    cell_re
        .replace_all(chunk, |caps: &Captures| {
            let attr = caps.get(1).unwrap().as_bytes();

            let style_id = extract_style_id(attr);

            let removable = match style_id {
                None => true,
                Some(id) => params
                    .patterns
                    .xfs_by_index
                    .get(&id)
                    .is_some_and(|xf| xf.has_only_font_style),
            };

            if removable {
                Vec::new()
            } else {
                caps.get(0).unwrap().as_bytes().to_vec()
            }
        })
        .into_owned()
}

fn extract_style_id(attr: &[u8]) -> Option<u32> {
    let needle = b"s=\"";
    let pos = crate::xml::find_bytes(attr, needle)?;
    let start = pos + needle.len();
    let end_rel = attr[start..].iter().position(|b| *b == b'"')?;
    let value = &attr[start..start + end_rel];
    std::str::from_utf8(value).ok()?.parse().ok()
}