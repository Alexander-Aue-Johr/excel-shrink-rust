use crate::models::CleanSheetParameters;
use crate::xml::{find_byte_from, find_bytes};

#[derive(Debug, Clone, Default)]
pub struct HiddenRowCollapseState {
    pub previous_kept_hidden_empty_row: bool,
}

pub fn remove_hidden_rows_if_present(chunk: &[u8], _params: &CleanSheetParameters) -> Vec<u8> {
    let mut state = HiddenRowCollapseState::default();
    remove_hidden_rows_if_present_with_state(chunk, &mut state)
}

pub fn remove_hidden_rows_if_present_with_state(
    chunk: &[u8],
    state: &mut HiddenRowCollapseState,
) -> Vec<u8> {
    return chunk.to_vec();

    if find_bytes(chunk, b"hidden=\"1\"").is_none() {
        state.previous_kept_hidden_empty_row = false;
        return chunk.to_vec();
    }

    collapse_consecutive_hidden_empty_rows(chunk, state)
}

fn collapse_consecutive_hidden_empty_rows(
    chunk: &[u8],
    state: &mut HiddenRowCollapseState,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(chunk.len());
    let mut pos = 0;

    while let Some(row_start_relative) = find_bytes(&chunk[pos..], b"<row") {
        let row_start = pos + row_start_relative;

        // Alles vor der Row unverändert übernehmen.
        out.extend_from_slice(&chunk[pos..row_start]);

        let Some(row_end) = find_row_end(chunk, row_start) else {
            // Sollte bei sauber geschnittenen Streaming-Chunks nicht passieren.
            out.extend_from_slice(&chunk[row_start..]);
            return out;
        };

        let row = &chunk[row_start..row_end];

        let is_hidden_empty_row = is_hidden_empty_row(row);

        if is_hidden_empty_row {
            if !state.previous_kept_hidden_empty_row {
                // Erste Row eines Hidden-Runs behalten.
                out.extend_from_slice(row);
                state.previous_kept_hidden_empty_row = true;
            }
            // Alle weiteren Hidden-Rows im Run löschen.
        } else {
            out.extend_from_slice(row);
            state.previous_kept_hidden_empty_row = false;
        }

        pos = row_end;
    }

    out.extend_from_slice(&chunk[pos..]);
    out
}

fn is_hidden_empty_row(row: &[u8]) -> bool {
    find_bytes(row, b"hidden=\"1\"").is_some()
        && find_bytes(row, b"<v").is_none()
        && find_bytes(row, b"<f").is_none()
}

fn find_row_end(chunk: &[u8], row_start: usize) -> Option<usize> {
    let open_end = find_byte_from(chunk, b'>', row_start)?;

    // Nur wenn der <row ...>-Starttag selbst self-closing ist:
    // <row r="123" hidden="1"/>
    if open_end > row_start && chunk[open_end - 1] == b'/' {
        return Some(open_end + 1);
    }

    // Sonst echte nicht-self-closing Row:
    // <row ...> ... </row>
    let content_start = open_end + 1;
    let closing_relative = find_bytes(&chunk[content_start..], b"</row>")?;

    Some(content_start + closing_relative + b"</row>".len())
}
