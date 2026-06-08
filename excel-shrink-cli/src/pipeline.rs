use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Cursor, Read, Seek, Write};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use threadpool::ThreadPool;
use tracing::{debug, error, info, warn};
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use excel_shrink_core::models::{
    AdvancedCleanupConfig, CellCoordinate, CleanSheetMetrics, CleanSheetParameters,
    ProtectedWorkbookRefs, SheetPreprocessConfig, SheetScanBuffer, WorkbookSpecificPatterns,
};
use excel_shrink_core::reference_scanner::{scan_for_references_by_sheet, ScanResult};
use excel_shrink_core::shared_strings::find_whitespace_sst_indices;
use excel_shrink_core::sheet::clean_sheet_data_streamwise;
use excel_shrink_core::styles::parse_xfs;
use excel_shrink_core::workbook::clean_workbook;
use excel_shrink_core::xml::extract_xml_block;

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
}

type ActiveTasks = Arc<Mutex<HashMap<String, ActiveTask>>>;
type ProcessedSheetOk = (
    CleanSheetParameters,
    usize,
    Vec<u8>,
    CleanSheetMetrics,
    Instant,
    usize,
);
type ProcessedSheetErr = (CleanSheetParameters, usize);
type ProcessedSheetResult = Result<ProcessedSheetOk, ProcessedSheetErr>;

fn default_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn normalize_sheet_target(target: &str) -> String {
    let without_leading_slash = target.trim_start_matches('/');

    if without_leading_slash.starts_with("xl/") {
        without_leading_slash.to_string()
    } else {
        format!("xl/{without_leading_slash}")
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

fn start_watchdog(active_tasks: ActiveTasks) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    let handle = std::thread::spawn(move || {
        let warn_after = Duration::from_secs(30);
        let tick = Duration::from_millis(100);

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

            for task in tasks_snapshot {
                let elapsed = now.duration_since(task.started);
                if elapsed >= warn_after {
                    warn!(
                        "active_slow_sheet: elapsed={:.2?}, file={}, sheet={}, xml_size={} bytes, phase={}",
                        elapsed, task.xlsx, task.sheet, task.xml_size, task.phase
                    );
                }
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
) {
    writer_pool.execute(move || {
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

        let out_file = match File::create(&out_path) {
            Ok(file) => file,
            Err(e) => {
                error!(event = "writer_output_create_failed", xlsx = %input_display, error = %e, "Failed to create output XLSX");
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
            error!(event = "file_flush_failed", xlsx = %input_display, output = %out_path.display(), error = %e, "Failed to flush output XLSX");
            return;
        }

        info!(event = "file_written", xlsx = %input_display, elapsed_ms = writer_start.elapsed().as_millis(), jobs_total = jobs_total, "Finished writing XLSX");
    });
}

fn send_write_job(tx: &mpsc::Sender<WriteJob>, job: WriteJob, xlsx: &str) -> bool {
    if let Err(e) = tx.send(job) {
        error!(event = "writer_channel_closed", xlsx = %xlsx, error = %e, "Cannot queue ZIP write job");
        return false;
    }
    true
}

fn apply_deferred_candidate_cleanup(
    mut xml: Vec<u8>,
    candidates: &[CellCoordinate],
    protected_refs: &ProtectedWorkbookRefs,
    sheet_file: &str,
) -> Vec<u8> {
    let mut safe_refs = std::collections::HashSet::new();
    for cand in candidates {
        let col = excel_shrink_core::utils::letters_byte_string_to_integer(&cand.column);
        if let Ok(row_str) = std::str::from_utf8(&cand.row) {
            if let Ok(row) = row_str.parse::<u32>() {
                if !protected_refs.contains(sheet_file, col, row) {
                    let mut r = cand.column.clone();
                    r.extend_from_slice(&cand.row);
                    safe_refs.insert(r);
                }
            }
        }
    }

    if safe_refs.is_empty() {
        return xml;
    }

    let re =
        regex::bytes::Regex::new(r#"(?s)<c\b[^>]*\br="([A-Z]+\d+)"[^>]*>(?:.*?</c>|/>)"#).unwrap();
    xml = re
        .replace_all(&xml, |caps: &regex::bytes::Captures| {
            let r_val = caps.get(1).unwrap().as_bytes();
            if safe_refs.contains(r_val) {
                Vec::new()
            } else {
                caps.get(0).unwrap().as_bytes().to_vec()
            }
        })
        .into_owned();

    xml
}

pub fn process_files(input_files: Vec<PathBuf>, cli: &Cli) -> anyhow::Result<()> {
    if !cli.output.exists() {
        std::fs::create_dir_all(&cli.output)?;
    }

    let active_tasks: ActiveTasks = Arc::new(Mutex::new(HashMap::new()));
    let (stop_watchdog_tx, watchdog_handle) = start_watchdog(active_tasks.clone());

    let writer_workers = cli
        .writer_workers
        .unwrap_or_else(default_parallelism)
        .max(1);
    let writer_pool = ThreadPool::new(writer_workers);

    let process_file = |input: &PathBuf| {
        let file_start = Instant::now();
        let input_display = input.display().to_string();
        let active_tasks = active_tasks.clone();
        debug!("Processing: {:?}", input);

        let out_path = cli.output.join(input.file_name().unwrap());
        if out_path.exists() && !cli.force_overwrite {
            warn!("Output file {:?} already exists. Skipping.", out_path);
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

        let (tx, rx) = mpsc::channel::<WriteJob>();
        spawn_xlsx_writer(
            &writer_pool,
            input.clone(),
            out_path,
            rx,
            input_display.clone(),
        );

        let mut all_xfs = Vec::new();
        if let Ok(mut styles_file) = archive.by_name("xl/styles.xml") {
            let mut styles_data = Vec::new();
            if styles_file.read_to_end(&mut styles_data).is_ok() {
                if let Some((cell_xfs, _, _, _)) =
                    extract_xml_block(&styles_data, b"<cellXfs", b"</cellXfs>", 0)
                {
                    if let Ok(xfs) = parse_xfs(&cell_xfs) {
                        all_xfs = xfs;
                    }
                }
            }
        }

        let mut whitespace_indices = None;
        if cli.remove_unreferenced_whitespace_cells {
            if let Ok(mut sst_file) = archive.by_name("xl/sharedStrings.xml") {
                let mut sst_data = Vec::new();
                if sst_file.read_to_end(&mut sst_data).is_ok() {
                    whitespace_indices = Some(find_whitespace_sst_indices(&sst_data));
                }
            }
        }

        let patterns = WorkbookSpecificPatterns::new(all_xfs, whitespace_indices);
        let mut sheet_tasks = Vec::new();
        let mut workbook_xml_for_scan: Option<Vec<u8>> = None;
        let mut workbook_rels_for_scan: Option<Vec<u8>> = None;

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
                && cli.sheets.as_ref().map_or(true, |sheets| {
                    std::path::Path::new(&name)
                        .file_name()
                        .and_then(|file_name| file_name.to_str())
                        .map_or(false, |file_name| sheets.iter().any(|s| file_name == s))
                });

            if should_process_sheet {
                sheet_tasks.push((
                    CleanSheetParameters {
                        xlsx_file_path: input.to_string_lossy().to_string(),
                        zip_info_filename: name,
                        compress_method: method,
                        patterns: patterns.clone(),
                        advanced_cleanup: None,
                        preprocess: SheetPreprocessConfig {
                            remove_hidden_rows: cli.preprocess_hidden_rows,
                            remove_empty_cells: cli.preprocess_empty_cells,
                        },
                    },
                    i,
                ));
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

                let cleaned_wb = match clean_workbook(&buf) {
                    Ok(cleaned) => cleaned,
                    Err(e) => {
                        error!(event = "workbook_clean_failed", xlsx = %input_display, entry = %name, error = %e, "Failed to clean workbook.xml; using original workbook.xml bytes");
                        buf
                    }
                };

                if cli.remove_unreferenced_whitespace_cells
                    || cli.remove_unreferenced_hidden_content_cells
                {
                    workbook_xml_for_scan = Some(cleaned_wb.clone());
                }

                let out_method = output_compression_method(method, cli.store_output);
                match precompress_entry(&name, cleaned_wb, out_method, cli.compression_level) {
                    Ok(zip_bytes) => {
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
                if cli.remove_unreferenced_whitespace_cells
                    || cli.remove_unreferenced_hidden_content_cells
                {
                    let mut rels_buf = Vec::new();
                    match file.read_to_end(&mut rels_buf) {
                        Ok(_) => {
                            workbook_rels_for_scan = Some(rels_buf);
                        }
                        Err(e) => {
                            error!(event = "workbook_rels_read_failed", xlsx = %input_display, entry = %name, error = %e, "Failed to read workbook relationships; sheet-scoped deferred cleanup may be skipped");
                        }
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

        let do_advanced = cli.remove_unreferenced_whitespace_cells
            || cli.remove_unreferenced_hidden_content_cells;
        let advanced_arc = if do_advanced {
            Some(Arc::new(AdvancedCleanupConfig {
                remove_unreferenced_whitespace: cli.remove_unreferenced_whitespace_cells,
                remove_unreferenced_hidden_content: cli.remove_unreferenced_hidden_content_cells,
            }))
        } else {
            None
        };

        let chunk_size = cli.chunk_size;
        let disable_splitting = cli.disable_sheetdata_splitting;

        // --- PHASE 1: Pre-Processing & Cell Cleanup (Collect Candidates) ---
        let process_sheet_initial = |mut params: CleanSheetParameters,
                                     index: usize,
                                     adv_config: Option<Arc<AdvancedCleanupConfig>>,
                                     source_zip_bytes: Arc<Vec<u8>>|
         -> ProcessedSheetResult {
            params.advanced_cleanup = adv_config;
            let mut archive = match ZipArchive::new(Cursor::new(source_zip_bytes.as_slice())) {
                Ok(archive) => archive,
                Err(e) => {
                    error!(event = "sheet_source_zip_open_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to reopen source XLSX for sheet processing");
                    return Err((params, index));
                }
            };

            let mut data = Vec::new();
            let mut sheet_file = match archive.by_name(&params.zip_info_filename) {
                Ok(file) => file,
                Err(e) => {
                    error!(event = "sheet_lookup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to find sheet XML in source XLSX");
                    return Err((params, index));
                }
            };

            if let Err(e) = sheet_file.read_to_end(&mut data) {
                error!(event = "sheet_read_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to read sheet XML");
                return Err((params, index));
            }

            let start_time = Instant::now();
            let task_key = format!("{}::{}", params.xlsx_file_path, params.zip_info_filename);

            {
                let mut guard = active_tasks.lock().unwrap();
                guard.insert(
                    task_key.clone(),
                    ActiveTask {
                        xlsx: params.xlsx_file_path.clone(),
                        sheet: params.zip_info_filename.clone(),
                        xml_size: data.len(),
                        started: start_time,
                        phase: "clean_sheet_data_streamwise".to_string(),
                    },
                );
            }

            let result = clean_sheet_data_streamwise(
                Cursor::new(&data),
                &params,
                chunk_size,
                disable_splitting,
            );

            {
                let mut guard = active_tasks.lock().unwrap();
                guard.remove(&task_key);
            }

            match result {
                Ok(cleaned) => Ok((
                    params,
                    index,
                    cleaned.xml,
                    cleaned.metrics,
                    start_time,
                    data.len(),
                )),
                Err(e) => {
                    error!(event = "sheet_clean_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to clean sheet XML; copying original");
                    Err((params, index))
                }
            }
        };

        let processed_sheets: Vec<ProcessedSheetResult> = if cli.disable_multiprocessing {
            sheet_tasks
                .into_iter()
                .map(|(params, index)| {
                    process_sheet_initial(params, index, advanced_arc.clone(), file_bytes.clone())
                })
                .collect()
        } else {
            sheet_tasks
                .into_par_iter()
                .map(|(params, index)| {
                    process_sheet_initial(params, index, advanced_arc.clone(), file_bytes.clone())
                })
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
            if let Ok((_params, _, _, metrics, _, _)) = res {
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

                if let Ok((params, _, xml, _, _, _)) = res {
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
        let finalize_sheet = |res: ProcessedSheetResult| {
            let (params, index, mut xml, metrics, start_time, original_size) = match res {
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
                xml = apply_deferred_candidate_cleanup(
                    xml,
                    &metrics.candidates,
                    &protected_refs,
                    &params.zip_info_filename,
                );
            }

            let reduction = original_size.saturating_sub(xml.len());
            let elapsed = start_time.elapsed();
            if elapsed.as_secs_f64() >= 5.0 {
                warn!(
                    "Slow Sheet {}: Clean size {} bytes, reduction {} bytes, time {:.2?}",
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

        debug!(event = "file_done", xlsx = %input_display, elapsed_ms = file_start.elapsed().as_millis(), "Finished XLSX processing");
    };

    if cli.disable_multiprocessing {
        input_files.iter().for_each(process_file);
    } else {
        input_files.par_iter().for_each(process_file);
    }

    writer_pool.join();
    let _ = stop_watchdog_tx.send(());
    watchdog_handle.join().ok();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate_s41() -> CellCoordinate {
        CellCoordinate {
            column: b"S".to_vec(),
            row: b"41".to_vec(),
        }
    }

    #[test]
    fn phase3_removes_unprotected_hidden_candidate_cell() {
        let xml = br#"<sheetData><row r="41"><c r="S41"><v>secret</v></c><c r="A1"><v>x</v></c></row></sheetData>"#
            .to_vec();
        let candidates = vec![candidate_s41()];
        let protected = ProtectedWorkbookRefs::default();

        let cleaned = apply_deferred_candidate_cleanup(
            xml,
            &candidates,
            &protected,
            "xl/worksheets/sheet1.xml",
        );

        assert!(
            !String::from_utf8_lossy(&cleaned).contains("r=\"S41\""),
            "sheet1!S41 should be removed when unprotected"
        );
    }

    #[test]
    fn phase3_keeps_sheet1_candidate_when_protected_by_sheet1_ref() {
        let xml =
            br#"<sheetData><row r="41"><c r="S41"><v>secret</v></c></row></sheetData>"#.to_vec();
        let candidates = vec![candidate_s41()];
        let mut protected = ProtectedWorkbookRefs::default();
        protected
            .refs_for_sheet_mut("xl/worksheets/sheet1.xml")
            .insert_cell(19, 41);

        let cleaned = apply_deferred_candidate_cleanup(
            xml,
            &candidates,
            &protected,
            "xl/worksheets/sheet1.xml",
        );

        assert!(
            String::from_utf8_lossy(&cleaned).contains("r=\"S41\""),
            "sheet1!S41 should be kept when protected on target sheet"
        );
    }
}
