use crate::error::{Result, ShrinkError};
use crate::models::{
    CleanSheetMetrics, CleanSheetParameters, CleanupStats, ColumnRangeMap, SheetReferenceSummary,
    VisibleContentBounds,
};
use crate::reference_scanner::scan_sheet_fragment;
use crate::sheet::ranges::find_visible_content_bounds;
use crate::sheet::sheet_data::{pre_process_sheet_data_chunk, SheetDataPreprocessState};
use crate::sheet::streaming::{
    extend_without_large_growth, PreprocessedSheetXml, SheetReferenceScanOptions,
};
use crate::xml::{find_byte_from, find_bytes};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use tracing::debug;

static ONLINE_PREPROCESS_TASKS: AtomicUsize = AtomicUsize::new(0);

struct PreprocessedSheetDataChunk {
    index: usize,
    xml: Vec<u8>,
    visible_bounds: VisibleContentBounds,
    cleanup_stats: CleanupStats,
    reference_summary: SheetReferenceSummary,
}

pub(super) struct SheetDataChunkExecutor {
    params: Arc<CleanSheetParameters>,
    columns: Arc<ColumnRangeMap>,
    pub(super) reference_scan: Option<SheetReferenceScanOptions>,
    target_size: usize,
    max_in_flight: usize,
    input: Vec<u8>,
    sender: mpsc::Sender<Result<PreprocessedSheetDataChunk>>,
    receiver: mpsc::Receiver<Result<PreprocessedSheetDataChunk>>,
    in_flight: usize,
    next_chunk_index: usize,
    next_output_index: usize,
    completed: BTreeMap<usize, PreprocessedSheetDataChunk>,
    visible_bounds: VisibleContentBounds,
}

impl SheetDataChunkExecutor {
    pub(super) fn new(
        params: &CleanSheetParameters,
        target_size: usize,
        reference_scan: Option<SheetReferenceScanOptions>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            params: Arc::new(params.clone()),
            columns: Arc::new(ColumnRangeMap::default()),
            reference_scan,
            target_size,
            max_in_flight: online_task_limit(),
            input: Vec::new(),
            sender,
            receiver,
            in_flight: 0,
            next_chunk_index: 0,
            next_output_index: 0,
            completed: BTreeMap::new(),
            visible_bounds: VisibleContentBounds::default(),
        }
    }

    pub(super) fn configure_columns(&mut self, columns: ColumnRangeMap) {
        self.columns = Arc::new(columns);
    }

    pub(super) fn push(&mut self, bytes: &[u8], result: &mut PreprocessedSheetXml) -> Result<()> {
        self.input.extend_from_slice(bytes);
        if self.target_size == 0 {
            return self.process_buffer_inline(result);
        }

        while self.input.len() >= self.target_size {
            let Some(end) = row_aligned_prefix_end(&self.input, self.target_size) else {
                break;
            };
            let chunk = take_vec_prefix(&mut self.input, end);
            self.dispatch(chunk);
            self.collect_available(result)?;
            self.apply_backpressure(result)?;
        }
        Ok(())
    }

    pub(super) fn finish(&mut self, result: &mut PreprocessedSheetXml) -> Result<()> {
        if !self.input.is_empty() {
            if self.target_size == 0 {
                self.process_buffer_inline(result)?;
            } else {
                let input = std::mem::take(&mut self.input);
                self.dispatch(input);
            }
        }

        while self.in_flight > 0 {
            self.collect_one(result)?;
        }
        self.flush_completed(result);
        result.preprocessed_visible_data_bounds = Some(self.visible_bounds);
        if self.next_chunk_index > 1 {
            debug!(
                event = "online_preprocess_chunks_finished",
                sheet = %self.params.zip_info_filename,
                chunks = self.next_chunk_index,
                target_bytes = self.target_size,
                "Finished row-aligned online preprocessing chunks"
            );
        }
        Ok(())
    }

    fn process_buffer_inline(&mut self, result: &mut PreprocessedSheetXml) -> Result<()> {
        if self.input.is_empty() {
            return Ok(());
        }
        let input = std::mem::take(&mut self.input);
        let chunk = preprocess_sheet_data_chunk_task(
            self.next_chunk_index,
            input,
            &self.params,
            &self.columns,
            self.reference_scan.as_ref(),
        )?;
        self.next_chunk_index += 1;
        self.completed.insert(chunk.index, chunk);
        self.flush_completed(result);
        Ok(())
    }

    fn dispatch(&mut self, input: Vec<u8>) {
        acquire_online_task_slot();
        let index = self.next_chunk_index;
        self.next_chunk_index += 1;
        self.in_flight += 1;

        let params = self.params.clone();
        let columns = self.columns.clone();
        let reference_scan = self.reference_scan.clone();
        let sender = self.sender.clone();
        rayon::spawn_fifo(move || {
            let task = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                preprocess_sheet_data_chunk_task(
                    index,
                    input,
                    &params,
                    &columns,
                    reference_scan.as_ref(),
                )
            }))
            .unwrap_or_else(|_| {
                Err(ShrinkError::MissingAttribute(
                    "parallel sheet-data preprocessing task panicked".to_string(),
                ))
            });
            let _ = sender.send(task);
            ONLINE_PREPROCESS_TASKS.fetch_sub(1, Ordering::Release);
        });
    }

    fn apply_backpressure(&mut self, result: &mut PreprocessedSheetXml) -> Result<()> {
        while self.in_flight >= self.max_in_flight {
            match self.receiver.try_recv() {
                Ok(chunk) => self.accept(chunk?, result),
                Err(mpsc::TryRecvError::Empty) => yield_to_worker(),
                Err(mpsc::TryRecvError::Disconnected) => return Err(ShrinkError::UnexpectedEof),
            }
        }
        Ok(())
    }

    fn collect_available(&mut self, result: &mut PreprocessedSheetXml) -> Result<()> {
        loop {
            match self.receiver.try_recv() {
                Ok(chunk) => self.accept(chunk?, result),
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => return Err(ShrinkError::UnexpectedEof),
            }
        }
    }

    fn collect_one(&mut self, result: &mut PreprocessedSheetXml) -> Result<()> {
        loop {
            match self.receiver.try_recv() {
                Ok(chunk) => {
                    self.accept(chunk?, result);
                    return Ok(());
                }
                Err(mpsc::TryRecvError::Empty) => yield_to_worker(),
                Err(mpsc::TryRecvError::Disconnected) => return Err(ShrinkError::UnexpectedEof),
            }
        }
    }

    fn accept(&mut self, chunk: PreprocessedSheetDataChunk, result: &mut PreprocessedSheetXml) {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.completed.insert(chunk.index, chunk);
        self.flush_completed(result);
    }

    fn flush_completed(&mut self, result: &mut PreprocessedSheetXml) {
        while let Some(chunk) = self.completed.remove(&self.next_output_index) {
            extend_without_large_growth(&mut result.sheet_data, &chunk.xml);
            merge_bounds(&mut self.visible_bounds, chunk.visible_bounds);
            result
                .clean_sheet_metrics
                .cleanup_stats
                .merge(chunk.cleanup_stats);
            result
                .clean_sheet_metrics
                .reference_summary
                .merge(chunk.reference_summary);
            self.next_output_index += 1;
        }
    }
}

fn preprocess_sheet_data_chunk_task(
    index: usize,
    input: Vec<u8>,
    params: &CleanSheetParameters,
    columns: &ColumnRangeMap,
    reference_scan: Option<&SheetReferenceScanOptions>,
) -> Result<PreprocessedSheetDataChunk> {
    let mut state = SheetDataPreprocessState::new(columns, params);
    let mut metrics = CleanSheetMetrics::default();
    metrics.cleanup_stats.enabled = params.collect_cleanup_stats;
    let cleaned = pre_process_sheet_data_chunk(&input, params, &mut state, &mut metrics)?;
    let xml = match cleaned {
        std::borrow::Cow::Borrowed(_) => input,
        std::borrow::Cow::Owned(xml) => xml,
    };
    let visible_bounds = find_visible_content_bounds(&xml, params, columns);
    let reference_summary = reference_scan.map_or_else(SheetReferenceSummary::default, |scan| {
        scan_sheet_fragment(&xml, &params.zip_info_filename, &scan.sheet_name_to_file)
    });

    Ok(PreprocessedSheetDataChunk {
        index,
        xml,
        visible_bounds,
        cleanup_stats: metrics.cleanup_stats,
        reference_summary,
    })
}

fn merge_bounds(target: &mut VisibleContentBounds, additional: VisibleContentBounds) {
    if let Some(col) = additional.max_col {
        target.include_col(col);
    }
    if let Some(row) = additional.max_row {
        target.include_row(row);
    }
}

fn take_vec_prefix(data: &mut Vec<u8>, end: usize) -> Vec<u8> {
    let suffix = data.split_off(end);
    std::mem::replace(data, suffix)
}

fn row_aligned_prefix_end(data: &[u8], target: usize) -> Option<usize> {
    let mut position = 0;
    let mut last_at_or_before_target = None;
    let mut first_after_target = None;

    while let Some(row_start_relative) = find_bytes(&data[position..], b"<row") {
        let row_start = position + row_start_relative;
        let Some(tag_end) = find_byte_from(data, b'>', row_start) else {
            break;
        };
        let row_end = if tag_end > row_start && data[tag_end - 1] == b'/' {
            tag_end + 1
        } else {
            let content_start = tag_end + 1;
            let Some(closing_relative) = find_bytes(&data[content_start..], b"</row>") else {
                break;
            };
            content_start + closing_relative + b"</row>".len()
        };

        if row_end <= target {
            last_at_or_before_target = Some(row_end);
        } else {
            first_after_target = Some(row_end);
            break;
        }
        position = row_end;
    }

    last_at_or_before_target.or(first_after_target)
}

fn yield_to_worker() {
    rayon::yield_now();
    std::thread::yield_now();
}

fn online_task_limit() -> usize {
    rayon::current_num_threads().max(1).saturating_mul(2).min(8)
}

fn acquire_online_task_slot() {
    let limit = online_task_limit();
    loop {
        let active = ONLINE_PREPROCESS_TASKS.load(Ordering::Acquire);
        if active < limit
            && ONLINE_PREPROCESS_TASKS
                .compare_exchange_weak(active, active + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            return;
        }
        yield_to_worker();
    }
}

#[cfg(test)]
mod tests {
    use super::{row_aligned_prefix_end, SheetDataChunkExecutor};
    use crate::models::{CleanSheetParameters, SheetPreprocessConfig, WorkbookSpecificPatterns};
    use crate::sheet::streaming::PreprocessedSheetXml;
    use std::collections::HashMap;
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
            cleanup: Default::default(),
            collect_cleanup_stats: false,
        }
    }

    #[test]
    fn boundary_prefers_last_complete_row_before_target() {
        let xml = br#"<sheetData><row r="1"/><row r="2"><c r="A2"/></row><row r="3"/>"#;
        let second_row_end = xml
            .windows(b"</row>".len())
            .position(|window| window == b"</row>")
            .map(|position| position + b"</row>".len())
            .unwrap();

        assert_eq!(
            row_aligned_prefix_end(xml, second_row_end),
            Some(second_row_end)
        );
    }

    #[test]
    fn oversized_single_row_is_kept_whole() {
        let xml = br#"<sheetData><row r="1"><c r="A1"><v>123456789</v></c></row>"#;
        assert_eq!(row_aligned_prefix_end(xml, 8), Some(xml.len()));
    }

    #[test]
    fn online_executor_makes_progress_with_one_rayon_worker() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            let mut executor = SheetDataChunkExecutor::new(&params(), 24, None);
            let mut result = PreprocessedSheetXml::default();
            executor
                .push(
                    br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c r="A2"><v>2</v></c></row>"#,
                    &mut result,
                )
                .unwrap();
            executor.finish(&mut result).unwrap();

            assert!(String::from_utf8_lossy(&result.sheet_data).contains("r=\"A2\""));
        });
    }
}
