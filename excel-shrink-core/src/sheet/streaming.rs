use crate::error::{Result, ShrinkError};
use crate::models::{CellCoordinate, CleanSheetMetrics, CleanSheetParameters, SheetStreamingState};
use crate::sheet::cols::process_cols_node;
use crate::sheet::merge_cells::find_max_merge_cells;
use crate::sheet::output::build_clean_sheet_xml;
use crate::sheet::sheet_data::pre_process_sheet_data_chunk;
use crate::sheet::worksheet::{process_worksheet_node, WorksheetTag};
use crate::xml::{find_bytes, rfind_bytes};
use regex::bytes::Regex;
use std::io::Read;
use std::sync::OnceLock;

static WORKSHEET_START_RE: OnceLock<Regex> = OnceLock::new();

#[derive(Debug, Clone, Default)]
pub struct PreprocessedSheetXml {
    pub until_worksheet_start_output_chunks: Vec<u8>,
    pub before_cols_node_output_chunks: Vec<u8>,
    pub in_cols_node_output_chunks: Vec<u8>,
    pub before_sheet_data_node_output_chunks: Vec<u8>,
    pub sheet_data: Vec<u8>,
    pub before_merge_cells_node_output_chunks: Vec<u8>,
    pub in_merge_cells_node_output_chunks: Vec<u8>,
    pub until_worksheet_end_output_chunks: Vec<u8>,
    pub clean_sheet_metrics: CleanSheetMetrics,
}

pub fn clean_sheet_data_streamwise(
    stream: impl Read,
    params: &CleanSheetParameters,
    chunk_size: usize,
    disable_sheetdata_splitting: bool,
) -> Result<Vec<u8>> {
    let preprocessed = preprocess_sheet_xml_streamwise(stream, params, chunk_size)?;
    build_clean_sheet_xml(preprocessed, params, disable_sheetdata_splitting)
}

pub fn preprocess_sheet_xml_streamwise(
    mut stream: impl Read,
    params: &CleanSheetParameters,
    chunk_size: usize,
) -> Result<PreprocessedSheetXml> {
    let mut buffer = Vec::new();
    let mut read_buf = vec![0u8; chunk_size];

    let mut state = SheetStreamingState::SeekWorksheetNode;
    let mut reached_eof = false;
    let mut processed_tags: Vec<WorksheetTag> = Vec::new();

    let mut result = PreprocessedSheetXml::default();

    let mut column_definitions = None;
    let mut max_merge_cells_column_and_row: Option<CellCoordinate> = None;

    while state != SheetStreamingState::FoundWorksheetEnd {
        if !reached_eof {
            let n = stream.read(&mut read_buf)?;
            if n == 0 {
                reached_eof = true;
            } else {
                buffer.extend_from_slice(&read_buf[..n]);
            }
        }

        match state {
            SheetStreamingState::SeekWorksheetNode => {
                let re = WORKSHEET_START_RE
                    .get_or_init(|| Regex::new(r#"<([A-Za-z_][\w.-]*:)?worksheet\b"#).unwrap());

                if let Some(mat) = re.find(&buffer) {
                    let start_idx = mat.start();
                    // Finde das dazugehörige schließende '>' des Tags
                    if let Some(end_idx) = crate::xml::find_byte_from(&buffer, b'>', start_idx) {
                        let after_tag = end_idx + 1;
                        result.until_worksheet_start_output_chunks = buffer[..after_tag].to_vec();
                        buffer = buffer[after_tag..].to_vec();
                        state = SheetStreamingState::InWorksheetNode;
                        continue;
                    }
                }

                if reached_eof {
                    return Err(ShrinkError::InvalidRootNode);
                }
            }

            SheetStreamingState::InWorksheetNode => {
                let collect = process_worksheet_node(&buffer, state, &processed_tags);

                let Some(new_state) = collect.new_state else {
                    if reached_eof {
                        break;
                    }
                    continue;
                };

                let chunk = collect.chunk_to_add.unwrap_or_default();
                buffer = collect.updated_buffer.unwrap_or_default();

                if let Some(tag) = collect.processed_tag {
                    processed_tags.push(tag);

                    match tag {
                        WorksheetTag::Cols => {
                            result.before_cols_node_output_chunks = chunk;
                        }
                        WorksheetTag::SheetData => {
                            result.before_sheet_data_node_output_chunks = chunk;
                        }
                        WorksheetTag::MergeCells => {
                            result.before_merge_cells_node_output_chunks = chunk;
                        }
                        WorksheetTag::WorksheetEnd => {
                            result.until_worksheet_end_output_chunks = chunk;
                        }
                    }
                }

                state = new_state;
            }

            SheetStreamingState::InColsNode => {
                let Some(idx) = crate::xml::find_bytes(&buffer, b"</cols>") else {
                    if reached_eof {
                        return Err(ShrinkError::UnexpectedEof);
                    }
                    continue;
                };

                let content = buffer[..idx].to_vec();
                let after = buffer[idx + b"</cols>".len()..].to_vec();

                column_definitions = Some(process_cols_node(&content, params));
                result.in_cols_node_output_chunks = content;

                buffer = buffer[idx..].to_vec();
                state = SheetStreamingState::InWorksheetNode;
            }

            SheetStreamingState::InSheetDataNode => {
                if let Some(idx) = find_bytes(&buffer, b"</sheetData>") {
                    let chunk = &buffer[..idx];

                    let cleaned_chunk = pre_process_sheet_data_chunk(chunk, params)?;
                    result.sheet_data.extend_from_slice(&cleaned_chunk);

                    buffer = buffer[idx..].to_vec();
                    state = SheetStreamingState::InWorksheetNode;
                    continue;
                }

                // Noch kein </sheetData> gefunden.
                // Dann verarbeiten wir nur vollständige Rows und lassen die letzte mögliche Row im Buffer.
                if let Some(last_row_start) = rfind_bytes(&buffer, b"<row") {
                    if last_row_start > 0 {
                        let chunk = &buffer[..last_row_start];

                        let cleaned_chunk = pre_process_sheet_data_chunk(chunk, params)?;
                        result.sheet_data.extend_from_slice(&cleaned_chunk);

                        buffer = buffer[last_row_start..].to_vec();
                    }
                }

                if reached_eof {
                    return Err(ShrinkError::UnexpectedEof);
                }
            }

            SheetStreamingState::InMergeCellsNode => {
                let Some(idx) = crate::xml::find_bytes(&buffer, b"</mergeCells>") else {
                    if reached_eof {
                        return Err(ShrinkError::UnexpectedEof);
                    }
                    continue;
                };

                let content = buffer[..idx].to_vec();
                max_merge_cells_column_and_row = find_max_merge_cells(&content);
                result.in_merge_cells_node_output_chunks = content;

                buffer = buffer[idx..].to_vec();
                state = SheetStreamingState::InWorksheetNode;
            }

            SheetStreamingState::FoundWorksheetEnd => break,
        }
    }

    result.clean_sheet_metrics = CleanSheetMetrics {
        column_definitions: column_definitions.unwrap_or_default(),
        max_merge_cells_column_and_row,
        ..CleanSheetMetrics::default()
    };

    Ok(result)
}
