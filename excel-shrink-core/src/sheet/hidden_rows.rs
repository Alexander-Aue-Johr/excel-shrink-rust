use crate::models::{CleanSheetMetrics, CleanSheetParameters};
use std::borrow::Cow;

#[derive(Debug, Clone, Default)]
pub struct HiddenRowCollapseState;

pub fn remove_hidden_rows_if_present(chunk: &[u8], _params: &CleanSheetParameters) -> Vec<u8> {
    chunk.to_vec()
}

pub fn remove_hidden_rows_if_present_with_state<'a>(
    chunk: &'a [u8],
    _state: &mut HiddenRowCollapseState,
    _metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    Cow::Borrowed(chunk)
}

#[cfg(test)]
mod tests {
    use super::{remove_hidden_rows_if_present_with_state, HiddenRowCollapseState};
    use crate::models::CleanSheetMetrics;

    #[test]
    fn hidden_rows_in_separate_or_consecutive_blocks_are_preserved() {
        let mut state = HiddenRowCollapseState;
        let mut metrics = CleanSheetMetrics::default();
        let chunk = br#"<row r="1" hidden="1"></row><row r="2" hidden="1"></row><row r="3"><c r="A3"><v>1</v></c></row><row r="4" hidden="1"></row><row r="5" hidden="1"></row>"#;

        let cleaned = remove_hidden_rows_if_present_with_state(chunk, &mut state, &mut metrics);

        assert_eq!(cleaned.as_ref(), chunk);
    }
}
