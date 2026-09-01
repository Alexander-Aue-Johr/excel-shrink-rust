use crate::error::{Result, ShrinkError};
use crate::models::{
    CleanSheetMetrics, CleanSheetParameters, CleanedSheetData, SheetStreamingState,
    VisibleContentBounds,
};
use crate::reference_scanner::scan_sheet_fragment;
use crate::sheet::cols::process_cols_node;
use crate::sheet::merge_cells::{find_max_merge_cells, find_merge_cell_ranges};
use crate::sheet::preprocess_chunks::SheetDataChunkExecutor;
use crate::sheet::worksheet::{process_worksheet_node, WorksheetTag};
use crate::xml::{find_byte_from, find_bytes, rfind_bytes};
use regex::bytes::Regex;
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, OnceLock};

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
    pub preprocessed_visible_data_bounds: Option<VisibleContentBounds>,
}

#[derive(Clone, Default)]
pub struct SheetReferenceScanOptions {
    pub sheet_name_to_file: Arc<HashMap<String, String>>,
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
        0,
        &mut |_| {},
    )
}

pub fn clean_sheet_data_streamwise_with_progress(
    stream: impl Read,
    params: &CleanSheetParameters,
    chunk_size: usize,
    disable_sheetdata_splitting: bool,
    parallel_range_chunk_size: usize,
    progress: &mut impl FnMut(&'static str),
) -> Result<CleanedSheetData> {
    clean_sheet_data_streamwise_with_reference_scan_and_progress(
        stream,
        params,
        chunk_size,
        disable_sheetdata_splitting,
        parallel_range_chunk_size,
        None,
        progress,
    )
}

pub fn clean_sheet_data_streamwise_with_reference_scan_and_progress(
    stream: impl Read,
    params: &CleanSheetParameters,
    chunk_size: usize,
    disable_sheetdata_splitting: bool,
    parallel_range_chunk_size: usize,
    reference_scan: Option<SheetReferenceScanOptions>,
    progress: &mut impl FnMut(&'static str),
) -> Result<CleanedSheetData> {
    progress("preprocess_sheet_xml_streamwise");
    let preprocessed = preprocess_sheet_xml_streamwise_parallel_with_progress(
        stream,
        params,
        chunk_size,
        parallel_range_chunk_size,
        reference_scan,
        progress,
    )?;

    progress("build_clean_sheet_xml");
    crate::sheet::output::build_clean_sheet_xml_with_progress(
        preprocessed,
        params,
        disable_sheetdata_splitting,
        parallel_range_chunk_size,
        progress,
    )
}

pub fn preprocess_sheet_xml_streamwise(
    stream: impl Read,
    params: &CleanSheetParameters,
    chunk_size: usize,
) -> Result<PreprocessedSheetXml> {
    preprocess_sheet_xml_streamwise_with_progress(stream, params, chunk_size, &mut |_| {})
}

pub fn preprocess_sheet_xml_streamwise_with_progress(
    stream: impl Read,
    params: &CleanSheetParameters,
    chunk_size: usize,
    progress: &mut impl FnMut(&'static str),
) -> Result<PreprocessedSheetXml> {
    preprocess_sheet_xml_streamwise_parallel_with_progress(
        stream, params, chunk_size, 0, None, progress,
    )
}

fn preprocess_sheet_xml_streamwise_parallel_with_progress(
    stream: impl Read,
    params: &CleanSheetParameters,
    read_chunk_size: usize,
    work_chunk_size: usize,
    reference_scan: Option<SheetReferenceScanOptions>,
    progress: &mut impl FnMut(&'static str),
) -> Result<PreprocessedSheetXml> {
    SheetXmlPreprocessor::new(
        stream,
        params,
        read_chunk_size,
        work_chunk_size,
        reference_scan,
        progress,
    )
    .run()
}

struct SheetXmlPreprocessor<'a, R, P> {
    stream: R,
    params: &'a CleanSheetParameters,
    progress: &'a mut P,
    buffer: Vec<u8>,
    read_buffer: Vec<u8>,
    state: SheetStreamingState,
    last_reported_state: Option<SheetStreamingState>,
    reached_eof: bool,
    processed_tags: Vec<WorksheetTag>,
    result: PreprocessedSheetXml,
    sheet_data_executor: SheetDataChunkExecutor,
}

impl<'a, R, P> SheetXmlPreprocessor<'a, R, P>
where
    R: Read,
    P: FnMut(&'static str),
{
    fn new(
        stream: R,
        params: &'a CleanSheetParameters,
        read_chunk_size: usize,
        work_chunk_size: usize,
        reference_scan: Option<SheetReferenceScanOptions>,
        progress: &'a mut P,
    ) -> Self {
        let mut result = PreprocessedSheetXml::default();
        result.clean_sheet_metrics.cleanup_stats.enabled = params.collect_cleanup_stats;

        Self {
            stream,
            params,
            progress,
            buffer: Vec::new(),
            read_buffer: vec![0u8; read_chunk_size],
            state: SheetStreamingState::SeekWorksheetNode,
            last_reported_state: None,
            reached_eof: false,
            processed_tags: Vec::new(),
            result,
            sheet_data_executor: SheetDataChunkExecutor::new(
                params,
                work_chunk_size,
                reference_scan,
            ),
        }
    }

    fn run(mut self) -> Result<PreprocessedSheetXml> {
        while self.state != SheetStreamingState::FoundWorksheetEnd {
            self.report_state_progress();
            self.read_next_chunk()?;

            if self.process_current_state()? {
                break;
            }
        }

        self.sheet_data_executor.finish(&mut self.result)?;
        self.finish_reference_scan();

        Ok(self.result)
    }

    fn report_state_progress(&mut self) {
        if self.last_reported_state == Some(self.state) {
            return;
        }

        (self.progress)(progress_label(self.state));
        self.last_reported_state = Some(self.state);
    }

    fn read_next_chunk(&mut self) -> Result<()> {
        if self.reached_eof {
            return Ok(());
        }

        let bytes_read = self.stream.read(&mut self.read_buffer)?;
        if bytes_read == 0 {
            self.reached_eof = true;
        } else {
            self.buffer
                .extend_from_slice(&self.read_buffer[..bytes_read]);
        }

        Ok(())
    }

    fn process_current_state(&mut self) -> Result<bool> {
        match self.state {
            SheetStreamingState::SeekWorksheetNode => self.seek_worksheet_node(),
            SheetStreamingState::InWorksheetNode => self.process_worksheet_children(),
            SheetStreamingState::InColsNode => self.process_columns_node(),
            SheetStreamingState::InSheetDataNode => self.process_sheet_data_node(),
            SheetStreamingState::InMergeCellsNode => self.process_merge_cells_node(),
            SheetStreamingState::FoundWorksheetEnd => Ok(true),
        }
    }

    fn seek_worksheet_node(&mut self) -> Result<bool> {
        let Some(after_tag) = worksheet_start_tag_end(&self.buffer) else {
            return if self.reached_eof {
                Err(ShrinkError::InvalidRootNode)
            } else {
                Ok(false)
            };
        };

        self.result.until_worksheet_start_output_chunks = self.split_buffer_at(after_tag);
        self.state = SheetStreamingState::InWorksheetNode;
        Ok(false)
    }

    fn process_worksheet_children(&mut self) -> Result<bool> {
        let collected = process_worksheet_node(&self.buffer, self.state, &self.processed_tags);
        let Some(new_state) = collected.new_state else {
            return Ok(self.reached_eof);
        };

        let chunk = collected.chunk_to_add.unwrap_or_default();
        self.buffer = collected.updated_buffer.unwrap_or_default();
        if let Some(tag) = collected.processed_tag {
            self.record_processed_tag(tag, chunk);
        }
        self.state = new_state;

        Ok(false)
    }

    fn record_processed_tag(&mut self, tag: WorksheetTag, chunk: Vec<u8>) {
        self.processed_tags.push(tag);

        match tag {
            WorksheetTag::Cols => self.result.before_cols_node_output_chunks = chunk,
            WorksheetTag::SheetData => self.result.before_sheet_data_node_output_chunks = chunk,
            WorksheetTag::MergeCells => {
                self.result.before_merge_cells_node_output_chunks = chunk;
            }
            WorksheetTag::WorksheetEnd => {
                self.result.until_worksheet_end_output_chunks = chunk;
            }
        }
    }

    fn process_columns_node(&mut self) -> Result<bool> {
        let Some(closing_start) = find_bytes(&self.buffer, b"</cols>") else {
            return self.continue_or_unexpected_eof();
        };

        let content = self.split_buffer_at(closing_start);
        self.configure_column_definitions(&content);
        self.result.in_cols_node_output_chunks = content;
        self.state = SheetStreamingState::InWorksheetNode;

        Ok(false)
    }

    fn configure_column_definitions(&mut self, content: &[u8]) {
        let column_definitions = process_cols_node(content, self.params);
        self.result.clean_sheet_metrics.column_definitions = column_definitions.clone();
        self.sheet_data_executor
            .configure_columns(column_definitions);
    }

    fn process_sheet_data_node(&mut self) -> Result<bool> {
        if let Some(closing_start) = rfind_bytes(&self.buffer, b"</sheetData>") {
            self.preprocess_sheet_data_prefix(closing_start)?;
            self.discard_buffer_prefix(closing_start);
            self.state = SheetStreamingState::InWorksheetNode;
            return Ok(false);
        }

        if let Some(row_start) = rfind_bytes(&self.buffer, b"<row").filter(|start| *start > 0) {
            self.preprocess_sheet_data_prefix(row_start)?;
            self.discard_buffer_prefix(row_start);
        }

        self.continue_or_unexpected_eof()
    }

    fn preprocess_sheet_data_prefix(&mut self, end: usize) -> Result<()> {
        self.sheet_data_executor
            .push(&self.buffer[..end], &mut self.result)
    }

    fn finish_reference_scan(&mut self) {
        let Some(scan) = self.sheet_data_executor.reference_scan.clone() else {
            return;
        };
        let chunks = [
            self.result.until_worksheet_start_output_chunks.as_slice(),
            self.result.before_cols_node_output_chunks.as_slice(),
            self.result.in_cols_node_output_chunks.as_slice(),
            self.result.before_sheet_data_node_output_chunks.as_slice(),
            self.result.before_merge_cells_node_output_chunks.as_slice(),
            self.result.in_merge_cells_node_output_chunks.as_slice(),
            self.result.until_worksheet_end_output_chunks.as_slice(),
        ];

        for chunk in chunks {
            self.result
                .clean_sheet_metrics
                .reference_summary
                .merge(scan_sheet_fragment(
                    chunk,
                    &self.params.zip_info_filename,
                    &scan.sheet_name_to_file,
                ));
        }
        self.result.clean_sheet_metrics.reference_summary.complete = true;
    }

    fn process_merge_cells_node(&mut self) -> Result<bool> {
        let Some(closing_start) = find_bytes(&self.buffer, b"</mergeCells>") else {
            return self.continue_or_unexpected_eof();
        };

        let content = self.split_buffer_at(closing_start);
        self.result
            .clean_sheet_metrics
            .max_merge_cells_column_and_row = find_max_merge_cells(&content);
        self.result.clean_sheet_metrics.merge_cell_ranges = find_merge_cell_ranges(&content);
        self.result.in_merge_cells_node_output_chunks = content;
        self.state = SheetStreamingState::InWorksheetNode;

        Ok(false)
    }

    fn continue_or_unexpected_eof(&self) -> Result<bool> {
        if self.reached_eof {
            Err(ShrinkError::UnexpectedEof)
        } else {
            Ok(false)
        }
    }

    fn split_buffer_at(&mut self, index: usize) -> Vec<u8> {
        let before = self.buffer[..index].to_vec();
        self.discard_buffer_prefix(index);
        before
    }

    fn discard_buffer_prefix(&mut self, index: usize) {
        self.buffer = self.buffer[index..].to_vec();
    }
}

fn worksheet_start_tag_end(buffer: &[u8]) -> Option<usize> {
    let worksheet_start_re = WORKSHEET_START_RE
        .get_or_init(|| Regex::new(r#"<([A-Za-z_][\w.-]*:)?worksheet\b"#).unwrap());
    let worksheet_start = worksheet_start_re.find(buffer)?.start();
    find_byte_from(buffer, b'>', worksheet_start).map(|end| end + 1)
}

#[inline]
fn progress_label(state: SheetStreamingState) -> &'static str {
    match state {
        SheetStreamingState::SeekWorksheetNode => "seek_worksheet_node",
        SheetStreamingState::InWorksheetNode => "scan_worksheet_node_children",
        SheetStreamingState::InColsNode => "process_cols_node",
        SheetStreamingState::InSheetDataNode => "preprocess_sheet_data_node",
        SheetStreamingState::InMergeCellsNode => "process_merge_cells_node",
        SheetStreamingState::FoundWorksheetEnd => "found_worksheet_end",
    }
}

#[inline]
pub(super) fn extend_without_large_growth(out: &mut Vec<u8>, bytes: &[u8]) {
    let spare = out.capacity().saturating_sub(out.len());
    if spare < bytes.len() {
        out.reserve_exact((bytes.len() - spare).max(16 * 1024 * 1024));
    }
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CleanupSwitches, SheetPreprocessConfig, WorkbookSpecificPatterns};
    use zip::CompressionMethod;

    fn params() -> CleanSheetParameters {
        CleanSheetParameters {
            xlsx_file_path: "test.xlsx".to_string(),
            zip_info_filename: "xl/worksheets/sheet1.xml".to_string(),
            compress_method: CompressionMethod::Deflated,
            patterns: WorkbookSpecificPatterns {
                xfs_by_index: HashMap::new(),
                no_empty_cell_visible_style_ids: Vec::new(),
                fill_or_border_or_protection_ids: Vec::new(),
                whitespace_indices: None,
            },
            advanced_cleanup: None,
            preprocess: SheetPreprocessConfig::default(),
            cleanup: CleanupSwitches::default(),
            collect_cleanup_stats: true,
        }
    }

    fn worksheet() -> &'static [u8] {
        br#"<worksheet><cols><col min="1" max="1"/></cols><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c r="B2"><f>'Other Sheet'!$C$3</f><v>2</v></c></row><row r="3"/></sheetData><mergeCells count="1"><mergeCell ref="A1:B1"/></mergeCells></worksheet>"#
    }

    #[test]
    fn online_chunks_preserve_serial_preprocessing_output() {
        let params = params();
        let serial = preprocess_sheet_xml_streamwise_parallel_with_progress(
            worksheet(),
            &params,
            17,
            0,
            None,
            &mut |_| {},
        )
        .unwrap();
        let parallel = preprocess_sheet_xml_streamwise_parallel_with_progress(
            worksheet(),
            &params,
            17,
            48,
            None,
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(parallel.sheet_data, serial.sheet_data);
        assert_eq!(
            parallel.preprocessed_visible_data_bounds,
            serial.preprocessed_visible_data_bounds
        );
        assert_eq!(
            parallel.clean_sheet_metrics.merge_cell_ranges,
            serial.clean_sheet_metrics.merge_cell_ranges
        );
    }

    #[test]
    fn online_chunks_collect_cross_sheet_references() {
        let params = params();
        let names = Arc::new(HashMap::from([
            (
                "Other Sheet".to_string(),
                "xl/worksheets/sheet2.xml".to_string(),
            ),
            ("Sheet1".to_string(), "xl/worksheets/sheet1.xml".to_string()),
        ]));
        let preprocessed = preprocess_sheet_xml_streamwise_parallel_with_progress(
            worksheet(),
            &params,
            13,
            40,
            Some(SheetReferenceScanOptions {
                sheet_name_to_file: names,
            }),
            &mut |_| {},
        )
        .unwrap();

        assert!(preprocessed.clean_sheet_metrics.reference_summary.complete);
        assert!(preprocessed
            .clean_sheet_metrics
            .reference_summary
            .protected_refs
            .contains("xl/worksheets/sheet2.xml", 3, 3));
    }
}
