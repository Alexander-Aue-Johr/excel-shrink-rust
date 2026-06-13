use crate::models::{CleanSheetMetrics, CleanSheetParameters, CleanupCause};
use crate::xml::{find_byte_from, find_bytes};
use std::borrow::Cow;

#[derive(Debug, Clone, Default)]
pub struct HiddenRowCollapseState {
    pub previous_kept_hidden_empty_row: bool,
}

pub fn remove_hidden_rows_if_present(chunk: &[u8], _params: &CleanSheetParameters) -> Vec<u8> {
    let mut state = HiddenRowCollapseState::default();
    let mut metrics = CleanSheetMetrics::default();
    remove_hidden_rows_if_present_with_state(chunk, &mut state, &mut metrics).into_owned()
}

pub fn remove_hidden_rows_if_present_with_state<'a>(
    chunk: &'a [u8],
    state: &mut HiddenRowCollapseState,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    if find_bytes(chunk, b"hidden=\"1\"").is_none() {
        state.previous_kept_hidden_empty_row = false;
        return Cow::Borrowed(chunk);
    }

    collapse_consecutive_hidden_empty_rows(chunk, state, metrics)
}

fn collapse_consecutive_hidden_empty_rows<'a>(
    chunk: &'a [u8],
    state: &mut HiddenRowCollapseState,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    let mut out: Option<Vec<u8>> = None;
    let mut pos = 0;

    while let Some(row_start_relative) = find_bytes(&chunk[pos..], b"<row") {
        let row_start = pos + row_start_relative;

        let Some(row_end) = find_row_end(chunk, row_start) else {
            if let Some(out) = out.as_mut() {
                out.extend_from_slice(&chunk[pos..]);
            }
            return out.map_or(Cow::Borrowed(chunk), Cow::Owned);
        };

        let row = &chunk[row_start..row_end];

        let is_hidden_empty_row = is_hidden_empty_row(row);

        if is_hidden_empty_row {
            if !state.previous_kept_hidden_empty_row {
                if let Some(out) = out.as_mut() {
                    out.extend_from_slice(&chunk[pos..row_end]);
                }
                state.previous_kept_hidden_empty_row = true;
            } else {
                if out.is_none() {
                    let mut new_out = Vec::with_capacity(chunk.len().min(64 * 1024 * 1024));
                    new_out.extend_from_slice(&chunk[..row_start]);
                    out = Some(new_out);
                } else if let Some(out) = out.as_mut() {
                    out.extend_from_slice(&chunk[pos..row_start]);
                }
                metrics.cleanup_stats.record_removed(
                    CleanupCause::PreprocessConsecutiveHiddenEmptyRows,
                    row.len(),
                );
            }
        } else {
            if let Some(out) = out.as_mut() {
                out.extend_from_slice(&chunk[pos..row_end]);
            }
            state.previous_kept_hidden_empty_row = false;
        }

        pos = row_end;
    }

    if let Some(out) = out.as_mut() {
        out.extend_from_slice(&chunk[pos..]);
    }

    out.map_or(Cow::Borrowed(chunk), Cow::Owned)
}

fn is_hidden_empty_row(row: &[u8]) -> bool {
    find_bytes(row, b"hidden=\"1\"").is_some()
        && find_bytes(row, b"<v").is_none()
        && find_bytes(row, b"<f").is_none()
}

fn find_row_end(chunk: &[u8], row_start: usize) -> Option<usize> {
    let open_end = find_byte_from(chunk, b'>', row_start)?;

    // <row r="123" hidden="1"/>
    if open_end > row_start && chunk[open_end - 1] == b'/' {
        return Some(open_end + 1);
    }

    // <row ...> ... </row>
    let content_start = open_end + 1;
    let closing_relative = find_bytes(&chunk[content_start..], b"</row>")?;

    Some(content_start + closing_relative + b"</row>".len())
}
