use crate::models::SheetStreamingState;
use crate::xml::{find_and_slice_after_tag, find_and_slice_after_tag_range, SliceAtTagResult};

const WORKSHEET_TAGS: [WorksheetTag; 4] = [
    WorksheetTag::Cols,
    WorksheetTag::SheetData,
    WorksheetTag::MergeCells,
    WorksheetTag::WorksheetEnd,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorksheetTag {
    Cols,
    SheetData,
    MergeCells,
    WorksheetEnd,
}

#[derive(Debug, Clone)]
pub struct CollectChunksResult {
    pub new_state: Option<SheetStreamingState>,
    pub chunk_to_add: Option<Vec<u8>>,
    pub updated_buffer: Option<Vec<u8>>,
    pub processed_tag: Option<WorksheetTag>,
}

pub fn process_worksheet_node(
    buffer: &[u8],
    state: SheetStreamingState,
    processed_tags: &[WorksheetTag],
) -> CollectChunksResult {
    assert_eq!(state, SheetStreamingState::InWorksheetNode);

    find_next_worksheet_tag(buffer, processed_tags)
        .map(|(tag, found)| collect_processed_tag(tag, found))
        .unwrap_or_else(no_worksheet_tag_found)
}

fn find_next_worksheet_tag(
    buffer: &[u8],
    processed_tags: &[WorksheetTag],
) -> Option<(WorksheetTag, SliceAtTagResult)> {
    for tag in WORKSHEET_TAGS {
        if processed_tags.contains(&tag) {
            continue;
        }

        let Some(found) = find_worksheet_tag(buffer, tag) else {
            continue;
        };
        if found.is_self_closing_tag {
            continue;
        }

        return Some((tag, found));
    }

    None
}

fn find_worksheet_tag(buffer: &[u8], tag: WorksheetTag) -> Option<SliceAtTagResult> {
    match tag {
        WorksheetTag::Cols => find_and_slice_after_tag_range(buffer, b"<cols", b">", false),
        WorksheetTag::SheetData => {
            find_and_slice_after_tag_range(buffer, b"<sheetData", b">", false)
        }
        WorksheetTag::MergeCells => {
            find_and_slice_after_tag_range(buffer, b"<mergeCells", b">", false)
        }
        WorksheetTag::WorksheetEnd => find_and_slice_after_tag(buffer, b"</worksheet>", false),
    }
}

fn collect_processed_tag(tag: WorksheetTag, found: SliceAtTagResult) -> CollectChunksResult {
    CollectChunksResult {
        new_state: Some(streaming_state_for_tag(tag)),
        chunk_to_add: Some(found.until_tag),
        updated_buffer: Some(found.after_tag),
        processed_tag: Some(tag),
    }
}

#[inline]
fn streaming_state_for_tag(tag: WorksheetTag) -> SheetStreamingState {
    match tag {
        WorksheetTag::Cols => SheetStreamingState::InColsNode,
        WorksheetTag::SheetData => SheetStreamingState::InSheetDataNode,
        WorksheetTag::MergeCells => SheetStreamingState::InMergeCellsNode,
        WorksheetTag::WorksheetEnd => SheetStreamingState::FoundWorksheetEnd,
    }
}

fn no_worksheet_tag_found() -> CollectChunksResult {
    CollectChunksResult {
        new_state: None,
        chunk_to_add: None,
        updated_buffer: None,
        processed_tag: None,
    }
}
