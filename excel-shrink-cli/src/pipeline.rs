use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Cursor, Read, Seek, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use threadpool::ThreadPool;
use tracing::{debug, error, info, warn};
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use excel_shrink_core::models::{CleanSheetParameters, WorkbookSpecificPatterns};
use excel_shrink_core::shared_strings::find_whitespace_sst_indices;
use excel_shrink_core::sheet::clean_sheet_data_streamwise;
use excel_shrink_core::styles::parse_xfs;
use excel_shrink_core::workbook::clean_workbook;
use excel_shrink_core::xml::extract_xml_block;

use crate::Cli;

/// The writer no longer receives uncompressed cleaned XML.
///
/// For unchanged files we raw-copy the original compressed member.
/// For cleaned files we receive a one-entry ZIP whose entry is already compressed.
/// The final XLSX writer only assembles/copies raw compressed entries.
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

fn default_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn start_watchdog(active_tasks: ActiveTasks, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let warn_after = Duration::from_secs(30);
        let tick = Duration::from_secs(10);

        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(tick);

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
                        elapsed,
                        task.xlsx,
                        task.sheet,
                        task.xml_size,
                        task.phase
                    );
                }
            }
        }
    })
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

        // The workspace currently enables only normal deflate support.
        // If an unusual input uses a method we cannot encode, write cleaned XML as deflated.
        _ => CompressionMethod::Deflated,
    }
}

fn normalized_compression_level(level: i32) -> i32 {
    level.clamp(0, 9)
}

/// Compress one cleaned XLSX member into a temporary single-entry ZIP.
///
/// This is the trick:
/// - the expensive Deflate work happens here, inside Rayon worker threads
/// - the final XLSX writer later raw-copies this already-compressed entry
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
                error!(
                    event = "writer_source_open_failed",
                    xlsx = %input_display,
                    error = %e,
                    "Failed to open source XLSX for raw copying"
                );
                return;
            }
        };

        let mut source_archive = match ZipArchive::new(source_file) {
            Ok(archive) => archive,
            Err(e) => {
                error!(
                    event = "writer_source_zip_open_failed",
                    xlsx = %input_display,
                    error = %e,
                    "Failed to open source XLSX as ZIP for raw copying"
                );
                return;
            }
        };
        if let Some(parent) = out_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                error!(
                    event = "writer_output_parent_create_failed",
                    xlsx = %input_display,
                    output = %out_path.display(),
                    parent = %parent.display(),
                    error = %e,
                    "Failed to create output parent directory"
                );
                return;
            }
        }
        let out_file = match File::create(&out_path) {
            Ok(file) => file,
            Err(e) => {
                error!(
                    event = "writer_output_create_failed",
                    xlsx = %input_display,
                    output = %out_path.display(),
                    error = %e,
                    "Failed to create output XLSX"
                );
                return;
            }
        };

        let buffered_out_file = BufWriter::with_capacity(16 * 1024 * 1024, out_file);
        let mut zip_writer = ZipWriter::new(buffered_out_file);
        let mut jobs_total: usize = 0;
        let mut copy_original_jobs: usize = 0;
        let mut copy_precompressed_jobs: usize = 0;

        let mut receive_wait_ms: u128 = 0;
        let mut copy_original_ms: u128 = 0;
        let mut copy_precompressed_ms: u128 = 0;

        loop {
            let wait_start = Instant::now();

            let job = match rx.recv() {
                Ok(job) => job,
                Err(_) => break,
            };

            receive_wait_ms += wait_start.elapsed().as_millis();
            jobs_total += 1;

            let entry_start = Instant::now();

            match job {
                WriteJob::CopyOriginal { index, filename } => {
                    copy_original_jobs += 1;

                    match source_archive.by_index(index) {
                        Ok(file) => {
                            if let Err(e) = zip_writer.raw_copy_file(file) {
                                error!(
                                    event = "zip_entry_raw_copy_failed",
                                    xlsx = %input_display,
                                    entry = %filename,
                                    error = %e,
                                    "Failed to raw-copy original ZIP entry"
                                );
                            }
                        }
                        Err(e) => {
                            error!(
                                event = "zip_entry_lookup_failed",
                                xlsx = %input_display,
                                entry = %filename,
                                index = index,
                                error = %e,
                                "Failed to find original ZIP entry by index"
                            );
                        }
                    }

                    copy_original_ms += entry_start.elapsed().as_millis();
                }

                WriteJob::CopyPrecompressed {
                    filename,
                    zip_bytes,
                } => {
                    copy_precompressed_jobs += 1;

                    if let Err(e) = raw_copy_single_entry_zip(&mut zip_writer, zip_bytes) {
                        error!(
                            event = "zip_entry_precompressed_copy_failed",
                            xlsx = %input_display,
                            entry = %filename,
                            error = %e,
                            "Failed to raw-copy precompressed ZIP entry"
                        );
                    }

                    copy_precompressed_ms += entry_start.elapsed().as_millis();
                }
            }
        }

        let finish_start = Instant::now();

        let mut buffered_out_file = match zip_writer.finish() {
            Ok(writer) => writer,
            Err(e) => {
                error!(
                    event = "file_finish_failed",
                    xlsx = %input_display,
                    output = %out_path.display(),
                    error = %e,
                    "Failed to finish output XLSX"
                );
                return;
            }
        };

        let finish_ms = finish_start.elapsed().as_millis();

        let flush_start = Instant::now();

        if let Err(e) = buffered_out_file.flush() {
            error!(
                event = "file_flush_failed",
                xlsx = %input_display,
                output = %out_path.display(),
                error = %e,
                "Failed to flush output XLSX"
            );
            return;
        }

        let flush_ms = flush_start.elapsed().as_millis();

        info!(
            event = "file_written",
            xlsx = %input_display,
            elapsed_ms = writer_start.elapsed().as_millis(),
            receive_wait_ms = receive_wait_ms,
            copy_original_ms = copy_original_ms,
            copy_precompressed_ms = copy_precompressed_ms,
            finish_ms = finish_ms,
            flush_ms = flush_ms,
            jobs_total = jobs_total,
            copy_original_jobs = copy_original_jobs,
            copy_precompressed_jobs = copy_precompressed_jobs,
            "Finished writing XLSX"
        );
    });
}

fn send_write_job(tx: &mpsc::Sender<WriteJob>, job: WriteJob, xlsx: &str) -> bool {
    if let Err(e) = tx.send(job) {
        error!(
            event = "writer_channel_closed",
            xlsx = %xlsx,
            error = %e,
            "Cannot queue ZIP write job because the XLSX writer has already stopped"
        );
        return false;
    }

    true
}

pub fn process_files(input_files: Vec<PathBuf>, cli: &Cli) -> anyhow::Result<()> {
    if !cli.output.exists() {
        std::fs::create_dir_all(&cli.output)?;
    }

    let active_tasks: ActiveTasks = Arc::new(Mutex::new(HashMap::new()));
    let stop_watchdog = Arc::new(AtomicBool::new(false));
    let watchdog_handle = start_watchdog(active_tasks.clone(), stop_watchdog.clone());

    let writer_workers = cli
        .writer_workers
        .unwrap_or_else(default_parallelism)
        .max(1);
    let writer_pool = ThreadPool::new(writer_workers);

    info!(
        writer_workers = writer_workers,
        "Configured writer thread pool"
    );

    info!(
        event = "pipeline_config",
        writer_workers = writer_workers,
        compression_level = cli.compression_level,
        store_output = cli.store_output,
        "Configured XLSX writing pipeline"
    );

    let process_file = |input: &PathBuf| {
        let file_start = Instant::now();
        let input_display = input.display().to_string();
        let active_tasks = active_tasks.clone();

        debug!("Processing: {:?}", input);

        let out_path = cli.output.join(input.file_name().unwrap());

        if let Some(parent) = out_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                error!(
                    event = "output_parent_create_failed_before_writer_start",
                    xlsx = %input_display,
                    output = %out_path.display(),
                    parent = %parent.display(),
                    error = %e,
                    "Failed to create output parent directory before starting XLSX writer"
                );
                return;
            }

            match std::fs::metadata(parent) {
                Ok(metadata) if metadata.is_dir() => {
                    debug!(
                        event = "output_parent_ready",
                        xlsx = %input_display,
                        parent = %parent.display(),
                        "Output parent directory exists"
                    );
                }
                Ok(_) => {
                    error!(
                        event = "output_parent_is_not_directory",
                        xlsx = %input_display,
                        output = %out_path.display(),
                        parent = %parent.display(),
                        "Output parent path exists but is not a directory"
                    );
                    return;
                }
                Err(e) => {
                    error!(
                        event = "output_parent_metadata_failed",
                        xlsx = %input_display,
                        output = %out_path.display(),
                        parent = %parent.display(),
                        error = %e,
                        "Output parent directory still cannot be inspected after create_dir_all"
                    );
                    return;
                }
            }
        }

        if out_path.exists() && !cli.force_overwrite {
            warn!("Output file {:?} already exists. Skipping.", out_path);
            return;
        }

        let (tx, rx) = mpsc::channel::<WriteJob>();

        spawn_xlsx_writer(
            &writer_pool,
            input.clone(),
            out_path,
            rx,
            input_display.clone(),
        );
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

        let mut archive = match ZipArchive::new(Cursor::new(file_bytes)) {
            Ok(arc) => arc,
            Err(e) => {
                error!("Failed to parse ZIP archive {:?}: {}", input, e);
                return;
            }
        };

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

        let mut sheet_params = Vec::new();

        for i in 0..archive.len() {
            let mut file = match archive.by_index(i) {
                Ok(file) => file,
                Err(e) => {
                    error!(
                        event = "zip_entry_read_failed",
                        xlsx = %input_display,
                        index = i,
                        error = %e,
                        "Failed to read ZIP entry"
                    );
                    continue;
                }
            };

            let name = file.name().to_string();
            let method = file.compression();

            let is_sheet = name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml");

            let should_process_sheet = is_sheet
                && !cli.only_clean_workbook
                && cli.sheets.as_ref().map_or(true, |sheets| {
                    let file_name = std::path::Path::new(&name)
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap();

                    sheets.iter().any(|s| file_name == s)
                });

            if should_process_sheet {
                sheet_params.push(CleanSheetParameters {
                    xlsx_file_path: input.to_string_lossy().to_string(),
                    zip_info_filename: name,
                    compress_method: method,
                    patterns: patterns.clone(),
                    advanced_cleanup: None,
                });
            } else if name == "xl/workbook.xml" {
                let mut buf = Vec::new();

                if let Err(e) = file.read_to_end(&mut buf) {
                    error!(
                        event = "workbook_read_failed",
                        xlsx = %input_display,
                        entry = %name,
                        error = %e,
                        "Failed to read workbook.xml; copying original"
                    );

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
                        error!(
                            event = "workbook_clean_failed",
                            xlsx = %input_display,
                            entry = %name,
                            error = %e,
                            "Failed to clean workbook.xml; using original workbook.xml bytes"
                        );
                        buf
                    }
                };

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
                    }
                    Err(e) => {
                        error!(
                            event = "workbook_precompress_failed",
                            xlsx = %input_display,
                            entry = %name,
                            error = %e,
                            "Failed to precompress cleaned workbook.xml; copying original"
                        );

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

        let mut sheets_data = Vec::with_capacity(sheet_params.len());

        for params in sheet_params {
            if let Ok(mut sheet_file) = archive.by_name(&params.zip_info_filename) {
                let mut buf = Vec::new();
                if let Err(e) = sheet_file.read_to_end(&mut buf) {
                    error!(
                        event = "sheet_read_failed",
                        xlsx = %input_display,
                        sheet = %params.zip_info_filename,
                        error = %e,
                        "Failed to read sheet XML"
                    );
                    continue;
                }

                sheets_data.push((params, buf));
            }
        }

        use excel_shrink_core::models::AdvancedCleanupConfig;
        use excel_shrink_core::reference_scanner::{scan_for_references, ScanResult};

        let do_advanced = cli.remove_unreferenced_whitespace_cells
            || cli.remove_unreferenced_hidden_content_cells;
        let mut advanced_config = AdvancedCleanupConfig {
            remove_unreferenced_whitespace: cli.remove_unreferenced_whitespace_cells,
            remove_unreferenced_hidden_content: cli.remove_unreferenced_hidden_content_cells,
            safe_to_clean: false,
            protected_cells: Default::default(),
        };

        let mut unsafe_reason = None;

        if do_advanced {
            let mut buffers_to_scan = Vec::new();
            let mut workbook_xml_for_scan = Vec::new();

            if let Ok(mut wb_file) = archive.by_name("xl/workbook.xml") {
                if wb_file.read_to_end(&mut workbook_xml_for_scan).is_ok() {
                    buffers_to_scan.push(workbook_xml_for_scan.as_slice());
                }
            }

            for (_, buf) in &sheets_data {
                buffers_to_scan.push(buf.as_slice());
            }

            match scan_for_references(&buffers_to_scan, &mut advanced_config.protected_cells) {
                ScanResult::Safe => {
                    advanced_config.safe_to_clean = true;
                }
                ScanResult::Unsafe(reason) => {
                    unsafe_reason = Some(reason);
                    advanced_config.safe_to_clean = false;
                }
            }
        }

        let advanced_arc = if do_advanced {
            Some(Arc::new(advanced_config))
        } else {
            None
        };

        if do_advanced {
            if let Some(reason) = &unsafe_reason {
                warn!(
                    event = "unsafe_dynamic_reference_found",
                    xlsx = %input_display,
                    function = %reason,
                    "Unsafe dynamic reference found; skipping advanced cleanup for this workbook."
                );
            } else {
                let protected_refs = &advanced_arc.as_ref().unwrap().protected_cells;

                debug!(
                    event = "advanced_cleanup_safe",
                    xlsx = %input_display,
                    protected_cell_refs_count = protected_refs.cell_ref_count(),
                    protected_range_refs_count = protected_refs.range_ref_count(),
                    "Advanced cleanup safe to proceed."
                );
            }
        }

        let sheet_count = sheets_data.len();
        let chunk_size = cli.chunk_size;
        let disable_splitting = cli.disable_sheetdata_splitting;
        let store_output = cli.store_output;
        let compression_level = cli.compression_level;

        let process_sheet =
            |mut params: CleanSheetParameters,
             data: Vec<u8>,
             tx: &mpsc::Sender<WriteJob>,
             adv_config: Option<Arc<AdvancedCleanupConfig>>| {
                params.advanced_cleanup = adv_config;

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
                            phase: "clean_sheet_data_streamwise + precompress_entry".to_string(),
                        },
                    );
                }

                debug!(
                    "Starting Sheet {} of {}: Original size {} bytes",
                    params.zip_info_filename,
                    params.xlsx_file_path,
                    data.len()
                );

                let original_size = data.len();

                let result = clean_sheet_data_streamwise(
                    Cursor::new(&data),
                    &params,
                    chunk_size,
                    disable_splitting,
                );

                let cleaned_or_original = match result {
                    Ok(cleaned_sheet) => {
                        let cleaned_data = cleaned_sheet.xml;
                        let metrics = cleaned_sheet.metrics;

                        let reduction = original_size.saturating_sub(cleaned_data.len());
                        let ratio = if original_size > 0 {
                            (reduction as f64 / original_size as f64) * 100.0
                        } else {
                            0.0
                        };

                        if let Some(adv) = &params.advanced_cleanup {
                            if adv.remove_unreferenced_whitespace {
                                info!(
                    "[{}] whitespace cleanup: sheet={} candidates={} removed={} protected_by_reference={} skipped={}",
                    params.xlsx_file_path,
                    params.zip_info_filename,
                    metrics.whitespace_candidates,
                    metrics.whitespace_removed,
                    metrics.whitespace_protected_by_reference,
                    !adv.safe_to_clean
                );
                            }

                            if adv.remove_unreferenced_hidden_content {
                                info!(
                    "[{}] hidden content cleanup: sheet={} candidates={} removed={} protected_by_reference={} skipped={}",
                    params.xlsx_file_path,
                    params.zip_info_filename,
                    metrics.hidden_candidates,
                    metrics.hidden_removed,
                    metrics.hidden_protected_by_reference,
                    !adv.safe_to_clean
                );
                            }
                        }

                        let elapsed = start_time.elapsed();

                        if elapsed.as_secs_f64() >= 5.0 {
                            warn!(
                "Slow Sheet {} of {}: Original size {} bytes, Clean size {} bytes, reduction {} bytes ({:.2}% reduction), cleaning completed in {:.2?}",
                params.zip_info_filename,
                params.xlsx_file_path,
                original_size,
                cleaned_data.len(),
                reduction,
                ratio,
                elapsed
            );
                        } else {
                            debug!(
                                "Processed Sheet {} of {} completed cleaning in {:.2?}",
                                params.zip_info_filename, params.xlsx_file_path, elapsed
                            );
                        }

                        cleaned_data
                    }

                    Err(e) => {
                        error!(
                            "Error processing {} of {}: {}. Falling back to original data.",
                            params.zip_info_filename, params.xlsx_file_path, e
                        );

                        data
                    }
                };

                let uncompressed_size = cleaned_or_original.len();
                let out_method = output_compression_method(params.compress_method, store_output);
                let precompress_start = Instant::now();

                match precompress_entry(
                    &params.zip_info_filename,
                    cleaned_or_original,
                    out_method,
                    compression_level,
                ) {
                    Ok(zip_bytes) => {
                        debug!(
                            event = "sheet_precompressed",
                            xlsx = %params.xlsx_file_path,
                            sheet = %params.zip_info_filename,
                            uncompressed_size = uncompressed_size,
                            single_entry_zip_size = zip_bytes.len(),
                            elapsed_ms = precompress_start.elapsed().as_millis(),
                            "Precompressed cleaned sheet entry"
                        );

                        if !send_write_job(
                            tx,
                            WriteJob::CopyPrecompressed {
                                filename: params.zip_info_filename.clone(),
                                zip_bytes,
                            },
                            &params.xlsx_file_path,
                        ) {
                            return;
                        }
                    }

                    Err(e) => {
                        error!(
                            event = "sheet_precompress_failed",
                            xlsx = %params.xlsx_file_path,
                            sheet = %params.zip_info_filename,
                            error = %e,
                            "Failed to precompress cleaned sheet. This sheet will be omitted from optimized output."
                        );
                    }
                }

                {
                    let mut guard = active_tasks.lock().unwrap();
                    guard.remove(&task_key);
                }
            };

        if cli.disable_multiprocessing {
            for (params, data) in sheets_data {
                process_sheet(params, data, &tx, advanced_arc.clone());
            }
        } else {
            let tx_for_sheets = tx.clone();
            sheets_data.into_par_iter().for_each_with(
                (tx_for_sheets, advanced_arc.clone()),
                |(tx, arc), (params, data)| {
                    process_sheet(params, data, tx, arc.clone());
                },
            );
        }

        debug!(
            event = "file_done",
            xlsx = %input_display,
            sheet_count = sheet_count,
            elapsed_ms = file_start.elapsed().as_millis(),
            "Finished XLSX cleaning and precompression; queued final assembly"
        );
    };

    if cli.disable_multiprocessing {
        input_files.iter().for_each(process_file);
    } else {
        input_files.par_iter().for_each(process_file);
    }

    writer_pool.join();

    stop_watchdog.store(true, Ordering::Relaxed);
    watchdog_handle.join().ok();

    Ok(())
}
