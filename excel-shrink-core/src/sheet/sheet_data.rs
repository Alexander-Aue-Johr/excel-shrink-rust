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
        .replace_all(&sheet_data, |caps: &Captures| {
            replace_row(caps, params, metrics)
        })
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
