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

    fn acquire(&self) -> WorkbookPermit {
        let mut active = self.inner.active.lock().unwrap();
        while *active >= self.inner.max_active {
            active = self.inner.available.wait(active).unwrap();
        }
        *active += 1;

        WorkbookPermit {
            inner: self.inner.clone(),
        }
    }
}

impl Drop for WorkbookPermit {
    fn drop(&mut self) {
        let mut active = self.inner.active.lock().unwrap();
        *active = active.saturating_sub(1);
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
type ProcessedSheetOk = (
    CleanSheetParameters,
    usize,
    Vec<u8>,
    Range<usize>,
    CleanSheetMetrics,
    Instant,
    usize,
);
type ProcessedSheetErr = (CleanSheetParameters, usize);
type ProcessedSheetResult = Result<ProcessedSheetOk, ProcessedSheetErr>;

#[derive(Debug, Clone)]
struct SheetTask {
    params: CleanSheetParameters,
    zip_index: usize,
    original_xml_size: usize,
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
    rx: mpsc::Receiver<WriteJob>,
    input_display: String,
    mut progress: XlsxProgress,
    workbook_permit: WorkbookPermit,
) {
    writer_pool.execute(move || {
        let _workbook_permit = workbook_permit;
        let writer_start = Instant::now();
        let source_file = match File::open(&input_path) {
            Ok(file) => file,
            Err(e) => {
                error!(event = "writer_source_open_failed", xlsx = %input_display, error = %e, "Failed to open source XLSX for raw copying");
                return;
            }
        };

        let mut source_archive = match ZipArchive::new(source_file) {
            Ok(archive) => archive,
            Err(e) => {
                error!(event = "writer_source_zip_open_failed", xlsx = %input_display, error = %e, "Failed to open source XLSX as ZIP");
                return;
            }
        };
        if let Some(parent) = out_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                error!(event = "writer_output_parent_create_failed", xlsx = %input_display, output = %out_path.display(), parent = %parent.display(), error = %e, "Failed to create output parent directory");
                return;
            }
        }

        let partial_out_path = partial_output_path(&out_path);
        if partial_out_path.exists() {
            if let Err(e) = std::fs::remove_file(&partial_out_path) {
                error!(event = "writer_partial_output_remove_failed", xlsx = %input_display, output = %partial_out_path.display(), error = %e, "Failed to remove stale partial output XLSX");
                return;
            }
        }

        let out_file = match File::create(&partial_out_path) {
            Ok(file) => file,
            Err(e) => {
                error!(event = "writer_output_create_failed", xlsx = %input_display, output = %partial_out_path.display(), error = %e, "Failed to create partial output XLSX");
                return;
            }
        };

        let buffered_out_file = BufWriter::with_capacity(16 * 1024 * 1024, out_file);
        let mut zip_writer = ZipWriter::new(buffered_out_file);
        let mut jobs_total: usize = 0;

        loop {
            let job = match rx.recv() {
                Ok(job) => job,
                Err(_) => break,
            };
            jobs_total += 1;

            match job {
                WriteJob::CopyOriginal { index, filename } => {
                    match source_archive.by_index(index) {
                        Ok(file) => {
                            if let Err(e) = zip_writer.raw_copy_file(file) {
                                error!(event = "zip_entry_raw_copy_failed", xlsx = %input_display, entry = %filename, error = %e, "Failed to raw-copy original ZIP entry");
                            }
                        }
                        Err(e) => {
                            error!(event = "zip_entry_lookup_failed", xlsx = %input_display, entry = %filename, index = index, error = %e, "Failed to find original ZIP entry by index");
                        }
                    }
                }
                WriteJob::CopyPrecompressed { filename, zip_bytes } => {
                    if let Err(e) = raw_copy_single_entry_zip(&mut zip_writer, zip_bytes) {
                        error!(event = "zip_entry_precompressed_copy_failed", xlsx = %input_display, entry = %filename, error = %e, "Copy precompressed failed");
                    }
                }
            }
        }

        let mut buffered_out_file = match zip_writer.finish() {
            Ok(writer) => writer,
            Err(e) => {
                error!(event = "file_finish_failed", xlsx = %input_display, output = %out_path.display(), error = %e, "Failed to finish output XLSX");
                return;
            }
        };

        if let Err(e) = buffered_out_file.flush() {
            error!(event = "file_flush_failed", xlsx = %input_display, output = %partial_out_path.display(), error = %e, "Failed to flush partial output XLSX");
            return;
        }
        drop(buffered_out_file);

        if out_path.exists() {
            if let Err(e) = std::fs::remove_file(&out_path) {
                error!(event = "writer_existing_output_remove_failed", xlsx = %input_display, output = %out_path.display(), error = %e, "Failed to remove existing output XLSX before replacing it");
                return;
            }
        }

        if let Err(e) = std::fs::rename(&partial_out_path, &out_path) {
            error!(event = "writer_output_rename_failed", xlsx = %input_display, partial_output = %partial_out_path.display(), output = %out_path.display(), error = %e, "Failed to rename partial output XLSX");
            return;
        }

        progress.finish(
            "file_written",
            true,
            writer_start.elapsed().as_millis(),
            Some(jobs_total),
            "Finished writing XLSX",
        );
    });
}

fn partial_output_path(out_path: &Path) -> PathBuf {
    let mut partial = out_path.as_os_str().to_os_string();
    partial.push(".partial");
    PathBuf::from(partial)
}

fn send_write_job(tx: &mpsc::Sender<WriteJob>, job: WriteJob, xlsx: &str) -> bool {
    if let Err(e) = tx.send(job) {
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
    let mut safe_refs = HashMap::new();
    for cand in candidates {
        if !protected_refs.contains(sheet_file, cand.col, cand.row) {
            safe_refs.insert((cand.col, cand.row), cand.cause);
        }
    }

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

        let Some(tag_end) = find_byte_from(&xml, b'>', cell_start) else {
            break;
        };

        let attr_start = cell_start + b"<c".len();
        let self_closing = tag_end > cell_start && xml[tag_end - 1] == b'/';
        let attr_end = if self_closing { tag_end - 1 } else { tag_end };
        let attr = &xml[attr_start..attr_end];

        let cell_end = if self_closing {
            tag_end + 1
        } else {
            let content_start = tag_end + 1;
            let Some(close_rel) = find_bytes(&xml[content_start..], b"</c>") else {
                break;
            };
            content_start + close_rel + b"</c>".len()
        };

        out.extend_from_slice(&xml[pos..cell_start]);
        let raw = &xml[cell_start..cell_end];
        let replacement = extract_xml_attr_value(attr, b"r")
            .and_then(parse_cell_reference)
            .and_then(|reference| safe_refs.get(&reference).map(|cause| (reference, *cause)))
            .map(|((_col, row), cause)| {
                affected_rows.insert(row);
                if cause == CleanupCause::UnreferencedWhitespaceCells {
                    let cleaned = generate_self_closing_cell_xml(attr);
                    stats.record_delta(cause, raw.len(), cleaned.len());
                    cleaned
                } else {
                    stats.record_removed(cause, raw.len());
                    Vec::new()
                }
            });

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
    if metrics
        .tail_anchor_max_row
        .is_some_and(|row| touched_rows.contains(&row))
    {
        metrics.tail_anchor_max_row = None;
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

pub fn process_files(input_files: Vec<PathBuf>, cli: &Cli) -> anyhow::Result<()> {
    if !cli.output.exists() {
        std::fs::create_dir_all(&cli.output)?;
    }

    let (analysis_tx, analysis_handle) = match &cli.analysis_csv {
        Some(path) => {
            let (tx, handle) = spawn_analysis_csv_writer(path)?;
            (Some(tx), Some(handle))
        }
        None => (None, None),
    };

    let batch_progress = BatchProgress::new(input_files.len());
    let active_tasks: ActiveTasks = Arc::new(Mutex::new(HashMap::new()));
    let (stop_watchdog_tx, watchdog_handle) = start_watchdog(active_tasks.clone());

    let writer_workers = cli
        .writer_workers
        .unwrap_or_else(default_parallelism)
        .max(1);
    let max_active_workbooks = cli
        .max_active_workbooks
        .unwrap_or_else(default_parallelism)
        .max(1);
    let workbook_limiter = WorkbookLimiter::new(max_active_workbooks);
    let writer_pool = ThreadPool::new(writer_workers);
    info!(
        files_total = batch_progress.files_total,
        writer_workers,
        max_active_workbooks,
        multiprocessing_disabled = cli.disable_multiprocessing,
        "Processing XLSX files"
    );

    let process_file = |input: &PathBuf| {
        let workbook_permit = workbook_limiter.acquire();
        let file_start = Instant::now();
        let input_display = input.display().to_string();
        let active_tasks = active_tasks.clone();
        let analysis_tx = analysis_tx.clone();
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

        let mut file_bytes = Vec::new();
        {
            let mut f = match File::open(input) {
                Ok(f) => f,
                Err(e) => {
                    error!("Failed to open file {:?}: {}", input, e);
                    return;
                }
            };

            if let Err(e) = f.read_to_end(&mut file_bytes) {
                error!("Failed to read file {:?}: {}", input, e);
                return;
            }
        }

        let file_bytes = Arc::new(file_bytes);

        let mut archive = match ZipArchive::new(Cursor::new(file_bytes.as_slice())) {
            Ok(arc) => arc,
            Err(e) => {
                error!("Failed to parse ZIP archive {:?}: {}", input, e);
                return;
            }
        };
        let remove_unreferenced_whitespace_cells = cli.remove_unreferenced_whitespace_cells;
        let remove_unreferenced_hidden_content_cells = !cli.keep_unreferenced_hidden_content_cells;
        let do_advanced =
            remove_unreferenced_whitespace_cells || remove_unreferenced_hidden_content_cells;

        let inactive_protection_cell_policy = match cli.inactive_protection_cell_policy {
            crate::InactiveProtectionCellPolicyArg::Remove => InactiveProtectionCellPolicy::Remove,
            crate::InactiveProtectionCellPolicyArg::KeepInDataRange => {
                InactiveProtectionCellPolicy::KeepInDataRange
            }
            crate::InactiveProtectionCellPolicyArg::KeepAll => {
                InactiveProtectionCellPolicy::KeepAll
            }
        };

        let (tx, rx) = mpsc::channel::<WriteJob>();
        spawn_xlsx_writer(
            &writer_pool,
            input.clone(),
            out_path,
            rx,
            input_display.clone(),
            file_progress,
            workbook_permit,
        );

        let mut all_xfs = Vec::new();
        if let Ok(mut styles_file) = archive.by_name("xl/styles.xml") {
            let mut styles_data = Vec::new();
            if styles_file.read_to_end(&mut styles_data).is_ok() {
                if let Ok(xfs) = parse_xfs(&styles_data) {
                    all_xfs = xfs;
                }
            }
        }

        let mut whitespace_indices = None;
        if remove_unreferenced_whitespace_cells {
            if let Ok(mut sst_file) = archive.by_name("xl/sharedStrings.xml") {
                let mut sst_data = Vec::new();
                if sst_file.read_to_end(&mut sst_data).is_ok() {
                    whitespace_indices = Some(find_whitespace_sst_indices(&sst_data));
                }
            }
        }

        let patterns = WorkbookSpecificPatterns::new(all_xfs, whitespace_indices);
        let attribution_safe_analysis = attribution_safe_analysis(cli);
        let cleanup = CleanupSwitches {
            preprocess_empty_self_closing_cells: cli.preprocess_empty_cells,

            unreferenced_whitespace_cells: remove_unreferenced_whitespace_cells,
            unreferenced_hidden_content_cells: remove_unreferenced_hidden_content_cells,

            sheetdata_range_tail_pruning: !attribution_safe_analysis && !cli.disable_tail_pruning,
            trailing_layout_only_rows: !cli.disable_trailing_layout_only_rows,
            extraneous_column_range_merge: !cli.disable_extraneous_column_merge,
            column_tail_fill_merge: !cli.disable_column_tail_fill_merge,
            keep_textless_default_font_representatives: !cli
                .remove_textless_default_font_representatives,
            keep_textless_style_representatives: !cli.remove_textless_style_representatives,
            keep_textless_column_style_representatives: !cli
                .remove_textless_column_style_representatives,
            inactive_protection_cell_policy,

            ..CleanupSwitches::default()
        };
        let collect_cleanup_stats = cleanup_stats_enabled(cli);
        let selected_sheets = selected_sheet_targets(&cli.sheets);
        let mut sheet_tasks = Vec::new();
        let mut workbook_xml_for_scan: Option<Vec<u8>> = None;
        let mut workbook_rels_for_scan: Option<Vec<u8>> = None;
        let mut workbook_xml_for_calc_chain: Option<Vec<u8>> = None;
        let mut workbook_rels_for_calc_chain: Option<Vec<u8>> = None;
        let mut calc_chain_part: Option<(usize, CompressionMethod, Vec<u8>)> = None;

        for i in 0..archive.len() {
            let mut file = match archive.by_index(i) {
                Ok(file) => file,
                Err(e) => {
                    error!(event = "zip_entry_read_failed", xlsx = %input_display, index = i, error = %e, "Failed to read ZIP entry");
                    continue;
                }
            };
            let name = file.name().to_string();
            let method = file.compression();

            let is_sheet = name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml");
            let should_process_sheet = is_sheet
                && !cli.only_clean_workbook
                && sheet_selected_for_cleanup(&name, &selected_sheets);

            if should_process_sheet {
                let original_xml_size = usize::try_from(file.size()).unwrap_or(usize::MAX);

                sheet_tasks.push(SheetTask {
                    params: CleanSheetParameters {
                        xlsx_file_path: input.to_string_lossy().to_string(),
                        zip_info_filename: name,
                        compress_method: method,
                        patterns: patterns.clone(),
                        advanced_cleanup: None,
                        preprocess: SheetPreprocessConfig {
                            remove_empty_cells: cli.preprocess_empty_cells,
                        },
                        cleanup: cleanup.clone(),
                        collect_cleanup_stats,
                    },
                    zip_index: i,
                    original_xml_size,
                });
            } else if name == "xl/workbook.xml" {
                let mut buf = Vec::new();
                if let Err(e) = file.read_to_end(&mut buf) {
                    error!(event = "workbook_read_failed", xlsx = %input_display, entry = %name, error = %e, "Failed to read workbook.xml; copying original");

                    if !send_write_job(
                        &tx,
                        WriteJob::CopyOriginal {
                            index: i,
                            filename: name,
                        },
                        &input_display,
                    ) {
                        return;
                    }

                    continue;
                }

                let original_wb_size = buf.len();
                let mut cleanup_stats = if collect_cleanup_stats {
                    CleanupStats::enabled()
                } else {
                    CleanupStats::default()
                };

                let cleaned_wb = match clean_workbook_with_stats(&buf, &mut cleanup_stats) {
                    Ok(cleaned) => cleaned,
                    Err(e) => {
                        error!(event = "workbook_clean_failed", xlsx = %input_display, entry = %name, error = %e, "Failed to clean workbook.xml; using original workbook.xml bytes");
                        buf
                    }
                };
                let cleaned_wb_size = cleaned_wb.len();

                if do_advanced {
                    workbook_xml_for_scan = Some(cleaned_wb.clone());
                }
                workbook_xml_for_calc_chain = Some(cleaned_wb.clone());

                let out_method = output_compression_method(method, cli.store_output);
                match precompress_entry(&name, cleaned_wb, out_method, cli.compression_level) {
                    Ok(zip_bytes) => {
                        send_cleanup_stats_rows(
                            analysis_tx.as_ref(),
                            &input_display,
                            &name,
                            &cleanup_stats,
                            original_wb_size,
                            cleaned_wb_size,
                            attribution_safe_analysis,
                        );

                        if !send_write_job(
                            &tx,
                            WriteJob::CopyPrecompressed {
                                filename: name,
                                zip_bytes,
                            },
                            &input_display,
                        ) {
                            return;
                        }
                        continue;
                    }
                    Err(e) => {
                        error!(event = "workbook_precompress_failed", xlsx = %input_display, entry = %name, error = %e, "Failed to precompress cleaned workbook.xml; copying original");

                        if !send_write_job(
                            &tx,
                            WriteJob::CopyOriginal {
                                index: i,
                                filename: name,
                            },
                            &input_display,
                        ) {
                            return;
                        }
                    }
                }
            } else if name == "xl/_rels/workbook.xml.rels" {
                let mut rels_buf = Vec::new();
                match file.read_to_end(&mut rels_buf) {
                    Ok(_) => {
                        workbook_rels_for_calc_chain = Some(rels_buf.clone());
                        if do_advanced {
                            workbook_rels_for_scan = Some(rels_buf);
                        }
                    }
                    Err(e) => {
                        error!(event = "workbook_rels_read_failed", xlsx = %input_display, entry = %name, error = %e, "Failed to read workbook relationships; sheet-scoped deferred cleanup may be skipped");
                    }
                }
                if !send_write_job(
                    &tx,
                    WriteJob::CopyOriginal {
                        index: i,
                        filename: name,
                    },
                    &input_display,
                ) {
                    return;
                }
            } else if name == "xl/calcChain.xml" {
                let mut calc_chain_buf = Vec::new();
                match file.read_to_end(&mut calc_chain_buf) {
                    Ok(_) => {
                        calc_chain_part = Some((i, method, calc_chain_buf));
                    }
                    Err(e) => {
                        error!(event = "calc_chain_read_failed", xlsx = %input_display, entry = %name, error = %e, "Failed to read calcChain.xml; copying original");
                        if !send_write_job(
                            &tx,
                            WriteJob::CopyOriginal {
                                index: i,
                                filename: name,
                            },
                            &input_display,
                        ) {
                            return;
                        }
                    }
                }
            } else {
                if !send_write_job(
                    &tx,
                    WriteJob::CopyOriginal {
                        index: i,
                        filename: name,
                    },
                    &input_display,
                ) {
                    return;
                }
            }
        }

        sheet_tasks.sort_by(|a, b| {
            b.original_xml_size
                .cmp(&a.original_xml_size)
                .then_with(|| a.params.zip_info_filename.cmp(&b.params.zip_info_filename))
        });

        let largest_sheet_xml_bytes = sheet_tasks
            .iter()
            .map(|task| task.original_xml_size)
            .max()
            .unwrap_or(0);

        let total_sheet_xml_bytes: usize =
            sheet_tasks.iter().map(|task| task.original_xml_size).sum();

        info!(
            xlsx = %input_display,
            sheets_to_clean = sheet_tasks.len(),
            total_sheet_xml_bytes,
            largest_sheet_xml_bytes,
            parallel_range_chunk_size = cli.parallel_range_chunk_size,
            "Planned workbook cleanup"
        );

        let advanced_arc = if do_advanced {
            Some(Arc::new(AdvancedCleanupConfig {
                remove_unreferenced_whitespace: remove_unreferenced_whitespace_cells,
                remove_unreferenced_hidden_content: remove_unreferenced_hidden_content_cells,
            }))
        } else {
            None
        };

        let chunk_size = cli.chunk_size;
        let disable_splitting = cli.disable_sheetdata_splitting;

        // --- PHASE 1: Pre-Processing & Cell Cleanup (Collect Candidates) ---
        let process_sheet_initial = |task: SheetTask,
                                     adv_config: Option<Arc<AdvancedCleanupConfig>>,
                                     source_zip_bytes: Arc<Vec<u8>>|
         -> ProcessedSheetResult {
            let SheetTask {
                mut params,
                zip_index: index,
                original_xml_size: planned_original_size,
            } = task;
            params.advanced_cleanup = adv_config;
            let mut archive = match ZipArchive::new(Cursor::new(source_zip_bytes.as_slice())) {
                Ok(archive) => archive,
                Err(e) => {
                    error!(event = "sheet_source_zip_open_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to reopen source XLSX for sheet processing");
                    return Err((params, index));
                }
            };

            let mut sheet_file = match archive.by_name(&params.zip_info_filename) {
                Ok(file) => file,
                Err(e) => {
                    error!(event = "sheet_lookup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to find sheet XML in source XLSX");
                    return Err((params, index));
                }
            };
            let original_size = usize::try_from(sheet_file.size()).unwrap_or(planned_original_size);

            let start_time = Instant::now();
            let task_key = format!("{}::{}", params.xlsx_file_path, params.zip_info_filename);
            debug!(
                xlsx = %params.xlsx_file_path,
                sheet = %params.zip_info_filename,
                original_xml_bytes = original_size,
                "Processing sheet"
            );
            {
                let mut guard = active_tasks.lock().unwrap();
                guard.insert(
                    task_key.clone(),
                    ActiveTask {
                        xlsx: params.xlsx_file_path.clone(),
                        sheet: params.zip_info_filename.clone(),
                        xml_size: original_size,
                        started: start_time,
                        phase: "clean_sheet_data_streamwise".to_string(),
                        phase_started: start_time,
                    },
                );
            }

            let progress_active_tasks = active_tasks.clone();
            let progress_task_key = task_key.clone();
            let mut update_phase = move |phase: &'static str| {
                let mut guard = progress_active_tasks.lock().unwrap();
                if let Some(task) = guard.get_mut(&progress_task_key) {
                    if task.phase != phase {
                        task.phase = phase.to_string();
                        task.phase_started = Instant::now();
                    }
                }
            };

            let result = clean_sheet_data_streamwise_with_progress(
                &mut sheet_file,
                &params,
                chunk_size,
                disable_splitting,
                cli.parallel_range_chunk_size,
                &mut update_phase,
            );

            {
                let mut guard = active_tasks.lock().unwrap();
                guard.remove(&task_key);
            }

            match result {
                Ok(cleaned) => {
                    let cleaned_size = cleaned.xml.len();
                    let reduction = original_size.saturating_sub(cleaned_size);
                    debug!(
                        xlsx = %params.xlsx_file_path,
                        sheet = %params.zip_info_filename,
                        original_xml_bytes = original_size,
                        cleaned_xml_bytes = cleaned_size,
                        reduction_bytes = reduction,
                        elapsed_ms = start_time.elapsed().as_millis(),
                        "Finished sheet cleanup"
                    );
                    Ok((
                        params,
                        index,
                        cleaned.xml,
                        cleaned.sheet_data_range,
                        cleaned.metrics,
                        start_time,
                        original_size,
                    ))
                }
                Err(e) => {
                    error!(event = "sheet_clean_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to clean sheet XML; copying original");
                    Err((params, index))
                }
            }
        };

        let processed_sheets: Vec<ProcessedSheetResult> = if cli.disable_multiprocessing {
            sheet_tasks
                .into_iter()
                .map(|task| process_sheet_initial(task, advanced_arc.clone(), file_bytes.clone()))
                .collect()
        } else {
            sheet_tasks
                .into_par_iter()
                .map(|task| process_sheet_initial(task, advanced_arc.clone(), file_bytes.clone()))
                .collect()
        };

        // --- PHASE 2: Deferred Reference Validation Phase ---
        let mut has_candidates = false;
        let mut include_sheet_for_scan = vec![false; processed_sheets.len()];

        let mut sheet_name_to_file_map = HashMap::new();
        let mut sheet_files_in_workbook_order = Vec::new();
        if let (Some(wb_xml), Some(wb_rels)) = (&workbook_xml_for_scan, &workbook_rels_for_scan) {
            let (name_map, sheet_order) = build_sheet_name_to_file_map(wb_xml, wb_rels);
            sheet_name_to_file_map = name_map;
            sheet_files_in_workbook_order = sheet_order;
        }

        let mut file_to_sheet_name_map = HashMap::new();
        for (sheet_name, sheet_file) in &sheet_name_to_file_map {
            file_to_sheet_name_map.insert(sheet_file.clone(), sheet_name.clone());
        }

        for (index, res) in processed_sheets.iter().enumerate() {
            if let Ok((_params, _, _, _, metrics, _, _)) = res {
                if !metrics.candidates.is_empty() {
                    has_candidates = true;
                }

                if metrics.has_hints || !metrics.candidates.is_empty() {
                    include_sheet_for_scan[index] = true;
                }
            }
        }

        let mut protected_refs = ProtectedWorkbookRefs::default();
        let mut safe_to_clean = false;

        if has_candidates {
            let mut sheet_scan_buffers = Vec::new();

            for (index, res) in processed_sheets.iter().enumerate() {
                if !include_sheet_for_scan[index] {
                    continue;
                }

                if let Ok((params, _, xml, _, _, _, _)) = res {
                    let sheet_name = file_to_sheet_name_map
                        .get(&params.zip_info_filename)
                        .map(String::as_str)
                        .unwrap_or("Unknown");

                    sheet_scan_buffers.push(SheetScanBuffer {
                        sheet_file: &params.zip_info_filename,
                        sheet_name,
                        xml: xml.as_slice(),
                    });
                }
            }

            if let Some(wb_xml) = &workbook_xml_for_scan {
                sheet_scan_buffers.push(SheetScanBuffer {
                    sheet_file: "xl/workbook.xml",
                    sheet_name: "Workbook",
                    xml: wb_xml.as_slice(),
                });
            }

            match scan_for_references_by_sheet(
                &sheet_scan_buffers,
                &mut protected_refs,
                Some(&sheet_name_to_file_map),
                Some(&sheet_files_in_workbook_order),
            ) {
                ScanResult::Safe => {
                    safe_to_clean = true;
                }
                ScanResult::Unsafe(reason) => {
                    warn!(
                        "Unsafe dynamic reference found: {}. Skipping deferred cleanup for {}.",
                        reason, input_display
                    );
                }
            }
        }

        // --- PHASE 3: Fast Regex Cleanup & Precompression ---
        let known_cells_by_sheet_file: Arc<Mutex<HashMap<String, HashSet<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let finalize_sheet = |res: ProcessedSheetResult| {
            let (
                params,
                index,
                mut xml,
                mut sheet_data_range,
                mut metrics,
                start_time,
                original_size,
            ) = match res {
                Ok(val) => val,
                Err((params, index)) => {
                    if !send_write_job(
                        &tx,
                        WriteJob::CopyOriginal {
                            index,
                            filename: params.zip_info_filename,
                        },
                        &params.xlsx_file_path,
                    ) {
                        return;
                    }
                    return;
                }
            };

            if safe_to_clean && !metrics.candidates.is_empty() {
                let mut deferred_cleanup_rows = HashSet::new();
                match transform_sheet_data_range(&xml, sheet_data_range.clone(), |sheet_data| {
                    let (cleaned, touched_rows) = apply_deferred_candidate_cleanup(
                        sheet_data.to_vec(),
                        &metrics.candidates,
                        &protected_refs,
                        &params.zip_info_filename,
                        &mut metrics.cleanup_stats,
                    );
                    deferred_cleanup_rows = touched_rows;
                    Ok(cleaned)
                }) {
                    Ok((cleaned, new_range)) => {
                        xml = cleaned;
                        sheet_data_range = new_range;
                    }
                    Err(e) => {
                        error!(event = "deferred_candidate_cleanup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to clean deferred candidates; keeping initial XML");
                    }
                }

                invalidate_tail_anchor_for_touched_rows(&mut metrics, &deferred_cleanup_rows);

                if !deferred_cleanup_rows.is_empty() {
                    metrics.row_attributes.clear();
                    metrics.kept_row_states.clear();
                    metrics.deleted_row_states.clear();
                    match transform_sheet_data_range(&xml, sheet_data_range.clone(), |sheet_data| {
                        clean_selected_rows_and_cells_in_cell_range(
                            sheet_data,
                            &params,
                            &mut metrics,
                            &deferred_cleanup_rows,
                        )
                    }) {
                        Ok((cleaned, new_range)) => {
                            xml = cleaned;
                            sheet_data_range = new_range;
                        }
                        Err(e) => {
                            error!(event = "deferred_row_cell_cleanup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, rows = deferred_cleanup_rows.len(), error = %e, "Failed to clean rows touched by deferred cleanup; keeping deferred-cleaned XML");
                        }
                    }
                }

                metrics.row_attributes.clear();
                metrics.kept_row_states.clear();
                metrics.deleted_row_states.clear();
                match transform_sheet_data_range(&xml, sheet_data_range.clone(), |sheet_data| {
                    clean_empty_rows_from_after_cell_range(sheet_data, &params, &mut metrics)
                }) {
                    Ok((cleaned, new_range)) => {
                        xml = cleaned;
                        sheet_data_range = new_range;
                    }
                    Err(e) => {
                        error!(event = "deferred_empty_row_cleanup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to clean rows emptied by deferred cleanup; keeping deferred-cleaned XML");
                    }
                }

                if params.cleanup.trailing_layout_only_rows {
                    match transform_sheet_data_range(&xml, sheet_data_range.clone(), |sheet_data| {
                        Ok(trim_trailing_layout_only_rows_vec(
                            sheet_data.to_vec(),
                            &params,
                            &mut metrics,
                        ))
                    }) {
                        Ok((cleaned, _new_range)) => {
                            xml = cleaned;
                        }
                        Err(e) => {
                            error!(event = "deferred_tail_cleanup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to trim rows after deferred cleanup; keeping deferred-cleaned XML");
                        }
                    }
                }
            }

            let reduction = original_size.saturating_sub(xml.len());
            let cleaned_size = xml.len();
            let cleaned_cell_refs = collect_cell_refs(&xml);

            let elapsed = start_time.elapsed();
            if elapsed.as_secs_f64() >= 5.0 {
                warn!(
                    xlsx = %params.xlsx_file_path,
                    sheet = %params.zip_info_filename,
                    cleaned_xml_bytes = xml.len(),
                    reduction_bytes = reduction,
                    elapsed_ms = elapsed.as_millis(),
                    "Slow Sheet in {} / {}: Clean size {} bytes, reduction {} bytes, time {:.2?}",
                    params.xlsx_file_path,
                    params.zip_info_filename,
                    xml.len(),
                    reduction,
                    elapsed
                );
            }

            let out_method = output_compression_method(params.compress_method, cli.store_output);
            match precompress_entry(
                &params.zip_info_filename,
                xml,
                out_method,
                cli.compression_level,
            ) {
                Ok(zip_bytes) => {
                    debug!(
                        xlsx = %params.xlsx_file_path,
                        sheet = %params.zip_info_filename,
                        cleaned_xml_bytes = cleaned_size,
                        reduction_bytes = reduction,
                        "Queued cleaned sheet for writing"
                    );
                    send_cleanup_stats_rows(
                        analysis_tx.as_ref(),
                        &params.xlsx_file_path,
                        &params.zip_info_filename,
                        &metrics.cleanup_stats,
                        original_size,
                        cleaned_size,
                        attribution_safe_analysis,
                    );
                    {
                        let mut guard = known_cells_by_sheet_file.lock().unwrap();
                        guard.insert(params.zip_info_filename.clone(), cleaned_cell_refs);
                    }

                    if !send_write_job(
                        &tx,
                        WriteJob::CopyPrecompressed {
                            filename: params.zip_info_filename,
                            zip_bytes,
                        },
                        &params.xlsx_file_path,
                    ) {
                        return;
                    }
                }
                Err(e) => {
                    error!(event = "sheet_precompress_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to precompress cleaned sheet; copying original");

                    if !send_write_job(
                        &tx,
                        WriteJob::CopyOriginal {
                            index,
                            filename: params.zip_info_filename,
                        },
                        &params.xlsx_file_path,
                    ) {
                        return;
                    }
                }
            }
        };

        if cli.disable_multiprocessing {
            processed_sheets.into_iter().for_each(finalize_sheet);
        } else {
            processed_sheets.into_par_iter().for_each(finalize_sheet);
        }

        if let Some((index, method, calc_chain_xml)) = calc_chain_part {
            let maybe_cleaned_calc_chain = workbook_xml_for_calc_chain
                .as_ref()
                .zip(workbook_rels_for_calc_chain.as_ref())
                .map(|(wb_xml, wb_rels)| {
                    let sheet_id_to_file = build_sheet_id_to_file_map(wb_xml, wb_rels);
                    let known_cells = known_cells_by_sheet_file.lock().unwrap();
                    clean_calc_chain(calc_chain_xml.clone(), &sheet_id_to_file, &known_cells)
                });

            if let Some(cleaned_calc_chain) = maybe_cleaned_calc_chain {
                let out_method = output_compression_method(method, cli.store_output);
                match precompress_entry(
                    "xl/calcChain.xml",
                    cleaned_calc_chain,
                    out_method,
                    cli.compression_level,
                ) {
                    Ok(zip_bytes) => {
                        if !send_write_job(
                            &tx,
                            WriteJob::CopyPrecompressed {
                                filename: "xl/calcChain.xml".to_string(),
                                zip_bytes,
                            },
                            &input_display,
                        ) {
                            return;
                        }
                    }
                    Err(e) => {
                        error!(event = "calc_chain_precompress_failed", xlsx = %input_display, error = %e, "Failed to precompress cleaned calcChain.xml; copying original");
                        if !send_write_job(
                            &tx,
                            WriteJob::CopyOriginal {
                                index,
                                filename: "xl/calcChain.xml".to_string(),
                            },
                            &input_display,
                        ) {
                            return;
                        }
                    }
                }
            } else if !send_write_job(
                &tx,
                WriteJob::CopyOriginal {
                    index,
                    filename: "xl/calcChain.xml".to_string(),
                },
                &input_display,
            ) {
                return;
            }
        }

        debug!(event = "file_queued_for_writer", xlsx = %input_display, elapsed_ms = file_start.elapsed().as_millis(), "Queued XLSX for writing");
    };

    if cli.disable_multiprocessing {
        input_files.iter().for_each(process_file);
    } else {
        input_files.par_iter().for_each(process_file);
    }

    drop(analysis_tx);
    if let Some(handle) = analysis_handle {
        handle.join().ok();
    }

    writer_pool.join();
    let _ = stop_watchdog_tx.send(());
    watchdog_handle.join().ok();

    Ok(())
}

#[cfg(test)]
#[path = "pipeline_regression_tests.rs"]
mod regression_tests;
