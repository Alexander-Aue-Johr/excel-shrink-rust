use crate::error::{Result, ShrinkError};
use crate::models::{CleanSheetMetrics, CleanSheetParameters, ColumnRangeMap};
use crate::sheet::hidden_rows::{remove_hidden_rows_if_present_with_state, HiddenRowCollapseState};
use crate::sheet::rows::{replace_empty_row_after_cell_range, replace_row};
use crate::xml::{find_byte_from, find_bytes};
use regex::bytes::{Captures, Regex};
use std::collections::HashSet;
use std::sync::OnceLock;

static PROCESS_ROWS_RE: OnceLock<Regex> = OnceLock::new();
static PROCESS_EMPTY_ROWS_IGNORING_HEIGHT_RE: OnceLock<Regex> = OnceLock::new();
static SELF_CLOSING_EMPTY_CELL_RE: OnceLock<Regex> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct SheetDataPreprocessState {
    any_column_has_non_default_fill_or_border: bool,
    active_row_has_non_default_fill_or_border_style: bool,

    hidden_row_collapse_state: HiddenRowCollapseState,

    /// Style IDs whose XF has fillId != 0 or borderId != 0.
    /// Keys are the raw ASCII bytes of the style index, e.g. b"429".
    fill_or_border_style_ids_by_bytes: HashSet<Vec<u8>>,

    /// Style IDs that are safe to remove from empty self-closing cells.
    /// These are style IDs where the XF is only font-like / non-visible.
    only_font_style_ids_by_bytes: HashSet<Vec<u8>>,

    /// Cache for row style IDs we have already checked.
    checked_row_style_ids: HashSet<Vec<u8>>,
}

impl SheetDataPreprocessState {
    pub fn new(column_definitions: &ColumnRangeMap, params: &CleanSheetParameters) -> Self {
        let any_column_has_non_default_fill_or_border =
            column_definitions.has_any_non_default_fill_or_border_style();

        let mut fill_or_border_style_ids_by_bytes = HashSet::new();
        let mut only_font_style_ids_by_bytes = HashSet::new();

        for (style_id, xf) in &params.patterns.xfs_by_index {
            let style_id_bytes = style_id.to_string().into_bytes();

            if xf.fill_id.unwrap_or(0) != 0 || xf.border_id.unwrap_or(0) != 0 {
                fill_or_border_style_ids_by_bytes.insert(style_id_bytes.clone());
            }

            if xf.has_only_font_style {
                only_font_style_ids_by_bytes.insert(style_id_bytes);
            }
        }

        Self {
            any_column_has_non_default_fill_or_border,
            active_row_has_non_default_fill_or_border_style: false,
            hidden_row_collapse_state: HiddenRowCollapseState::default(),
            fill_or_border_style_ids_by_bytes,
            only_font_style_ids_by_bytes,
            checked_row_style_ids: HashSet::new(),
        }
    }

    fn row_style_id_has_non_default_fill_or_border(&mut self, style_id: &[u8]) -> bool {
        if !self.checked_row_style_ids.contains(style_id) {
            self.checked_row_style_ids.insert(style_id.to_vec());
        }

        self.fill_or_border_style_ids_by_bytes.contains(style_id)
    }

    fn empty_cell_style_id_is_removable(&self, style_id: &[u8]) -> bool {
        self.only_font_style_ids_by_bytes.contains(style_id)
    }
}

pub fn pre_process_sheet_data_chunk(
    chunk: &[u8],
    params: &CleanSheetParameters,
    state: &mut SheetDataPreprocessState,
) -> Result<Vec<u8>> {
    if chunk.windows(b"<![CDATA[".len()).any(|w| w == b"<![CDATA[") {
        return Err(ShrinkError::UnsupportedCData);
    }

    let chunk = if params.preprocess.remove_hidden_rows {
        remove_hidden_rows_if_present_with_state(chunk, &mut state.hidden_row_collapse_state)
    } else {
        chunk.to_vec()
    };

    if !params.preprocess.remove_empty_cells {
        return Ok(chunk);
    }

    let skip_empty_cell_cleanup = should_skip_empty_cell_cleanup_for_chunk(&chunk, state);

    if skip_empty_cell_cleanup {
        return Ok(chunk);
    }

    Ok(remove_consecutive_empty_cells(&chunk, state))
}

pub fn clean_sheet_rows_and_cells_in_cell_range(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Result<Vec<u8>> {
    let row_re = PROCESS_ROWS_RE.get_or_init(|| {
        Regex::new(
            r#"(?s)<row\b((?:[^>]*\br="([^"]+?)")?(?:[^>]*\bhidden="(1)")?(?:[^>]*\bs="([^"]+?)")?(?:[^>]*\bht="([^"]+?)")?[^/>]*?)(?:/>|>(.*?)</row>)"#,
        )
        .unwrap()
    });

    Ok(row_re
        .replace_all(sheet_data, |caps: &Captures| {
            replace_row(caps, params, metrics)
        })
        .into_owned())
}

pub fn clean_empty_rows_from_after_cell_range(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Result<Vec<u8>> {
    let row_re = PROCESS_EMPTY_ROWS_IGNORING_HEIGHT_RE.get_or_init(|| {
        Regex::new(
            r#"(?s)<row\b((?:[^>]*\br="([^"]+?)")?(?:[^>]*\bhidden="(1)")?(?:[^>]*\bs="([^"]+?)")?(?:[^>]*\bht="([^"]+?)")?[^/>]*?)(?:/>|></row>)"#,
        )
        .unwrap()
    });

    Ok(row_re
        .replace_all(sheet_data, |caps: &Captures| {
            replace_empty_row_after_cell_range(caps, params, metrics)
        })
        .into_owned())
}

fn should_skip_empty_cell_cleanup_for_chunk(
    chunk: &[u8],
    state: &mut SheetDataPreprocessState,
) -> bool {
    let chunk_has_risky_active_or_declared_row_style =
        scan_row_styles_for_empty_cell_cleanup_risk(chunk, state);

    state.any_column_has_non_default_fill_or_border || chunk_has_risky_active_or_declared_row_style
}

fn scan_row_styles_for_empty_cell_cleanup_risk(
    chunk: &[u8],
    state: &mut SheetDataPreprocessState,
) -> bool {
    let mut chunk_is_risky = state.active_row_has_non_default_fill_or_border_style;
    let mut pos = 0;

    while let Some(row_start_rel) = find_bytes(&chunk[pos..], b"<row") {
        let row_start = pos + row_start_rel;

        let row_tag_end_exclusive = match find_byte_from(chunk, b'>', row_start) {
            Some(tag_end) => tag_end + 1,
            None => chunk.len(),
        };

        let row_tag = &chunk[row_start..row_tag_end_exclusive];

        let row_has_non_default_fill_or_border_style = extract_xml_attr_value(row_tag, b"s")
            .is_some_and(|style_id| state.row_style_id_has_non_default_fill_or_border(style_id));

        if row_has_non_default_fill_or_border_style {
            chunk_is_risky = true;
        }

        let row_start_tag_is_self_closing =
            row_tag_end_exclusive >= 2 && chunk[row_tag_end_exclusive - 2] == b'/';

        let row_continues_beyond_this_chunk = if row_start_tag_is_self_closing {
            false
        } else {
            find_bytes(&chunk[row_tag_end_exclusive..], b"</row>").is_none()
        };

        state.active_row_has_non_default_fill_or_border_style =
            row_continues_beyond_this_chunk && row_has_non_default_fill_or_border_style;

        if row_tag_end_exclusive <= row_start {
            break;
        }

        pos = row_tag_end_exclusive;
    }

    chunk_is_risky
}

fn remove_consecutive_empty_cells(chunk: &[u8], state: &SheetDataPreprocessState) -> Vec<u8> {
    if find_bytes(chunk, b"<c").is_none() {
        return chunk.to_vec();
    }

    let cell_re =
        SELF_CLOSING_EMPTY_CELL_RE.get_or_init(|| Regex::new(r#"<c\b([^>]*)/>"#).unwrap());

    cell_re
        .replace_all(chunk, |caps: &Captures| {
            let attr = caps.get(1).map_or(b"".as_slice(), |m| m.as_bytes());

            let removable = match extract_xml_attr_value(attr, b"s") {
                None => true,
                Some(style_id) => state.empty_cell_style_id_is_removable(style_id),
            };

            if removable {
                Vec::new()
            } else {
                caps.get(0).unwrap().as_bytes().to_vec()
            }
        })
        .into_owned()
}

fn extract_xml_attr_value<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let mut needle = Vec::with_capacity(attr_name.len() + 2);
    needle.extend_from_slice(attr_name);
    needle.extend_from_slice(b"=\"");

    let mut search_start = 0;

    while search_start < data.len() {
        let Some(pos_rel) = find_bytes(&data[search_start..], &needle) else {
            return None;
        };

        let pos = search_start + pos_rel;

        let has_attribute_boundary = pos == 0 || data[pos - 1].is_ascii_whitespace();

        if !has_attribute_boundary {
            search_start = pos + 1;
            continue;
        }

        let value_start = pos + needle.len();

        let value_end_rel = data[value_start..].iter().position(|b| *b == b'"')?;

        return Some(&data[value_start..value_start + value_end_rel]);
    }

    None
}
