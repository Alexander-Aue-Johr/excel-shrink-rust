use crate::models::SheetStreamingState;
use crate::xml::{find_and_slice_after_tag, find_and_slice_after_tag_range};

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

    let tags = [
        WorksheetTag::Cols,
        WorksheetTag::SheetData,
        WorksheetTag::MergeCells,
        WorksheetTag::WorksheetEnd,
    ];

    for tag in tags {
        if processed_tags.contains(&tag) {
            continue;
        }

        let found = match tag {
            WorksheetTag::Cols => find_and_slice_after_tag_range(buffer, b"<cols", b">", false),
            WorksheetTag::SheetData => {
                find_and_slice_after_tag_range(buffer, b"<sheetData", b">", false)
            }
            WorksheetTag::MergeCells => {
                find_and_slice_after_tag_range(buffer, b"<mergeCells", b">", false)
            }
            WorksheetTag::WorksheetEnd => {
                find_and_slice_after_tag(buffer, b"</worksheet>", false)
            }
        };

        let Some(found) = found else {
            continue;
        };

        if found.is_self_closing_tag {
            continue;
        }

        let new_state = match tag {
            WorksheetTag::Cols => SheetStreamingState::InColsNode,
            WorksheetTag::SheetData => SheetStreamingState::InSheetDataNode,
            WorksheetTag::MergeCells => SheetStreamingState::InMergeCellsNode,
            WorksheetTag::WorksheetEnd => SheetStreamingState::FoundWorksheetEnd,
        };

        return CollectChunksResult {
            new_state: Some(new_state),
            chunk_to_add: Some(found.until_tag),
            updated_buffer: Some(found.after_tag),
            processed_tag: Some(tag),
        };
    }

    CollectChunksResult {
        new_state: None,
        chunk_to_add: None,
        updated_buffer: None,
        processed_tag: None,
    }
}