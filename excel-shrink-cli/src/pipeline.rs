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
    AdvancedCleanupConfig, CleanSheetMetrics, CleanSheetParameters, CleanupCandidate, CleanupCause,
    CleanupStats, CleanupSwitches, ProtectedWorkbookRefs, SheetPreprocessConfig, SheetScanBuffer,
    WorkbookSpecificPatterns,
};
use excel_shrink_core::reference_scanner::{scan_for_references_by_sheet, ScanResult};
use excel_shrink_core::shared_strings::find_whitespace_sst_indices;
use excel_shrink_core::sheet::clean_sheet_data_streamwise;
use excel_shrink_core::sheet::output::trim_trailing_layout_only_rows_vec;
use excel_shrink_core::styles::parse_xfs;
use excel_shrink_core::workbook::clean_workbook_with_stats;

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
    } else {
        format!("xl/{without_leading_slash}")
    }
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
    candidates: &[CleanupCandidate],
    protected_refs: &ProtectedWorkbookRefs,
    sheet_file: &str,
    stats: &mut CleanupStats,
) -> Vec<u8> {
    let mut safe_refs = HashMap::new();
    for cand in candidates {
        if !protected_refs.contains(sheet_file, cand.col, cand.row) {
            safe_refs.insert((cand.col, cand.row), cand.cause);
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
            if let Some(cause) =
                parse_cell_reference(r_val).and_then(|reference| safe_refs.get(&reference))
            {
                stats.record_removed(*cause, caps.get(0).unwrap().as_bytes().len());
                Vec::new()
            } else {
                caps.get(0).unwrap().as_bytes().to_vec()
            }
        })
        .into_owned();

    xml
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
        let analysis_tx = analysis_tx.clone();
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
        let remove_unreferenced_whitespace_cells = !cli.disable_unreferenced_whitespace_cells;
        let remove_unreferenced_hidden_content_cells =
            !cli.disable_unreferenced_hidden_content_cells;
        let do_advanced =
            remove_unreferenced_whitespace_cells || remove_unreferenced_hidden_content_cells;

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
            preprocess_consecutive_hidden_empty_rows: cli.preprocess_hidden_rows,
            preprocess_empty_self_closing_cells: cli.preprocess_empty_cells,

            unreferenced_whitespace_cells: remove_unreferenced_whitespace_cells,
            unreferenced_hidden_content_cells: remove_unreferenced_hidden_content_cells,

            sheetdata_range_tail_pruning: !attribution_safe_analysis && !cli.disable_tail_pruning,
            trailing_layout_only_rows: !cli.disable_trailing_layout_only_rows,
            extraneous_column_range_merge: !cli.disable_extraneous_column_merge,

            ..CleanupSwitches::default()
        };
        let collect_cleanup_stats = cleanup_stats_enabled(cli);
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
                        cleanup: cleanup.clone(),
                        collect_cleanup_stats,
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
                if do_advanced {
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

            let mut sheet_file = match archive.by_name(&params.zip_info_filename) {
                Ok(file) => file,
                Err(e) => {
                    error!(event = "sheet_lookup_failed", xlsx = %params.xlsx_file_path, sheet = %params.zip_info_filename, error = %e, "Failed to find sheet XML in source XLSX");
                    return Err((params, index));
                }
            };
            let original_size = usize::try_from(sheet_file.size()).unwrap_or(usize::MAX);

            let start_time = Instant::now();
            let task_key = format!("{}::{}", params.xlsx_file_path, params.zip_info_filename);

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
                    },
                );
            }

            let result = clean_sheet_data_streamwise(
                &mut sheet_file,
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
                    original_size,
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
            let (params, index, mut xml, mut metrics, start_time, original_size) = match res {
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
                    &mut metrics.cleanup_stats,
                );
                if params.cleanup.trailing_layout_only_rows {
                    xml = trim_trailing_layout_only_rows_vec(xml, &params, &mut metrics);
                }
            }

            let reduction = original_size.saturating_sub(xml.len());
            let cleaned_size = xml.len();

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
                    send_cleanup_stats_rows(
                        analysis_tx.as_ref(),
                        &params.xlsx_file_path,
                        &params.zip_info_filename,
                        &metrics.cleanup_stats,
                        original_size,
                        cleaned_size,
                        attribution_safe_analysis,
                    );

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
