use crate::error::{Result, ShrinkError};
use crate::models::{
    CleanSheetMetrics, CleanSheetParameters, CleanedSheetData, ColumnRangeMap, SheetStreamingState,
};
use crate::sheet::cols::process_cols_node;
use crate::sheet::merge_cells::find_max_merge_cells;
use crate::sheet::sheet_data::{pre_process_sheet_data_chunk, SheetDataPreprocessState};
use crate::sheet::worksheet::{process_worksheet_node, WorksheetTag};
use crate::xml::rfind_bytes;
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
) -> Result<CleanedSheetData> {
    clean_sheet_data_streamwise_with_progress(
        stream,
        params,
        chunk_size,
        disable_sheetdata_splitting,
        &mut |_| {},
    )
}

pub fn clean_sheet_data_streamwise_with_progress(
    stream: impl Read,
    params: &CleanSheetParameters,
    chunk_size: usize,
    disable_sheetdata_splitting: bool,
    progress: &mut impl FnMut(&'static str),
) -> Result<CleanedSheetData> {
    progress("preprocess_sheet_xml_streamwise");
    let preprocessed =
        preprocess_sheet_xml_streamwise_with_progress(stream, params, chunk_size, progress)?;
    progress("build_clean_sheet_xml");
    crate::sheet::output::build_clean_sheet_xml_with_progress(
        preprocessed,
        params,
        disable_sheetdata_splitting,
        progress,
    )
}

fn get_or_create_sheet_data_preprocess_state<'a>(
    state: &'a mut Option<SheetDataPreprocessState>,
    column_definitions: &Option<ColumnRangeMap>,
    params: &CleanSheetParameters,
) -> &'a mut SheetDataPreprocessState {
    if state.is_none() {
        let empty_column_definitions = ColumnRangeMap::default();
        let columns = column_definitions
            .as_ref()
            .unwrap_or(&empty_column_definitions);

        *state = Some(SheetDataPreprocessState::new(columns, params));
    }

    state.as_mut().unwrap()
}

pub fn preprocess_sheet_xml_streamwise(
    mut stream: impl Read,
    params: &CleanSheetParameters,
    chunk_size: usize,
) -> Result<PreprocessedSheetXml> {
    preprocess_sheet_xml_streamwise_with_progress(&mut stream, params, chunk_size, &mut |_| {})
}

pub fn preprocess_sheet_xml_streamwise_with_progress(
    mut stream: impl Read,
    params: &CleanSheetParameters,
    chunk_size: usize,
    progress: &mut impl FnMut(&'static str),
) -> Result<PreprocessedSheetXml> {
    let mut buffer = Vec::new();
    let mut read_buf = vec![0u8; chunk_size];

    let mut state = SheetStreamingState::SeekWorksheetNode;
    let mut last_reported_state: Option<SheetStreamingState> = None;
    let mut reached_eof = false;
    let mut processed_tags: Vec<WorksheetTag> = Vec::new();

    let mut result = PreprocessedSheetXml::default();
    result.clean_sheet_metrics.cleanup_stats.enabled = params.collect_cleanup_stats;

    let mut column_definitions = None;
    let mut sheet_data_preprocess_state: Option<SheetDataPreprocessState> = None;
    while state != SheetStreamingState::FoundWorksheetEnd {
        if last_reported_state != Some(state) {
            match state {
                SheetStreamingState::SeekWorksheetNode => progress("seek_worksheet_node"),
                SheetStreamingState::InWorksheetNode => progress("scan_worksheet_node_children"),
                SheetStreamingState::InColsNode => progress("process_cols_node"),
                SheetStreamingState::InSheetDataNode => progress("preprocess_sheet_data_node"),
                SheetStreamingState::InMergeCellsNode => progress("process_merge_cells_node"),
                SheetStreamingState::FoundWorksheetEnd => progress("found_worksheet_end"),
            }
            last_reported_state = Some(state);
        }

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

                let parsed_column_definitions = process_cols_node(&content, params);
                result.clean_sheet_metrics.column_definitions = parsed_column_definitions.clone();
                sheet_data_preprocess_state = Some(SheetDataPreprocessState::new(
                    &parsed_column_definitions,
                    params,
                ));
                column_definitions = Some(parsed_column_definitions);

                result.in_cols_node_output_chunks = content;

                buffer = buffer[idx..].to_vec();
                state = SheetStreamingState::InWorksheetNode;
            }

            SheetStreamingState::InSheetDataNode => {
                if let Some(idx) = rfind_bytes(&buffer, b"</sheetData>") {
                    let chunk = &buffer[..idx];

                    let preprocess_state = get_or_create_sheet_data_preprocess_state(
                        &mut sheet_data_preprocess_state,
                        &column_definitions,
                        params,
                    );

                    let cleaned_chunk = pre_process_sheet_data_chunk(
                        chunk,
                        params,
                        preprocess_state,
                        &mut result.clean_sheet_metrics,
                    )?;

                    extend_without_large_growth(&mut result.sheet_data, &cleaned_chunk);

                    buffer = buffer[idx..].to_vec();
                    state = SheetStreamingState::InWorksheetNode;
                    continue;
                }

                if let Some(last_row_start) = rfind_bytes(&buffer, b"<row") {
                    if last_row_start > 0 {
                        let chunk = &buffer[..last_row_start];

                        let preprocess_state = get_or_create_sheet_data_preprocess_state(
                            &mut sheet_data_preprocess_state,
                            &column_definitions,
                            params,
                        );

                        let cleaned_chunk = pre_process_sheet_data_chunk(
                            chunk,
                            params,
                            preprocess_state,
                            &mut result.clean_sheet_metrics,
                        )?;

                        extend_without_large_growth(&mut result.sheet_data, &cleaned_chunk);

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

                result.clean_sheet_metrics.max_merge_cells_column_and_row =
                    find_max_merge_cells(&content);
                result.in_merge_cells_node_output_chunks = content;

                buffer = buffer[idx..].to_vec();
                state = SheetStreamingState::InWorksheetNode;
            }

            SheetStreamingState::FoundWorksheetEnd => break,
        }
    }

    Ok(result)
}

fn extend_without_large_growth(out: &mut Vec<u8>, bytes: &[u8]) {
    let spare = out.capacity().saturating_sub(out.len());
    if spare < bytes.len() {
        out.reserve_exact((bytes.len() - spare).max(16 * 1024 * 1024));
    }
    out.extend_from_slice(bytes);
}
