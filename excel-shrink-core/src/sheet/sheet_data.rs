use crate::error::{Result, ShrinkError};
use crate::models::{CleanSheetMetrics, CleanSheetParameters, CleanupCause, ColumnRangeMap};
use crate::sheet::rows::{replace_empty_row_after_cell_range_bytes, replace_row_bytes};
use crate::xml::{find_byte_from, find_bytes};
use std::borrow::Cow;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct SheetDataPreprocessState {
    any_column_has_non_default_fill_or_border: bool,
    active_row_has_non_default_fill_or_border_style: bool,

    /// Style IDs whose XF can apply a visible fill or border to empty cells.
    /// Keys are the raw ASCII bytes of the style index, e.g. b"429".
    fill_or_border_style_ids_by_bytes: HashSet<Vec<u8>>,

    /// Cache for row style IDs we have already checked.
    checked_row_style_ids: HashSet<Vec<u8>>,
}

impl SheetDataPreprocessState {
    pub fn new(column_definitions: &ColumnRangeMap, params: &CleanSheetParameters) -> Self {
        let any_column_has_non_default_fill_or_border =
            column_definitions.has_any_non_default_fill_or_border_style();

        let mut fill_or_border_style_ids_by_bytes = HashSet::new();
        for (style_id, xf) in &params.patterns.xfs_by_index {
            let style_id_bytes = style_id.to_string().into_bytes();

            if xf.applies_visible_fill_or_border() {
                fill_or_border_style_ids_by_bytes.insert(style_id_bytes.clone());
            }
        }

        Self {
            any_column_has_non_default_fill_or_border,
            active_row_has_non_default_fill_or_border_style: false,
            fill_or_border_style_ids_by_bytes,
            checked_row_style_ids: HashSet::new(),
        }
    }

    fn row_style_id_has_non_default_fill_or_border(&mut self, style_id: &[u8]) -> bool {
        if !self.checked_row_style_ids.contains(style_id) {
            self.checked_row_style_ids.insert(style_id.to_vec());
        }

        self.fill_or_border_style_ids_by_bytes.contains(style_id)
    }

    fn empty_cell_style_id_is_removable(&self, _style_id: &[u8]) -> bool {
        false
    }
}

pub fn pre_process_sheet_data_chunk<'a>(
    chunk: &'a [u8],
    params: &CleanSheetParameters,
    state: &mut SheetDataPreprocessState,
    metrics: &mut CleanSheetMetrics,
) -> Result<Cow<'a, [u8]>> {
    if find_bytes(chunk, b"<![CDATA[").is_some() {
        return Err(ShrinkError::UnsupportedCData);
    }

    let chunk = Cow::Borrowed(chunk);

    if !params.cleanup.preprocess_empty_self_closing_cells {
        return Ok(chunk);
    }

    let skip_empty_cell_cleanup = should_skip_empty_cell_cleanup_for_chunk(chunk.as_ref(), state);

    if skip_empty_cell_cleanup {
        return Ok(chunk);
    }

    Ok(
        match remove_consecutive_empty_cells(chunk.as_ref(), state, metrics) {
            Cow::Borrowed(_) => chunk,
            Cow::Owned(cleaned) => Cow::Owned(cleaned),
        },
    )
}

pub fn clean_sheet_rows_and_cells_in_cell_range(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Result<Vec<u8>> {
    Ok(apply_row_byte_substitution(sheet_data, |row| {
        Cow::Owned(replace_row_bytes(
            row.raw,
            row.attr,
            row.content,
            params,
            metrics,
        ))
    }))
}

pub fn clean_empty_rows_from_after_cell_range(
    sheet_data: &[u8],
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Result<Vec<u8>> {
    Ok(apply_row_byte_substitution(sheet_data, |row| {
        if row.content.is_some_and(|content| !content.is_empty()) {
            metrics.kept_row_states.clear();
            return Cow::Borrowed(row.raw);
        }

        Cow::Owned(replace_empty_row_after_cell_range_bytes(
            row.raw, row.attr, params, metrics,
        ))
    }))
}

#[derive(Debug, Clone, Copy)]
struct ParsedRowBlock<'a> {
    raw: &'a [u8],
    attr: &'a [u8],
    content: Option<&'a [u8]>,
}

fn apply_row_byte_substitution<F>(sheet_data: &[u8], mut replace: F) -> Vec<u8>
where
    F: for<'a> FnMut(ParsedRowBlock<'a>) -> Cow<'a, [u8]>,
{
    let mut out = Vec::with_capacity(sheet_data.len().min(64 * 1024 * 1024));
    let mut pos = 0;

    while let Some(row_start_rel) = find_bytes(&sheet_data[pos..], b"<row") {
        let row_start = pos + row_start_rel;

        let Some(tag_end) = find_byte_from(sheet_data, b'>', row_start) else {
            break;
        };

        let attr_start = row_start + b"<row".len();
        let self_closing = tag_end > row_start && sheet_data[tag_end - 1] == b'/';
        let attr_end = if self_closing { tag_end - 1 } else { tag_end };

        let (row_end, content) = if self_closing {
            (tag_end + 1, None)
        } else {
            let content_start = tag_end + 1;
            let Some(close_rel) = find_bytes(&sheet_data[content_start..], b"</row>") else {
                break;
            };
            let content_end = content_start + close_rel;
            (
                content_end + b"</row>".len(),
                Some(&sheet_data[content_start..content_end]),
            )
        };

        extend_without_large_growth(&mut out, &sheet_data[pos..row_start]);
        let row = ParsedRowBlock {
            raw: &sheet_data[row_start..row_end],
            attr: &sheet_data[attr_start..attr_end],
            content,
        };
        let replacement = replace(row);
        extend_without_large_growth(&mut out, replacement.as_ref());
        pos = row_end;
    }

    extend_without_large_growth(&mut out, &sheet_data[pos..]);
    out
}

fn extend_without_large_growth(out: &mut Vec<u8>, bytes: &[u8]) {
    let spare = out.capacity().saturating_sub(out.len());
    if spare < bytes.len() {
        out.reserve_exact((bytes.len() - spare).max(16 * 1024 * 1024));
    }
    out.extend_from_slice(bytes);
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

fn remove_consecutive_empty_cells<'a>(
    chunk: &'a [u8],
    state: &SheetDataPreprocessState,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    if find_bytes(chunk, b"<c").is_none() {
        return Cow::Borrowed(chunk);
    }

    let mut out: Option<Vec<u8>> = None;
    let mut pos = 0;

    while let Some(cell_start_rel) = find_bytes(&chunk[pos..], b"<c") {
        let cell_start = pos + cell_start_rel;

        if !is_cell_tag_start(chunk, cell_start) {
            if let Some(out) = out.as_mut() {
                extend_without_large_growth(out, &chunk[pos..cell_start + b"<c".len()]);
            }
            pos = cell_start + b"<c".len();
            continue;
        }

        let Some(tag_end) = find_byte_from(chunk, b'>', cell_start) else {
            break;
        };

        if tag_end == cell_start || chunk[tag_end - 1] != b'/' {
            if let Some(out) = out.as_mut() {
                extend_without_large_growth(out, &chunk[pos..tag_end + 1]);
            }
            pos = tag_end + 1;
            continue;
        }

        let attr_start = cell_start + b"<c".len();
        let attr_end = tag_end - 1;
        let attr = &chunk[attr_start..attr_end];
        let cell_end = tag_end + 1;

        let removable = match extract_xml_attr_value(attr, b"s") {
            None => true,
            Some(style_id) => state.empty_cell_style_id_is_removable(style_id),
        };

        if removable {
            if out.is_none() {
                let mut new_out = Vec::with_capacity(chunk.len().min(64 * 1024 * 1024));
                new_out.extend_from_slice(&chunk[..cell_start]);
                out = Some(new_out);
            } else if let Some(out) = out.as_mut() {
                extend_without_large_growth(out, &chunk[pos..cell_start]);
            }
            metrics.cleanup_stats.record_removed(
                CleanupCause::PreprocessEmptySelfClosingCells,
                cell_end - cell_start,
            );
        } else if let Some(out) = out.as_mut() {
            extend_without_large_growth(out, &chunk[pos..cell_end]);
        }

        pos = cell_end;
    }

    if let Some(out) = out.as_mut() {
        extend_without_large_growth(out, &chunk[pos..]);
    }

    out.map_or(Cow::Borrowed(chunk), Cow::Owned)
}

fn is_cell_tag_start(data: &[u8], cell_start: usize) -> bool {
    matches!(
        data.get(cell_start + b"<c".len()),
        Some(byte) if byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>')
    )
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

#[cfg(test)]
mod tests {
    use super::{
        pre_process_sheet_data_chunk, remove_consecutive_empty_cells, SheetDataPreprocessState,
    };
    use crate::models::{
        CleanSheetMetrics, CleanSheetParameters, CleanupStats, CleanupSwitches,
        SheetPreprocessConfig, WorkbookSpecificPatterns, Xf,
    };
    use std::collections::HashMap;
    use zip::CompressionMethod;

    fn default_params() -> CleanSheetParameters {
        CleanSheetParameters {
            xlsx_file_path: "test.xlsx".to_string(),
            zip_info_filename: "xl/worksheets/sheet1.xml".to_string(),
            compress_method: CompressionMethod::Deflated,
            patterns: WorkbookSpecificPatterns {
                xfs_by_index: HashMap::new(),
                only_font_style_ids: Vec::new(),
                fill_or_border_or_protection_ids: Vec::new(),
                whitespace_indices: None,
            },
            advanced_cleanup: None,
            preprocess: SheetPreprocessConfig::default(),
            cleanup: CleanupSwitches {
                preprocess_consecutive_hidden_empty_rows: true,
                ..CleanupSwitches::default()
            },
            collect_cleanup_stats: false,
        }
    }

    #[test]
    fn preprocess_preserves_consecutive_hidden_empty_rows() {
        let params = default_params();
        let mut state = SheetDataPreprocessState::new(&Default::default(), &params);
        let mut metrics = CleanSheetMetrics::default();
        let sheet_data = br#"<row r="307905" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307906" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307907" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307908" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row>"#;

        let cleaned =
            pre_process_sheet_data_chunk(sheet_data, &params, &mut state, &mut metrics).unwrap();
        let cleaned = String::from_utf8_lossy(cleaned.as_ref());

        assert_eq!(cleaned.as_bytes(), sheet_data);
    }

    #[test]
    fn preprocess_empty_cell_scanner_removes_unstyled_but_keeps_font_only_self_closing_cells() {
        let mut params = default_params();
        params.patterns.xfs_by_index.insert(
            7,
            Xf::new(None, None, false, false, false, false, true, false),
        );
        let state = SheetDataPreprocessState::new(&Default::default(), &params);
        let mut metrics = CleanSheetMetrics {
            cleanup_stats: CleanupStats::enabled(),
            ..CleanSheetMetrics::default()
        };
        let chunk = br#"<row r="1"><c r="A1"/><c r="B1" s="7"/><c r="C1"><v>1</v></c><custom r="D1"/></row>"#;

        let cleaned = remove_consecutive_empty_cells(chunk, &state, &mut metrics);
        let cleaned = String::from_utf8_lossy(cleaned.as_ref());

        assert!(!cleaned.contains(r#"r="A1""#));
        assert!(cleaned.contains(r#"r="B1""#));
        assert!(cleaned.contains(r#"r="C1""#));
        assert!(cleaned.contains(r#"<custom r="D1"/>"#));
        assert_eq!(
            metrics
                .cleanup_stats
                .removed_items_by_cause
                .get(&crate::models::CleanupCause::PreprocessEmptySelfClosingCells),
            Some(&1)
        );
    }

    #[test]
    fn preprocess_empty_cell_scanner_keeps_non_removable_styled_self_closing_cell() {
        let params = default_params();
        let state = SheetDataPreprocessState::new(&Default::default(), &params);
        let mut metrics = CleanSheetMetrics::default();
        let chunk = br#"<row r="1"><c r="A1" s="9"/></row>"#;

        let cleaned = remove_consecutive_empty_cells(chunk, &state, &mut metrics);

        assert_eq!(cleaned.as_ref(), chunk);
    }
}
