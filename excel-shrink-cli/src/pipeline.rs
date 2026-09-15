use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Cursor, Read, Seek, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use threadpool::ThreadPool;
use tracing::{debug, error, info, warn};
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use excel_shrink_core::models::{
    AdvancedCleanupConfig, CleanSheetMetrics, CleanSheetParameters, CleanupCandidate, CleanupCause,
    CleanupStats, CleanupSwitches, InactiveProtectionCellPolicy, ProtectedWorkbookRefs,
    SheetPreprocessConfig, SheetScanBuffer, WorkbookSpecificPatterns,
};
use excel_shrink_core::reference_scanner::{scan_for_references_by_sheet, ScanResult};
use excel_shrink_core::shared_strings::find_whitespace_sst_indices;
use excel_shrink_core::sheet::clean_sheet_data_streamwise_with_progress;
use excel_shrink_core::sheet::output::trim_trailing_layout_only_rows_vec;
use excel_shrink_core::sheet::sheet_data::{
    clean_empty_rows_from_after_cell_range, clean_selected_rows_and_cells_in_cell_range,
};
use excel_shrink_core::styles::parse_xfs;
use excel_shrink_core::workbook::clean_workbook_with_stats;
use excel_shrink_core::xml::{find_byte_from, find_bytes};

use crate::Cli;
use excel_shrink_core::profiling::{self, Handoff, ResourceGuard};

struct QueuedWriteJob {
    job: WriteJob,
    handoff: Handoff,
    queued_at: Instant,
    pending_jobs: ResourceGuard,
    pending_bytes: ResourceGuard,
}

enum WriteJob {
    CopyOriginal {
        index: usize,
        filename: String,
    },
    CopyPrecompressed {
        filename: String,
        zip_bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
struct ActiveTask {
    xlsx: String,
    sheet: String,
    xml_size: usize,
    started: Instant,
    phase: String,
    phase_started: Instant,
}

type ActiveTasks = Arc<Mutex<HashMap<String, ActiveTask>>>;

#[derive(Clone)]
struct WorkbookLimiter {
    inner: Arc<WorkbookLimiterInner>,
}

struct WorkbookLimiterInner {
    max_active: usize,
    active: Mutex<usize>,
    available: Condvar,
}

struct WorkbookPermit {
    inner: Arc<WorkbookLimiterInner>,
}

impl WorkbookLimiter {
    fn new(max_active: usize) -> Self {
        Self {
            inner: Arc::new(WorkbookLimiterInner {
                max_active: max_active.max(1),
                active: Mutex::new(0),
                available: Condvar::new(),
            }),
        }
    }

    fn acquire(&self, xlsx: &str) -> WorkbookPermit {
        let mut active = self.inner.active.lock().unwrap();
        while *active >= self.inner.max_active {
            let _wait = profiling::phase("wait_workbook_permit", xlsx, "");
            active = self.inner.available.wait(active).unwrap();
        }
        *active += 1;
        tracing::info!(target: "excel_shrink::profile", event = "active_workbooks", value = *active as u64);

        WorkbookPermit {
            inner: self.inner.clone(),
        }
    }
}

impl Drop for WorkbookPermit {
    fn drop(&mut self) {
        let mut active = self.inner.active.lock().unwrap();
        *active = active.saturating_sub(1);
        tracing::info!(target: "excel_shrink::profile", event = "active_workbooks", value = *active as u64);
        self.inner.available.notify_one();
    }
}

#[derive(Clone)]
struct BatchProgress {
    files_completed: Arc<AtomicUsize>,
    files_active: Arc<AtomicUsize>,
    files_total: usize,
}

impl BatchProgress {
    fn new(files_total: usize) -> Self {
        Self {
            files_completed: Arc::new(AtomicUsize::new(0)),
            files_active: Arc::new(AtomicUsize::new(0)),
            files_total,
        }
    }

    fn start_file(&self, xlsx: String) -> XlsxProgress {
        let files_active = self.files_active.fetch_add(1, Ordering::Relaxed) + 1;
        let files_completed = self.files_completed.load(Ordering::Relaxed);
        let files_remaining = self.files_total.saturating_sub(files_completed);
        info!(
            event = "file_started",
            xlsx = %xlsx,
            files_total = self.files_total,
            files_completed,
            files_remaining,
            files_active,
            "Started XLSX processing"
        );

        XlsxProgress {
            batch: self.clone(),
            xlsx,
            finished: false,
        }
    }
}

struct XlsxProgress {
    batch: BatchProgress,
    xlsx: String,
    finished: bool,
}

impl XlsxProgress {
    fn finish(
        &mut self,
        event: &'static str,
        output_written: bool,
        elapsed_ms: u128,
        jobs_total: Option<usize>,
        message: &'static str,
    ) {
        if !self.finished {
            self.finished = true;
            let files_completed = self.batch.files_completed.fetch_add(1, Ordering::Relaxed) + 1;
            let files_active = self
                .batch
                .files_active
                .fetch_sub(1, Ordering::Relaxed)
                .saturating_sub(1);
            let files_remaining = self.batch.files_total.saturating_sub(files_completed);
            info!(
                event,
                xlsx = %self.xlsx,
                output_written,
                elapsed_ms,
                jobs_total,
                files_total = self.batch.files_total,
                files_completed,
                files_remaining,
                files_active,
                "{message}"
            );
        }
    }
}

impl Drop for XlsxProgress {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(
                "file_processing_stopped",
                false,
                0,
                None,
                "XLSX processing stopped before output was written",
            );
        }
    }
}
struct ProcessedSheet {
    params: CleanSheetParameters,
    zip_index: usize,
    xml: Vec<u8>,
    sheet_data_range: Range<usize>,
    metrics: CleanSheetMetrics,
    started: Instant,
    original_size: usize,
    buffered_bytes: ResourceGuard,
    initial_finished: Instant,
    handoff: Handoff,
}

struct FailedSheet {
    params: CleanSheetParameters,
    zip_index: usize,
}

type ProcessedSheetResult = Result<ProcessedSheet, FailedSheet>;

#[derive(Debug, Clone)]
struct SheetTask {
    params: CleanSheetParameters,
    zip_index: usize,
    original_xml_size: usize,
    handoff: Handoff,
}

struct ArchivePlan {
    sheet_tasks: Vec<SheetTask>,
    workbook_xml_for_scan: Option<Vec<u8>>,
    workbook_rels_for_scan: Option<Vec<u8>>,
    workbook_xml_for_calc_chain: Option<Vec<u8>>,
    workbook_rels_for_calc_chain: Option<Vec<u8>>,
    calc_chain_part: Option<(usize, CompressionMethod, Vec<u8>)>,
}

impl ArchivePlan {
    fn new() -> Self {
        Self {
            sheet_tasks: Vec::new(),
            workbook_xml_for_scan: None,
            workbook_rels_for_scan: None,
            workbook_xml_for_calc_chain: None,
            workbook_rels_for_calc_chain: None,
            calc_chain_part: None,
        }
    }
}

struct ReferenceValidation {
    protected_refs: ProtectedWorkbookRefs,
    safe_to_clean: bool,
}

#[derive(Clone, Copy)]
struct AdvancedCleanupSelection {
    remove_whitespace: bool,
    remove_hidden_content: bool,
}

impl AdvancedCleanupSelection {
    fn enabled(self) -> bool {
        self.remove_whitespace || self.remove_hidden_content
    }

    fn config(self) -> Option<Arc<AdvancedCleanupConfig>> {
        self.enabled().then(|| {
            Arc::new(AdvancedCleanupConfig {
                remove_unreferenced_whitespace: self.remove_whitespace,
                remove_unreferenced_hidden_content: self.remove_hidden_content,
            })
        })
    }
}

struct WorkbookPreparation {
    patterns: WorkbookSpecificPatterns,
    cleanup: CleanupSwitches,
    collect_cleanup_stats: bool,
    selected_sheets: HashSet<String>,
}

struct ArchivePlanningContext<'a> {
    input: &'a Path,
    input_display: &'a str,
    cli: &'a Cli,
    patterns: &'a WorkbookSpecificPatterns,
    cleanup: &'a CleanupSwitches,
    collect_cleanup_stats: bool,
    selected_sheets: &'a HashSet<String>,
    tx: &'a mpsc::Sender<QueuedWriteJob>,
    advanced_cleanup_enabled: bool,
    analysis_tx: Option<&'a mpsc::Sender<AnalysisCsvRow>>,
    attribution_safe_analysis: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArchiveEntryKind {
    Sheet,
    Workbook,
    WorkbookRelationships,
    CalculationChain,
    CopyOriginal,
}

enum ArchiveEntryPlan {
    Continue,
    Abort,
}

struct SheetFinalizer<'a> {
    tx: &'a mpsc::Sender<QueuedWriteJob>,
    analysis_tx: Option<&'a mpsc::Sender<AnalysisCsvRow>>,
    known_cells_by_sheet_file: &'a Mutex<HashMap<String, HashSet<String>>>,
    protected_refs: &'a ProtectedWorkbookRefs,
    safe_to_clean: bool,
    attribution_safe_analysis: bool,
    store_output: bool,
    compression_level: i32,
}

struct InitialSheetProcessor {
    source_zip_bytes: Arc<Vec<u8>>,
    active_tasks: ActiveTasks,
    chunk_size: usize,
    disable_splitting: bool,
    parallel_range_chunk_size: usize,
}

struct ActiveSheetGuard {
    active_tasks: ActiveTasks,
    task_key: String,
}

impl Drop for ActiveSheetGuard {
    fn drop(&mut self) {
        let mut guard = self.active_tasks.lock().unwrap();
        guard.remove(&self.task_key);
    }
}
#[derive(Debug, Clone)]
struct AnalysisCsvRow {
    input_file: String,
    sheet_file: String,
    cause: CleanupCause,
    removed_items: usize,
    removed_bytes_uncompressed_xml: usize,
    added_bytes_uncompressed_xml: usize,
    reduction_bytes_uncompressed_xml: isize,
    reduction_ratio: f64,
    original_sheet_xml_bytes: usize,
    cleaned_sheet_xml_bytes: usize,
    sheet_reduction_bytes_uncompressed_xml: isize,
    sheet_reduction_ratio: f64,
    analysis_mode: bool,
}

fn default_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn normalize_sheet_target(target: &str) -> String {
    let without_leading_slash = target.trim_start_matches('/');

    if without_leading_slash.starts_with("xl/") {
        without_leading_slash.to_string()
    } else if without_leading_slash.starts_with("worksheets/") {
        format!("xl/{without_leading_slash}")
    } else if without_leading_slash.starts_with("sheet") && without_leading_slash.ends_with(".xml")
    {
        format!("xl/worksheets/{without_leading_slash}")
    } else {
        format!("xl/{without_leading_slash}")
    }
}

fn selected_sheet_targets(sheets: &[String]) -> HashSet<String> {
    sheets
        .iter()
        .map(|sheet| sheet.trim())
        .filter(|sheet| !sheet.is_empty())
        .map(normalize_sheet_target)
        .collect()
}

fn sheet_selected_for_cleanup(sheet_file: &str, selected_sheets: &HashSet<String>) -> bool {
    selected_sheets.is_empty() || selected_sheets.contains(sheet_file)
}

fn attribution_safe_analysis(cli: &Cli) -> bool {
    cli.analysis_mode || cli.analysis_csv.is_some()
}

fn cleanup_stats_enabled(cli: &Cli) -> bool {
    attribution_safe_analysis(cli)
}

fn csv_escape(value: &str) -> String {
    if !value
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b',' | b'"' | b'\n' | b'\r'))
    {
        return value.to_string();
    }

    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn write_analysis_csv_row<W: Write>(writer: &mut W, row: &AnalysisCsvRow) -> std::io::Result<()> {
    let fields = vec![
        csv_escape(&row.input_file),
        csv_escape(&row.sheet_file),
        row.cause.as_str().to_string(),
        row.removed_items.to_string(),
        row.removed_bytes_uncompressed_xml.to_string(),
        row.added_bytes_uncompressed_xml.to_string(),
        row.reduction_bytes_uncompressed_xml.to_string(),
        format!("{:.6}", row.reduction_ratio),
        row.original_sheet_xml_bytes.to_string(),
        row.cleaned_sheet_xml_bytes.to_string(),
        row.sheet_reduction_bytes_uncompressed_xml.to_string(),
        format!("{:.6}", row.sheet_reduction_ratio),
        row.analysis_mode.to_string(),
    ];

    writeln!(writer, "{}", fields.join(","))
}

fn spawn_analysis_csv_writer(
    path: &PathBuf,
) -> anyhow::Result<(mpsc::Sender<AnalysisCsvRow>, std::thread::JoinHandle<()>)> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    write_analysis_csv_header(&mut writer)?;
    writer.flush()?;

    let (tx, rx) = mpsc::channel::<AnalysisCsvRow>();
    let handle = std::thread::spawn(move || {
        for row in rx {
            if let Err(e) = write_analysis_csv_row(&mut writer, &row) {
                error!(event = "analysis_csv_write_failed", error = %e, "Failed to write analysis CSV row");
                break;
            }
            let _ = writer.flush();
        }

        let _ = writer.flush();
    });

    Ok((tx, handle))
}

fn send_cleanup_stats_rows(
    tx: Option<&mpsc::Sender<AnalysisCsvRow>>,
    input_file: &str,
    sheet_file: &str,
    stats: &CleanupStats,
    original_sheet_xml_bytes: usize,
    cleaned_sheet_xml_bytes: usize,
    analysis_mode: bool,
) {
    let Some(tx) = tx else {
        return;
    };

    let sheet_reduction_bytes =
        original_sheet_xml_bytes as isize - cleaned_sheet_xml_bytes as isize;

    for cause in CleanupCause::ALL {
        let removed_bytes = *stats.removed_bytes_by_cause.get(&cause).unwrap_or(&0);
        let added_bytes = *stats.added_bytes_by_cause.get(&cause).unwrap_or(&0);
        let reduction_bytes = removed_bytes as isize - added_bytes as isize;
        let row = AnalysisCsvRow {
            input_file: input_file.to_string(),
            sheet_file: sheet_file.to_string(),
            cause,
            removed_items: *stats.removed_items_by_cause.get(&cause).unwrap_or(&0),
            removed_bytes_uncompressed_xml: removed_bytes,
            added_bytes_uncompressed_xml: added_bytes,
            reduction_bytes_uncompressed_xml: reduction_bytes,
            reduction_ratio: reduction_ratio(reduction_bytes, original_sheet_xml_bytes),
            original_sheet_xml_bytes,
            cleaned_sheet_xml_bytes,
            sheet_reduction_bytes_uncompressed_xml: sheet_reduction_bytes,
            sheet_reduction_ratio: reduction_ratio(sheet_reduction_bytes, original_sheet_xml_bytes),
            analysis_mode,
        };

        if let Err(e) = tx.send(row) {
            error!(event = "analysis_csv_channel_closed", error = %e, "Cannot queue analysis CSV row");
            return;
        }
    }
}

fn write_analysis_csv_header<W: Write>(writer: &mut W) -> std::io::Result<()> {
    writeln!(
        writer,
        "input_file,sheet_file,cause,removed_items,removed_bytes_uncompressed_xml,added_bytes_uncompressed_xml,reduction_bytes_uncompressed_xml,reduction_ratio,original_sheet_xml_bytes,cleaned_sheet_xml_bytes,sheet_reduction_bytes_uncompressed_xml,sheet_reduction_ratio,analysis_mode"
    )
}

fn reduction_ratio(reduced_bytes: isize, original_bytes: usize) -> f64 {
    if original_bytes == 0 {
        0.0
    } else {
        reduced_bytes as f64 / original_bytes as f64
    }
}

fn extract_sheet_name_and_rid_in_order(workbook_xml: &[u8]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let re = regex::bytes::Regex::new(
        r#"(?s)<(?:[A-Za-z_][\w.-]*:)?sheet\b[^>]*\bname="([^"]+)"[^>]*\br:id="([^"]+)"[^>]*>"#,
    )
    .unwrap();

    for caps in re.captures_iter(workbook_xml) {
        let Some(name_bytes) = caps.get(1).map(|m| m.as_bytes()) else {
            continue;
        };
        let Some(rid_bytes) = caps.get(2).map(|m| m.as_bytes()) else {
            continue;
        };

        let Ok(name) = std::str::from_utf8(name_bytes) else {
            continue;
        };
        let Ok(rid) = std::str::from_utf8(rid_bytes) else {
            continue;
        };

        pairs.push((name.to_string(), rid.to_string()));
    }

    pairs
}

fn extract_sheet_id_name_and_rid_in_order(workbook_xml: &[u8]) -> Vec<(String, String, String)> {
    let mut triples = Vec::new();
    let re = regex::bytes::Regex::new(r#"(?s)<(?:[A-Za-z_][\w.-]*:)?sheet\b[^>]*>"#).unwrap();

    for sheet_match in re.find_iter(workbook_xml) {
        let sheet = sheet_match.as_bytes();
        let Some(sheet_id) = extract_attr_string(sheet, b"sheetId") else {
            continue;
        };
        let Some(name) = extract_attr_string(sheet, b"name") else {
            continue;
        };
        let Some(rid) = extract_attr_string(sheet, b"r:id") else {
            continue;
        };

        triples.push((sheet_id, name, rid));
    }

    triples
}

fn extract_rid_to_target(workbook_rels_xml: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let re = regex::bytes::Regex::new(
        r#"(?s)<(?:[A-Za-z_][\w.-]*:)?Relationship\b[^>]*\bId="([^"]+)"[^>]*\bTarget="([^"]+)"[^>]*>"#,
    )
    .unwrap();

    for caps in re.captures_iter(workbook_rels_xml) {
        let Some(rid_bytes) = caps.get(1).map(|m| m.as_bytes()) else {
            continue;
        };
        let Some(target_bytes) = caps.get(2).map(|m| m.as_bytes()) else {
            continue;
        };

        let Ok(rid) = std::str::from_utf8(rid_bytes) else {
            continue;
        };
        let Ok(target) = std::str::from_utf8(target_bytes) else {
            continue;
        };

        map.insert(rid.to_string(), normalize_sheet_target(target));
    }

    map
}

fn extract_attr_string(element: &[u8], attr_name: &[u8]) -> Option<String> {
    let mut pos = 0;
    while pos + attr_name.len() + 2 <= element.len() {
        let rel = find_bytes(&element[pos..], attr_name)?;
        let start = pos + rel;
        let before_ok =
            start == 0 || element[start - 1].is_ascii_whitespace() || element[start - 1] == b'<';
        let after_name = start + attr_name.len();
        if before_ok && after_name < element.len() && element[after_name] == b'=' {
            let quote_pos = after_name + 1;
            if quote_pos < element.len() && element[quote_pos] == b'"' {
                let value_start = quote_pos + 1;
                let value_end =
                    value_start + element[value_start..].iter().position(|b| *b == b'"')?;
                return std::str::from_utf8(&element[value_start..value_end])
                    .ok()
                    .map(str::to_string);
            }
        }
        pos = after_name;
    }

    None
}

fn build_sheet_name_to_file_map(
    workbook_xml: &[u8],
    workbook_rels_xml: &[u8],
) -> (HashMap<String, String>, Vec<String>) {
    let name_rid_pairs = extract_sheet_name_and_rid_in_order(workbook_xml);
    let rid_to_target = extract_rid_to_target(workbook_rels_xml);

    let mut map = HashMap::new();
    let mut sheet_files_in_workbook_order = Vec::new();

    for (name, rid) in name_rid_pairs {
        if let Some(target) = rid_to_target.get(&rid) {
            map.insert(name, target.clone());
            sheet_files_in_workbook_order.push(target.clone());
        }
    }

    (map, sheet_files_in_workbook_order)
}

fn build_sheet_id_to_file_map(
    workbook_xml: &[u8],
    workbook_rels_xml: &[u8],
) -> HashMap<String, String> {
    let sheet_id_name_rid_triples = extract_sheet_id_name_and_rid_in_order(workbook_xml);
    let rid_to_target = extract_rid_to_target(workbook_rels_xml);

    let mut map = HashMap::new();
    for (sheet_id, _name, rid) in sheet_id_name_rid_triples {
        if let Some(target) = rid_to_target.get(&rid) {
            map.insert(sheet_id, target.clone());
        }
    }

    map
}

fn collect_cell_refs(xml: &[u8]) -> HashSet<String> {
    let _profile = profiling::phase("collect_cell_references", "", "");
    let mut refs = HashSet::new();
    let re = regex::bytes::Regex::new(r#"(?s)<c\b[^>]*\br="([A-Z]+\d+)"[^>]*(?:/|>)"#).unwrap();

    for caps in re.captures_iter(xml) {
        if let Some(reference) = caps.get(1) {
            if let Ok(reference) = std::str::from_utf8(reference.as_bytes()) {
                refs.insert(reference.to_string());
            }
        }
    }

    refs
}

fn clean_calc_chain(
    calc_chain_xml: Vec<u8>,
    sheet_id_to_file: &HashMap<String, String>,
    known_cells_by_sheet_file: &HashMap<String, HashSet<String>>,
) -> Vec<u8> {
    if known_cells_by_sheet_file.is_empty() {
        return calc_chain_xml;
    }

    let re = regex::bytes::Regex::new(r#"(?s)<(?:[A-Za-z_][\w.-]*:)?c\b[^>]*/>"#).unwrap();
    let mut out = Vec::with_capacity(calc_chain_xml.len());
    let mut last_end = 0;
    let mut current_sheet_id: Option<String> = None;

    for m in re.find_iter(&calc_chain_xml) {
        let entry = m.as_bytes();
        if let Some(sheet_id) = extract_attr_string(entry, b"i") {
            current_sheet_id = Some(sheet_id);
        } else if current_sheet_id.is_none() {
            current_sheet_id = Some("1".to_string());
        }

        let remove = extract_attr_string(entry, b"r").is_some_and(|cell_ref| {
            current_sheet_id
                .as_deref()
                .and_then(|sheet_id| sheet_id_to_file.get(sheet_id))
                .and_then(|sheet_file| known_cells_by_sheet_file.get(sheet_file))
                .is_some_and(|known_cells| !known_cells.contains(&cell_ref))
        });

        out.extend_from_slice(&calc_chain_xml[last_end..m.start()]);
        if !remove {
            out.extend_from_slice(entry);
        }
        last_end = m.end();
    }

    out.extend_from_slice(&calc_chain_xml[last_end..]);
    out
}

fn start_watchdog(active_tasks: ActiveTasks) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    let handle = std::thread::spawn(move || {
        let warn_after = Duration::from_secs(30);
        let warn_interval = Duration::from_secs(120);
        let tick = Duration::from_secs(1);
        let mut last_warn_by_task: HashMap<String, Instant> = HashMap::new();

        loop {
            match stop_rx.recv_timeout(tick) {
                Ok(_) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            let now = Instant::now();
            let tasks_snapshot: Vec<ActiveTask> = {
                let guard = active_tasks.lock().unwrap();
                guard.values().cloned().collect()
            };
            let active_keys: std::collections::HashSet<String> = tasks_snapshot
                .iter()
                .map(|task| format!("{}::{}", task.xlsx, task.sheet))
                .collect();
            last_warn_by_task.retain(|key, _| active_keys.contains(key));

            for task in tasks_snapshot {
                let elapsed = now.duration_since(task.started);
                if elapsed < warn_after {
                    continue;
                }

                let task_key = format!("{}::{}", task.xlsx, task.sheet);
                let should_warn = last_warn_by_task
                    .get(&task_key)
                    .is_none_or(|last_warn| now.duration_since(*last_warn) >= warn_interval);
                if !should_warn {
                    continue;
                }

                last_warn_by_task.insert(task_key, now);
                warn!(
                    "active_slow_sheet: elapsed={:.2?}, phase_elapsed={:.2?}, file={}, sheet={}, xml_size={} bytes, phase={}",
                    elapsed,
                    now.duration_since(task.phase_started),
                    task.xlsx,
                    task.sheet,
                    task.xml_size,
                    task.phase
                );
            }
        }
    });

    (stop_tx, handle)
}

fn output_compression_method(
    original_method: CompressionMethod,
    store_output: bool,
) -> CompressionMethod {
    if store_output {
        return CompressionMethod::Stored;
    }
    match original_method {
        CompressionMethod::Stored => CompressionMethod::Stored,
        CompressionMethod::Deflated => CompressionMethod::Deflated,
        _ => CompressionMethod::Deflated,
    }
}

fn normalized_compression_level(level: i32) -> i32 {
    level.clamp(0, 9)
}

fn precompress_entry(
    filename: &str,
    data: Vec<u8>,
    method: CompressionMethod,
    compression_level: i32,
) -> anyhow::Result<Vec<u8>> {
    let _profile = tracing::info_span!(target: "excel_shrink::profile", "compress_entry", sheet = filename, input_bytes = data.len(), compression_level).entered();
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip_writer = ZipWriter::new(&mut cursor);
        let mut options = zip::write::FileOptions::default()
            .compression_method(method)
            .large_file(true);

        if method == CompressionMethod::Deflated {
            options =
                options.compression_level(Some(normalized_compression_level(compression_level)));
        }

        zip_writer.start_file(filename, options)?;
        zip_writer.write_all(&data)?;
        zip_writer.finish()?;
    }
    Ok(cursor.into_inner())
}

fn raw_copy_single_entry_zip<W: Write + Seek>(
    zip_writer: &mut ZipWriter<W>,
    zip_bytes: Vec<u8>,
) -> zip::result::ZipResult<()> {
    let cursor = Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(cursor)?;
    let file = archive.by_index(0)?;
    zip_writer.raw_copy_file(file)
}

fn spawn_xlsx_writer(
    writer_pool: &ThreadPool,
    input_path: PathBuf,
    out_path: PathBuf,
    rx: mpsc::Receiver<QueuedWriteJob>,
    input_display: String,
    mut progress: XlsxProgress,
    workbook_permit: WorkbookPermit,
) {
    let handoff = Handoff::start("writer_scheduled", &input_display, "");
    writer_pool.execute(move || {
        handoff.finish();
        let _profile = profiling::phase("writer_lifecycle_including_wait", &input_display, "");
        write_xlsx(
            input_path,
            out_path,
            rx,
            input_display,
            &mut progress,
            workbook_permit,
        );
    });
}

fn write_xlsx(
    input_path: PathBuf,
    out_path: PathBuf,
    rx: mpsc::Receiver<QueuedWriteJob>,
    input_display: String,
    progress: &mut XlsxProgress,
    workbook_permit: WorkbookPermit,
) {
    let _workbook_permit = workbook_permit;
    let writer_start = Instant::now();
    let Some(mut source_archive) = open_source_archive(&input_path, &input_display) else {
        return;
    };
    let Some((partial_out_path, buffered_output)) =
        prepare_partial_output(&out_path, &input_display)
    else {
        return;
    };

    let mut zip_writer = ZipWriter::new(buffered_output);
    let timings = copy_write_jobs(rx, &mut source_archive, &mut zip_writer, &input_display);
    info!(event = "writer_timing", xlsx = %input_display, receive_wait_ms = timings.wait.as_millis(), copy_entries_ms = timings.copy.as_millis(), "Writer receive wait and entry-copy wall times");
    if !finish_output_archive(zip_writer, &partial_out_path, &out_path, &input_display) {
        return;
    }

    progress.finish(
        "file_written",
        true,
        writer_start.elapsed().as_millis(),
        Some(timings.jobs),
        "Finished writing XLSX",
    );
}

fn open_source_archive(input_path: &Path, input_display: &str) -> Option<ZipArchive<File>> {
    let source_file = match File::open(input_path) {
        Ok(file) => file,
        Err(error) => {
            error!(event = "writer_source_open_failed", xlsx = %input_display, error = %error, "Failed to open source XLSX for raw copying");
            return None;
        }
    };

    match ZipArchive::new(source_file) {
        Ok(archive) => Some(archive),
        Err(error) => {
            error!(event = "writer_source_zip_open_failed", xlsx = %input_display, error = %error, "Failed to open source XLSX as ZIP");
            None
        }
    }
}

fn prepare_partial_output(
    out_path: &Path,
    input_display: &str,
) -> Option<(PathBuf, BufWriter<File>)> {
    if !create_output_parent(out_path, input_display) {
        return None;
    }
    let partial_out_path = partial_output_path(out_path);
    if !remove_stale_partial_output(&partial_out_path, input_display) {
        return None;
    }

    let output_file = match File::create(&partial_out_path) {
        Ok(file) => file,
        Err(error) => {
            error!(event = "writer_output_create_failed", xlsx = %input_display, output = %partial_out_path.display(), error = %error, "Failed to create partial output XLSX");
            return None;
        }
    };
    Some((
        partial_out_path,
        BufWriter::with_capacity(16 * 1024 * 1024, output_file),
    ))
}

fn create_output_parent(out_path: &Path, input_display: &str) -> bool {
    let Some(parent) = out_path.parent() else {
        return true;
    };
    if let Err(error) = std::fs::create_dir_all(parent) {
        error!(event = "writer_output_parent_create_failed", xlsx = %input_display, output = %out_path.display(), parent = %parent.display(), error = %error, "Failed to create output parent directory");
        return false;
    }
    true
}

fn remove_stale_partial_output(partial_out_path: &Path, input_display: &str) -> bool {
    if !partial_out_path.exists() {
        return true;
    }
    if let Err(error) = std::fs::remove_file(partial_out_path) {
        error!(event = "writer_partial_output_remove_failed", xlsx = %input_display, output = %partial_out_path.display(), error = %error, "Failed to remove stale partial output XLSX");
        return false;
    }
    true
}

#[derive(Default)]
struct WriterTimings {
    jobs: usize,
    wait: Duration,
    copy: Duration,
}

fn copy_write_jobs(
    rx: mpsc::Receiver<QueuedWriteJob>,
    source_archive: &mut ZipArchive<File>,
    zip_writer: &mut ZipWriter<BufWriter<File>>,
    input_display: &str,
) -> WriterTimings {
    let mut timings = WriterTimings::default();
    loop {
        let started = Instant::now();
        let waiting = profiling::phase("writer_wait_for_job", input_display, "");
        let received = rx.recv();
        profiling::drop_value("writer.waiting_span", waiting);
        timings.wait += started.elapsed();
        let Ok(queued) = received else {
            break;
        };
        profiling::drop_value("writer.pending_jobs", queued.pending_jobs);
        profiling::drop_value("writer.pending_bytes", queued.pending_bytes);
        queued.handoff.finish();
        tracing::info!(target: "excel_shrink::profile", event = "writer_queue_delay",
            xlsx = input_display, queue_wait_us = queued.queued_at.elapsed().as_micros() as u64);
        timings.jobs += 1;
        let started = Instant::now();
        copy_write_job(queued.job, source_archive, zip_writer, input_display);
        timings.copy += started.elapsed();
    }
    timings
}

fn copy_write_job(
    job: WriteJob,
    source_archive: &mut ZipArchive<File>,
    zip_writer: &mut ZipWriter<BufWriter<File>>,
    input_display: &str,
) {
    let filename = match &job {
        WriteJob::CopyOriginal { filename, .. } | WriteJob::CopyPrecompressed { filename, .. } => {
            filename
        }
    };
    let _profile = profiling::phase("writer_copy_entry", input_display, filename);
    match job {
        WriteJob::CopyOriginal { index, filename } => {
            copy_original_entry(index, &filename, source_archive, zip_writer, input_display)
        }
        WriteJob::CopyPrecompressed {
            filename,
            zip_bytes,
        } => {
            if let Err(error) = raw_copy_single_entry_zip(zip_writer, zip_bytes) {
                error!(event = "zip_entry_precompressed_copy_failed", xlsx = %input_display, entry = %filename, error = %error, "Copy precompressed failed");
            }
        }
    }
}

fn copy_original_entry(
    index: usize,
    filename: &str,
    source_archive: &mut ZipArchive<File>,
    zip_writer: &mut ZipWriter<BufWriter<File>>,
    input_display: &str,
) {
    let file = match source_archive.by_index(index) {
        Ok(file) => file,
        Err(error) => {
            error!(event = "zip_entry_lookup_failed", xlsx = %input_display, entry = %filename, index, error = %error, "Failed to find original ZIP entry by index");
            return;
        }
    };
    if let Err(error) = zip_writer.raw_copy_file(file) {
        error!(event = "zip_entry_raw_copy_failed", xlsx = %input_display, entry = %filename, error = %error, "Failed to raw-copy original ZIP entry");
    }
}

fn finish_output_archive(
    mut zip_writer: ZipWriter<BufWriter<File>>,
    partial_out_path: &Path,
    out_path: &Path,
    input_display: &str,
) -> bool {
    let _profile = profiling::phase("finish_zip", input_display, "");
    let mut buffered_output = match zip_writer.finish() {
        Ok(writer) => writer,
        Err(error) => {
            error!(event = "file_finish_failed", xlsx = %input_display, output = %out_path.display(), error = %error, "Failed to finish output XLSX");
            return false;
        }
    };
    if let Err(error) = buffered_output.flush() {
        error!(event = "file_flush_failed", xlsx = %input_display, output = %partial_out_path.display(), error = %error, "Failed to flush partial output XLSX");
        return false;
    }
    profiling::drop_value("writer.buffered_output", buffered_output);
    replace_output_file(partial_out_path, out_path, input_display)
}

fn replace_output_file(partial_out_path: &Path, out_path: &Path, input_display: &str) -> bool {
    if out_path.exists() {
        if let Err(error) = std::fs::remove_file(out_path) {
            error!(event = "writer_existing_output_remove_failed", xlsx = %input_display, output = %out_path.display(), error = %error, "Failed to remove existing output XLSX before replacing it");
            return false;
        }
    }
    if let Err(error) = std::fs::rename(partial_out_path, out_path) {
        error!(event = "writer_output_rename_failed", xlsx = %input_display, partial_output = %partial_out_path.display(), output = %out_path.display(), error = %error, "Failed to rename partial output XLSX");
        return false;
    }
    true
}

fn partial_output_path(out_path: &Path) -> PathBuf {
    let mut partial = out_path.as_os_str().to_os_string();
    partial.push(".partial");
    PathBuf::from(partial)
}

fn send_write_job(tx: &mpsc::Sender<QueuedWriteJob>, job: WriteJob, xlsx: &str) -> bool {
    let (filename, bytes) = match &job {
        WriteJob::CopyOriginal { filename, .. } => (filename.as_str(), 0),
        WriteJob::CopyPrecompressed {
            filename,
            zip_bytes,
        } => (filename.as_str(), zip_bytes.len()),
    };
    let handoff = Handoff::start("writer_job", xlsx, filename);
    tracing::info!(target: "excel_shrink::profile", event = "writer_job_queued", xlsx, sheet = filename, compressed_buffer_bytes = bytes as u64);
    let pending_jobs = ResourceGuard::add("writer_pending_jobs", 1);
    let pending_bytes = ResourceGuard::add("writer_pending_compressed_bytes", bytes as u64);
    let queued = QueuedWriteJob {
        job,
        handoff,
        queued_at: Instant::now(),
        pending_jobs,
        pending_bytes,
    };
    if let Err(e) = tx.send(queued) {
        error!(event = "writer_channel_closed", xlsx = %xlsx, error = %e, "Cannot queue ZIP write job");
        return false;
    }
    true
}

fn apply_deferred_candidate_cleanup(
    xml: Vec<u8>,
    candidates: &[CleanupCandidate],
    protected_refs: &ProtectedWorkbookRefs,
    sheet_file: &str,
    stats: &mut CleanupStats,
) -> (Vec<u8>, HashSet<u32>) {
    let safe_refs = collect_safe_candidate_causes(candidates, protected_refs, sheet_file);
    if safe_refs.is_empty() {
        return (xml, HashSet::new());
    }

    let mut out = Vec::with_capacity(xml.len());
    let mut affected_rows = HashSet::new();
    let mut pos = 0;

    while let Some(cell_start_rel) = find_bytes(&xml[pos..], b"<c") {
        let cell_start = pos + cell_start_rel;

        if !is_cell_tag_start(&xml, cell_start) {
            out.extend_from_slice(&xml[pos..cell_start + b"<c".len()]);
            pos = cell_start + b"<c".len();
            continue;
        }

        let Some(cell) = parse_cell_slice(&xml, cell_start) else {
            break;
        };

        out.extend_from_slice(&xml[pos..cell.start]);
        let raw = &xml[cell.start..cell.end];
        let cell_end = cell.end;
        let attributes = &xml[cell.attributes];
        let replacement =
            candidate_cell_replacement(raw, attributes, &safe_refs, &mut affected_rows, stats);

        if let Some(replacement) = replacement {
            out.extend_from_slice(&replacement);
        } else {
            out.extend_from_slice(raw);
        }

        pos = cell_end;
    }

    out.extend_from_slice(&xml[pos..]);

    (out, affected_rows)
}

fn invalidate_tail_anchor_for_touched_rows(
    metrics: &mut CleanSheetMetrics,
    touched_rows: &HashSet<u32>,
) {
    for row in touched_rows {
        metrics.remove_all_tail_anchors_for_row(*row);
    }
}

fn transform_sheet_data_range<F>(
    xml: &[u8],
    sheet_data_range: Range<usize>,
    transform: F,
) -> excel_shrink_core::error::Result<(Vec<u8>, Range<usize>)>
where
    F: FnOnce(&[u8]) -> excel_shrink_core::error::Result<Vec<u8>>,
{
    if sheet_data_range.start > sheet_data_range.end || sheet_data_range.end > xml.len() {
        return Ok((xml.to_vec(), sheet_data_range.start..sheet_data_range.start));
    }

    let _profile = profiling::phase("rebuild_sheet_data_pass", "", "");
    tracing::info!(target: "excel_shrink::profile", event = "xml_rebuild", scanned_sheet_data_bytes = sheet_data_range.len() as u64, input_xml_bytes = xml.len() as u64);
    let cleaned_sheet_data = transform(&xml[sheet_data_range.clone()])?;
    let mut out = Vec::with_capacity(xml.len() - sheet_data_range.len() + cleaned_sheet_data.len());
    out.extend_from_slice(&xml[..sheet_data_range.start]);
    out.extend_from_slice(&cleaned_sheet_data);
    out.extend_from_slice(&xml[sheet_data_range.end..]);
    let new_end = sheet_data_range.start + cleaned_sheet_data.len();
    let new_range = sheet_data_range.start..new_end;

    Ok((out, new_range))
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
        let pos_rel = find_bytes(&data[search_start..], &needle)?;
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

fn generate_self_closing_cell_xml(cell_attr: &[u8]) -> Vec<u8> {
    let cell_attr = cell_attr
        .strip_suffix(b"/")
        .unwrap_or(cell_attr)
        .trim_ascii_end();
    let mut out = Vec::with_capacity(cell_attr.len() + 4);
    out.extend_from_slice(b"<c");
    out.extend_from_slice(cell_attr);
    out.extend_from_slice(b"/>");
    out
}

struct CellSlice {
    start: usize,
    end: usize,
    attributes: Range<usize>,
}

fn collect_safe_candidate_causes(
    candidates: &[CleanupCandidate],
    protected_refs: &ProtectedWorkbookRefs,
    sheet_file: &str,
) -> HashMap<(u32, u32), CleanupCause> {
    candidates
        .iter()
        .filter(|candidate| !protected_refs.contains(sheet_file, candidate.col, candidate.row))
        .map(|candidate| ((candidate.col, candidate.row), candidate.cause))
        .collect()
}

fn parse_cell_slice(xml: &[u8], cell_start: usize) -> Option<CellSlice> {
    let tag_end = find_byte_from(xml, b'>', cell_start)?;
    let self_closing = tag_end > cell_start && xml[tag_end - 1] == b'/';
    let attributes = cell_start + b"<c".len()..if self_closing { tag_end - 1 } else { tag_end };
    let end = find_cell_end(xml, tag_end, self_closing)?;
    Some(CellSlice {
        start: cell_start,
        end,
        attributes,
    })
}

#[inline]
fn find_cell_end(xml: &[u8], tag_end: usize, self_closing: bool) -> Option<usize> {
    if self_closing {
        return Some(tag_end + 1);
    }
    let content_start = tag_end + 1;
    let close = find_bytes(&xml[content_start..], b"</c>")?;
    Some(content_start + close + b"</c>".len())
}

fn candidate_cell_replacement(
    raw: &[u8],
    attributes: &[u8],
    safe_refs: &HashMap<(u32, u32), CleanupCause>,
    affected_rows: &mut HashSet<u32>,
    stats: &mut CleanupStats,
) -> Option<Vec<u8>> {
    let reference = extract_xml_attr_value(attributes, b"r").and_then(parse_cell_reference)?;
    let cause = *safe_refs.get(&reference)?;
    affected_rows.insert(reference.1);
    Some(replacement_for_cleanup_cause(raw, attributes, cause, stats))
}

fn replacement_for_cleanup_cause(
    raw: &[u8],
    attributes: &[u8],
    cause: CleanupCause,
    stats: &mut CleanupStats,
) -> Vec<u8> {
    if cause == CleanupCause::UnreferencedWhitespaceCells {
        let cleaned = generate_self_closing_cell_xml(attributes);
        stats.record_delta(cause, raw.len(), cleaned.len());
        return cleaned;
    }

    stats.record_removed(cause, raw.len());
    Vec::new()
}

fn parse_cell_reference(reference: &[u8]) -> Option<(u32, u32)> {
    let split = reference
        .iter()
        .position(|byte| byte.is_ascii_digit())
        .filter(|split| *split > 0 && *split < reference.len())?;

    let column = &reference[..split];
    let row = &reference[split..];

    if !column.iter().all(|byte| byte.is_ascii_uppercase())
        || !row.iter().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    Some((
        excel_shrink_core::utils::letters_byte_string_to_integer(column),
        excel_shrink_core::utils::digits_byte_string_to_integer(row),
    ))
}

fn advanced_cleanup_selection(cli: &Cli) -> AdvancedCleanupSelection {
    AdvancedCleanupSelection {
        remove_whitespace: cli.remove_unreferenced_whitespace_cells,
        remove_hidden_content: !cli.keep_unreferenced_hidden_content_cells,
    }
}

fn inactive_protection_policy(cli: &Cli) -> InactiveProtectionCellPolicy {
    match cli.inactive_protection_cell_policy {
        crate::InactiveProtectionCellPolicyArg::Remove => InactiveProtectionCellPolicy::Remove,
        crate::InactiveProtectionCellPolicyArg::KeepInDataRange => {
            InactiveProtectionCellPolicy::KeepInDataRange
        }
        crate::InactiveProtectionCellPolicyArg::KeepAll => InactiveProtectionCellPolicy::KeepAll,
    }
}

fn read_input_file(input: &Path) -> Option<Arc<Vec<u8>>> {
    let _profile = tracing::info_span!(target: "excel_shrink::profile", "read_input_archive", xlsx = %input.display()).entered();
    let mut file = match File::open(input) {
        Ok(file) => file,
        Err(error) => {
            error!("Failed to open file {:?}: {}", input, error);
            return None;
        }
    };
    let mut bytes = Vec::new();
    if let Err(error) = file.read_to_end(&mut bytes) {
        error!("Failed to read file {:?}: {}", input, error);
        return None;
    }
    Some(Arc::new(bytes))
}

fn prepare_workbook<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    cli: &Cli,
    advanced: AdvancedCleanupSelection,
) -> WorkbookPreparation {
    WorkbookPreparation {
        patterns: load_workbook_patterns(archive, advanced.remove_whitespace),
        cleanup: cleanup_switches(cli, advanced),
        collect_cleanup_stats: cleanup_stats_enabled(cli),
        selected_sheets: selected_sheet_targets(&cli.sheets),
    }
}

fn load_workbook_patterns<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    include_whitespace_indices: bool,
) -> WorkbookSpecificPatterns {
    let _profile = profiling::phase("load_styles_and_shared_strings", "", "");
    let all_xfs = read_zip_entry(archive, "xl/styles.xml")
        .and_then(|styles| parse_xfs(&styles).ok())
        .unwrap_or_default();
    let whitespace_indices = include_whitespace_indices
        .then(|| {
            read_zip_entry(archive, "xl/sharedStrings.xml")
                .map(|strings| find_whitespace_sst_indices(&strings))
        })
        .flatten();
    WorkbookSpecificPatterns::new(all_xfs, whitespace_indices)
}

fn read_zip_entry<R: Read + Seek>(archive: &mut ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    let mut file = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn cleanup_switches(cli: &Cli, advanced: AdvancedCleanupSelection) -> CleanupSwitches {
    let analysis_safe = attribution_safe_analysis(cli);
    CleanupSwitches {
        preprocess_empty_self_closing_cells: cli.preprocess_empty_cells,
        unreferenced_whitespace_cells: advanced.remove_whitespace,
        unreferenced_hidden_content_cells: advanced.remove_hidden_content,
        sheetdata_range_tail_pruning: !analysis_safe && !cli.disable_tail_pruning,
        trailing_layout_only_rows: !cli.disable_trailing_layout_only_rows,
        extraneous_column_range_merge: !cli.disable_extraneous_column_merge,
        column_tail_fill_merge: !cli.disable_column_tail_fill_merge,
        keep_textless_default_font_representatives: !cli
            .remove_textless_default_font_representatives,
        keep_textless_style_representatives: !cli.remove_textless_style_representatives,
        keep_textless_column_style_representatives: !cli
            .remove_textless_column_style_representatives,
        inactive_protection_cell_policy: inactive_protection_policy(cli),
        ..CleanupSwitches::default()
    }
}

impl InitialSheetProcessor {
    fn process(
        &self,
        task: SheetTask,
        advanced_cleanup: Option<Arc<AdvancedCleanupConfig>>,
    ) -> ProcessedSheetResult {
        let SheetTask {
            mut params,
            zip_index,
            original_xml_size: planned_original_size,
            handoff,
        } = task;
        handoff.finish();
        let _profile = profiling::phase(
            "sheet_initial_cleanup",
            &params.xlsx_file_path,
            &params.zip_info_filename,
        );
        params.advanced_cleanup = advanced_cleanup;

        let mut archive = match ZipArchive::new(Cursor::new(self.source_zip_bytes.as_slice())) {
            Ok(archive) => archive,
            Err(error) => {
                error!(event = "sheet_source_zip_open_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %error, "Failed to reopen source XLSX for sheet processing");
                return failed_sheet(params, zip_index);
            }
        };
        let mut sheet_file = match archive.by_name(&params.zip_info_filename) {
            Ok(file) => file,
            Err(error) => {
                error!(event = "sheet_lookup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %error, "Failed to find sheet XML in source XLSX");
                return failed_sheet(params, zip_index);
            }
        };
        let original_size = usize::try_from(sheet_file.size()).unwrap_or(planned_original_size);
        let (active_guard, started) =
            ActiveSheetGuard::start(&params, original_size, self.active_tasks.clone());
        let mut report_progress = active_guard.progress_reporter(&params);
        let result = clean_sheet_data_streamwise_with_progress(
            &mut sheet_file,
            &params,
            self.chunk_size,
            self.disable_splitting,
            self.parallel_range_chunk_size,
            &mut report_progress,
        );

        profiling::drop_value("sheet.progress_reporter", report_progress);
        match result {
            Ok(cleaned) => {
                complete_initial_sheet(params, zip_index, cleaned, started, original_size)
            }
            Err(error) => {
                error!(event = "sheet_clean_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %error, "Failed to clean sheet XML; copying original");
                failed_sheet(params, zip_index)
            }
        }
    }
}

impl ActiveSheetGuard {
    fn start(
        params: &CleanSheetParameters,
        original_size: usize,
        active_tasks: ActiveTasks,
    ) -> (Self, Instant) {
        let started = Instant::now();
        let task_key = format!("{}::{}", params.xlsx_file_path, params.zip_info_filename);
        debug!(
            xlsx = %params.xlsx_file_path,
            sheet = %params.zip_info_filename,
            original_xml_bytes = original_size,
            "Processing sheet"
        );
        active_tasks.lock().unwrap().insert(
            task_key.clone(),
            ActiveTask {
                xlsx: params.xlsx_file_path.clone(),
                sheet: params.zip_info_filename.clone(),
                xml_size: original_size,
                started,
                phase: "clean_sheet_data_streamwise".to_string(),
                phase_started: started,
            },
        );
        (
            Self {
                active_tasks,
                task_key,
            },
            started,
        )
    }

    fn progress_reporter(&self, params: &CleanSheetParameters) -> impl FnMut(&'static str) {
        let active_tasks = self.active_tasks.clone();
        let task_key = self.task_key.clone();
        let xlsx = params.xlsx_file_path.clone();
        let sheet = params.zip_info_filename.clone();
        let mut phase_span = None;
        let mut previous_phase = None;
        move |phase| {
            if previous_phase != Some(phase) {
                profiling::drop_optional("sheet.previous_phase_span", phase_span.take());
                phase_span = Some(profiling::phase(phase, &xlsx, &sheet));
                previous_phase = Some(phase);
            }
            let mut guard = active_tasks.lock().unwrap();
            if let Some(task) = guard.get_mut(&task_key) {
                if task.phase != phase {
                    task.phase = phase.to_string();
                    task.phase_started = Instant::now();
                }
            }
        }
    }
}

fn complete_initial_sheet(
    params: CleanSheetParameters,
    zip_index: usize,
    cleaned: excel_shrink_core::models::CleanedSheetData,
    started: Instant,
    original_size: usize,
) -> ProcessedSheetResult {
    let cleaned_size = cleaned.xml.len();
    debug!(
        xlsx = %params.xlsx_file_path,
        sheet = %params.zip_info_filename,
        original_xml_bytes = original_size,
        cleaned_xml_bytes = cleaned_size,
        reduction_bytes = original_size.saturating_sub(cleaned_size),
        elapsed_ms = started.elapsed().as_millis(),
        "Finished sheet cleanup"
    );
    let handoff = Handoff::start(
        "sheet_awaiting_finalization",
        &params.xlsx_file_path,
        &params.zip_info_filename,
    );
    let buffered_bytes =
        ResourceGuard::add("initial_results_buffered_bytes", cleaned.xml.len() as u64);
    Ok(ProcessedSheet {
        buffered_bytes,
        initial_finished: Instant::now(),
        handoff,
        params,
        zip_index,
        xml: cleaned.xml,
        sheet_data_range: cleaned.sheet_data_range,
        metrics: cleaned.metrics,
        started,
        original_size,
    })
}

fn failed_sheet(params: CleanSheetParameters, zip_index: usize) -> ProcessedSheetResult {
    Err(FailedSheet { params, zip_index })
}

fn plan_archive<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    context: &ArchivePlanningContext<'_>,
) -> Option<ArchivePlan> {
    let _profile = profiling::phase("plan_archive", context.input_display, "");
    let mut plan = ArchivePlan::new();

    for index in 0..archive.len() {
        let mut file = match archive.by_index(index) {
            Ok(file) => file,
            Err(error) => {
                error!(event = "zip_entry_read_failed", xlsx = %context.input_display, index, error = %error, "Failed to read ZIP entry");
                continue;
            }
        };
        let filename = file.name().to_string();
        let compression = file.compression();
        let kind = classify_archive_entry(&filename, context);
        let entry_plan = match kind {
            ArchiveEntryKind::Sheet => {
                plan.sheet_tasks.push(build_sheet_task(
                    index,
                    &filename,
                    compression,
                    file.size(),
                    context,
                ));
                ArchiveEntryPlan::Continue
            }
            ArchiveEntryKind::Workbook => {
                plan_workbook_entry(&mut file, index, filename, compression, context, &mut plan)
            }
            ArchiveEntryKind::WorkbookRelationships => {
                plan_relationships_entry(&mut file, index, filename, context, &mut plan)
            }
            ArchiveEntryKind::CalculationChain => plan_calculation_chain_entry(
                &mut file,
                index,
                filename,
                compression,
                context,
                &mut plan,
            ),
            ArchiveEntryKind::CopyOriginal => {
                queue_original_entry(context.tx, index, filename, context.input_display)
            }
        };

        if matches!(entry_plan, ArchiveEntryPlan::Abort) {
            return None;
        }
    }

    Some(plan)
}

fn classify_archive_entry(
    filename: &str,
    context: &ArchivePlanningContext<'_>,
) -> ArchiveEntryKind {
    let selected_sheet = filename.starts_with("xl/worksheets/sheet")
        && filename.ends_with(".xml")
        && !context.cli.only_clean_workbook
        && sheet_selected_for_cleanup(filename, context.selected_sheets);
    if selected_sheet {
        return ArchiveEntryKind::Sheet;
    }

    match filename {
        "xl/workbook.xml" => ArchiveEntryKind::Workbook,
        "xl/_rels/workbook.xml.rels" => ArchiveEntryKind::WorkbookRelationships,
        "xl/calcChain.xml" => ArchiveEntryKind::CalculationChain,
        _ => ArchiveEntryKind::CopyOriginal,
    }
}

fn build_sheet_task(
    index: usize,
    filename: &str,
    compression: CompressionMethod,
    original_size: u64,
    context: &ArchivePlanningContext<'_>,
) -> SheetTask {
    SheetTask {
        params: CleanSheetParameters {
            xlsx_file_path: context.input.to_string_lossy().to_string(),
            zip_info_filename: filename.to_string(),
            compress_method: compression,
            patterns: context.patterns.clone(),
            advanced_cleanup: None,
            preprocess: SheetPreprocessConfig {
                remove_empty_cells: context.cli.preprocess_empty_cells,
            },
            cleanup: context.cleanup.clone(),
            collect_cleanup_stats: context.collect_cleanup_stats,
        },
        zip_index: index,
        original_xml_size: usize::try_from(original_size).unwrap_or(usize::MAX),
        handoff: Handoff::start("sheet_scheduled", context.input_display, filename),
    }
}

fn plan_workbook_entry(
    file: &mut impl Read,
    index: usize,
    filename: String,
    compression: CompressionMethod,
    context: &ArchivePlanningContext<'_>,
    plan: &mut ArchivePlan,
) -> ArchiveEntryPlan {
    let workbook_xml = match read_archive_entry(file) {
        Ok(bytes) => bytes,
        Err(error) => {
            error!(event = "workbook_read_failed", xlsx = %context.input_display, entry = %filename, error = %error, "Failed to read workbook.xml; copying original");
            return queue_original_entry(context.tx, index, filename, context.input_display);
        }
    };
    let original_size = workbook_xml.len();
    let mut stats = cleanup_stats(context.collect_cleanup_stats);
    let cleaned = clean_workbook_or_original(workbook_xml, &filename, context, &mut stats);
    let cleaned_size = cleaned.len();

    if context.advanced_cleanup_enabled {
        plan.workbook_xml_for_scan = Some(cleaned.clone());
    }
    plan.workbook_xml_for_calc_chain = Some(cleaned.clone());

    let output_method = output_compression_method(compression, context.cli.store_output);
    match precompress_entry(
        &filename,
        cleaned,
        output_method,
        context.cli.compression_level,
    ) {
        Ok(zip_bytes) => {
            send_cleanup_stats_rows(
                context.analysis_tx,
                context.input_display,
                &filename,
                &stats,
                original_size,
                cleaned_size,
                context.attribution_safe_analysis,
            );
            queue_precompressed_entry(context.tx, filename, zip_bytes, context.input_display)
        }
        Err(error) => {
            error!(event = "workbook_precompress_failed", xlsx = %context.input_display, entry = %filename, error = %error, "Failed to precompress cleaned workbook.xml; copying original");
            queue_original_entry(context.tx, index, filename, context.input_display)
        }
    }
}

fn cleanup_stats(enabled: bool) -> CleanupStats {
    if enabled {
        CleanupStats::enabled()
    } else {
        CleanupStats::default()
    }
}

fn clean_workbook_or_original(
    workbook_xml: Vec<u8>,
    filename: &str,
    context: &ArchivePlanningContext<'_>,
    stats: &mut CleanupStats,
) -> Vec<u8> {
    match clean_workbook_with_stats(&workbook_xml, stats) {
        Ok(cleaned) => cleaned,
        Err(error) => {
            error!(event = "workbook_clean_failed", xlsx = %context.input_display, entry = %filename, error = %error, "Failed to clean workbook.xml; using original workbook.xml bytes");
            workbook_xml
        }
    }
}

fn plan_relationships_entry(
    file: &mut impl Read,
    index: usize,
    filename: String,
    context: &ArchivePlanningContext<'_>,
    plan: &mut ArchivePlan,
) -> ArchiveEntryPlan {
    match read_archive_entry(file) {
        Ok(relationships) => {
            plan.workbook_rels_for_calc_chain = Some(relationships.clone());
            if context.advanced_cleanup_enabled {
                plan.workbook_rels_for_scan = Some(relationships);
            }
        }
        Err(error) => {
            error!(event = "workbook_rels_read_failed", xlsx = %context.input_display, entry = %filename, error = %error, "Failed to read workbook relationships; sheet-scoped deferred cleanup may be skipped");
        }
    }
    queue_original_entry(context.tx, index, filename, context.input_display)
}

fn plan_calculation_chain_entry(
    file: &mut impl Read,
    index: usize,
    filename: String,
    compression: CompressionMethod,
    context: &ArchivePlanningContext<'_>,
    plan: &mut ArchivePlan,
) -> ArchiveEntryPlan {
    match read_archive_entry(file) {
        Ok(calc_chain) => {
            plan.calc_chain_part = Some((index, compression, calc_chain));
            ArchiveEntryPlan::Continue
        }
        Err(error) => {
            error!(event = "calc_chain_read_failed", xlsx = %context.input_display, entry = %filename, error = %error, "Failed to read calcChain.xml; copying original");
            queue_original_entry(context.tx, index, filename, context.input_display)
        }
    }
}

fn read_archive_entry(file: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn queue_original_entry(
    tx: &mpsc::Sender<QueuedWriteJob>,
    index: usize,
    filename: String,
    input_display: &str,
) -> ArchiveEntryPlan {
    if send_write_job(
        tx,
        WriteJob::CopyOriginal { index, filename },
        input_display,
    ) {
        ArchiveEntryPlan::Continue
    } else {
        ArchiveEntryPlan::Abort
    }
}

fn queue_precompressed_entry(
    tx: &mpsc::Sender<QueuedWriteJob>,
    filename: String,
    zip_bytes: Vec<u8>,
    input_display: &str,
) -> ArchiveEntryPlan {
    if send_write_job(
        tx,
        WriteJob::CopyPrecompressed {
            filename,
            zip_bytes,
        },
        input_display,
    ) {
        ArchiveEntryPlan::Continue
    } else {
        ArchiveEntryPlan::Abort
    }
}

#[derive(Default)]
struct SheetReferenceMap {
    name_to_file: HashMap<String, String>,
    file_to_name: HashMap<String, String>,
    files_in_workbook_order: Vec<String>,
}

impl SheetReferenceMap {
    fn from_workbook(workbook_xml: Option<&[u8]>, workbook_rels: Option<&[u8]>) -> Self {
        let (name_to_file, files_in_workbook_order) = workbook_xml
            .zip(workbook_rels)
            .map(|(workbook, relationships)| build_sheet_name_to_file_map(workbook, relationships))
            .unwrap_or_default();
        let file_to_name = name_to_file
            .iter()
            .map(|(sheet_name, sheet_file)| (sheet_file.clone(), sheet_name.clone()))
            .collect();

        Self {
            name_to_file,
            file_to_name,
            files_in_workbook_order,
        }
    }
}

impl ReferenceValidation {
    fn disabled() -> Self {
        Self {
            protected_refs: ProtectedWorkbookRefs::default(),
            safe_to_clean: false,
        }
    }
}

fn validate_deferred_references(
    processed_sheets: &[ProcessedSheetResult],
    workbook_xml: Option<&[u8]>,
    workbook_rels: Option<&[u8]>,
    input_display: &str,
) -> ReferenceValidation {
    let _profile = profiling::phase("validate_references", input_display, "");
    let Some(scan_selection) = deferred_reference_scan_selection(processed_sheets) else {
        return ReferenceValidation::disabled();
    };
    let reference_map = SheetReferenceMap::from_workbook(workbook_xml, workbook_rels);
    let scan_buffers = build_reference_scan_buffers(
        processed_sheets,
        &scan_selection,
        &reference_map,
        workbook_xml,
    );

    scan_deferred_references(&scan_buffers, &reference_map, input_display)
}

fn deferred_reference_scan_selection(
    processed_sheets: &[ProcessedSheetResult],
) -> Option<Vec<bool>> {
    let mut has_candidates = false;
    let mut include_sheet = Vec::with_capacity(processed_sheets.len());

    for result in processed_sheets {
        let Some(metrics) = result.as_ref().ok().map(|sheet| &sheet.metrics) else {
            include_sheet.push(false);
            continue;
        };
        has_candidates |= !metrics.candidates.is_empty();
        include_sheet.push(metrics.has_hints || !metrics.candidates.is_empty());
    }

    has_candidates.then_some(include_sheet)
}

fn build_reference_scan_buffers<'a>(
    processed_sheets: &'a [ProcessedSheetResult],
    scan_selection: &[bool],
    reference_map: &'a SheetReferenceMap,
    workbook_xml: Option<&'a [u8]>,
) -> Vec<SheetScanBuffer<'a>> {
    let mut buffers = Vec::new();

    for (result, include) in processed_sheets.iter().zip(scan_selection) {
        if !*include {
            continue;
        }
        let Ok(sheet) = result else {
            continue;
        };
        let sheet_name = reference_map
            .file_to_name
            .get(&sheet.params.zip_info_filename)
            .map(String::as_str)
            .unwrap_or("Unknown");
        buffers.push(SheetScanBuffer {
            sheet_file: &sheet.params.zip_info_filename,
            sheet_name,
            xml: &sheet.xml,
        });
    }

    if let Some(xml) = workbook_xml {
        buffers.push(SheetScanBuffer {
            sheet_file: "xl/workbook.xml",
            sheet_name: "Workbook",
            xml,
        });
    }

    buffers
}

fn scan_deferred_references(
    scan_buffers: &[SheetScanBuffer<'_>],
    reference_map: &SheetReferenceMap,
    input_display: &str,
) -> ReferenceValidation {
    let mut protected_refs = ProtectedWorkbookRefs::default();
    let safe_to_clean = match scan_for_references_by_sheet(
        scan_buffers,
        &mut protected_refs,
        Some(&reference_map.name_to_file),
        Some(&reference_map.files_in_workbook_order),
    ) {
        ScanResult::Safe => true,
        ScanResult::Unsafe(reason) => {
            warn!(
                "Unsafe dynamic reference found: {}. Skipping deferred cleanup for {}.",
                reason, input_display
            );
            false
        }
    };

    ReferenceValidation {
        protected_refs,
        safe_to_clean,
    }
}

impl SheetFinalizer<'_> {
    fn finalize(&self, result: ProcessedSheetResult) {
        let Some(mut sheet) = self.cleaned_sheet_or_queue_original(result) else {
            return;
        };
        profiling::drop_value(
            "sheet.buffered_bytes",
            std::mem::take(&mut sheet.buffered_bytes),
        );
        std::mem::take(&mut sheet.handoff).finish();
        let _profile = profiling::phase(
            "sheet_finalize",
            &sheet.params.xlsx_file_path,
            &sheet.params.zip_info_filename,
        );
        let initial_ms = sheet
            .initial_finished
            .duration_since(sheet.started)
            .as_millis();
        let wait_ms = sheet.initial_finished.elapsed().as_millis();
        let xlsx = sheet.params.xlsx_file_path.clone();
        let sheet_file = sheet.params.zip_info_filename.clone();
        let finalized_at = Instant::now();
        self.apply_deferred_cleanup(&mut sheet);
        self.queue_cleaned_sheet(sheet);
        let finalize_ms = finalized_at.elapsed().as_millis();
        debug!(
            event = "sheet_timing",
            xlsx,
            sheet = sheet_file,
            initial_cleanup_ms = initial_ms,
            wait_before_finalize_ms = wait_ms,
            finalize_ms,
            "Sheet phase wall times (not CPU times)"
        );
        tracing::info!(target: "excel_shrink::profile", event = "sheet_timing", xlsx, sheet = sheet_file,
            initial_cleanup_ms = initial_ms as u64, wait_before_finalize_ms = wait_ms as u64,
            finalize_ms = finalize_ms as u64);
        if initial_ms > 5000 || finalize_ms > 5000 {
            warn!(
                event = "slow_sheet_phase",
                xlsx,
                sheet = sheet_file,
                initial_cleanup_ms = initial_ms,
                wait_before_finalize_ms = wait_ms,
                finalize_ms,
                "Slow sheet phase; time before finalization is reported separately"
            );
        }
    }

    fn cleaned_sheet_or_queue_original(
        &self,
        result: ProcessedSheetResult,
    ) -> Option<ProcessedSheet> {
        match result {
            Ok(sheet) => Some(sheet),
            Err(FailedSheet { params, zip_index }) => {
                send_write_job(
                    self.tx,
                    WriteJob::CopyOriginal {
                        index: zip_index,
                        filename: params.zip_info_filename,
                    },
                    &params.xlsx_file_path,
                );
                None
            }
        }
    }

    fn apply_deferred_cleanup(&self, sheet: &mut ProcessedSheet) {
        let _profile = profiling::phase(
            "deferred_cleanup",
            &sheet.params.xlsx_file_path,
            &sheet.params.zip_info_filename,
        );
        if !self.safe_to_clean || sheet.metrics.candidates.is_empty() {
            return;
        }

        tracing::info!(target: "excel_shrink::profile", event = "cleanup_candidates", candidates = sheet.metrics.candidates.len() as u64);
        let touched_rows = self.remove_deferred_candidates(sheet);
        tracing::info!(target: "excel_shrink::profile", event = "changed_rows", rows = touched_rows.len() as u64);
        invalidate_tail_anchor_for_touched_rows(&mut sheet.metrics, &touched_rows);
        if !touched_rows.is_empty() {
            self.reclean_touched_rows(sheet, &touched_rows);
        }
        self.remove_empty_rows(sheet);
        if sheet.params.cleanup.trailing_layout_only_rows {
            self.trim_trailing_layout_rows(sheet);
        }
    }

    fn remove_deferred_candidates(&self, sheet: &mut ProcessedSheet) -> HashSet<u32> {
        let _profile = profiling::phase(
            "remove_candidates",
            &sheet.params.xlsx_file_path,
            &sheet.params.zip_info_filename,
        );
        let ProcessedSheet {
            params,
            xml,
            sheet_data_range,
            metrics,
            ..
        } = sheet;
        let mut touched_rows = HashSet::new();
        let result = transform_sheet_data_range(xml, sheet_data_range.clone(), |sheet_data| {
            let (cleaned, rows) = apply_deferred_candidate_cleanup(
                sheet_data.to_vec(),
                &metrics.candidates,
                self.protected_refs,
                &params.zip_info_filename,
                &mut metrics.cleanup_stats,
            );
            touched_rows = rows;
            Ok(cleaned)
        });

        match result {
            Ok((cleaned, new_range)) => {
                *xml = cleaned;
                *sheet_data_range = new_range;
            }
            Err(error) => {
                error!(event = "deferred_candidate_cleanup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %error, "Failed to clean deferred candidates; keeping initial XML");
            }
        }
        touched_rows
    }

    fn reclean_touched_rows(&self, sheet: &mut ProcessedSheet, touched_rows: &HashSet<u32>) {
        let _profile = profiling::phase(
            "reclean_touched_rows",
            &sheet.params.xlsx_file_path,
            &sheet.params.zip_info_filename,
        );
        let ProcessedSheet {
            params,
            xml,
            sheet_data_range,
            metrics,
            ..
        } = sheet;
        match transform_sheet_data_range(xml, sheet_data_range.clone(), |sheet_data| {
            clean_selected_rows_and_cells_in_cell_range(sheet_data, params, metrics, touched_rows)
        }) {
            Ok((cleaned, new_range)) => {
                *xml = cleaned;
                *sheet_data_range = new_range;
            }
            Err(error) => {
                error!(event = "deferred_row_cell_cleanup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, rows = touched_rows.len(), error = %error, "Failed to clean rows touched by deferred cleanup; keeping deferred-cleaned XML");
            }
        }
    }

    fn remove_empty_rows(&self, sheet: &mut ProcessedSheet) {
        let _profile = profiling::phase(
            "remove_empty_rows_after_candidates",
            &sheet.params.xlsx_file_path,
            &sheet.params.zip_info_filename,
        );
        let ProcessedSheet {
            params,
            xml,
            sheet_data_range,
            metrics,
            ..
        } = sheet;
        match transform_sheet_data_range(xml, sheet_data_range.clone(), |sheet_data| {
            clean_empty_rows_from_after_cell_range(sheet_data, params, metrics)
        }) {
            Ok((cleaned, new_range)) => {
                *xml = cleaned;
                *sheet_data_range = new_range;
            }
            Err(error) => {
                error!(event = "deferred_empty_row_cleanup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %error, "Failed to clean rows emptied by deferred cleanup; keeping deferred-cleaned XML");
            }
        }
    }

    fn trim_trailing_layout_rows(&self, sheet: &mut ProcessedSheet) {
        let _profile = profiling::phase(
            "trim_tail_after_candidates",
            &sheet.params.xlsx_file_path,
            &sheet.params.zip_info_filename,
        );
        let ProcessedSheet {
            params,
            xml,
            sheet_data_range,
            metrics,
            ..
        } = sheet;
        match transform_sheet_data_range(xml, sheet_data_range.clone(), |sheet_data| {
            Ok(trim_trailing_layout_only_rows_vec(
                sheet_data.to_vec(),
                params,
                metrics,
            ))
        }) {
            Ok((cleaned, new_range)) => {
                *xml = cleaned;
                *sheet_data_range = new_range;
            }
            Err(error) => {
                error!(event = "deferred_tail_cleanup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %error, "Failed to trim rows after deferred cleanup; keeping deferred-cleaned XML");
            }
        }
    }

    fn queue_cleaned_sheet(&self, sheet: ProcessedSheet) {
        let ProcessedSheet {
            params,
            zip_index,
            xml,
            metrics,
            original_size,
            ..
        } = sheet;
        let cleaned_size = xml.len();
        let reduction = original_size.saturating_sub(cleaned_size);
        let cleaned_cell_refs = collect_cell_refs(&xml);
        let output_method = output_compression_method(params.compress_method, self.store_output);

        match precompress_entry(
            &params.zip_info_filename,
            xml,
            output_method,
            self.compression_level,
        ) {
            Ok(zip_bytes) => self.queue_precompressed_sheet(
                params,
                metrics,
                original_size,
                cleaned_size,
                reduction,
                cleaned_cell_refs,
                zip_bytes,
            ),
            Err(error) => self.queue_original_after_compression_error(params, zip_index, error),
        }
    }

    fn queue_precompressed_sheet(
        &self,
        params: CleanSheetParameters,
        metrics: CleanSheetMetrics,
        original_size: usize,
        cleaned_size: usize,
        reduction: usize,
        cleaned_cell_refs: HashSet<String>,
        zip_bytes: Vec<u8>,
    ) {
        let queue_phase = profiling::phase(
            "record_stats_and_queue_sheet",
            &params.xlsx_file_path,
            &params.zip_info_filename,
        );
        debug!(
            xlsx = %params.xlsx_file_path,
            sheet = %params.zip_info_filename,
            cleaned_xml_bytes = cleaned_size,
            reduction_bytes = reduction,
            "Queued cleaned sheet for writing"
        );
        send_cleanup_stats_rows(
            self.analysis_tx,
            &params.xlsx_file_path,
            &params.zip_info_filename,
            &metrics.cleanup_stats,
            original_size,
            cleaned_size,
            self.attribution_safe_analysis,
        );
        {
            let _profile = profiling::phase("publish_cell_references", "", "");
            self.known_cells_by_sheet_file
                .lock()
                .unwrap()
                .insert(params.zip_info_filename.clone(), cleaned_cell_refs);
        }
        send_write_job(
            self.tx,
            WriteJob::CopyPrecompressed {
                filename: params.zip_info_filename,
                zip_bytes,
            },
            &params.xlsx_file_path,
        );
        profiling::drop_value("sheet.queue_phase_span", queue_phase);
        // Destruction of per-cell maps can dominate finalization on large sheets.
        // Keep that work visible instead of hiding it after compression/queuing.
        let _release = profiling::phase("release_sheet_metrics", "", "");
        profiling::drop_sheet_metrics(metrics);
    }

    fn queue_original_after_compression_error(
        &self,
        params: CleanSheetParameters,
        zip_index: usize,
        error: anyhow::Error,
    ) {
        error!(event = "sheet_precompress_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %error, "Failed to precompress cleaned sheet; copying original");
        send_write_job(
            self.tx,
            WriteJob::CopyOriginal {
                index: zip_index,
                filename: params.zip_info_filename,
            },
            &params.xlsx_file_path,
        );
    }
}

fn finalize_sheets(
    processed_sheets: Vec<ProcessedSheetResult>,
    finalizer: &SheetFinalizer<'_>,
    disable_multiprocessing: bool,
) {
    let _profile = profiling::phase("finalize_sheets_fork_join", "", "");
    if disable_multiprocessing {
        processed_sheets
            .into_iter()
            .for_each(|result| finalizer.finalize(result));
    } else {
        processed_sheets
            .into_par_iter()
            .for_each(|result| finalizer.finalize(result));
    }
}

fn queue_calculation_chain(
    calc_chain_part: Option<(usize, CompressionMethod, Vec<u8>)>,
    workbook_xml: Option<&[u8]>,
    workbook_rels: Option<&[u8]>,
    known_cells_by_sheet_file: &Mutex<HashMap<String, HashSet<String>>>,
    tx: &mpsc::Sender<QueuedWriteJob>,
    input_display: &str,
    store_output: bool,
    compression_level: i32,
) -> bool {
    let _profile = profiling::phase("clean_calc_chain", input_display, "");
    let Some((index, method, calc_chain_xml)) = calc_chain_part else {
        return true;
    };
    let Some((workbook_xml, workbook_rels)) = workbook_xml.zip(workbook_rels) else {
        return queue_original_calc_chain(tx, index, input_display);
    };
    let sheet_id_to_file = build_sheet_id_to_file_map(workbook_xml, workbook_rels);
    let cleaned = {
        let known_cells = known_cells_by_sheet_file.lock().unwrap();
        clean_calc_chain(calc_chain_xml, &sheet_id_to_file, &known_cells)
    };

    queue_cleaned_calc_chain(
        tx,
        index,
        method,
        cleaned,
        input_display,
        store_output,
        compression_level,
    )
}

fn queue_cleaned_calc_chain(
    tx: &mpsc::Sender<QueuedWriteJob>,
    index: usize,
    method: CompressionMethod,
    xml: Vec<u8>,
    input_display: &str,
    store_output: bool,
    compression_level: i32,
) -> bool {
    let output_method = output_compression_method(method, store_output);
    match precompress_entry("xl/calcChain.xml", xml, output_method, compression_level) {
        Ok(zip_bytes) => send_write_job(
            tx,
            WriteJob::CopyPrecompressed {
                filename: "xl/calcChain.xml".to_string(),
                zip_bytes,
            },
            input_display,
        ),
        Err(error) => {
            error!(event = "calc_chain_precompress_failed", xlsx = %input_display, error = %error, "Failed to precompress cleaned calcChain.xml; copying original");
            queue_original_calc_chain(tx, index, input_display)
        }
    }
}

fn queue_original_calc_chain(
    tx: &mpsc::Sender<QueuedWriteJob>,
    index: usize,
    input_display: &str,
) -> bool {
    send_write_job(
        tx,
        WriteJob::CopyOriginal {
            index,
            filename: "xl/calcChain.xml".to_string(),
        },
        input_display,
    )
}

fn prioritize_sheet_tasks(
    mut sheet_tasks: Vec<SheetTask>,
    input_display: &str,
    parallel_range_chunk_size: usize,
) -> Vec<SheetTask> {
    sheet_tasks.sort_by(|left, right| {
        right
            .original_xml_size
            .cmp(&left.original_xml_size)
            .then_with(|| {
                left.params
                    .zip_info_filename
                    .cmp(&right.params.zip_info_filename)
            })
    });
    let (total_sheet_xml_bytes, largest_sheet_xml_bytes) = sheet_task_size_summary(&sheet_tasks);
    info!(
        xlsx = %input_display,
        sheets_to_clean = sheet_tasks.len(),
        total_sheet_xml_bytes,
        largest_sheet_xml_bytes,
        parallel_range_chunk_size,
        "Planned workbook cleanup"
    );
    sheet_tasks
}

fn sheet_task_size_summary(sheet_tasks: &[SheetTask]) -> (usize, usize) {
    let total = sheet_tasks.iter().map(|task| task.original_xml_size).sum();
    let largest = sheet_tasks
        .iter()
        .map(|task| task.original_xml_size)
        .max()
        .unwrap_or(0);
    (total, largest)
}

fn process_initial_sheets(
    sheet_tasks: Vec<SheetTask>,
    source_zip_bytes: Arc<Vec<u8>>,
    active_tasks: ActiveTasks,
    advanced_cleanup: AdvancedCleanupSelection,
    cli: &Cli,
) -> Vec<ProcessedSheetResult> {
    let _profile = profiling::phase("initial_sheets_fork_join", "", "");
    let processor = InitialSheetProcessor {
        source_zip_bytes,
        active_tasks,
        chunk_size: cli.chunk_size,
        disable_splitting: cli.disable_sheetdata_splitting,
        parallel_range_chunk_size: cli.parallel_range_chunk_size,
    };
    let advanced_config = advanced_cleanup.config();
    let process_sheet = |task| processor.process(task, advanced_config.clone());

    if cli.disable_multiprocessing {
        sheet_tasks.into_iter().map(process_sheet).collect()
    } else {
        sheet_tasks.into_par_iter().map(process_sheet).collect()
    }
}

fn process_file(
    input: &PathBuf,
    cli: &Cli,
    writer_pool: &ThreadPool,
    workbook_limiter: &WorkbookLimiter,
    batch_progress: &BatchProgress,
    active_tasks: &ActiveTasks,
    analysis_tx: Option<mpsc::Sender<AnalysisCsvRow>>,
) {
    let input_display = input.display().to_string();
    let _profile = profiling::phase("workbook_orchestration", &input_display, "");
    let workbook_permit = workbook_limiter.acquire(&input_display);
    let file_start = Instant::now();
    let active_tasks = active_tasks.clone();
    let mut file_progress = batch_progress.start_file(input_display.clone());

    let out_path = cli.output.join(input.file_name().unwrap());
    if out_path.exists() && !cli.force_overwrite {
        warn!("Output file {:?} already exists. Skipping.", out_path);
        file_progress.finish(
            "file_skipped",
            false,
            file_start.elapsed().as_millis(),
            None,
            "Skipped XLSX because output file already exists",
        );
        return;
    }

    let Some(file_bytes) = read_input_file(input) else {
        return;
    };
    let mut archive = match ZipArchive::new(Cursor::new(file_bytes.as_slice())) {
        Ok(archive) => archive,
        Err(error) => {
            error!("Failed to parse ZIP archive {:?}: {}", input, error);
            return;
        }
    };
    let advanced_cleanup = advanced_cleanup_selection(cli);
    let do_advanced = advanced_cleanup.enabled();

    let (tx, rx) = mpsc::channel::<QueuedWriteJob>();
    spawn_xlsx_writer(
        writer_pool,
        input.clone(),
        out_path,
        rx,
        input_display.clone(),
        file_progress,
        workbook_permit,
    );

    let WorkbookPreparation {
        patterns,
        cleanup,
        collect_cleanup_stats,
        selected_sheets,
    } = prepare_workbook(&mut archive, cli, advanced_cleanup);
    let attribution_safe_analysis = attribution_safe_analysis(cli);
    let planning_context = ArchivePlanningContext {
        input,
        input_display: &input_display,
        cli,
        patterns: &patterns,
        cleanup: &cleanup,
        collect_cleanup_stats,
        selected_sheets: &selected_sheets,
        tx: &tx,
        advanced_cleanup_enabled: do_advanced,
        analysis_tx: analysis_tx.as_ref(),
        attribution_safe_analysis,
    };
    let Some(archive_plan) = plan_archive(&mut archive, &planning_context) else {
        return;
    };
    profiling::drop_value("workbook.archive", archive);

    let ArchivePlan {
        sheet_tasks,
        workbook_xml_for_scan,
        workbook_rels_for_scan,
        workbook_xml_for_calc_chain,
        workbook_rels_for_calc_chain,
        calc_chain_part,
    } = archive_plan;

    let sheet_tasks =
        prioritize_sheet_tasks(sheet_tasks, &input_display, cli.parallel_range_chunk_size);

    let processed_sheets =
        process_initial_sheets(sheet_tasks, file_bytes, active_tasks, advanced_cleanup, cli);

    let ReferenceValidation {
        protected_refs,
        safe_to_clean,
    } = validate_deferred_references(
        &processed_sheets,
        workbook_xml_for_scan.as_deref(),
        workbook_rels_for_scan.as_deref(),
        &input_display,
    );

    let known_cells_by_sheet_file = Mutex::new(HashMap::new());
    let finalizer = SheetFinalizer {
        tx: &tx,
        analysis_tx: analysis_tx.as_ref(),
        known_cells_by_sheet_file: &known_cells_by_sheet_file,
        protected_refs: &protected_refs,
        safe_to_clean,
        attribution_safe_analysis,
        store_output: cli.store_output,
        compression_level: cli.compression_level,
    };
    finalize_sheets(processed_sheets, &finalizer, cli.disable_multiprocessing);

    if !queue_calculation_chain(
        calc_chain_part,
        workbook_xml_for_calc_chain.as_deref(),
        workbook_rels_for_calc_chain.as_deref(),
        &known_cells_by_sheet_file,
        &tx,
        &input_display,
        cli.store_output,
        cli.compression_level,
    ) {
        return;
    }
    debug!(event = "file_queued_for_writer", xlsx = %input_display, elapsed_ms = file_start.elapsed().as_millis(), "Queued XLSX for writing");
}

pub fn process_files(input_files: Vec<PathBuf>, cli: &Cli) -> anyhow::Result<()> {
    let _profile = profiling::phase("batch", "", "");
    ensure_output_directory(cli)?;
    let (analysis_tx, analysis_handle) = start_optional_analysis_writer(cli)?;
    let batch_progress = BatchProgress::new(input_files.len());
    let active_tasks: ActiveTasks = Arc::new(Mutex::new(HashMap::new()));
    let (stop_watchdog_tx, watchdog_handle) = start_watchdog(active_tasks.clone());
    let writer_workers = configured_worker_count(cli.writer_workers);
    let max_active_workbooks = configured_worker_count(cli.max_active_workbooks);
    let workbook_limiter = WorkbookLimiter::new(max_active_workbooks);
    let writer_pool = ThreadPool::new(writer_workers);

    log_batch_plan(cli, &batch_progress, writer_workers, max_active_workbooks);
    run_input_files(
        &input_files,
        cli,
        &writer_pool,
        &workbook_limiter,
        &batch_progress,
        &active_tasks,
        analysis_tx.clone(),
    );
    finish_batch(
        analysis_tx,
        analysis_handle,
        &writer_pool,
        stop_watchdog_tx,
        watchdog_handle,
    );
    Ok(())
}

fn ensure_output_directory(cli: &Cli) -> anyhow::Result<()> {
    if !cli.output.exists() {
        std::fs::create_dir_all(&cli.output)?;
    }
    Ok(())
}

fn start_optional_analysis_writer(
    cli: &Cli,
) -> anyhow::Result<(
    Option<mpsc::Sender<AnalysisCsvRow>>,
    Option<std::thread::JoinHandle<()>>,
)> {
    match &cli.analysis_csv {
        Some(path) => {
            let (sender, handle) = spawn_analysis_csv_writer(path)?;
            Ok((Some(sender), Some(handle)))
        }
        None => Ok((None, None)),
    }
}

fn configured_worker_count(configured: Option<usize>) -> usize {
    configured.unwrap_or_else(default_parallelism).max(1)
}

fn log_batch_plan(
    cli: &Cli,
    progress: &BatchProgress,
    writer_workers: usize,
    max_active_workbooks: usize,
) {
    info!(
        files_total = progress.files_total,
        writer_workers,
        max_active_workbooks,
        multiprocessing_disabled = cli.disable_multiprocessing,
        "Processing XLSX files"
    );
}

fn run_input_files(
    input_files: &[PathBuf],
    cli: &Cli,
    writer_pool: &ThreadPool,
    workbook_limiter: &WorkbookLimiter,
    batch_progress: &BatchProgress,
    active_tasks: &ActiveTasks,
    analysis_tx: Option<mpsc::Sender<AnalysisCsvRow>>,
) {
    let process = |input: &PathBuf| {
        process_file(
            input,
            cli,
            writer_pool,
            workbook_limiter,
            batch_progress,
            active_tasks,
            analysis_tx.clone(),
        )
    };
    if cli.disable_multiprocessing {
        input_files.iter().for_each(process);
    } else {
        input_files.par_iter().for_each(process);
    }
}

fn finish_batch(
    analysis_tx: Option<mpsc::Sender<AnalysisCsvRow>>,
    analysis_handle: Option<std::thread::JoinHandle<()>>,
    writer_pool: &ThreadPool,
    stop_watchdog_tx: mpsc::Sender<()>,
    watchdog_handle: std::thread::JoinHandle<()>,
) {
    let _profile = profiling::phase("batch_completion", "", "");
    profiling::drop_value("batch.analysis_sender", analysis_tx);
    if let Some(handle) = analysis_handle {
        let _wait = profiling::phase("wait_analysis_writer", "", "");
        handle.join().ok();
    }
    {
        let _wait = profiling::phase("wait_writer_pool", "", "");
        writer_pool.join();
    }
    let _ = stop_watchdog_tx.send(());
    {
        let _wait = profiling::phase("wait_watchdog", "", "");
        watchdog_handle.join().ok();
    }
}

#[cfg(test)]
#[path = "pipeline_regression_tests.rs"]
mod regression_tests;
