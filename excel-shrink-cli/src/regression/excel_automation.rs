use super::xlsx_value_compare::{workbook_sheet_order, WorksheetCaptureRange};
use super::visual_diagnostics;
use crate::pipeline::process_files;
use crate::visual_profile::{
    profile_artifact_root, write_visual_profile_metadata, ResolvedVisualProfile,
};
use crate::{Cli, LogLevel};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const SCREENSHOT_CACHE_VERSION: u32 = 6;
const SCREENSHOT_PNG_METADATA_KEYWORD: &str = "excel-shrink";
const WORKBOOK_XML_REPAIR_MARKER_VERSION: u32 = 1;
const EXCEL_SCREENSHOT_STALL_TIMEOUT: Duration = Duration::from_secs(60);
const EXCEL_SCREENSHOT_POLL_INTERVAL: Duration = Duration::from_millis(500);
const WORKBOOK_XML_REPAIRED_SUFFIX: &str = "workbook-xml-repaired";
const MAX_VERTICAL_REFLOW_SEARCH_PX: usize = 2048;

#[derive(Debug, Clone, Serialize)]
pub struct ExcelSavedWorkbook {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExcelScreenshotReport {
    pub report_path: PathBuf,
    pub output_dir: PathBuf,
    pub differences_found: bool,
    pub difference_summary: ExcelScreenshotDifferenceSummary,
    pub kept_screenshots: bool,
    pub original_workbook_xml_repaired_path: Option<PathBuf>,
    pub screenshot_cache_root: PathBuf,
    pub original_cache_dir: PathBuf,
    pub shrunk_cache_dir: PathBuf,
    pub original_cache_hit: bool,
    pub shrunk_cache_hit: bool,
    pub range_limits: Vec<WorksheetCaptureRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_timing_path: Option<PathBuf>,
    pub open_timings: Vec<WorkbookOpenTiming>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ExcelScreenshotDifferenceSummary {
    pub total_differences: usize,
    pub shift_only_differences: usize,
    pub entropy_changed_differences: usize,
    pub screenshot_error_differences: usize,
    pub diff_pixels: u64,
    pub shift_only_diff_pixels: u64,
    pub entropy_changed_diff_pixels: u64,
    pub shift_movement_px: u64,
    pub max_shift_movement_px: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkbookOpenTiming {
    pub label: String,
    pub path: PathBuf,
    pub attempt: u32,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_at: Option<String>,
}

struct RecalculationTransfer(PathBuf);

impl RecalculationTransfer {
    fn new() -> io::Result<Self> {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("es-recalc-{}-{unique}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for RecalculationTransfer {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn save_recalculated_copy(
    input_path: &Path,
    output_path: &Path,
    work_dir: &Path,
) -> anyhow::Result<ExcelSavedWorkbook> {
    let _visual_phase = visual_diagnostics::phase("excel.recalculate_and_save", input_path.display().to_string());
    let before = super::artifact_cache::snapshot(input_path)?;
    anyhow::ensure!(input_path != output_path, "recalculation input and output must differ");
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir_all(work_dir)?;
    fs::remove_file(output_path).ok();

    let script_path = work_dir.join("excel_recalculate_and_save.ps1");
    fs::write(&script_path, RECALCULATE_AND_SAVE_SCRIPT)?;

    // Keep both Copy-Item endpoints short too: Windows PowerShell can reject
    // long UNC paths even though Excel itself only sees the local staged paths.
    let transfer = RecalculationTransfer::new()?;
    let transfer_input = transfer.0.join("input.xlsx");
    let transfer_output = transfer.0.join("output.xlsx");
    fs::copy(input_path, &transfer_input)?;
    let local_script = transfer.0.join("recalculate.ps1");
    let local_progress = transfer.0.join("recalc-progress.log");
    fs::write(&local_script, RECALCULATE_AND_SAVE_SCRIPT)?;
    let result = run_power_shell_script(
        &local_script,
        &[
            ("InputPath", power_shell_path(&transfer_input)?),
            ("OutputPath", power_shell_path(&transfer_output)?),
            ("ProgressPath", power_shell_path(&local_progress)?),
        ],
    );
    if local_progress.is_file() {
        fs::copy(&local_progress, work_dir.join("excel-recalc-progress.log"))?;
    }
    result?;
    anyhow::ensure!(
        transfer_output.is_file(),
        "Excel did not create recalculated transfer workbook: {}",
        transfer_output.display()
    );
    fs::copy(&transfer_output, output_path).map_err(|error| {
        anyhow::anyhow!(
            "failed to copy recalculated workbook to '{}': {error}",
            output_path.display()
        )
    })?;

    anyhow::ensure!(
        output_path.exists(),
        "Excel did not create recalculated workbook: {}",
        output_path.display()
    );

    super::artifact_cache::assert_snapshot_unchanged(&before, input_path, &work_dir.join("source-mutation"))?;
    super::artifact_cache::record_stage_change(&before, output_path, &work_dir.join("recalc-package-changes"))?;
    replace_workbook_with_shared_cache_link(output_path, work_dir)?;

    Ok(ExcelSavedWorkbook {
        path: output_path.to_path_buf(),
    })
}

pub fn compare_sheet_screenshots(
    original_path: &Path,
    shrunk_path: &Path,
    output_dir: &Path,
    work_dir: &Path,
    range_limits: &[WorksheetCaptureRange],
    visual_profile: &ResolvedVisualProfile,
    console_log_level: LogLevel,
) -> anyhow::Result<ExcelScreenshotReport> {
    let show_info = console_log_level.includes(LogLevel::Info);
    macro_rules! screenshot_info {
        ($($arg:tt)*) => {
            if show_info {
                eprintln!($($arg)*);
            }
        };
    }

    screenshot_info!(
        "[excel-screenshot] original={} shrunk={} output_dir={} range_limits={}",
        original_path.display(),
        shrunk_path.display(),
        output_dir.display(),
        range_limits.len()
    );

    fs::create_dir_all(output_dir)?;
    fs::create_dir_all(work_dir)?;

    let profile_root = screenshot_profile_root(work_dir, visual_profile);
    write_visual_profile_metadata(
        &profile_root.join("profile-metadata.json"),
        visual_profile,
        None,
    )?;
    let cache_root = screenshot_cache_root(work_dir, visual_profile);
    fs::create_dir_all(&cache_root)?;
    let capture_range_hash = capture_range_hash(range_limits)?;
    let mut original_for_capture = original_path.to_path_buf();
    let mut original_workbook_xml_repaired_path = None;
    let mut original_cache =
        WorkbookScreenshotCache::new(original_path, &cache_root, &capture_range_hash)?
            .with_range_limits(range_limits)?;
    let unrepaired_original_workbook_hash = original_cache.workbook_hash.clone();
    let shrunk_cache = WorkbookScreenshotCache::new(shrunk_path, &cache_root, &capture_range_hash)?
        .with_range_limits(range_limits)?;

    if original_workbook_xml_repair_is_marked(&cache_root, &unrepaired_original_workbook_hash) {
        screenshot_info!(
            "[excel-screenshot] original_hash={} previously required workbook.xml repair; repairing before screenshot capture",
            unrepaired_original_workbook_hash
        );
        let repaired = repair_original_workbook_xml_only(original_path, work_dir)?;
        screenshot_info!(
            "[excel-screenshot] repaired original workbook.xml path={}",
            repaired.display()
        );
        original_for_capture = repaired.clone();
        original_workbook_xml_repaired_path = Some(repaired);
        original_cache =
            WorkbookScreenshotCache::new(&original_for_capture, &cache_root, &capture_range_hash)?
                .with_range_limits(range_limits)?;
    }

    let script_path = work_dir.join("excel_compare_screenshots.ps1");
    fs::write(&script_path, SCREENSHOT_COMPARE_SCRIPT)?;
    write_visual_capture_support_files(work_dir)?;
    let range_limit_path = work_dir.join("screenshot-range-limits.json");
    fs::write(
        &range_limit_path,
        serde_json::to_string_pretty(range_limits)?,
    )?;

    let capture_manifest_path = output_dir.join("screenshot-captures.json");
    let progress_path = output_dir.join("screenshot-progress.log");
    let report_path = output_dir.join("screenshot-differences.json");
    let open_timing_path = output_dir.join("screenshot-open-timings.json");
    let mut original_cache_hit = original_cache.is_complete()?;
    let shrunk_cache_hit = shrunk_cache.is_complete()?;
    let original_sheet_cache_hits = original_cache.valid_current_sheet_captures()?.len();
    let shrunk_sheet_cache_hits = shrunk_cache.valid_current_sheet_captures()?.len();
    screenshot_info!(
        "[excel-screenshot] cache original_hash={} hit={} sheet_hits={}/{} shrunk_hash={} hit={} sheet_hits={}/{} root={}",
        original_cache.workbook_hash,
        original_cache_hit,
        original_sheet_cache_hits,
        original_cache.sheet_identities.len(),
        shrunk_cache.workbook_hash,
        shrunk_cache_hit,
        shrunk_sheet_cache_hits,
        shrunk_cache.sheet_identities.len(),
        cache_root.display()
    );

    if original_cache_hit && shrunk_cache_hit {
        screenshot_info!(
            "[excel-screenshot] all sheets cached; writing capture manifest without opening Excel"
        );
        write_cached_capture_manifest_without_excel(
            &capture_manifest_path,
            &progress_path,
            &original_cache,
            &shrunk_cache,
        )?;
    } else {
        let run_capture = |original_for_capture: &Path,
                           original_cache: &WorkbookScreenshotCache,
                           original_cache_hit: bool,
                           shrunk_cache_hit: bool|
         -> anyhow::Result<ScreenshotCaptureRun> {
            fs::create_dir_all(&original_cache.cache_dir)?;
            fs::create_dir_all(&shrunk_cache.cache_dir)?;
            let original_sheet_cache_path = work_dir.join("original-sheet-screenshot-cache.json");
            let shrunk_sheet_cache_path = work_dir.join("shrunk-sheet-screenshot-cache.json");
            let original_sheet_cache_count =
                write_sheet_cache_plan(original_cache, &original_sheet_cache_path)?;
            let shrunk_sheet_cache_count =
                write_sheet_cache_plan(&shrunk_cache, &shrunk_sheet_cache_path)?;
            screenshot_info!(
                "[excel-screenshot] sheet cache original_hits={}/{} shrunk_hits={}/{}",
                original_sheet_cache_count,
                original_cache.sheet_identities.len(),
                shrunk_sheet_cache_count,
                shrunk_cache.sheet_identities.len()
            );
            let original_excel_path =
                link_workbook_with_suffix(original_for_capture, work_dir, "original")?;
            let shrunk_excel_path = link_workbook_with_suffix(shrunk_path, work_dir, "shrunk")?;
            screenshot_info!(
            "[excel-screenshot] capture inputs original_excel={} shrunk_excel={} progress={} manifest={}",
            original_excel_path.display(),
            shrunk_excel_path.display(),
            progress_path.display(),
            capture_manifest_path.display()
        );
            let _ = fs::remove_file(&capture_manifest_path);
            let _ = fs::remove_file(&open_timing_path);

            let outcome = run_power_shell_script_with_status_and_stall_timeout(
                &script_path,
                &[
                    ("OriginalPath", power_shell_path(&original_excel_path)?),
                    ("ShrunkPath", power_shell_path(&shrunk_excel_path)?),
                    ("OutputDir", power_shell_path(output_dir)?),
                    (
                        "CaptureManifestPath",
                        power_shell_path(&capture_manifest_path)?,
                    ),
                    ("RangeLimitPath", power_shell_path(&range_limit_path)?),
                    (
                        "OriginalCacheDir",
                        power_shell_path(&original_cache.cache_dir)?,
                    ),
                    ("ShrunkCacheDir", power_shell_path(&shrunk_cache.cache_dir)?),
                    (
                        "OriginalSheetCachePath",
                        power_shell_path(&original_sheet_cache_path)?,
                    ),
                    (
                        "ShrunkSheetCachePath",
                        power_shell_path(&shrunk_sheet_cache_path)?,
                    ),
                    ("OriginalAlreadyCached", original_cache_hit.to_string()),
                    ("ShrunkAlreadyCached", shrunk_cache_hit.to_string()),
                ],
                &progress_path,
                EXCEL_SCREENSHOT_STALL_TIMEOUT,
            )?;

            Ok(ScreenshotCaptureRun {
                outcome,
                original_excel_path,
                shrunk_excel_path,
            })
        };

        let capture_run = run_capture(
            &original_for_capture,
            &original_cache,
            original_cache_hit,
            shrunk_cache_hit,
        )?;
        let repair_reason = match &capture_run.outcome {
            PowerShellScriptOutcome::Completed(status) => {
                if screenshot_capture_needs_original_workbook_repair(
                    status,
                    &progress_path,
                    &capture_manifest_path,
                ) {
                    Some("Excel failed before opening original workbook".to_string())
                } else {
                    ensure_screenshot_capture_completed(
                        status,
                        &capture_manifest_path,
                        &progress_path,
                        &capture_run.original_excel_path,
                        &capture_run.shrunk_excel_path,
                    )?;
                    None
                }
            }
            PowerShellScriptOutcome::TimedOut { stalled_for } => Some(format!(
                "Excel automation stalled for {:.1}s",
                stalled_for.as_secs_f64()
            )),
        };

        if let Some(repair_reason) = repair_reason {
            preserve_stalled_progress_log(output_dir, &progress_path)?;
            screenshot_info!(
                "[excel-screenshot] {repair_reason}; repairing original workbook.xml and retrying"
            );

            let repaired = repair_original_workbook_xml_only(original_path, work_dir)?;
            screenshot_info!(
                "[excel-screenshot] repaired original workbook.xml path={}",
                repaired.display()
            );

            original_for_capture = repaired.clone();
            original_workbook_xml_repaired_path = Some(repaired);
            original_cache = WorkbookScreenshotCache::new(
                &original_for_capture,
                &cache_root,
                &capture_range_hash,
            )?
            .with_range_limits(range_limits)?;
            original_cache_hit = original_cache.is_complete()?;

            let retry_run = run_capture(
                &original_for_capture,
                &original_cache,
                original_cache_hit,
                shrunk_cache_hit,
            )?;
            match &retry_run.outcome {
                PowerShellScriptOutcome::Completed(status) => {
                    ensure_screenshot_capture_completed(
                        status,
                        &capture_manifest_path,
                        &progress_path,
                        &retry_run.original_excel_path,
                        &retry_run.shrunk_excel_path,
                    )?;
                    mark_original_workbook_xml_repair_needed(
                        &cache_root,
                        &unrepaired_original_workbook_hash,
                    )?;
                }
                PowerShellScriptOutcome::TimedOut { stalled_for } => {
                    anyhow::bail!(
                    "Excel screenshot capture stalled for {:.1}s after original workbook.xml repair; manifest: {}\n{}",
                    stalled_for.as_secs_f64(),
                    capture_manifest_path.display(),
                    screenshot_capture_failure_details(
                        &progress_path,
                        &retry_run.original_excel_path,
                        &retry_run.shrunk_excel_path
                    )
                );
                }
            }
        }

        update_workbook_screenshot_cache(&capture_manifest_path, &original_cache, true)?;
        update_workbook_screenshot_cache(&capture_manifest_path, &shrunk_cache, false)?;
    }

    let sheet_xml_diff_context = SheetXmlDiffContext {
        original_workbook_path: original_for_capture.as_path(),
        shrunk_workbook_path: shrunk_path,
    };
    let difference_summary = compare_captured_screenshots(
        &capture_manifest_path,
        &report_path,
        Some(&sheet_xml_diff_context),
    )?;
    let differences_found = difference_summary.total_differences > 0;
    let (open_timing_path, open_timings) = if open_timing_path.exists() {
        (
            Some(open_timing_path.clone()),
            read_json_file(&open_timing_path).unwrap_or_default(),
        )
    } else {
        (None, Vec::new())
    };

    screenshot_info!(
        "[excel-screenshot] done differences_found={} report={} open_timings={}",
        differences_found,
        report_path.display(),
        open_timings.len()
    );

    Ok(ExcelScreenshotReport {
        report_path,
        output_dir: output_dir.to_path_buf(),
        differences_found,
        difference_summary,
        kept_screenshots: true,
        original_workbook_xml_repaired_path,
        screenshot_cache_root: cache_root,
        original_cache_dir: original_cache.cache_dir,
        shrunk_cache_dir: shrunk_cache.cache_dir,
        original_cache_hit,
        shrunk_cache_hit,
        range_limits: range_limits.to_vec(),
        open_timing_path,
        open_timings,
    })
}

fn write_visual_capture_support_files(directory: &Path) -> anyhow::Result<()> {
    for (name, source) in [
        (
            "excel_visual_bounds.ps1",
            include_str!("excel_visual_bounds.ps1"),
        ),
        (
            "excel_visual_clipboard.ps1",
            include_str!("excel_visual_clipboard.ps1"),
        ),
        ("excel_visual_plan.cs", include_str!("excel_visual_plan.cs")),
    ] {
        fs::write(directory.join(name), source)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ScreenshotCapture {
    sheet_index: usize,
    sheet_name: String,
    capture_range: Option<String>,
    capture_strategy: Option<String>,
    used_range_error: Option<String>,
    screenshot_error: Option<String>,
    original_image: Option<PathBuf>,
    shrunk_image: Option<PathBuf>,
    #[serde(default)]
    original_background_rgba: Option<ScreenshotRgba>,
    #[serde(default)]
    shrunk_background_rgba: Option<ScreenshotRgba>,
    capture_info: Option<ScreenshotCaptureInfo>,
    #[serde(default)]
    tiles: Vec<ScreenshotCaptureTile>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
struct ScreenshotRgba {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ScreenshotCaptureInfo {
    strategy: String,
    address: String,
    start_row: u32,
    start_column: u32,
    rows: u32,
    columns: u32,
    used_range_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ScreenshotCaptureTile {
    tile_index: usize,
    capture_range: String,
    capture_info: ScreenshotCaptureInfo,
    original_image: Option<PathBuf>,
    shrunk_image: Option<PathBuf>,
    #[serde(default)]
    original_background_rgba: Option<ScreenshotRgba>,
    #[serde(default)]
    shrunk_background_rgba: Option<ScreenshotRgba>,
    original_width: Option<u32>,
    original_height: Option<u32>,
    shrunk_width: Option<u32>,
    shrunk_height: Option<u32>,
}

#[derive(Debug, Clone)]
struct WorkbookScreenshotCache {
    workbook_hash: String,
    workbook_file_name: String,
    capture_range_hash: String,
    sheet_capture_range_hashes: BTreeMap<usize, String>,
    cache_root: PathBuf,
    cache_dir: PathBuf,
    manifest_path: PathBuf,
    marker_path: PathBuf,
    sheet_identities: BTreeMap<usize, WorkbookScreenshotSheetIdentity>,
}

#[derive(Debug, Clone)]
struct WorkbookScreenshotSheetIdentity {
    sheet_file: String,
    sheet_xml_hash: String,
}

#[derive(Debug)]
struct MaterializedScreenshotImage {
    path: PathBuf,
    image_hash: String,
}

#[derive(Serialize)]
struct ScreenshotPngMetadata<'a> {
    workbook_file: &'a str,
    workbook_hash: &'a str,
    sheet_index: usize,
    sheet_name: &'a str,
    sheet_file: &'a str,
    sheet_xml_hash: &'a str,
    sheet_capture_range_hash: &'a str,
    side: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tile_index: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkbookScreenshotCacheManifest {
    version: u32,
    workbook_hash: String,
    capture_range_hash: String,
    captures: Vec<WorkbookScreenshotCapture>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SheetXmlScreenshotCacheManifest {
    version: u32,
    sheet_xml_hash: String,
    sheet_capture_range_hash: String,
    capture: WorkbookScreenshotCapture,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkbookScreenshotCapture {
    sheet_index: usize,
    sheet_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sheet_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sheet_xml_hash: Option<String>,
    capture_range: Option<String>,
    capture_strategy: Option<String>,
    used_range_error: Option<String>,
    screenshot_error: Option<String>,
    image: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image_hash: Option<String>,
    #[serde(default)]
    background_rgba: Option<ScreenshotRgba>,
    capture_info: Option<ScreenshotCaptureInfo>,
    #[serde(default)]
    tiles: Vec<WorkbookScreenshotTile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkbookScreenshotTile {
    tile_index: usize,
    capture_range: String,
    capture_info: ScreenshotCaptureInfo,
    image: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image_hash: Option<String>,
    #[serde(default)]
    background_rgba: Option<ScreenshotRgba>,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkbookScreenshotSheetCachePlan {
    captures: Vec<WorkbookScreenshotCapture>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkbookScreenshotCacheMarker {
    version: u32,
    workbook_hash: String,
    capture_range_hash: String,
    capture_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkbookXmlRepairMarker {
    version: u32,
    original_workbook_hash: String,
}

impl WorkbookScreenshotCache {
    fn new(
        workbook_path: &Path,
        cache_root: &Path,
        capture_range_hash: &str,
    ) -> anyhow::Result<Self> {
        let workbook_hash = workbook_content_hash(workbook_path)?;
        let workbook_file_name = workbook_path
            .file_name()
            .and_then(OsStr::to_str)
            .filter(|name| !name.is_empty())
            .unwrap_or("workbook.xlsx")
            .to_string();
        let sheet_identities = workbook_sheet_identities(workbook_path)?;
        let cache_dir = workbook_screenshot_cache_root(cache_root).join(&workbook_hash);
        Ok(Self {
            manifest_path: cache_dir.join("workbook-screenshots.json"),
            marker_path: cache_dir.join("complete.json"),
            workbook_hash,
            workbook_file_name,
            capture_range_hash: capture_range_hash.to_string(),
            sheet_capture_range_hashes: BTreeMap::new(),
            cache_root: cache_root.to_path_buf(),
            cache_dir,
            sheet_identities,
        })
    }

    fn with_range_limits(mut self, range_limits: &[WorksheetCaptureRange]) -> anyhow::Result<Self> {
        self.sheet_capture_range_hashes =
            sheet_capture_range_hashes(range_limits, &self.sheet_identities)?;
        Ok(self)
    }

    fn sheet_capture_range_hash(&self, sheet_index: usize) -> &str {
        self.sheet_capture_range_hashes
            .get(&sheet_index)
            .map(String::as_str)
            .unwrap_or(self.capture_range_hash.as_str())
    }

    fn is_complete(&self) -> anyhow::Result<bool> {
        let valid = self.valid_current_sheet_captures()?;
        Ok(!self.sheet_identities.is_empty() && valid.len() == self.sheet_identities.len())
    }

    fn valid_current_sheet_captures(
        &self,
    ) -> anyhow::Result<BTreeMap<usize, WorkbookScreenshotCapture>> {
        let mut valid = BTreeMap::new();

        for (&sheet_index, identity) in &self.sheet_identities {
            if let Some(capture) = self.valid_shared_sheet_capture(sheet_index, identity)? {
                valid.insert(sheet_index, capture);
            }
        }
        if valid.len() == self.sheet_identities.len() {
            return Ok(valid);
        }

        for manifest_path in self.candidate_manifest_paths()? {
            let Ok(manifest) = read_json_file::<WorkbookScreenshotCacheManifest>(&manifest_path)
            else {
                continue;
            };
            if manifest.version != SCREENSHOT_CACHE_VERSION
                || manifest.capture_range_hash != self.capture_range_hash
            {
                continue;
            }

            for capture in manifest.captures {
                let Some(sheet_xml_hash) = capture.sheet_xml_hash.as_deref() else {
                    continue;
                };
                if !capture_has_valid_images(&capture)? {
                    continue;
                }

                for (&sheet_index, identity) in &self.sheet_identities {
                    if valid.contains_key(&sheet_index)
                        || sheet_xml_hash != identity.sheet_xml_hash.as_str()
                    {
                        continue;
                    }
                    valid.insert(
                        sheet_index,
                        capture_for_current_sheet(capture.clone(), sheet_index, identity),
                    );
                }
            }

            if valid.len() == self.sheet_identities.len() {
                break;
            }
        }

        Ok(valid)
    }

    fn valid_shared_sheet_capture(
        &self,
        sheet_index: usize,
        identity: &WorkbookScreenshotSheetIdentity,
    ) -> anyhow::Result<Option<WorkbookScreenshotCapture>> {
        let manifest_path = shared_sheet_screenshot_manifest_path(self, sheet_index, identity);
        if !manifest_path.exists() {
            return Ok(None);
        }

        let Ok(manifest) = read_json_file::<SheetXmlScreenshotCacheManifest>(&manifest_path) else {
            return Ok(None);
        };
        if manifest.version != SCREENSHOT_CACHE_VERSION
            || manifest.sheet_xml_hash != identity.sheet_xml_hash
            || manifest.sheet_capture_range_hash != self.sheet_capture_range_hash(sheet_index)
        {
            return Ok(None);
        }

        let capture = capture_for_current_sheet(manifest.capture, sheet_index, identity);
        if !capture_has_valid_images(&capture)? {
            return Ok(None);
        }

        Ok(Some(capture))
    }

    fn candidate_manifest_paths(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        if self.manifest_path.exists() {
            paths.push(self.manifest_path.clone());
        }
        if !self.cache_root.exists() {
            return Ok(paths);
        }

        let workbook_cache_root = workbook_screenshot_cache_root(&self.cache_root);
        if !workbook_cache_root.exists() {
            return Ok(paths);
        }

        for entry in fs::read_dir(workbook_cache_root)? {
            let entry = entry?;
            let path = entry.path();
            if path == self.cache_dir || !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("workbook-screenshots.json");
            if manifest_path.exists() {
                paths.push(manifest_path);
            }
        }

        Ok(paths)
    }
}

fn capture_for_current_sheet(
    mut capture: WorkbookScreenshotCapture,
    sheet_index: usize,
    identity: &WorkbookScreenshotSheetIdentity,
) -> WorkbookScreenshotCapture {
    capture.sheet_index = sheet_index;
    capture.sheet_file = Some(identity.sheet_file.clone());
    capture.sheet_xml_hash = Some(identity.sheet_xml_hash.clone());
    capture
}

fn workbook_xml_repair_marker_path(cache_root: &Path, original_workbook_hash: &str) -> PathBuf {
    cache_root
        .join("workbook-xml-repair-needed")
        .join(format!("{original_workbook_hash}.json"))
}

fn original_workbook_xml_repair_is_marked(cache_root: &Path, original_workbook_hash: &str) -> bool {
    let marker_path = workbook_xml_repair_marker_path(cache_root, original_workbook_hash);
    let Ok(marker) = read_json_file::<WorkbookXmlRepairMarker>(&marker_path) else {
        return false;
    };

    marker.version == WORKBOOK_XML_REPAIR_MARKER_VERSION
        && marker.original_workbook_hash == original_workbook_hash
}

fn mark_original_workbook_xml_repair_needed(
    cache_root: &Path,
    original_workbook_hash: &str,
) -> anyhow::Result<()> {
    let marker_path = workbook_xml_repair_marker_path(cache_root, original_workbook_hash);
    if let Some(parent) = marker_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let marker = WorkbookXmlRepairMarker {
        version: WORKBOOK_XML_REPAIR_MARKER_VERSION,
        original_workbook_hash: original_workbook_hash.to_string(),
    };
    fs::write(marker_path, serde_json::to_vec_pretty(&marker)?)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct ScreenshotDifference {
    sheet_index: usize,
    sheet_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tile_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sheet_file: Option<String>,
    capture_range: Option<String>,
    capture_strategy: Option<String>,
    used_range_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screenshot_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_box: Option<DiffBox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_pixel_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approximate_cells: Option<ApproximateCells>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approximate_cells_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_crop: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shrunk_crop: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_pixels: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vertical_reflow_proof: Option<VerticalReflowProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_sheet_xml: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shrunk_sheet_xml: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sheet_xml_diff: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sheet_xml_visual_diff: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sheet_xml_diff_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_image: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shrunk_image: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
struct VerticalReflowProof {
    classification: &'static str,
    reconstructs_original: bool,
    original_width: u32,
    original_height: u32,
    shrunk_width: u32,
    shrunk_height: u32,
    height_delta_px: i32,
    matched_rows: u32,
    inserted_rows: u32,
    removed_rows: u32,
    non_spacer_rows: u32,
    uses_only_spacer_rows: bool,
    repair_completed: bool,
    operations: Vec<VerticalReflowOperation>,
    residual_diff_pixels: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    residual_diff_box: Option<DiffBox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reconstructed_from_shrunk: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    residual_diff_pixels_image: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
struct VerticalReflowOperation {
    at_y: u32,
    shift_suffix_by_px: i32,
    direction: &'static str,
    row_count: u32,
    spacer: VerticalReflowSpacer,
}

#[derive(Debug, Clone, Serialize)]
struct VerticalReflowSpacer {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    rgba: Option<ScreenshotRgba>,
}

struct VerticalReflowProofResult {
    proof: VerticalReflowProof,
    reconstructed: image::RgbaImage,
}

struct SheetXmlDiffContext<'a> {
    original_workbook_path: &'a Path,
    shrunk_workbook_path: &'a Path,
}

struct SheetXmlDiffArtifacts {
    sheet_file: String,
    original_sheet_xml: PathBuf,
    shrunk_sheet_xml: PathBuf,
    sheet_xml_diff: PathBuf,
    sheet_xml_visual_diff: Option<PathBuf>,
}

#[derive(Debug, Default, Clone)]
struct SheetXmlDiffSourceContext {
    line_info: Vec<SheetXmlDiffLineInfo>,
    rows_by_index: BTreeMap<u32, SheetRowXmlInfo>,
    columns: Vec<SheetColumnXmlInfo>,
    style_xfs: Vec<String>,
    shared_strings: Vec<String>,
}

#[derive(Debug, Default, Clone)]
struct SheetXmlDiffLineInfo {
    row: Option<SheetRowXmlInfo>,
    cell: Option<SheetCellXmlInfo>,
    shared_string_index: Option<usize>,
}

#[derive(Debug, Clone)]
struct SheetRowXmlInfo {
    row_index: u32,
    style_id: Option<usize>,
    node: String,
}

#[derive(Debug, Clone)]
struct SheetCellXmlInfo {
    reference: String,
    row_index: u32,
    column_name: String,
    column_index: u32,
    style_id: Option<usize>,
    cell_type: Option<String>,
    node: String,
}

#[derive(Debug, Clone)]
struct SheetColumnXmlInfo {
    min: u32,
    max: u32,
    style_id: Option<usize>,
    node: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct DiffBox {
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
    image_width: u32,
    image_height: u32,
}

#[derive(Debug, Clone, Serialize)]
struct ApproximateCells {
    top_left: String,
    bottom_right: String,
}

#[derive(Debug, Clone, Copy)]
struct VisualCellRange {
    top_row: u32,
    bottom_row: u32,
    left_column: u32,
    right_column: u32,
}

#[derive(Debug, Clone)]
struct VisualDiffCells {
    range: VisualCellRange,
    rows: BTreeSet<u32>,
    cells: BTreeSet<(u32, u32)>,
}

#[derive(Debug, Clone, Copy)]
struct CropRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct ScreenshotImageComparison {
    sheet_index: usize,
    sheet_name: String,
    tile_index: Option<usize>,
    capture_range: Option<String>,
    capture_strategy: Option<String>,
    used_range_error: Option<String>,
    capture_info: ScreenshotCaptureInfo,
    original_image: PathBuf,
    shrunk_image: PathBuf,
    original_background_rgba: Option<ScreenshotRgba>,
    shrunk_background_rgba: Option<ScreenshotRgba>,
}

fn compare_captured_screenshots(
    capture_manifest_path: &Path,
    report_path: &Path,
    sheet_xml_diff_context: Option<&SheetXmlDiffContext<'_>>,
) -> anyhow::Result<ExcelScreenshotDifferenceSummary> {
    let captures: Vec<ScreenshotCapture> = read_json_file(capture_manifest_path)?;

    let mut differences = Vec::new();

    for capture in captures {
        let original_image = capture
            .original_image
            .as_ref()
            .map(normalize_automation_path);
        let shrunk_image = capture.shrunk_image.as_ref().map(normalize_automation_path);

        if let Some(screenshot_error) = capture.screenshot_error.clone() {
            differences.push(screenshot_error_difference(
                capture,
                original_image,
                shrunk_image,
                screenshot_error,
            ));
            continue;
        }

        if !capture.tiles.is_empty() {
            for tile in &capture.tiles {
                let original_image = tile.original_image.as_ref().map(normalize_automation_path);
                let shrunk_image = tile.shrunk_image.as_ref().map(normalize_automation_path);
                let (original_image, shrunk_image) = match (original_image, shrunk_image) {
                    (Some(original_image), Some(shrunk_image)) => (original_image, shrunk_image),
                    (original_image, shrunk_image) => {
                        differences.push(screenshot_error_difference_for_parts(
                            capture.sheet_index,
                            capture.sheet_name.clone(),
                            Some(tile.tile_index),
                            Some(tile.capture_range.clone()),
                            capture.capture_strategy.clone(),
                            capture.used_range_error.clone(),
                            original_image,
                            shrunk_image,
                            "missing cached screenshot tile".to_string(),
                        ));
                        continue;
                    }
                };

                compare_screenshot_image_pair(
                    ScreenshotImageComparison {
                        sheet_index: capture.sheet_index,
                        sheet_name: capture.sheet_name.clone(),
                        tile_index: Some(tile.tile_index),
                        capture_range: Some(tile.capture_range.clone()),
                        capture_strategy: capture.capture_strategy.clone(),
                        used_range_error: capture.used_range_error.clone(),
                        capture_info: tile.capture_info.clone(),
                        original_image,
                        shrunk_image,
                        original_background_rgba: tile.original_background_rgba,
                        shrunk_background_rgba: tile.shrunk_background_rgba,
                    },
                    report_path,
                    sheet_xml_diff_context,
                    &mut differences,
                )?;
            }
            continue;
        }

        let Some(original_image) = original_image else {
            continue;
        };
        let Some(shrunk_image) = shrunk_image else {
            continue;
        };
        let capture_info = capture
            .capture_info
            .clone()
            .unwrap_or(ScreenshotCaptureInfo {
                strategy: capture
                    .capture_strategy
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                address: capture.capture_range.clone().unwrap_or_default(),
                start_row: 1,
                start_column: 1,
                rows: 1,
                columns: 1,
                used_range_error: capture.used_range_error.clone(),
            });

        compare_screenshot_image_pair(
            ScreenshotImageComparison {
                sheet_index: capture.sheet_index,
                sheet_name: capture.sheet_name,
                tile_index: None,
                capture_range: capture.capture_range,
                capture_strategy: capture.capture_strategy,
                used_range_error: capture.used_range_error,
                capture_info,
                original_image,
                shrunk_image,
                original_background_rgba: capture.original_background_rgba,
                shrunk_background_rgba: capture.shrunk_background_rgba,
            },
            report_path,
            sheet_xml_diff_context,
            &mut differences,
        )?;
    }

    let summary = summarize_screenshot_differences(&differences);
    fs::write(report_path, serde_json::to_vec_pretty(&differences)?)?;
    Ok(summary)
}

fn summarize_screenshot_differences(
    differences: &[ScreenshotDifference],
) -> ExcelScreenshotDifferenceSummary {
    let mut summary = ExcelScreenshotDifferenceSummary {
        total_differences: differences.len(),
        ..ExcelScreenshotDifferenceSummary::default()
    };

    for difference in differences {
        let diff_pixels = difference.diff_pixel_count.unwrap_or(0);
        summary.diff_pixels = summary.diff_pixels.saturating_add(diff_pixels);
        if difference.screenshot_error.is_some() {
            summary.screenshot_error_differences += 1;
        }

        if let Some(proof) = &difference.vertical_reflow_proof {
            if proof.reconstructs_original
                && proof.uses_only_spacer_rows
                && proof.residual_diff_pixels == 0
            {
                let movement = u64::from(proof.inserted_rows) + u64::from(proof.removed_rows);
                summary.shift_only_differences += 1;
                summary.shift_only_diff_pixels =
                    summary.shift_only_diff_pixels.saturating_add(diff_pixels);
                summary.shift_movement_px = summary.shift_movement_px.saturating_add(movement);
                summary.max_shift_movement_px = summary.max_shift_movement_px.max(
                    proof
                        .operations
                        .iter()
                        .map(|operation| operation.shift_suffix_by_px.unsigned_abs())
                        .max()
                        .unwrap_or(movement.min(u64::from(u32::MAX)) as u32),
                );
                continue;
            }
        }

        summary.entropy_changed_differences += 1;
        summary.entropy_changed_diff_pixels = summary
            .entropy_changed_diff_pixels
            .saturating_add(diff_pixels);
    }

    summary
}

fn compare_screenshot_image_pair(
    comparison: ScreenshotImageComparison,
    report_path: &Path,
    sheet_xml_diff_context: Option<&SheetXmlDiffContext<'_>>,
    differences: &mut Vec<ScreenshotDifference>,
) -> anyhow::Result<()> {
    let original_pixels_unpadded = match load_screenshot_image(&comparison.original_image) {
        Ok(pixels) => pixels,
        Err(error) => {
            differences.push(screenshot_error_difference_for_parts(
                comparison.sheet_index,
                comparison.sheet_name,
                comparison.tile_index,
                comparison.capture_range,
                comparison.capture_strategy,
                comparison.used_range_error,
                Some(comparison.original_image),
                Some(comparison.shrunk_image),
                format!("failed to load original screenshot: {error:#}"),
            ));
            return Ok(());
        }
    };
    let shrunk_pixels_unpadded = match load_screenshot_image(&comparison.shrunk_image) {
        Ok(pixels) => pixels,
        Err(error) => {
            differences.push(screenshot_error_difference_for_parts(
                comparison.sheet_index,
                comparison.sheet_name,
                comparison.tile_index,
                comparison.capture_range,
                comparison.capture_strategy,
                comparison.used_range_error,
                Some(comparison.original_image),
                Some(comparison.shrunk_image),
                format!("failed to load shrunk screenshot: {error:#}"),
            ));
            return Ok(());
        }
    };
    let vertical_reflow_result =
        prove_vertical_suffix_reflow(&original_pixels_unpadded, &shrunk_pixels_unpadded);
    let output_dir = report_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("report path has no parent"))?;
    let artifact_stem = screenshot_diff_artifact_stem(
        comparison.sheet_index,
        &comparison.sheet_name,
        comparison.tile_index,
    );
    let vertical_reflow_proof = match vertical_reflow_result {
        Some(mut result) => {
            let reconstructed_path =
                output_dir.join(format!("{artifact_stem}-vertical-reflow-reconstructed.png"));
            result
                .reconstructed
                .save(&reconstructed_path)
                .map_err(|error| {
                    anyhow::anyhow!(
                        "failed to save image '{}': {error}",
                        reconstructed_path.display()
                    )
                })?;
            result.proof.reconstructed_from_shrunk = Some(reconstructed_path);
            if let Some(residual_diff_box) = result.proof.residual_diff_box {
                let residual_diff_pixels_path = output_dir.join(format!(
                    "{artifact_stem}-vertical-reflow-residual-diff-pixels.png"
                ));
                let (residual_original, residual_reconstructed) =
                    pad_screenshot_pair_to_common_canvas(
                        original_pixels_unpadded.clone(),
                        result.reconstructed,
                        comparison.original_background_rgba,
                        None,
                    );
                save_diff_pixels(
                    &residual_original,
                    &residual_reconstructed,
                    &residual_diff_pixels_path,
                    residual_diff_box,
                    &comparison.capture_info,
                )?;
                result.proof.residual_diff_pixels_image = Some(residual_diff_pixels_path);
            }
            Some(result.proof)
        }
        None => None,
    };
    let (original_pixels, shrunk_pixels) = pad_screenshot_pair_to_common_canvas(
        original_pixels_unpadded,
        shrunk_pixels_unpadded,
        comparison.original_background_rgba,
        comparison.shrunk_background_rgba,
    );

    let Some(diff_box) = find_diff_box(&original_pixels, &shrunk_pixels) else {
        return Ok(());
    };

    let original_crop = output_dir.join(format!("{artifact_stem}-diff-original.png"));
    let shrunk_crop = output_dir.join(format!("{artifact_stem}-diff-shrunk.png"));
    let diff_pixels = output_dir.join(format!("{artifact_stem}-diff-pixels.png"));

    save_diff_crop(
        &original_pixels,
        &original_crop,
        diff_box,
        &comparison.capture_info,
    )?;
    save_diff_crop(
        &shrunk_pixels,
        &shrunk_crop,
        diff_box,
        &comparison.capture_info,
    )?;
    save_diff_pixels(
        &original_pixels,
        &shrunk_pixels,
        &diff_pixels,
        diff_box,
        &comparison.capture_info,
    )?;
    let diff_pixel_count = count_diff_pixels(&original_pixels, &shrunk_pixels, diff_box);

    let visual_diff_cells = visual_diff_cells(
        &original_pixels,
        &shrunk_pixels,
        diff_box,
        &comparison.capture_info,
    );
    let approximate_cells = approximate_cells(visual_diff_cells.range);
    let sheet_xml_diff = match sheet_xml_diff_context {
        Some(context) => create_sheet_xml_git_diff(
            context,
            output_dir,
            comparison.sheet_index,
            &comparison.sheet_name,
            comparison.tile_index,
            Some(&visual_diff_cells),
        ),
        None => Ok(None),
    };
    let (
        sheet_file,
        original_sheet_xml,
        shrunk_sheet_xml,
        sheet_xml_diff,
        sheet_xml_visual_diff,
        sheet_xml_diff_error,
    ) = match sheet_xml_diff {
        Ok(Some(artifacts)) => (
            Some(artifacts.sheet_file),
            Some(artifacts.original_sheet_xml),
            Some(artifacts.shrunk_sheet_xml),
            Some(artifacts.sheet_xml_diff),
            artifacts.sheet_xml_visual_diff,
            None,
        ),
        Ok(None) => (None, None, None, None, None, None),
        Err(error) => (None, None, None, None, None, Some(format!("{error:#}"))),
    };

    differences.push(ScreenshotDifference {
        sheet_index: comparison.sheet_index,
        sheet_name: comparison.sheet_name,
        tile_index: comparison.tile_index,
        sheet_file,
        capture_range: comparison.capture_range,
        capture_strategy: comparison.capture_strategy,
        used_range_error: comparison.used_range_error,
        screenshot_error: None,
        diff_box: Some(diff_box),
        diff_pixel_count: Some(diff_pixel_count),
        approximate_cells: Some(approximate_cells),
        approximate_cells_error: None,
        original_crop: Some(original_crop),
        shrunk_crop: Some(shrunk_crop),
        diff_pixels: Some(diff_pixels),
        vertical_reflow_proof,
        original_sheet_xml,
        shrunk_sheet_xml,
        sheet_xml_diff,
        sheet_xml_visual_diff,
        sheet_xml_diff_error,
        original_image: None,
        shrunk_image: None,
    });

    Ok(())
}

fn screenshot_diff_artifact_stem(
    sheet_index: usize,
    sheet_name: &str,
    tile_index: Option<usize>,
) -> String {
    let safe_name = sanitize_sheet_name(sheet_name);
    match tile_index {
        Some(tile_index) => format!("{sheet_index}-{safe_name}-tile-{tile_index:03}"),
        None => format!("{sheet_index}-{safe_name}"),
    }
}

fn screenshot_error_difference(
    capture: ScreenshotCapture,
    original_image: Option<PathBuf>,
    shrunk_image: Option<PathBuf>,
    screenshot_error: String,
) -> ScreenshotDifference {
    screenshot_error_difference_for_parts(
        capture.sheet_index,
        capture.sheet_name,
        None,
        capture.capture_range,
        capture.capture_strategy,
        capture.used_range_error,
        original_image,
        shrunk_image,
        screenshot_error,
    )
}

fn screenshot_error_difference_for_parts(
    sheet_index: usize,
    sheet_name: String,
    tile_index: Option<usize>,
    capture_range: Option<String>,
    capture_strategy: Option<String>,
    used_range_error: Option<String>,
    original_image: Option<PathBuf>,
    shrunk_image: Option<PathBuf>,
    screenshot_error: String,
) -> ScreenshotDifference {
    ScreenshotDifference {
        sheet_index,
        sheet_name,
        tile_index,
        sheet_file: None,
        capture_range,
        capture_strategy,
        used_range_error,
        screenshot_error: Some(screenshot_error),
        diff_box: None,
        diff_pixel_count: None,
        approximate_cells: None,
        approximate_cells_error: None,
        original_crop: None,
        shrunk_crop: None,
        diff_pixels: None,
        vertical_reflow_proof: None,
        original_sheet_xml: None,
        shrunk_sheet_xml: None,
        sheet_xml_diff: None,
        sheet_xml_visual_diff: None,
        sheet_xml_diff_error: None,
        original_image,
        shrunk_image,
    }
}

fn write_sheet_cache_plan(
    cache: &WorkbookScreenshotCache,
    cache_plan_path: &Path,
) -> anyhow::Result<usize> {
    let captures: Vec<_> = cache
        .valid_current_sheet_captures()?
        .into_values()
        .collect();
    let count = captures.len();
    fs::write(
        cache_plan_path,
        serde_json::to_vec_pretty(&WorkbookScreenshotSheetCachePlan { captures })?,
    )?;
    Ok(count)
}

fn write_cached_capture_manifest_without_excel(
    capture_manifest_path: &Path,
    progress_path: &Path,
    original_cache: &WorkbookScreenshotCache,
    shrunk_cache: &WorkbookScreenshotCache,
) -> anyhow::Result<()> {
    let original_captures = original_cache.valid_current_sheet_captures()?;
    let shrunk_captures = shrunk_cache.valid_current_sheet_captures()?;
    anyhow::ensure!(
        original_captures.len() == original_cache.sheet_identities.len()
            && shrunk_captures.len() == shrunk_cache.sheet_identities.len(),
        "cannot build cached screenshot manifest without complete original and shrunk sheet caches"
    );

    let mut captures = Vec::new();
    for (&sheet_index, original_capture) in &original_captures {
        let Some(shrunk_capture) = shrunk_captures.get(&sheet_index) else {
            anyhow::bail!("missing shrunk cached screenshot for sheet index {sheet_index}");
        };
        captures.push(cached_capture_pair(
            sheet_index,
            original_capture,
            shrunk_capture,
        ));
    }

    if let Some(parent) = capture_manifest_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(capture_manifest_path, serde_json::to_vec_pretty(&captures)?)?;
    fs::write(
        progress_path,
        format!(
            "screenshot capture skipped {}; all sheets reused from cache without opening Excel\ncached manifest='{}' captures={}\n",
            current_timestamp_for_progress_log(),
            capture_manifest_path.display(),
            captures.len()
        ),
    )?;
    Ok(())
}

fn cached_capture_pair(
    sheet_index: usize,
    original_capture: &WorkbookScreenshotCapture,
    shrunk_capture: &WorkbookScreenshotCapture,
) -> ScreenshotCapture {
    let tiles = cached_capture_tiles(original_capture, shrunk_capture);
    let preferred_capture = if original_capture.capture_info.is_some() {
        original_capture
    } else {
        shrunk_capture
    };

    ScreenshotCapture {
        sheet_index,
        sheet_name: first_non_empty_string(
            Some(original_capture.sheet_name.as_str()),
            Some(shrunk_capture.sheet_name.as_str()),
        )
        .unwrap_or_else(|| format!("sheet {sheet_index}")),
        capture_range: first_non_empty_string(
            original_capture.capture_range.as_deref(),
            shrunk_capture.capture_range.as_deref(),
        ),
        capture_strategy: first_non_empty_string(
            original_capture.capture_strategy.as_deref(),
            shrunk_capture.capture_strategy.as_deref(),
        ),
        used_range_error: first_non_empty_string(
            original_capture.used_range_error.as_deref(),
            shrunk_capture.used_range_error.as_deref(),
        ),
        screenshot_error: None,
        original_image: if original_capture.tiles.is_empty() {
            original_capture.image.clone()
        } else {
            None
        },
        shrunk_image: if shrunk_capture.tiles.is_empty() {
            shrunk_capture.image.clone()
        } else {
            None
        },
        original_background_rgba: original_capture.background_rgba,
        shrunk_background_rgba: shrunk_capture.background_rgba,
        capture_info: preferred_capture.capture_info.clone(),
        tiles,
    }
}

fn cached_capture_tiles(
    original_capture: &WorkbookScreenshotCapture,
    shrunk_capture: &WorkbookScreenshotCapture,
) -> Vec<ScreenshotCaptureTile> {
    let original_by_index = original_capture
        .tiles
        .iter()
        .map(|tile| (tile.tile_index, tile))
        .collect::<BTreeMap<_, _>>();
    let shrunk_by_index = shrunk_capture
        .tiles
        .iter()
        .map(|tile| (tile.tile_index, tile))
        .collect::<BTreeMap<_, _>>();
    let indexes = original_by_index
        .keys()
        .chain(shrunk_by_index.keys())
        .copied()
        .collect::<BTreeSet<_>>();

    indexes
        .into_iter()
        .filter_map(|tile_index| {
            let original_tile = original_by_index.get(&tile_index).copied();
            let shrunk_tile = shrunk_by_index.get(&tile_index).copied();
            let preferred_tile = original_tile.or(shrunk_tile)?;
            Some(ScreenshotCaptureTile {
                tile_index,
                capture_range: preferred_tile.capture_range.clone(),
                capture_info: preferred_tile.capture_info.clone(),
                original_image: original_tile.and_then(|tile| tile.image.clone()),
                shrunk_image: shrunk_tile.and_then(|tile| tile.image.clone()),
                original_background_rgba: original_tile.and_then(|tile| tile.background_rgba),
                shrunk_background_rgba: shrunk_tile.and_then(|tile| tile.background_rgba),
                original_width: original_tile.and_then(|tile| tile.width),
                original_height: original_tile.and_then(|tile| tile.height),
                shrunk_width: shrunk_tile.and_then(|tile| tile.width),
                shrunk_height: shrunk_tile.and_then(|tile| tile.height),
            })
        })
        .collect()
}

fn first_non_empty_string(primary: Option<&str>, fallback: Option<&str>) -> Option<String> {
    primary
        .filter(|value| !value.trim().is_empty())
        .or_else(|| fallback.filter(|value| !value.trim().is_empty()))
        .map(ToOwned::to_owned)
}

fn current_timestamp_for_progress_log() -> String {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| format!("unix_ms={}", duration.as_millis()))
        .unwrap_or_else(|_| "unix_ms=0".to_string())
}

fn update_workbook_screenshot_cache(
    capture_manifest_path: &Path,
    cache: &WorkbookScreenshotCache,
    original_side: bool,
) -> anyhow::Result<()> {
    let pair_captures: Vec<ScreenshotCapture> = read_json_file(capture_manifest_path)?;
    let mut captures = Vec::with_capacity(pair_captures.len());
    let mut complete = !pair_captures.is_empty();

    for pair in pair_captures {
        let image = if original_side {
            pair.original_image.as_ref()
        } else {
            pair.shrunk_image.as_ref()
        }
        .map(normalize_automation_path);

        let tiles = side_cache_tiles(
            cache,
            pair.sheet_index,
            &pair.sheet_name,
            &pair.tiles,
            original_side,
        )?;
        let sheet_identity = cache.sheet_identities.get(&pair.sheet_index);
        let mut image_hash = None;
        let image = match (image, sheet_identity) {
            (Some(image), Some(sheet_identity)) if image.exists() => {
                let materialized = materialize_shared_screenshot_image(
                    cache,
                    sheet_identity,
                    pair.sheet_index,
                    &pair.sheet_name,
                    original_side,
                    None,
                    &image,
                )?;
                image_hash = Some(materialized.image_hash);
                Some(materialized.path)
            }
            (Some(image), _) => {
                if image.exists() {
                    image_hash = Some(file_content_hash(&image)?);
                }
                Some(image)
            }
            (None, _) => None,
        };

        if pair.screenshot_error.is_some()
            || sheet_identity.is_none()
            || (tiles.is_empty() && image_hash.is_none())
            || tiles.iter().any(|tile| tile.image_hash.is_none())
        {
            complete = false;
        }

        let capture = WorkbookScreenshotCapture {
            sheet_index: pair.sheet_index,
            sheet_name: pair.sheet_name,
            sheet_file: sheet_identity.map(|identity| identity.sheet_file.clone()),
            sheet_xml_hash: sheet_identity.map(|identity| identity.sheet_xml_hash.clone()),
            capture_range: pair.capture_range,
            capture_strategy: pair.capture_strategy,
            used_range_error: pair.used_range_error,
            screenshot_error: pair.screenshot_error,
            image,
            image_hash,
            background_rgba: if original_side {
                pair.original_background_rgba
            } else {
                pair.shrunk_background_rgba
            },
            capture_info: pair.capture_info,
            tiles,
        };

        if let Some(sheet_identity) = sheet_identity {
            if capture_has_valid_images(&capture)? {
                write_shared_sheet_screenshot_cache_manifest(
                    cache,
                    pair.sheet_index,
                    sheet_identity,
                    &capture,
                )?;
            }
        }

        captures.push(capture);
    }

    if complete {
        fs::create_dir_all(&cache.cache_dir)?;
        let manifest = WorkbookScreenshotCacheManifest {
            version: SCREENSHOT_CACHE_VERSION,
            workbook_hash: cache.workbook_hash.clone(),
            capture_range_hash: cache.capture_range_hash.clone(),
            captures,
        };
        fs::write(&cache.manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
        let marker = WorkbookScreenshotCacheMarker {
            version: SCREENSHOT_CACHE_VERSION,
            workbook_hash: cache.workbook_hash.clone(),
            capture_range_hash: cache.capture_range_hash.clone(),
            capture_count: manifest.captures.len(),
        };
        fs::write(&cache.marker_path, serde_json::to_vec_pretty(&marker)?)?;
    }

    Ok(())
}

fn side_cache_tiles(
    cache: &WorkbookScreenshotCache,
    sheet_index: usize,
    sheet_name: &str,
    tiles: &[ScreenshotCaptureTile],
    original_side: bool,
) -> anyhow::Result<Vec<WorkbookScreenshotTile>> {
    tiles
        .iter()
        .map(|tile| {
            let image = if original_side {
                tile.original_image.as_ref()
            } else {
                tile.shrunk_image.as_ref()
            }
            .map(normalize_automation_path);
            let mut image_hash = None;
            let image = match (image, cache.sheet_identities.get(&sheet_index)) {
                (Some(image), Some(sheet_identity)) if image.exists() => {
                    let materialized = materialize_shared_screenshot_image(
                        cache,
                        sheet_identity,
                        sheet_index,
                        sheet_name,
                        original_side,
                        Some(tile.tile_index),
                        &image,
                    )?;
                    image_hash = Some(materialized.image_hash);
                    Some(materialized.path)
                }
                (Some(image), _) => {
                    if image.exists() {
                        image_hash = Some(file_content_hash(&image)?);
                    }
                    Some(image)
                }
                (None, _) => None,
            };
            let (width, height) = if original_side {
                (tile.original_width, tile.original_height)
            } else {
                (tile.shrunk_width, tile.shrunk_height)
            };
            let background_rgba = if original_side {
                tile.original_background_rgba
            } else {
                tile.shrunk_background_rgba
            };

            Ok(WorkbookScreenshotTile {
                tile_index: tile.tile_index,
                capture_range: tile.capture_range.clone(),
                capture_info: tile.capture_info.clone(),
                image,
                image_hash,
                background_rgba,
                width,
                height,
            })
        })
        .collect()
}

fn materialize_shared_screenshot_image(
    cache: &WorkbookScreenshotCache,
    sheet_identity: &WorkbookScreenshotSheetIdentity,
    sheet_index: usize,
    sheet_name: &str,
    original_side: bool,
    tile_index: Option<usize>,
    source: &Path,
) -> anyhow::Result<MaterializedScreenshotImage> {
    let target = shared_screenshot_image_path(
        cache,
        sheet_identity,
        sheet_index,
        sheet_name,
        original_side,
        tile_index,
    );
    let metadata = ScreenshotPngMetadata {
        workbook_file: &cache.workbook_file_name,
        workbook_hash: &cache.workbook_hash,
        sheet_index,
        sheet_name,
        sheet_file: &sheet_identity.sheet_file,
        sheet_xml_hash: &sheet_identity.sheet_xml_hash,
        sheet_capture_range_hash: cache.sheet_capture_range_hash(sheet_index),
        side: if original_side { "original" } else { "shrunk" },
        tile_index,
    };
    if source == target {
        write_png_screenshot_metadata(&target, &metadata)?;
        return Ok(MaterializedScreenshotImage {
            image_hash: file_content_hash(&target)?,
            path: target,
        });
    }

    if target.exists() {
        write_png_screenshot_metadata(&target, &metadata)?;
        return Ok(MaterializedScreenshotImage {
            image_hash: file_content_hash(&target)?,
            path: target,
        });
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::copy(source, &target).map_err(|error| {
        anyhow::anyhow!(
            "failed to copy shared screenshot '{}' from '{}': {error}",
            target.display(),
            source.display()
        )
    })?;
    write_png_screenshot_metadata(&target, &metadata)?;
    Ok(MaterializedScreenshotImage {
        image_hash: file_content_hash(&target)?,
        path: target,
    })
}

fn shared_screenshot_image_path(
    cache: &WorkbookScreenshotCache,
    sheet_identity: &WorkbookScreenshotSheetIdentity,
    sheet_index: usize,
    sheet_name: &str,
    original_side: bool,
    tile_index: Option<usize>,
) -> PathBuf {
    let sheet_file_name = Path::new(&sheet_identity.sheet_file)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(sheet_identity.sheet_file.as_str());
    let safe_sheet_file = sanitize_limited_path_segment(sheet_file_name, 48);
    let safe_name = sanitize_limited_path_segment(sheet_name, 80);
    let side = if original_side { "original" } else { "shrunk" };
    let base_name = format!(
        "{sheet_index:03}-{}-{safe_sheet_file}-{safe_name}-{side}",
        sheet_identity.sheet_xml_hash
    );
    let file_name = match tile_index {
        Some(tile_index) => format!("{base_name}-tile-{tile_index:03}.png"),
        None => format!("{base_name}.png"),
    };

    shared_sheet_screenshot_cache_dir(cache, sheet_index, sheet_identity).join(file_name)
}

fn shared_sheet_screenshot_cache_dir(
    cache: &WorkbookScreenshotCache,
    sheet_index: usize,
    sheet_identity: &WorkbookScreenshotSheetIdentity,
) -> PathBuf {
    shared_sheet_screenshot_cache_root(&cache.cache_root)
        .join(cache.sheet_capture_range_hash(sheet_index))
        .join(&sheet_identity.sheet_xml_hash)
}

fn shared_sheet_screenshot_manifest_path(
    cache: &WorkbookScreenshotCache,
    sheet_index: usize,
    sheet_identity: &WorkbookScreenshotSheetIdentity,
) -> PathBuf {
    shared_sheet_screenshot_cache_dir(cache, sheet_index, sheet_identity)
        .join("sheet-screenshot.json")
}

fn write_shared_sheet_screenshot_cache_manifest(
    cache: &WorkbookScreenshotCache,
    sheet_index: usize,
    sheet_identity: &WorkbookScreenshotSheetIdentity,
    capture: &WorkbookScreenshotCapture,
) -> anyhow::Result<()> {
    let manifest_path = shared_sheet_screenshot_manifest_path(cache, sheet_index, sheet_identity);
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let manifest = SheetXmlScreenshotCacheManifest {
        version: SCREENSHOT_CACHE_VERSION,
        sheet_xml_hash: sheet_identity.sheet_xml_hash.clone(),
        sheet_capture_range_hash: cache.sheet_capture_range_hash(sheet_index).to_string(),
        capture: capture_for_current_sheet(capture.clone(), sheet_index, sheet_identity),
    };
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}

fn load_screenshot_image(path: &Path) -> anyhow::Result<image::RgbaImage> {
    Ok(image::open(path)
        .map_err(|error| anyhow::anyhow!("failed to open image '{}': {error}", path.display()))?
        .to_rgba8())
}

fn pad_screenshot_pair_to_common_canvas(
    original: image::RgbaImage,
    shrunk: image::RgbaImage,
    original_background_rgba: Option<ScreenshotRgba>,
    shrunk_background_rgba: Option<ScreenshotRgba>,
) -> (image::RgbaImage, image::RgbaImage) {
    let width = original.width().max(shrunk.width());
    let height = original.height().max(shrunk.height());
    (
        pad_screenshot_canvas_if_needed(original, width, height, original_background_rgba),
        pad_screenshot_canvas_if_needed(shrunk, width, height, shrunk_background_rgba),
    )
}

fn pad_screenshot_canvas_if_needed(
    source: image::RgbaImage,
    width: u32,
    height: u32,
    background_rgba: Option<ScreenshotRgba>,
) -> image::RgbaImage {
    if source.width() == width && source.height() == height {
        return source;
    }

    let fill = screenshot_canvas_fill_color(background_rgba, &source);
    let mut padded = image::RgbaImage::from_pixel(width, height, fill);
    for y in 0..source.height() {
        for x in 0..source.width() {
            padded.put_pixel(x, y, *source.get_pixel(x, y));
        }
    }
    padded
}

fn screenshot_canvas_fill_color(
    background_rgba: Option<ScreenshotRgba>,
    source: &image::RgbaImage,
) -> image::Rgba<u8> {
    background_rgba
        .map(|rgba| image::Rgba([rgba.r, rgba.g, rgba.b, rgba.a]))
        .unwrap_or_else(|| infer_screenshot_padding_color(source))
}

fn infer_screenshot_padding_color(source: &image::RgbaImage) -> image::Rgba<u8> {
    if source.width() == 0 || source.height() == 0 {
        return image::Rgba([255, 255, 255, 255]);
    }
    *source.get_pixel(source.width() - 1, source.height() - 1)
}

fn read_json_file<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let bytes = fs::read(path)?;
    let json = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    serde_json::from_slice(json)
        .map_err(|error| anyhow::anyhow!("failed to parse JSON '{}': {error}", path.display()))
}

fn workbook_content_hash(path: &Path) -> anyhow::Result<String> {
    file_content_hash(path)
}

fn file_content_hash(path: &Path) -> anyhow::Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut buffer = [0u8; 64 * 1024];
    let mut hash = fnv_offset_basis();

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hash = fnv_hash_update(hash, &buffer[..bytes_read]);
    }

    Ok(format!("{hash:016x}"))
}

fn bytes_content_hash(bytes: &[u8]) -> String {
    format!("{:016x}", fnv_hash_update(fnv_offset_basis(), bytes))
}

fn capture_range_hash(range_limits: &[WorksheetCaptureRange]) -> anyhow::Result<String> {
    let bytes = serde_json::to_vec(range_limits)?;
    Ok(bytes_content_hash(&bytes))
}

#[derive(Serialize)]
struct SheetCaptureRangeCacheKey<'a> {
    mode: &'static str,
    max_cell: Option<&'a str>,
    max_row: Option<u32>,
    max_column: Option<u32>,
    max_row_source: Option<&'a str>,
    max_column_source: Option<&'a str>,
}

fn sheet_capture_range_hashes(
    range_limits: &[WorksheetCaptureRange],
    sheet_identities: &BTreeMap<usize, WorkbookScreenshotSheetIdentity>,
) -> anyhow::Result<BTreeMap<usize, String>> {
    let range_limits_by_index: BTreeMap<usize, &WorksheetCaptureRange> = range_limits
        .iter()
        .map(|range_limit| (range_limit.sheet_index, range_limit))
        .collect();
    let mut hashes = BTreeMap::new();
    for &sheet_index in sheet_identities.keys() {
        hashes.insert(
            sheet_index,
            sheet_capture_range_hash(range_limits_by_index.get(&sheet_index).copied())?,
        );
    }
    Ok(hashes)
}

fn sheet_capture_range_hash(range_limit: Option<&WorksheetCaptureRange>) -> anyhow::Result<String> {
    let key = match range_limit {
        Some(range_limit) => SheetCaptureRangeCacheKey {
            mode: "range_limit",
            max_cell: Some(range_limit.max_cell.as_str()),
            max_row: Some(range_limit.max_row),
            max_column: Some(range_limit.max_column),
            max_row_source: range_limit.max_row_source.as_deref(),
            max_column_source: range_limit.max_column_source.as_deref(),
        },
        None => SheetCaptureRangeCacheKey {
            mode: "used_range",
            max_cell: None,
            max_row: None,
            max_column: None,
            max_row_source: None,
            max_column_source: None,
        },
    };
    Ok(bytes_content_hash(&serde_json::to_vec(&key)?))
}

fn fnv_offset_basis() -> u64 {
    0xcbf29ce484222325
}

fn fnv_hash_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn screenshot_profile_root(work_dir: &Path, visual_profile: &ResolvedVisualProfile) -> PathBuf {
    for ancestor in work_dir.ancestors() {
        if ancestor.file_name() == Some(std::ffi::OsStr::new("regression")) {
            if let Some(repo_root) = ancestor.parent() {
                return profile_artifact_root(repo_root, "screenshots", visual_profile);
            }
        }
    }

    let repo_root = std::env::current_dir().unwrap_or_else(|_| work_dir.to_path_buf());
    profile_artifact_root(&repo_root, "screenshots", visual_profile)
}

fn screenshot_cache_root(work_dir: &Path, visual_profile: &ResolvedVisualProfile) -> PathBuf {
    let _ = visual_profile;
    shared_screenshot_root(work_dir).join("cache")
}

fn shared_screenshot_root(work_dir: &Path) -> PathBuf {
    for ancestor in work_dir.ancestors() {
        if ancestor.file_name() == Some(std::ffi::OsStr::new("regression")) {
            if let Some(repo_root) = ancestor.parent() {
                return repo_root.join(".tmp").join("screenshots").join("shared");
            }
        }
    }

    std::env::current_dir()
        .unwrap_or_else(|_| work_dir.to_path_buf())
        .join(".tmp")
        .join("screenshots")
        .join("shared")
}

fn workbook_screenshot_cache_root(cache_root: &Path) -> PathBuf {
    cache_root.join("workbooks")
}

fn shared_sheet_screenshot_cache_root(cache_root: &Path) -> PathBuf {
    cache_root.join("by-sheet-xml")
}

pub fn replace_workbook_with_shared_cache_link(
    workbook_path: &Path,
    anchor_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let destination_dir = workbook_path.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "workbook path has no parent directory: {}",
            workbook_path.display()
        )
    })?;
    let destination_file_name = workbook_path.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "workbook path has no file name: {}",
            workbook_path.display()
        )
    })?;

    link_workbook_artifact_to_shared_cache(
        workbook_path,
        anchor_dir,
        destination_dir,
        destination_file_name,
    )
}

pub fn link_workbook_artifact_to_shared_cache(
    source_path: &Path,
    anchor_dir: &Path,
    destination_dir: &Path,
    destination_file_name: &OsStr,
) -> anyhow::Result<PathBuf> {
    let shared_path = ensure_shared_workbook(source_path, anchor_dir)?;
    let destination = destination_dir.join(destination_file_name);
    materialize_workbook_reference(&shared_path, &destination)?;
    Ok(destination)
}

fn ensure_shared_workbook(source_path: &Path, anchor_dir: &Path) -> anyhow::Result<PathBuf> {
    let workbook_hash = file_content_hash(source_path)?;
    let extension = workbook_extension(source_path);
    let shared_path = shared_workbook_root(anchor_dir)
        .join(&workbook_hash)
        .join(format!("workbook.{extension}"));

    if shared_path == source_path {
        return Ok(shared_path);
    }

    if shared_path.exists() {
        if path_has_expected_hash(&shared_path, Some(&workbook_hash))? {
            return Ok(shared_path);
        }
        fs::remove_file(&shared_path)?;
    }

    if let Some(parent) = shared_path.parent() {
        fs::create_dir_all(parent)?;
    }

    match fs::hard_link(source_path, &shared_path) {
        Ok(()) => Ok(shared_path),
        Err(_) => {
            fs::copy(source_path, &shared_path).map_err(|error| {
                anyhow::anyhow!(
                    "failed to materialize shared workbook '{}' from '{}': {error}",
                    shared_path.display(),
                    source_path.display()
                )
            })?;
            Ok(shared_path)
        }
    }
}

fn materialize_workbook_reference(shared_path: &Path, destination: &Path) -> anyhow::Result<()> {
    if shared_path == destination {
        return Ok(());
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    if destination.exists() {
        fs::remove_file(destination)?;
    }

    match fs::hard_link(shared_path, destination) {
        Ok(()) => Ok(()),
        Err(hard_link_error) => match symlink_file(shared_path, destination) {
            Ok(()) => Ok(()),
            Err(symlink_error) => {
                fs::copy(shared_path, destination).map_err(|copy_error| {
                    anyhow::anyhow!(
                        "failed to link shared workbook '{}' to '{}' (hardlink: {}; symlink: {}; copy: {})",
                        shared_path.display(),
                        destination.display(),
                        hard_link_error,
                        symlink_error,
                        copy_error
                    )
                })?;
                Ok(())
            }
        },
    }
}

#[cfg(windows)]
fn symlink_file(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(source, destination)
}

#[cfg(unix)]
fn symlink_file(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(not(any(windows, unix)))]
fn symlink_file(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file symlinks are not supported on this platform",
    ))
}

fn workbook_extension(path: &Path) -> String {
    path.extension()
        .and_then(OsStr::to_str)
        .filter(|extension| !extension.is_empty())
        .unwrap_or("xlsx")
        .to_ascii_lowercase()
}

fn shared_workbook_root(anchor_dir: &Path) -> PathBuf {
    for ancestor in anchor_dir.ancestors() {
        if ancestor.file_name() == Some(std::ffi::OsStr::new("regression")) {
            if let Some(repo_root) = ancestor.parent() {
                return repo_root.join(".tmp").join("workbooks").join("shared");
            }
        }
    }

    std::env::current_dir()
        .unwrap_or_else(|_| anchor_dir.to_path_buf())
        .join(".tmp")
        .join("workbooks")
        .join("shared")
}

fn workbook_sheet_identities(
    workbook_path: &Path,
) -> anyhow::Result<BTreeMap<usize, WorkbookScreenshotSheetIdentity>> {
    let sheet_files = workbook_sheet_order(workbook_path)?;
    let file = File::open(workbook_path)?;
    let mut archive = ZipArchive::new(file)?;
    let mut identities = BTreeMap::new();
    for (index, sheet_file) in sheet_files.into_iter().enumerate() {
        let mut entry = archive.by_name(&sheet_file).map_err(|error| {
            anyhow::anyhow!(
                "failed to read '{}' from '{}': {error}",
                sheet_file,
                workbook_path.display()
            )
        })?;
        let mut sheet_xml = Vec::new();
        entry.read_to_end(&mut sheet_xml)?;
        identities.insert(
            index + 1,
            WorkbookScreenshotSheetIdentity {
                sheet_file,
                sheet_xml_hash: bytes_content_hash(&sheet_xml),
            },
        );
    }
    Ok(identities)
}

fn capture_has_valid_images(capture: &WorkbookScreenshotCapture) -> anyhow::Result<bool> {
    if capture.screenshot_error.is_some() {
        return Ok(false);
    }

    if capture.tiles.is_empty() {
        let Some(image) = capture.image.as_ref().map(normalize_automation_path) else {
            return Ok(false);
        };
        return path_has_expected_hash(&image, capture.image_hash.as_deref());
    }

    for tile in &capture.tiles {
        let Some(image) = tile.image.as_ref().map(normalize_automation_path) else {
            return Ok(false);
        };
        if !path_has_expected_hash(&image, tile.image_hash.as_deref())? {
            return Ok(false);
        }
    }

    Ok(true)
}

fn path_has_expected_hash(path: &Path, expected_hash: Option<&str>) -> anyhow::Result<bool> {
    let Some(expected_hash) = expected_hash else {
        return Ok(false);
    };
    if !path.exists() {
        return Ok(false);
    }
    Ok(file_content_hash(path)? == expected_hash)
}

fn prove_vertical_suffix_reflow(
    original: &image::RgbaImage,
    shrunk: &image::RgbaImage,
) -> Option<VerticalReflowProofResult> {
    if original.width() != shrunk.width() || original.width() == 0 {
        return None;
    }

    let width = original.width();
    let original_rows = image_rows(original);
    let mut reconstructed_rows = image_rows(shrunk);
    let original_hashes = row_hashes(&original_rows);
    let mut reconstructed_hashes = row_hashes(&reconstructed_rows);
    let mut operations = Vec::new();
    let mut matched_rows = 0u32;
    let mut inserted_rows = 0u32;
    let mut removed_rows = 0u32;
    let mut non_spacer_rows = 0u32;
    let mut original_y = 0usize;
    let mut reconstructed_y = 0usize;
    let mut repair_completed = true;

    while original_y < original_rows.len() && reconstructed_y < reconstructed_rows.len() {
        if original_hashes[original_y] == reconstructed_hashes[reconstructed_y]
            && original_rows[original_y] == reconstructed_rows[reconstructed_y]
        {
            matched_rows = matched_rows.saturating_add(1);
            original_y += 1;
            reconstructed_y += 1;
            continue;
        }

        let insert_candidate = find_vertical_reflow_insert(
            &original_rows,
            &original_hashes,
            original_y,
            &reconstructed_rows,
            &reconstructed_hashes,
            reconstructed_y,
        );
        let remove_candidate = find_vertical_reflow_remove(
            &original_rows,
            &original_hashes,
            original_y,
            &reconstructed_rows,
            &reconstructed_hashes,
            reconstructed_y,
        );

        match choose_vertical_reflow_candidate(insert_candidate, remove_candidate) {
            Some(VerticalReflowCandidate::Insert { len, spacer }) => {
                let inserted = if matches!(spacer, VerticalReflowSpacerKind::ContentRows) {
                    original_rows[original_y..original_y + len].to_vec()
                } else {
                    vertical_reflow_insert_rows(
                        &spacer,
                        len,
                        width,
                        reconstructed_y
                            .checked_sub(1)
                            .and_then(|index| reconstructed_rows.get(index)),
                        reconstructed_rows.get(reconstructed_y),
                    )?
                };
                let inserted_hashes = row_hashes(&inserted);
                reconstructed_rows.splice(reconstructed_y..reconstructed_y, inserted);
                reconstructed_hashes.splice(reconstructed_y..reconstructed_y, inserted_hashes);
                if matches!(spacer, VerticalReflowSpacerKind::ContentRows) {
                    non_spacer_rows = non_spacer_rows.saturating_add(len as u32);
                }
                operations.push(VerticalReflowOperation {
                    at_y: reconstructed_y as u32,
                    shift_suffix_by_px: len as i32,
                    direction: "down",
                    row_count: len as u32,
                    spacer: spacer.into_report(),
                });
                inserted_rows = inserted_rows.saturating_add(len as u32);
            }
            Some(VerticalReflowCandidate::Remove { len, spacer }) => {
                reconstructed_rows.drain(reconstructed_y..reconstructed_y + len);
                reconstructed_hashes.drain(reconstructed_y..reconstructed_y + len);
                if matches!(spacer, VerticalReflowSpacerKind::ContentRows) {
                    non_spacer_rows = non_spacer_rows.saturating_add(len as u32);
                }
                operations.push(VerticalReflowOperation {
                    at_y: reconstructed_y as u32,
                    shift_suffix_by_px: -(len as i32),
                    direction: "up",
                    row_count: len as u32,
                    spacer: spacer.into_report(),
                });
                removed_rows = removed_rows.saturating_add(len as u32);
            }
            None => {
                repair_completed = false;
                break;
            }
        }
    }

    if repair_completed && original_y < original_rows.len() {
        let len = original_rows.len() - original_y;
        let spacer = classify_vertical_spacer_rows(
            &original_rows,
            original_y,
            len,
            reconstructed_y
                .checked_sub(1)
                .and_then(|index| reconstructed_rows.get(index)),
            None,
        )
        .unwrap_or(VerticalReflowSpacerKind::ContentRows);
        let inserted = if matches!(spacer, VerticalReflowSpacerKind::ContentRows) {
            original_rows[original_y..].to_vec()
        } else {
            vertical_reflow_insert_rows(
                &spacer,
                len,
                width,
                reconstructed_y
                    .checked_sub(1)
                    .and_then(|index| reconstructed_rows.get(index)),
                None,
            )?
        };
        let inserted_hashes = row_hashes(&inserted);
        reconstructed_rows.splice(reconstructed_y..reconstructed_y, inserted);
        reconstructed_hashes.splice(reconstructed_y..reconstructed_y, inserted_hashes);
        if matches!(spacer, VerticalReflowSpacerKind::ContentRows) {
            non_spacer_rows = non_spacer_rows.saturating_add(len as u32);
        }
        operations.push(VerticalReflowOperation {
            at_y: reconstructed_y as u32,
            shift_suffix_by_px: len as i32,
            direction: "down",
            row_count: len as u32,
            spacer: spacer.into_report(),
        });
        inserted_rows = inserted_rows.saturating_add(len as u32);
    }

    if repair_completed && reconstructed_y < reconstructed_rows.len() {
        let len = reconstructed_rows.len() - reconstructed_y;
        let spacer = classify_vertical_spacer_rows(
            &reconstructed_rows,
            reconstructed_y,
            len,
            reconstructed_y
                .checked_sub(1)
                .and_then(|index| reconstructed_rows.get(index)),
            None,
        )
        .unwrap_or(VerticalReflowSpacerKind::ContentRows);
        reconstructed_rows.drain(reconstructed_y..);
        reconstructed_hashes.drain(reconstructed_y..);
        if matches!(spacer, VerticalReflowSpacerKind::ContentRows) {
            non_spacer_rows = non_spacer_rows.saturating_add(len as u32);
        }
        operations.push(VerticalReflowOperation {
            at_y: reconstructed_y as u32,
            shift_suffix_by_px: -(len as i32),
            direction: "up",
            row_count: len as u32,
            spacer: spacer.into_report(),
        });
        removed_rows = removed_rows.saturating_add(len as u32);
    }

    if operations.is_empty() {
        return None;
    }

    let reconstructed = rows_to_image(&reconstructed_rows, width)?;
    let residual_diff_box = find_diff_box(original, &reconstructed);
    let residual_diff_pixels = residual_diff_box.map_or(0, |diff_box| {
        let (padded_original, padded_reconstructed) = pad_screenshot_pair_to_common_canvas(
            original.clone(),
            reconstructed.clone(),
            None,
            None,
        );
        count_diff_pixels(&padded_original, &padded_reconstructed, diff_box)
    });
    let uses_only_spacer_rows = non_spacer_rows == 0;
    let reconstructs_original = residual_diff_pixels == 0 && repair_completed;
    let shift_only_reconstruction = reconstructs_original && uses_only_spacer_rows;
    let classification = if shift_only_reconstruction {
        "vertical_suffix_reflow"
    } else if repair_completed {
        "vertical_suffix_reflow_repair"
    } else {
        "vertical_suffix_reflow_partial_repair"
    };

    Some(VerticalReflowProofResult {
        proof: VerticalReflowProof {
            classification,
            reconstructs_original,
            original_width: original.width(),
            original_height: original.height(),
            shrunk_width: shrunk.width(),
            shrunk_height: shrunk.height(),
            height_delta_px: original.height() as i32 - shrunk.height() as i32,
            matched_rows,
            inserted_rows,
            removed_rows,
            non_spacer_rows,
            uses_only_spacer_rows,
            repair_completed,
            operations,
            residual_diff_pixels,
            residual_diff_box,
            reconstructed_from_shrunk: None,
            residual_diff_pixels_image: None,
        },
        reconstructed,
    })
}

enum VerticalReflowCandidate {
    Insert {
        len: usize,
        spacer: VerticalReflowSpacerKind,
    },
    Remove {
        len: usize,
        spacer: VerticalReflowSpacerKind,
    },
}

#[derive(Debug, Clone)]
enum VerticalReflowSpacerKind {
    DuplicatePreviousRow,
    DuplicateNextRow,
    UniformRgba(ScreenshotRgba),
    ContentRows,
}

impl VerticalReflowSpacerKind {
    fn into_report(self) -> VerticalReflowSpacer {
        match self {
            VerticalReflowSpacerKind::DuplicatePreviousRow => VerticalReflowSpacer {
                kind: "duplicate_previous_row",
                rgba: None,
            },
            VerticalReflowSpacerKind::DuplicateNextRow => VerticalReflowSpacer {
                kind: "duplicate_next_row",
                rgba: None,
            },
            VerticalReflowSpacerKind::UniformRgba(rgba) => VerticalReflowSpacer {
                kind: "uniform_rgba",
                rgba: Some(rgba),
            },
            VerticalReflowSpacerKind::ContentRows => VerticalReflowSpacer {
                kind: "content_rows",
                rgba: None,
            },
        }
    }
}

fn choose_vertical_reflow_candidate(
    insert: Option<VerticalReflowCandidate>,
    remove: Option<VerticalReflowCandidate>,
) -> Option<VerticalReflowCandidate> {
    match (insert, remove) {
        (Some(insert), Some(remove)) => {
            if vertical_reflow_candidate_len(&insert) <= vertical_reflow_candidate_len(&remove) {
                Some(insert)
            } else {
                Some(remove)
            }
        }
        (Some(insert), None) => Some(insert),
        (None, Some(remove)) => Some(remove),
        (None, None) => None,
    }
}

fn vertical_reflow_candidate_len(candidate: &VerticalReflowCandidate) -> usize {
    match candidate {
        VerticalReflowCandidate::Insert { len, .. }
        | VerticalReflowCandidate::Remove { len, .. } => *len,
    }
}

fn find_vertical_reflow_insert(
    original_rows: &[Vec<u8>],
    original_hashes: &[u64],
    original_y: usize,
    reconstructed_rows: &[Vec<u8>],
    reconstructed_hashes: &[u64],
    reconstructed_y: usize,
) -> Option<VerticalReflowCandidate> {
    let current_reconstructed = reconstructed_rows.get(reconstructed_y)?;
    let current_hash = *reconstructed_hashes.get(reconstructed_y)?;
    let max_len =
        (original_rows.len().saturating_sub(original_y + 1)).min(MAX_VERTICAL_REFLOW_SEARCH_PX);

    for len in 1..=max_len {
        let candidate_y = original_y + len;
        if original_hashes.get(candidate_y).copied() != Some(current_hash)
            || original_rows.get(candidate_y) != Some(current_reconstructed)
        {
            continue;
        }

        let spacer = classify_vertical_spacer_rows(
            original_rows,
            original_y,
            len,
            reconstructed_y
                .checked_sub(1)
                .and_then(|index| reconstructed_rows.get(index)),
            Some(current_reconstructed),
        )
        .unwrap_or(VerticalReflowSpacerKind::ContentRows);
        return Some(VerticalReflowCandidate::Insert { len, spacer });
    }

    None
}

fn find_vertical_reflow_remove(
    original_rows: &[Vec<u8>],
    original_hashes: &[u64],
    original_y: usize,
    reconstructed_rows: &[Vec<u8>],
    reconstructed_hashes: &[u64],
    reconstructed_y: usize,
) -> Option<VerticalReflowCandidate> {
    let current_original = original_rows.get(original_y)?;
    let current_hash = *original_hashes.get(original_y)?;
    let max_len = (reconstructed_rows.len().saturating_sub(reconstructed_y + 1))
        .min(MAX_VERTICAL_REFLOW_SEARCH_PX);

    for len in 1..=max_len {
        let candidate_y = reconstructed_y + len;
        if reconstructed_hashes.get(candidate_y).copied() != Some(current_hash)
            || reconstructed_rows.get(candidate_y) != Some(current_original)
        {
            continue;
        }

        let spacer = classify_vertical_spacer_rows(
            reconstructed_rows,
            reconstructed_y,
            len,
            reconstructed_y
                .checked_sub(1)
                .and_then(|index| reconstructed_rows.get(index)),
            Some(current_original),
        )
        .unwrap_or(VerticalReflowSpacerKind::ContentRows);
        return Some(VerticalReflowCandidate::Remove { len, spacer });
    }

    None
}

fn classify_vertical_spacer_rows(
    rows: &[Vec<u8>],
    start: usize,
    len: usize,
    previous_row: Option<&Vec<u8>>,
    next_row: Option<&Vec<u8>>,
) -> Option<VerticalReflowSpacerKind> {
    if len == 0 || start.checked_add(len)? > rows.len() {
        return None;
    }

    let gap = &rows[start..start + len];
    if let Some(previous_row) = previous_row {
        if gap.iter().all(|row| row == previous_row) {
            return Some(VerticalReflowSpacerKind::DuplicatePreviousRow);
        }
    }
    if let Some(next_row) = next_row {
        if gap.iter().all(|row| row == next_row) {
            return Some(VerticalReflowSpacerKind::DuplicateNextRow);
        }
    }

    let rgba = uniform_row_rgba(&gap[0])?;
    if gap.iter().all(|row| uniform_row_rgba(row) == Some(rgba)) {
        return Some(VerticalReflowSpacerKind::UniformRgba(rgba));
    }

    None
}

fn vertical_reflow_insert_rows(
    spacer: &VerticalReflowSpacerKind,
    len: usize,
    width: u32,
    previous_row: Option<&Vec<u8>>,
    next_row: Option<&Vec<u8>>,
) -> Option<Vec<Vec<u8>>> {
    let row = match spacer {
        VerticalReflowSpacerKind::DuplicatePreviousRow => previous_row?.clone(),
        VerticalReflowSpacerKind::DuplicateNextRow => next_row?.clone(),
        VerticalReflowSpacerKind::UniformRgba(rgba) => {
            uniform_image_row(width, image::Rgba([rgba.r, rgba.g, rgba.b, rgba.a]))
        }
        VerticalReflowSpacerKind::ContentRows => return None,
    };
    Some((0..len).map(|_| row.clone()).collect())
}

fn image_rows(image: &image::RgbaImage) -> Vec<Vec<u8>> {
    let row_len = image.width() as usize * 4;
    image
        .as_raw()
        .chunks(row_len.max(1))
        .map(|row| row.to_vec())
        .collect()
}

fn rows_to_image(rows: &[Vec<u8>], width: u32) -> Option<image::RgbaImage> {
    let row_len = width as usize * 4;
    let mut bytes = Vec::with_capacity(row_len.checked_mul(rows.len())?);
    for row in rows {
        if row.len() != row_len {
            return None;
        }
        bytes.extend_from_slice(row);
    }
    image::RgbaImage::from_raw(width, rows.len() as u32, bytes)
}

fn row_hashes(rows: &[Vec<u8>]) -> Vec<u64> {
    rows.iter().map(|row| row_hash(row)).collect()
}

fn row_hash(row: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    row.hash(&mut hasher);
    hasher.finish()
}

fn uniform_image_row(width: u32, color: image::Rgba<u8>) -> Vec<u8> {
    let mut row = Vec::with_capacity(width as usize * 4);
    for _ in 0..width {
        row.extend_from_slice(&color.0);
    }
    row
}

fn uniform_row_rgba(row: &[u8]) -> Option<ScreenshotRgba> {
    let first = row.get(0..4)?;
    if row.chunks_exact(4).all(|pixel| pixel == first) {
        Some(ScreenshotRgba {
            r: first[0],
            g: first[1],
            b: first[2],
            a: first[3],
        })
    } else {
        None
    }
}

fn count_diff_pixels(left: &image::RgbaImage, right: &image::RgbaImage, diff_box: DiffBox) -> u64 {
    let width = left.width().min(right.width());
    let height = left.height().min(right.height());
    if width == 0 || height == 0 {
        return 0;
    }

    let left_stride = left.width() as usize * 4;
    let right_stride = right.width() as usize * 4;
    let left_raw = left.as_raw();
    let right_raw = right.as_raw();
    let mut count = 0u64;
    let max_x = diff_box.max_x.min(width - 1);
    let max_y = diff_box.max_y.min(height - 1);
    for y in diff_box.min_y.min(max_y)..=max_y {
        for x in diff_box.min_x.min(max_x)..=max_x {
            let left_start = y as usize * left_stride + x as usize * 4;
            let right_start = y as usize * right_stride + x as usize * 4;
            if left_raw[left_start..left_start + 4] != right_raw[right_start..right_start + 4] {
                count += 1;
            }
        }
    }
    count
}

fn find_diff_box(left: &image::RgbaImage, right: &image::RgbaImage) -> Option<DiffBox> {
    if left.width() != right.width() || left.height() != right.height() {
        let width = left.width().max(right.width());
        let height = left.height().max(right.height());
        return Some(DiffBox {
            min_x: 0,
            min_y: 0,
            max_x: width.saturating_sub(1),
            max_y: height.saturating_sub(1),
            image_width: width,
            image_height: height,
        });
    }

    let width = left.width();
    let height = left.height();
    if width == 0 || height == 0 {
        return None;
    }

    let row_len = width as usize * 4;
    let left_raw = left.as_raw();
    let right_raw = right.as_raw();
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;

    for y in 0..height {
        let row_start = y as usize * row_len;
        let left_row = &left_raw[row_start..row_start + row_len];
        let right_row = &right_raw[row_start..row_start + row_len];
        if left_row == right_row {
            continue;
        }

        let mut row_min = 0usize;
        let pixel_count = width as usize;
        while row_min < pixel_count {
            let pixel_start = row_min * 4;
            if left_row[pixel_start..pixel_start + 4] != right_row[pixel_start..pixel_start + 4] {
                break;
            }
            row_min += 1;
        }

        let mut row_max = pixel_count - 1;
        while row_max > row_min {
            let pixel_start = row_max * 4;
            if left_row[pixel_start..pixel_start + 4] != right_row[pixel_start..pixel_start + 4] {
                break;
            }
            row_max -= 1;
        }

        found = true;
        min_x = min_x.min(row_min as u32);
        min_y = min_y.min(y);
        max_x = max_x.max(row_max as u32);
        max_y = max_y.max(y);
    }

    found.then_some(DiffBox {
        min_x,
        min_y,
        max_x,
        max_y,
        image_width: right.width(),
        image_height: right.height(),
    })
}

fn save_diff_crop(
    source: &image::RgbaImage,
    dest_path: &Path,
    diff_box: DiffBox,
    capture_info: &ScreenshotCaptureInfo,
) -> anyhow::Result<()> {
    let rect = row_context_crop_rect(source.width(), source.height(), diff_box, capture_info.rows);
    let crop =
        image::imageops::crop_imm(source, rect.x, rect.y, rect.width, rect.height).to_image();
    crop.save(dest_path)
        .map_err(|error| anyhow::anyhow!("failed to save image '{}': {error}", dest_path.display()))
}

fn save_diff_pixels(
    left: &image::RgbaImage,
    right: &image::RgbaImage,
    dest_path: &Path,
    diff_box: DiffBox,
    capture_info: &ScreenshotCaptureInfo,
) -> anyhow::Result<()> {
    let width = left.width().min(right.width());
    let height = left.height().min(right.height());
    let rect = row_context_crop_rect(width, height, diff_box, capture_info.rows);
    let mut mask = image::RgbaImage::new(rect.width, rect.height);
    let left_stride = left.width() as usize * 4;
    let right_stride = right.width() as usize * 4;
    let mask_stride = rect.width as usize * 4;
    let left_raw = left.as_raw();
    let right_raw = right.as_raw();
    let mask_raw: &mut [u8] = mask.as_mut();

    for y in 0..rect.height {
        let source_y = rect.y + y;
        for x in 0..rect.width {
            let source_x = rect.x + x;
            let mask_start = y as usize * mask_stride + x as usize * 4;
            let color = if source_x < width && source_y < height {
                let left_start = source_y as usize * left_stride + source_x as usize * 4;
                let right_start = source_y as usize * right_stride + source_x as usize * 4;
                if left_raw[left_start..left_start + 4] != right_raw[right_start..right_start + 4] {
                    [255, 0, 0, 255]
                } else {
                    [255, 255, 255, 255]
                }
            } else {
                [255, 255, 255, 255]
            };
            mask_raw[mask_start..mask_start + 4].copy_from_slice(&color);
        }
    }

    mask.save(dest_path)
        .map_err(|error| anyhow::anyhow!("failed to save image '{}': {error}", dest_path.display()))
}

fn row_context_crop_rect(
    source_width: u32,
    source_height: u32,
    diff_box: DiffBox,
    row_count: u32,
) -> CropRect {
    let rows = row_count.max(1);
    let top_row = pixel_to_cell_index(diff_box.min_y, diff_box.image_height, rows, 1);
    let bottom_row = pixel_to_cell_index(diff_box.max_y, diff_box.image_height, rows, 1);
    let context_top_row = top_row.min(bottom_row).saturating_sub(1).max(1);
    let context_bottom_row = top_row.max(bottom_row).saturating_add(1).min(rows);

    let mut y =
        (((context_top_row - 1) as f64 / rows as f64) * source_height as f64).floor() as u32;
    let mut bottom =
        ((context_bottom_row as f64 / rows as f64) * source_height as f64).ceil() as u32;
    bottom = bottom.saturating_sub(1);
    y = y.min(source_height.saturating_sub(1));
    bottom = bottom.max(y).min(source_height.saturating_sub(1));

    CropRect {
        x: 0,
        y,
        width: source_width.max(1),
        height: (bottom - y + 1).max(1),
    }
}

fn visual_cell_range(diff_box: DiffBox, capture_info: &ScreenshotCaptureInfo) -> VisualCellRange {
    let rows = capture_info.rows.max(1);
    let cols = capture_info.columns.max(1);
    let top_row = pixel_to_cell_index(
        diff_box.min_y,
        diff_box.image_height,
        rows,
        capture_info.start_row,
    );
    let bottom_row = pixel_to_cell_index(
        diff_box.max_y,
        diff_box.image_height,
        rows,
        capture_info.start_row,
    );
    let left_col = pixel_to_cell_index(
        diff_box.min_x,
        diff_box.image_width,
        cols,
        capture_info.start_column,
    );
    let right_col = pixel_to_cell_index(
        diff_box.max_x,
        diff_box.image_width,
        cols,
        capture_info.start_column,
    );

    VisualCellRange {
        top_row: top_row.min(bottom_row),
        bottom_row: top_row.max(bottom_row),
        left_column: left_col.min(right_col),
        right_column: left_col.max(right_col),
    }
}

fn visual_diff_cells(
    original_pixels: &image::RgbaImage,
    shrunk_pixels: &image::RgbaImage,
    diff_box: DiffBox,
    capture_info: &ScreenshotCaptureInfo,
) -> VisualDiffCells {
    let range = visual_cell_range(diff_box, capture_info);
    let rows = capture_info.rows.max(1);
    let cols = capture_info.columns.max(1);
    let width = original_pixels.width().min(shrunk_pixels.width());
    let height = original_pixels.height().min(shrunk_pixels.height());
    let mut diff_rows = BTreeSet::new();
    let mut diff_cells = BTreeSet::new();

    if width > 0 && height > 0 {
        let left_stride = original_pixels.width() as usize * 4;
        let right_stride = shrunk_pixels.width() as usize * 4;
        let left_raw = original_pixels.as_raw();
        let right_raw = shrunk_pixels.as_raw();
        let max_x = diff_box.max_x.min(width - 1);
        let max_y = diff_box.max_y.min(height - 1);

        for y in diff_box.min_y.min(max_y)..=max_y {
            let row_index =
                pixel_to_cell_index(y, diff_box.image_height, rows, capture_info.start_row);
            for x in diff_box.min_x.min(max_x)..=max_x {
                let left_start = y as usize * left_stride + x as usize * 4;
                let right_start = y as usize * right_stride + x as usize * 4;
                if left_raw[left_start..left_start + 4] == right_raw[right_start..right_start + 4] {
                    continue;
                }

                let column_index =
                    pixel_to_cell_index(x, diff_box.image_width, cols, capture_info.start_column);
                diff_rows.insert(row_index);
                diff_cells.insert((row_index, column_index));
            }
        }
    }

    if diff_cells.is_empty() {
        diff_rows.insert(range.top_row);
        diff_cells.insert((range.top_row, range.left_column));
    }

    VisualDiffCells {
        range,
        rows: diff_rows,
        cells: diff_cells,
    }
}

fn approximate_cells(range: VisualCellRange) -> ApproximateCells {
    ApproximateCells {
        top_left: format!("{}{}", column_name(range.left_column), range.top_row),
        bottom_right: format!("{}{}", column_name(range.right_column), range.bottom_row),
    }
}

fn pixel_to_cell_index(pixel: u32, image_size: u32, count: u32, start_index: u32) -> u32 {
    if image_size == 0 || count <= 1 {
        return start_index;
    }
    let relative = ((pixel as f64 / image_size as f64) * count as f64).floor() as u32;
    start_index + relative.min(count - 1)
}

fn column_name(mut index: u32) -> String {
    let mut name = String::new();
    while index > 0 {
        index -= 1;
        name.insert(0, (b'A' + (index % 26) as u8) as char);
        index /= 26;
    }
    name
}

fn sanitize_sheet_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '_' | '.' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn sanitize_limited_path_segment(name: &str, max_chars: usize) -> String {
    let safe = sanitize_sheet_name(name);
    let safe = if safe.is_empty() {
        "_".to_string()
    } else {
        safe
    };
    if safe.chars().count() <= max_chars {
        return safe;
    }

    let suffix_hash = bytes_content_hash(name.as_bytes());
    let suffix = &suffix_hash[..8];
    let prefix_chars = max_chars.saturating_sub(suffix.len() + 1).max(1);
    let prefix: String = safe.chars().take(prefix_chars).collect();
    format!("{prefix}-{suffix}")
}

fn write_png_screenshot_metadata(
    path: &Path,
    metadata: &ScreenshotPngMetadata<'_>,
) -> anyhow::Result<()> {
    let bytes = fs::read(path)?;
    let metadata_json = serde_json::to_string(metadata)?;
    let metadata_chunk = png_itxt_chunk(SCREENSHOT_PNG_METADATA_KEYWORD, &metadata_json)?;
    let updated = png_with_replaced_itxt_chunk(
        &bytes,
        SCREENSHOT_PNG_METADATA_KEYWORD,
        &metadata_chunk,
        path,
    )?;
    if updated.as_slice() != bytes.as_slice() {
        fs::write(path, updated)?;
    }
    Ok(())
}

fn png_itxt_chunk(keyword: &str, text: &str) -> anyhow::Result<Vec<u8>> {
    if keyword.is_empty()
        || keyword.len() > 79
        || !keyword.is_ascii()
        || keyword.as_bytes().contains(&0)
    {
        anyhow::bail!("invalid PNG iTXt keyword '{keyword}'");
    }

    let mut data = Vec::with_capacity(keyword.len() + text.len() + 5);
    data.extend_from_slice(keyword.as_bytes());
    data.push(0);
    data.push(0);
    data.push(0);
    data.push(0);
    data.push(0);
    data.extend_from_slice(text.as_bytes());
    Ok(png_chunk(*b"iTXt", &data))
}

fn png_chunk(chunk_type: [u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(data.len() + 12);
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(&chunk_type);
    chunk.extend_from_slice(data);

    let mut crc = crc32fast::Hasher::new();
    crc.update(&chunk_type);
    crc.update(data);
    chunk.extend_from_slice(&crc.finalize().to_be_bytes());
    chunk
}

fn png_with_replaced_itxt_chunk(
    bytes: &[u8],
    keyword: &str,
    metadata_chunk: &[u8],
    path: &Path,
) -> anyhow::Result<Vec<u8>> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(PNG_SIGNATURE) {
        anyhow::bail!("screenshot '{}' is not a PNG file", path.display());
    }

    let mut updated = Vec::with_capacity(bytes.len() + metadata_chunk.len());
    updated.extend_from_slice(PNG_SIGNATURE);
    let mut offset = PNG_SIGNATURE.len();

    while offset < bytes.len() {
        if offset + 12 > bytes.len() {
            anyhow::bail!("PNG '{}' ended inside a chunk header", path.display());
        }

        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let chunk_type = &bytes[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start.checked_add(length).ok_or_else(|| {
            anyhow::anyhow!("PNG '{}' has an overflowing chunk length", path.display())
        })?;
        let chunk_end = data_end.checked_add(4).ok_or_else(|| {
            anyhow::anyhow!("PNG '{}' has an overflowing chunk length", path.display())
        })?;
        if chunk_end > bytes.len() {
            anyhow::bail!("PNG '{}' ended inside a chunk payload", path.display());
        }

        if chunk_type == b"iTXt"
            && png_text_chunk_keyword(&bytes[data_start..data_end]) == Some(keyword)
        {
            offset = chunk_end;
            continue;
        }

        if chunk_type == b"IEND" {
            updated.extend_from_slice(metadata_chunk);
            updated.extend_from_slice(&bytes[offset..chunk_end]);
            updated.extend_from_slice(&bytes[chunk_end..]);
            return Ok(updated);
        }

        updated.extend_from_slice(&bytes[offset..chunk_end]);
        offset = chunk_end;
    }

    anyhow::bail!("PNG '{}' has no IEND chunk", path.display())
}

fn png_text_chunk_keyword(data: &[u8]) -> Option<&str> {
    let end = data.iter().position(|byte| *byte == 0)?;
    std::str::from_utf8(&data[..end]).ok()
}

fn create_sheet_xml_git_diff(
    context: &SheetXmlDiffContext<'_>,
    output_dir: &Path,
    sheet_index: usize,
    sheet_name: &str,
    tile_index: Option<usize>,
    visual_diff_cells: Option<&VisualDiffCells>,
) -> anyhow::Result<Option<SheetXmlDiffArtifacts>> {
    let Some(sheet_file) = sheet_file_for_index(
        context.original_workbook_path,
        context.shrunk_workbook_path,
        sheet_index,
    )?
    else {
        return Ok(None);
    };

    let safe_name = sanitize_sheet_name(sheet_name);
    let original_sheet_xml_name = format!("{sheet_index:03}-{safe_name}-sheet-xml-original.xml");
    let shrunk_sheet_xml_name = format!("{sheet_index:03}-{safe_name}-sheet-xml-shrunk.xml");
    let sheet_xml_diff_name = format!("{sheet_index:03}-{safe_name}-sheet-xml.diff");
    let original_sheet_xml = output_dir.join(format!("{original_sheet_xml_name}.zip"));
    let shrunk_sheet_xml = output_dir.join(format!("{shrunk_sheet_xml_name}.zip"));
    let sheet_xml_diff = output_dir.join(format!("{sheet_xml_diff_name}.zip"));
    let original_sheet_xml_temp = output_dir.join(format!("{original_sheet_xml_name}.tmp"));
    let shrunk_sheet_xml_temp = output_dir.join(format!("{shrunk_sheet_xml_name}.tmp"));
    let sheet_xml_visual_stem = match tile_index {
        Some(tile_index) => {
            format!("{sheet_index:03}-{safe_name}-tile-{tile_index:03}-sheet-xml-visual")
        }
        None => format!("{sheet_index:03}-{safe_name}-sheet-xml-visual"),
    };
    let sheet_xml_visual_original_name = format!("{sheet_xml_visual_stem}-original.xml");
    let sheet_xml_visual_shrunk_name = format!("{sheet_xml_visual_stem}-shrunk.xml");
    let sheet_xml_visual_diff_name = format!("{sheet_xml_visual_stem}.diff");
    let sheet_xml_visual_original =
        output_dir.join(format!("{sheet_xml_visual_original_name}.zip"));
    let sheet_xml_visual_shrunk = output_dir.join(format!("{sheet_xml_visual_shrunk_name}.zip"));
    let sheet_xml_visual_diff = output_dir.join(format!("{sheet_xml_visual_diff_name}.zip"));
    let sheet_xml_visual_original_temp =
        output_dir.join(format!("{sheet_xml_visual_original_name}.tmp"));
    let sheet_xml_visual_shrunk_temp =
        output_dir.join(format!("{sheet_xml_visual_shrunk_name}.tmp"));

    let original_xml = read_zip_entry(context.original_workbook_path, &sheet_file)?;
    let shrunk_xml = read_zip_entry(context.shrunk_workbook_path, &sheet_file)?;
    let original_formatted_xml = format_xml_for_diff(&original_xml);
    let shrunk_formatted_xml = format_xml_for_diff(&shrunk_xml);
    fs::write(&original_sheet_xml_temp, original_formatted_xml.as_bytes())?;
    fs::write(&shrunk_sheet_xml_temp, shrunk_formatted_xml.as_bytes())?;
    write_compressed_artifact(
        &original_sheet_xml,
        &original_sheet_xml_name,
        original_formatted_xml.as_bytes(),
    )?;
    write_compressed_artifact(
        &shrunk_sheet_xml,
        &shrunk_sheet_xml_name,
        shrunk_formatted_xml.as_bytes(),
    )?;

    let original_styles_xml =
        read_zip_entry_optional(context.original_workbook_path, "xl/styles.xml")?;
    let shrunk_styles_xml = read_zip_entry_optional(context.shrunk_workbook_path, "xl/styles.xml")?;
    let original_shared_strings_xml =
        read_zip_entry_optional(context.original_workbook_path, "xl/sharedStrings.xml")?;
    let shrunk_shared_strings_xml =
        read_zip_entry_optional(context.shrunk_workbook_path, "xl/sharedStrings.xml")?;
    let original_context = build_sheet_xml_diff_source_context(
        &original_formatted_xml,
        original_styles_xml.as_deref(),
        original_shared_strings_xml.as_deref(),
    );
    let shrunk_context = build_sheet_xml_diff_source_context(
        &shrunk_formatted_xml,
        shrunk_styles_xml.as_deref(),
        shrunk_shared_strings_xml.as_deref(),
    );

    let annotated_diff = create_annotated_git_no_index_diff(
        &original_sheet_xml_temp,
        &shrunk_sheet_xml_temp,
        &original_context,
        &shrunk_context,
    )?;
    write_compressed_artifact(
        &sheet_xml_diff,
        &sheet_xml_diff_name,
        annotated_diff.as_bytes(),
    )?;
    let sheet_xml_visual_diff = if let Some(visual_diff_cells) = visual_diff_cells {
        write_visual_sheet_xml_git_diff(
            &original_formatted_xml,
            &shrunk_formatted_xml,
            &original_context,
            &shrunk_context,
            visual_diff_cells,
            &sheet_xml_visual_original,
            &sheet_xml_visual_shrunk,
            &sheet_xml_visual_diff,
            &sheet_xml_visual_original_name,
            &sheet_xml_visual_shrunk_name,
            &sheet_xml_visual_diff_name,
            &sheet_xml_visual_original_temp,
            &sheet_xml_visual_shrunk_temp,
        )?;
        Some(sheet_xml_visual_diff)
    } else {
        None
    };
    remove_temp_file(&original_sheet_xml_temp);
    remove_temp_file(&shrunk_sheet_xml_temp);
    remove_temp_file(&sheet_xml_visual_original_temp);
    remove_temp_file(&sheet_xml_visual_shrunk_temp);

    Ok(Some(SheetXmlDiffArtifacts {
        sheet_file,
        original_sheet_xml,
        shrunk_sheet_xml,
        sheet_xml_diff,
        sheet_xml_visual_diff,
    }))
}

fn write_visual_sheet_xml_git_diff(
    original_formatted_xml: &str,
    shrunk_formatted_xml: &str,
    original_context: &SheetXmlDiffSourceContext,
    shrunk_context: &SheetXmlDiffSourceContext,
    visual_diff_cells: &VisualDiffCells,
    original_visual_path: &Path,
    shrunk_visual_path: &Path,
    diff_path: &Path,
    original_visual_entry_name: &str,
    shrunk_visual_entry_name: &str,
    diff_entry_name: &str,
    original_visual_temp_path: &Path,
    shrunk_visual_temp_path: &Path,
) -> anyhow::Result<()> {
    let (original_visual_xml, original_visual_context) =
        visual_sheet_xml_source(original_formatted_xml, original_context, visual_diff_cells);
    let (shrunk_visual_xml, shrunk_visual_context) =
        visual_sheet_xml_source(shrunk_formatted_xml, shrunk_context, visual_diff_cells);

    fs::write(original_visual_temp_path, original_visual_xml.as_bytes())?;
    fs::write(shrunk_visual_temp_path, shrunk_visual_xml.as_bytes())?;
    write_compressed_artifact(
        original_visual_path,
        original_visual_entry_name,
        original_visual_xml.as_bytes(),
    )?;
    write_compressed_artifact(
        shrunk_visual_path,
        shrunk_visual_entry_name,
        shrunk_visual_xml.as_bytes(),
    )?;

    let mut annotated_diff = create_annotated_git_no_index_diff(
        original_visual_temp_path,
        shrunk_visual_temp_path,
        &original_visual_context,
        &shrunk_visual_context,
    )?;
    let range_header = format!(
        "# excel-shrink visual cell range: {}:{}\n",
        visual_diff_cells.range.top_left_reference(),
        visual_diff_cells.range.bottom_right_reference()
    );
    if annotated_diff.is_empty() {
        annotated_diff.push_str(
            "# excel-shrink no sheet XML row/cell differences inside visual cell range\n",
        );
    }
    annotated_diff.insert_str(0, &range_header);
    write_compressed_artifact(diff_path, diff_entry_name, annotated_diff.as_bytes())?;
    Ok(())
}

fn visual_sheet_xml_source(
    formatted_xml: &str,
    context: &SheetXmlDiffSourceContext,
    visual_diff_cells: &VisualDiffCells,
) -> (String, SheetXmlDiffSourceContext) {
    let mut visual_xml = String::new();
    let mut visual_context = SheetXmlDiffSourceContext {
        rows_by_index: context.rows_by_index.clone(),
        columns: context.columns.clone(),
        style_xfs: context.style_xfs.clone(),
        shared_strings: context.shared_strings.clone(),
        ..SheetXmlDiffSourceContext::default()
    };

    for (line, line_info) in formatted_xml.lines().zip(&context.line_info) {
        if sheet_xml_line_is_in_visual_cells(line_info, visual_diff_cells) {
            visual_xml.push_str(line);
            visual_xml.push('\n');
            visual_context.line_info.push(line_info.clone());
        }
    }

    (visual_xml, visual_context)
}

fn sheet_xml_line_is_in_visual_cells(
    line_info: &SheetXmlDiffLineInfo,
    visual_diff_cells: &VisualDiffCells,
) -> bool {
    if let Some(cell) = &line_info.cell {
        return visual_diff_cells.contains_cell(cell.row_index, cell.column_index);
    }
    line_info
        .row
        .as_ref()
        .is_some_and(|row| visual_diff_cells.contains_row(row.row_index))
}

fn sheet_file_for_index(
    original_workbook_path: &Path,
    shrunk_workbook_path: &Path,
    sheet_index: usize,
) -> anyhow::Result<Option<String>> {
    if sheet_index == 0 {
        return Ok(None);
    }

    let index = sheet_index - 1;
    let original_order = workbook_sheet_order(original_workbook_path)?;
    if let Some(sheet_file) = original_order.get(index) {
        return Ok(Some(sheet_file.clone()));
    }

    let shrunk_order = workbook_sheet_order(shrunk_workbook_path)?;
    if let Some(sheet_file) = shrunk_order.get(index) {
        return Ok(Some(sheet_file.clone()));
    }

    Ok(Some(format!("xl/worksheets/sheet{sheet_index}.xml")))
}

fn read_zip_entry(workbook_path: &Path, entry_name: &str) -> anyhow::Result<Vec<u8>> {
    let file = File::open(workbook_path)?;
    let mut archive = ZipArchive::new(file)?;
    let mut entry = archive.by_name(entry_name).map_err(|error| {
        anyhow::anyhow!(
            "failed to read '{}' from '{}': {error}",
            entry_name,
            workbook_path.display()
        )
    })?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_zip_entry_optional(
    workbook_path: &Path,
    entry_name: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    let file = File::open(workbook_path)?;
    let mut archive = ZipArchive::new(file)?;
    let Ok(mut entry) = archive.by_name(entry_name) else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn format_xml_for_diff(xml: &[u8]) -> String {
    let text = String::from_utf8_lossy(xml);
    let mut output = String::new();
    let mut token = String::new();
    let mut in_tag = false;
    let mut indent = 0usize;

    for ch in text.chars() {
        if ch == '<' {
            write_xml_diff_text_token(&mut output, &mut token, indent);
            token.push(ch);
            in_tag = true;
        } else if ch == '>' && in_tag {
            token.push(ch);
            write_xml_diff_tag_token(&mut output, &mut token, &mut indent);
            in_tag = false;
        } else {
            token.push(ch);
        }
    }

    if in_tag {
        write_xml_diff_text_token(&mut output, &mut token, indent);
    } else {
        write_xml_diff_text_token(&mut output, &mut token, indent);
    }

    output
}

fn write_xml_diff_text_token(output: &mut String, token: &mut String, indent: usize) {
    let text = token.trim();
    if !text.is_empty() {
        output.push_str(&"  ".repeat(indent));
        output.push_str(text);
        output.push('\n');
    }
    token.clear();
}

fn write_xml_diff_tag_token(output: &mut String, token: &mut String, indent: &mut usize) {
    let tag = token.trim();
    if tag.is_empty() {
        token.clear();
        return;
    }

    if tag.starts_with("</") {
        *indent = indent.saturating_sub(1);
    }

    output.push_str(&"  ".repeat(*indent));
    output.push_str(tag);
    output.push('\n');

    if xml_diff_tag_increases_indent(tag) {
        *indent += 1;
    }
    token.clear();
}

fn xml_diff_tag_increases_indent(tag: &str) -> bool {
    tag.starts_with('<')
        && !tag.starts_with("</")
        && !tag.starts_with("<?")
        && !tag.starts_with("<!")
        && !tag.ends_with("/>")
}

fn build_sheet_xml_diff_source_context(
    formatted_xml: &str,
    styles_xml: Option<&[u8]>,
    shared_strings_xml: Option<&[u8]>,
) -> SheetXmlDiffSourceContext {
    let mut context = SheetXmlDiffSourceContext {
        style_xfs: extract_style_xfs(styles_xml),
        shared_strings: extract_shared_string_nodes(shared_strings_xml),
        ..SheetXmlDiffSourceContext::default()
    };
    let mut current_row: Option<SheetRowXmlInfo> = None;
    let mut current_cell: Option<SheetCellXmlInfo> = None;
    let mut inside_value = false;

    for line in formatted_xml.lines() {
        let trimmed = line.trim();
        let mut line_info = SheetXmlDiffLineInfo::default();

        if is_xml_start_tag(trimmed, "col") {
            if let Some(column) = sheet_column_info_from_tag(trimmed) {
                context.columns.push(column);
            }
        }

        if is_xml_start_tag(trimmed, "row") {
            if let Some(row) = sheet_row_info_from_tag(trimmed) {
                context.rows_by_index.insert(row.row_index, row.clone());
                line_info.row = Some(row.clone());
                current_row = Some(row);
            }
            if xml_tag_is_self_closing(trimmed) {
                current_row = None;
            }
        } else if is_xml_end_tag(trimmed, "row") {
            line_info.row = current_row.clone();
            current_row = None;
        } else if let Some(row) = current_row.clone() {
            line_info.row = Some(row);
        }

        if is_xml_start_tag(trimmed, "c") {
            if let Some(cell) = sheet_cell_info_from_tag(trimmed, current_row.as_ref()) {
                line_info.cell = Some(cell.clone());
                current_cell = Some(cell);
            }
            if xml_tag_is_self_closing(trimmed) {
                current_cell = None;
            }
        } else if is_xml_end_tag(trimmed, "c") {
            line_info.cell = current_cell.clone();
            current_cell = None;
        } else if let Some(cell) = current_cell.clone() {
            line_info.cell = Some(cell);
        }

        if inside_value
            && !trimmed.starts_with('<')
            && current_cell
                .as_ref()
                .and_then(|cell| cell.cell_type.as_deref())
                == Some("s")
        {
            line_info.shared_string_index = trimmed.parse::<usize>().ok();
        }

        if is_xml_start_tag(trimmed, "v") && !xml_tag_is_self_closing(trimmed) {
            inside_value = true;
        }
        if is_xml_end_tag(trimmed, "v") {
            inside_value = false;
        }

        context.line_info.push(line_info);
    }

    context
}

fn sheet_row_info_from_tag(tag: &str) -> Option<SheetRowXmlInfo> {
    let row_index = xml_attr_value(tag, "r")?.parse::<u32>().ok()?;
    Some(SheetRowXmlInfo {
        row_index,
        style_id: xml_attr_usize(tag, "s"),
        node: compact_xml_snippet(tag),
    })
}

fn sheet_cell_info_from_tag(
    tag: &str,
    current_row: Option<&SheetRowXmlInfo>,
) -> Option<SheetCellXmlInfo> {
    let reference = xml_attr_value(tag, "r")?;
    let (column_name, row_index, column_index) = parse_cell_reference(&reference)?;
    Some(SheetCellXmlInfo {
        reference,
        row_index: row_index.or_else(|| current_row.map(|row| row.row_index))?,
        column_name,
        column_index,
        style_id: xml_attr_usize(tag, "s"),
        cell_type: xml_attr_value(tag, "t"),
        node: compact_xml_snippet(tag),
    })
}

fn sheet_column_info_from_tag(tag: &str) -> Option<SheetColumnXmlInfo> {
    let min = xml_attr_value(tag, "min")?.parse::<u32>().ok()?;
    let max = xml_attr_value(tag, "max")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(min);
    Some(SheetColumnXmlInfo {
        min,
        max: max.max(min),
        style_id: xml_attr_usize(tag, "style"),
        node: compact_xml_snippet(tag),
    })
}

fn parse_cell_reference(reference: &str) -> Option<(String, Option<u32>, u32)> {
    let column_name: String = reference
        .chars()
        .take_while(|ch| ch.is_ascii_alphabetic())
        .collect();
    if column_name.is_empty() {
        return None;
    }
    let row_text = &reference[column_name.len()..];
    let row_index = if row_text.is_empty() {
        None
    } else {
        row_text.parse::<u32>().ok()
    };
    let column_index = column_index_from_name(&column_name)?;
    Some((column_name, row_index, column_index))
}

fn column_index_from_name(column_name: &str) -> Option<u32> {
    let mut index = 0u32;
    for ch in column_name.chars() {
        if !ch.is_ascii_alphabetic() {
            return None;
        }
        index = index
            .checked_mul(26)?
            .checked_add((ch.to_ascii_uppercase() as u32) - ('A' as u32) + 1)?;
    }
    (index > 0).then_some(index)
}

fn extract_style_xfs(styles_xml: Option<&[u8]>) -> Vec<String> {
    let Some(styles_xml) = styles_xml else {
        return Vec::new();
    };
    let styles_xml = String::from_utf8_lossy(styles_xml);
    let cell_xfs = find_xml_element_block(&styles_xml, "cellXfs").unwrap_or(&styles_xml);
    extract_xml_nodes(cell_xfs, "xf")
        .into_iter()
        .map(|node| compact_xml_snippet(&node))
        .collect()
}

fn extract_shared_string_nodes(shared_strings_xml: Option<&[u8]>) -> Vec<String> {
    let Some(shared_strings_xml) = shared_strings_xml else {
        return Vec::new();
    };
    let shared_strings_xml = String::from_utf8_lossy(shared_strings_xml);
    extract_xml_nodes(&shared_strings_xml, "si")
        .into_iter()
        .map(|node| compact_xml_snippet(&node))
        .collect()
}

fn find_xml_element_block<'a>(xml: &'a str, element_name: &str) -> Option<&'a str> {
    let start = find_xml_start_tag(xml, element_name, 0)?;
    let open_end = find_xml_tag_end(xml, start)?;
    if xml_tag_is_self_closing(&xml[start..=open_end]) {
        return Some(&xml[start..=open_end]);
    }
    let close_tag = format!("</{element_name}>");
    let close_start = xml[open_end + 1..].find(&close_tag)? + open_end + 1;
    Some(&xml[start..close_start + close_tag.len()])
}

fn extract_xml_nodes(xml: &str, element_name: &str) -> Vec<String> {
    let close_tag = format!("</{element_name}>");
    let mut nodes = Vec::new();
    let mut search_from = 0usize;

    while let Some(start) = find_xml_start_tag(xml, element_name, search_from) {
        let Some(open_end) = find_xml_tag_end(xml, start) else {
            break;
        };
        let end = if xml_tag_is_self_closing(&xml[start..=open_end]) {
            open_end + 1
        } else if let Some(close_start) = xml[open_end + 1..].find(&close_tag) {
            open_end + 1 + close_start + close_tag.len()
        } else {
            break;
        };
        nodes.push(xml[start..end].to_string());
        search_from = end;
    }

    nodes
}

fn find_xml_start_tag(xml: &str, element_name: &str, from: usize) -> Option<usize> {
    let needle = format!("<{element_name}");
    let mut search_from = from;
    while let Some(relative) = xml[search_from..].find(&needle) {
        let start = search_from + relative;
        let after = start + needle.len();
        let boundary = xml[after..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_whitespace() || matches!(ch, '>' | '/'));
        if boundary {
            return Some(start);
        }
        search_from = after;
    }
    None
}

fn find_xml_tag_end(xml: &str, start: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (offset, ch) in xml[start..].char_indices() {
        match (quote, ch) {
            (Some(current), quote_ch) if current == quote_ch => quote = None,
            (None, '"' | '\'') => quote = Some(ch),
            (None, '>') => return Some(start + offset),
            _ => {}
        }
    }
    None
}

fn is_xml_start_tag(line: &str, element_name: &str) -> bool {
    find_xml_start_tag(line, element_name, 0) == Some(0)
}

fn is_xml_end_tag(line: &str, element_name: &str) -> bool {
    line.starts_with(&format!("</{element_name}>"))
}

fn xml_tag_is_self_closing(tag: &str) -> bool {
    tag.trim_end().ends_with("/>")
}

fn xml_attr_usize(tag: &str, attr_name: &str) -> Option<usize> {
    xml_attr_value(tag, attr_name)?.parse::<usize>().ok()
}

fn xml_attr_value(tag: &str, attr_name: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let attr = attr_name.as_bytes();
    let mut index = 0usize;

    while index + attr.len() <= bytes.len() {
        if &bytes[index..index + attr.len()] != attr {
            index += 1;
            continue;
        }
        let before_ok =
            index == 0 || bytes[index - 1].is_ascii_whitespace() || bytes[index - 1] == b'<';
        let mut cursor = index + attr.len();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if !before_ok || cursor >= bytes.len() || bytes[cursor] != b'=' {
            index += 1;
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || !matches!(bytes[cursor], b'"' | b'\'') {
            return None;
        }
        let quote = bytes[cursor];
        cursor += 1;
        let value_start = cursor;
        while cursor < bytes.len() && bytes[cursor] != quote {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return None;
        }
        return Some(tag[value_start..cursor].to_string());
    }

    None
}

fn compact_xml_snippet(snippet: &str) -> String {
    let mut compact = String::new();
    let mut previous_was_space = false;
    for ch in snippet.trim().chars() {
        if ch.is_whitespace() {
            if !previous_was_space {
                compact.push(' ');
            }
            previous_was_space = true;
        } else {
            compact.push(ch);
            previous_was_space = false;
        }
    }
    compact
}

fn write_compressed_artifact(
    zip_path: &Path,
    entry_name: &str,
    contents: &[u8],
) -> anyhow::Result<()> {
    if let Some(parent) = zip_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(zip_path)?;
    let mut writer = ZipWriter::new(file);
    let options = FileOptions::default().compression_method(CompressionMethod::Deflated);
    writer.start_file(entry_name, options)?;
    writer.write_all(contents)?;
    writer.finish()?;
    Ok(())
}

fn remove_temp_file(path: &Path) {
    fs::remove_file(path).ok();
}

fn create_annotated_git_no_index_diff(
    original_path: &Path,
    shrunk_path: &Path,
    original_context: &SheetXmlDiffSourceContext,
    shrunk_context: &SheetXmlDiffSourceContext,
) -> anyhow::Result<String> {
    let output = Command::new("git")
        .arg("diff")
        .arg("--no-index")
        .arg("--no-ext-diff")
        .arg("--")
        .arg(original_path)
        .arg(shrunk_path)
        .output()
        .map_err(|error| anyhow::anyhow!("failed to run git diff --no-index: {error}"))?;

    if !(output.status.success() || output.status.code() == Some(1)) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff --no-index failed: {stderr}");
    }

    let diff = String::from_utf8_lossy(&output.stdout);
    let annotated_diff = annotate_sheet_xml_diff(&diff, original_context, shrunk_context);
    Ok(annotated_diff)
}

fn annotate_sheet_xml_diff(
    diff: &str,
    original_context: &SheetXmlDiffSourceContext,
    shrunk_context: &SheetXmlDiffSourceContext,
) -> String {
    let mut annotated = String::new();
    let mut original_line = 0usize;
    let mut shrunk_line = 0usize;

    for line in diff.lines() {
        annotated.push_str(line);
        annotated.push('\n');

        if let Some((old_start, new_start)) = parse_unified_diff_hunk_header(line) {
            original_line = old_start;
            shrunk_line = new_start;
            continue;
        }

        if line.starts_with("---") || line.starts_with("+++") {
            continue;
        }

        if line.starts_with('-') {
            append_sheet_xml_diff_annotations(
                &mut annotated,
                "original",
                original_line,
                original_context,
            );
            original_line += 1;
        } else if line.starts_with('+') {
            append_sheet_xml_diff_annotations(
                &mut annotated,
                "shrunk",
                shrunk_line,
                shrunk_context,
            );
            shrunk_line += 1;
        } else if line.starts_with(' ') {
            original_line += 1;
            shrunk_line += 1;
        }
    }

    annotated
}

fn parse_unified_diff_hunk_header(line: &str) -> Option<(usize, usize)> {
    if !line.starts_with("@@ ") {
        return None;
    }
    let old_marker = line.find('-')?;
    let new_marker = line.find('+')?;
    Some((
        parse_unified_diff_hunk_start(&line[old_marker + 1..])?,
        parse_unified_diff_hunk_start(&line[new_marker + 1..])?,
    ))
}

fn parse_unified_diff_hunk_start(text: &str) -> Option<usize> {
    let digits: String = text.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse::<usize>().ok()
}

fn append_sheet_xml_diff_annotations(
    annotated: &mut String,
    side: &str,
    line_number: usize,
    context: &SheetXmlDiffSourceContext,
) {
    if line_number == 0 {
        return;
    }
    let Some(line_info) = context.line_info.get(line_number - 1) else {
        return;
    };

    if let Some(cell) = &line_info.cell {
        append_cell_diff_annotations(annotated, side, line_number, context, line_info, cell);
    } else if let Some(row) = &line_info.row {
        append_row_diff_annotations(annotated, side, line_number, context, row);
    }
}

fn append_cell_diff_annotations(
    annotated: &mut String,
    side: &str,
    line_number: usize,
    context: &SheetXmlDiffSourceContext,
    line_info: &SheetXmlDiffLineInfo,
    cell: &SheetCellXmlInfo,
) {
    let row = context
        .rows_by_index
        .get(&cell.row_index)
        .or(line_info.row.as_ref());
    let column = context.column_for_index(cell.column_index);
    let effective_style = cell
        .style_id
        .map(|style_id| ("cell", style_id))
        .or_else(|| row.and_then(|row| row.style_id.map(|style_id| ("row", style_id))))
        .or_else(|| column.and_then(|column| column.style_id.map(|style_id| ("column", style_id))))
        .unwrap_or(("default", 0));

    append_annotation_line(
        annotated,
        format!(
            "side={side} line={line_number} cell={} row={} column={} cell_s={} row_s={} column_s={} effective_s={} effective_source={} cell_node={}",
            cell.reference,
            cell.row_index,
            cell.column_name,
            style_id_text(cell.style_id, true),
            style_id_text(row.and_then(|row| row.style_id), false),
            style_id_text(column.and_then(|column| column.style_id), false),
            effective_style.1,
            effective_style.0,
            cell.node,
        ),
    );
    append_style_annotation(
        annotated,
        side,
        line_number,
        "effective",
        Some(effective_style.1),
        context,
    );
    append_style_annotation(annotated, side, line_number, "cell", cell.style_id, context);

    if let Some(row) = row {
        append_annotation_line(
            annotated,
            format!(
                "side={side} line={line_number} row_node={} row_s={}",
                row.node,
                style_id_text(row.style_id, false)
            ),
        );
        append_style_annotation(annotated, side, line_number, "row", row.style_id, context);
    }

    if let Some(column) = column {
        append_annotation_line(
            annotated,
            format!(
                "side={side} line={line_number} column_node={} column_s={}",
                column.node,
                style_id_text(column.style_id, false)
            ),
        );
        append_style_annotation(
            annotated,
            side,
            line_number,
            "column",
            column.style_id,
            context,
        );
    }

    if let Some(shared_string_index) = line_info.shared_string_index {
        let node = context
            .shared_strings
            .get(shared_string_index)
            .map(String::as_str)
            .unwrap_or("<missing shared string node>");
        append_annotation_line(
            annotated,
            format!("side={side} line={line_number} shared_string[{shared_string_index}]={node}"),
        );
    }
}

fn append_row_diff_annotations(
    annotated: &mut String,
    side: &str,
    line_number: usize,
    context: &SheetXmlDiffSourceContext,
    row: &SheetRowXmlInfo,
) {
    let effective_style = row.style_id.unwrap_or(0);
    append_annotation_line(
        annotated,
        format!(
            "side={side} line={line_number} row={} row_s={} effective_s={} row_node={}",
            row.row_index,
            style_id_text(row.style_id, false),
            effective_style,
            row.node
        ),
    );
    append_style_annotation(
        annotated,
        side,
        line_number,
        if row.style_id.is_some() {
            "row"
        } else {
            "default"
        },
        Some(effective_style),
        context,
    );
}

fn append_style_annotation(
    annotated: &mut String,
    side: &str,
    line_number: usize,
    label: &str,
    style_id: Option<usize>,
    context: &SheetXmlDiffSourceContext,
) {
    let Some(style_id) = style_id else {
        return;
    };
    let node = context
        .style_xfs
        .get(style_id)
        .map(String::as_str)
        .unwrap_or("<missing style xf node>");
    append_annotation_line(
        annotated,
        format!("side={side} line={line_number} {label}_style_xf[{style_id}]={node}"),
    );
}

fn append_annotation_line(annotated: &mut String, text: String) {
    annotated.push_str("# excel-shrink-context ");
    annotated.push_str(&text);
    annotated.push('\n');
}

fn style_id_text(style_id: Option<usize>, default_zero: bool) -> String {
    match (style_id, default_zero) {
        (Some(style_id), _) => style_id.to_string(),
        (None, true) => "0 (implicit)".to_string(),
        (None, false) => "<none>".to_string(),
    }
}

impl VisualCellRange {
    fn contains_row(&self, row_index: u32) -> bool {
        self.top_row <= row_index && row_index <= self.bottom_row
    }

    fn top_left_reference(&self) -> String {
        format!("{}{}", column_name(self.left_column), self.top_row)
    }

    fn bottom_right_reference(&self) -> String {
        format!("{}{}", column_name(self.right_column), self.bottom_row)
    }
}

impl VisualDiffCells {
    fn contains_row(&self, row_index: u32) -> bool {
        if self.rows.is_empty() {
            self.range.contains_row(row_index)
        } else {
            self.rows.contains(&row_index)
        }
    }

    fn contains_cell(&self, row_index: u32, column_index: u32) -> bool {
        if self.cells.is_empty() {
            self.range.contains_row(row_index)
                && self.range.left_column <= column_index
                && column_index <= self.range.right_column
        } else {
            self.cells.contains(&(row_index, column_index))
        }
    }
}

impl SheetXmlDiffSourceContext {
    fn column_for_index(&self, column_index: u32) -> Option<&SheetColumnXmlInfo> {
        self.columns
            .iter()
            .find(|column| column.min <= column_index && column_index <= column.max)
    }
}

fn normalize_automation_path(path: &PathBuf) -> PathBuf {
    #[cfg(not(windows))]
    {
        if let Some(converted) = windows_path_to_wsl_mnt_path(&path.to_string_lossy()) {
            return PathBuf::from(converted);
        }
    }
    path.clone()
}

#[cfg(not(windows))]
fn windows_path_to_wsl_mnt_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || bytes[2] != b'\\' {
        return None;
    }
    let drive = bytes[0] as char;
    if !drive.is_ascii_alphabetic() {
        return None;
    }
    Some(format!(
        "/mnt/{}/{}",
        drive.to_ascii_lowercase(),
        path[3..].replace('\\', "/")
    ))
}

fn link_workbook_with_suffix(
    input_path: &Path,
    work_dir: &Path,
    suffix: &str,
) -> anyhow::Result<PathBuf> {
    let destination = workbook_path_with_suffix(input_path, work_dir, suffix)?;
    let destination_file_name = destination.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "workbook destination has no file name: {}",
            destination.display()
        )
    })?;

    link_workbook_artifact_to_shared_cache(input_path, work_dir, work_dir, destination_file_name)
}

fn copy_workbook_with_suffix(
    input_path: &Path,
    work_dir: &Path,
    suffix: &str,
) -> anyhow::Result<PathBuf> {
    let copied_path = workbook_path_with_suffix(input_path, work_dir, suffix)?;
    fs::remove_file(&copied_path).ok();
    fs::copy(input_path, &copied_path).map_err(|e| {
        anyhow::anyhow!(
            "failed to copy workbook '{}' to '{}': {e}",
            input_path.display(),
            copied_path.display()
        )
    })?;
    Ok(copied_path)
}

fn workbook_path_with_suffix(
    input_path: &Path,
    work_dir: &Path,
    suffix: &str,
) -> anyhow::Result<PathBuf> {
    let stem = input_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "workbook path has no UTF-8 file stem: {}",
                input_path.display()
            )
        })?;
    let extension = input_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("xlsx");
    Ok(work_dir.join(format!("{stem}.{suffix}.{extension}")))
}

#[derive(Debug)]
enum PowerShellScriptOutcome {
    Completed(ExitStatus),
    TimedOut { stalled_for: Duration },
}

#[derive(Debug)]
struct ScreenshotCaptureRun {
    outcome: PowerShellScriptOutcome,
    original_excel_path: PathBuf,
    shrunk_excel_path: PathBuf,
}

fn ensure_screenshot_capture_completed(
    status: &ExitStatus,
    capture_manifest_path: &Path,
    progress_path: &Path,
    original_excel_path: &Path,
    shrunk_excel_path: &Path,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        capture_manifest_path.exists(),
        "Excel screenshot capture did not create manifest: {}\n{}",
        capture_manifest_path.display(),
        screenshot_capture_failure_details(progress_path, original_excel_path, shrunk_excel_path)
    );

    anyhow::ensure!(
        status.success(),
        "Excel screenshot capture failed with status {status}; manifest: {}\n{}",
        capture_manifest_path.display(),
        screenshot_capture_failure_details(progress_path, original_excel_path, shrunk_excel_path)
    );

    Ok(())
}

fn screenshot_capture_failure_details(
    progress_path: &Path,
    original_excel_path: &Path,
    shrunk_excel_path: &Path,
) -> String {
    let progress = fs::read_to_string(progress_path).unwrap_or_default();
    let phase = infer_screenshot_capture_phase(&progress);
    let progress_tail = progress_tail(&progress, 40);
    format!(
        "likely_failed_phase={phase}\noriginal_excel={}\nshrunk_excel={}\nprogress_log={}\nprogress_tail:\n{}",
        original_excel_path.display(),
        shrunk_excel_path.display(),
        progress_path.display(),
        if progress_tail.is_empty() {
            "<empty>".to_string()
        } else {
            progress_tail
        }
    )
}

fn infer_screenshot_capture_phase(progress: &str) -> &'static str {
    if progress.contains("opened shrunk workbook") {
        "after opening shrunk workbook"
    } else if progress.contains("label='shrunk'") || progress.contains("opened original workbook") {
        "opening shrunk workbook"
    } else if progress.contains("label='original'") || progress.contains("excel pid=") {
        "opening original workbook"
    } else {
        "before opening workbooks"
    }
}

fn progress_tail(progress: &str, max_lines: usize) -> String {
    let lines: Vec<_> = progress.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

fn screenshot_capture_needs_original_workbook_repair(
    status: &ExitStatus,
    progress_path: &Path,
    capture_manifest_path: &Path,
) -> bool {
    if status.success() && capture_manifest_path.exists() {
        return false;
    }

    let progress = fs::read_to_string(progress_path).unwrap_or_default();
    !progress.contains("opened original workbook")
}

fn preserve_stalled_progress_log(output_dir: &Path, progress_path: &Path) -> anyhow::Result<()> {
    if !progress_path.exists() {
        return Ok(());
    }

    let preserved = output_dir.join("screenshot-progress.before-workbook-xml-repair.log");
    fs::copy(progress_path, &preserved).map_err(|e| {
        anyhow::anyhow!(
            "failed to preserve stalled screenshot progress log '{}' to '{}': {e}",
            progress_path.display(),
            preserved.display()
        )
    })?;
    Ok(())
}

fn repair_original_workbook_xml_only(
    original_path: &Path,
    work_dir: &Path,
) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(work_dir)?;
    let repair_input =
        copy_workbook_with_suffix(original_path, work_dir, WORKBOOK_XML_REPAIRED_SUFFIX)?;
    let repair_output_dir = work_dir.join(WORKBOOK_XML_REPAIRED_SUFFIX);
    fs::create_dir_all(&repair_output_dir)?;

    let repaired_path = repair_output_dir.join(repair_input.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "repair input path has no file name: {}",
            repair_input.display()
        )
    })?);
    let _ = fs::remove_file(&repaired_path);

    let cli = Cli {
        trace_file: None,
        path: repair_input.clone(),
        output: repair_output_dir,
        recursive: false,
        disable_multiprocessing: true,
        chunk_size: 16_777_216,
        parallel_range_chunk_size: crate::DEFAULT_PARALLEL_RANGE_CHUNK_SIZE,
        force_overwrite: true,
        sheets: Vec::new(),
        only_clean_workbook: true,
        disable_sheetdata_splitting: false,
        analysis_mode: false,
        analysis_csv: None,
        working_directory: None,
        disable_tail_pruning: false,
        disable_trailing_layout_only_rows: false,
        disable_extraneous_column_merge: false,
        disable_column_tail_fill_merge: false,
        remove_textless_default_font_representatives: false,
        remove_textless_style_representatives: false,
        remove_textless_column_style_representatives: false,
        preprocess_empty_cells: false,
        writer_workers: Some(1),
        max_active_workbooks: Some(1),
        compression_level: 2,
        store_output: false,
        remove_unreferenced_whitespace_cells: false,
        keep_unreferenced_hidden_content_cells: false,
        inactive_protection_cell_policy: crate::InactiveProtectionCellPolicyArg::Remove,
        console_log_level: LogLevel::Info,
        file_log_level: None,
    };

    process_files(vec![repair_input.clone()], &cli)?;
    fs::remove_file(&repair_input).ok();
    anyhow::ensure!(
        repaired_path.exists(),
        "workbook.xml-only repair did not create expected output: {}",
        repaired_path.display()
    );
    replace_workbook_with_shared_cache_link(&repaired_path, work_dir)?;

    Ok(repaired_path)
}

fn run_power_shell_script(script_path: &Path, args: &[(&str, String)]) -> anyhow::Result<()> {
    let status = run_power_shell_script_with_status(script_path, args)?;
    anyhow::ensure!(
        status.success(),
        "Excel automation script failed: {}",
        script_path.display()
    );
    Ok(())
}

fn run_power_shell_script_with_status(
    script_path: &Path,
    args: &[(&str, String)],
) -> anyhow::Result<std::process::ExitStatus> {
    let mut command = power_shell_script_command(script_path, args)?;
    command
        .status()
        .map_err(|e| anyhow::anyhow!("failed to start PowerShell automation: {e}"))
}

fn run_power_shell_script_with_status_and_stall_timeout(
    script_path: &Path,
    args: &[(&str, String)],
    progress_path: &Path,
    stall_timeout: Duration,
) -> anyhow::Result<PowerShellScriptOutcome> {
    let mut child = power_shell_script_command(script_path, args)?
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to start PowerShell automation: {e}"))?;
    let mut last_progress_at = Instant::now();
    let mut last_progress_modified: Option<SystemTime> = None;

    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| anyhow::anyhow!("failed to poll PowerShell automation: {e}"))?
        {
            return Ok(PowerShellScriptOutcome::Completed(status));
        }

        if let Some(modified) = file_modified_at(progress_path) {
            if last_progress_modified.is_none_or(|previous| modified > previous) {
                last_progress_modified = Some(modified);
                last_progress_at = Instant::now();
            }
        }

        let stalled_for = last_progress_at.elapsed();
        if stalled_for >= stall_timeout {
            terminate_child_process(&mut child);
            terminate_excel_processes_from_progress_log(progress_path);
            return Ok(PowerShellScriptOutcome::TimedOut { stalled_for });
        }

        thread::sleep(EXCEL_SCREENSHOT_POLL_INTERVAL);
    }
}

fn power_shell_script_command(
    script_path: &Path,
    args: &[(&str, String)],
) -> anyhow::Result<Command> {
    let Some(shell) = find_power_shell() else {
        anyhow::bail!(
            "PowerShell is not available. Install Excel-accessible PowerShell or run on Windows/WSL with powershell.exe."
        );
    };

    let mut command = Command::new(shell);
    command.arg("-NoProfile");
    let program = command.get_program().to_string_lossy().to_ascii_lowercase();
    if program.contains("powershell") {
        command.arg("-STA");
        command.arg("-ExecutionPolicy").arg("Bypass");
    }
    let script_arg = if cfg!(not(windows)) && program.ends_with(".exe") {
        power_shell_path(script_path)?
    } else {
        script_path.to_string_lossy().to_string()
    };
    command.arg("-File").arg(script_arg);
    for (name, value) in args {
        command.arg(format!("-{name}")).arg(value);
    }

    Ok(command)
}

fn file_modified_at(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn terminate_child_process(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn terminate_excel_processes_from_progress_log(progress_path: &Path) {
    for pid in excel_pids_from_progress_log(progress_path) {
        eprintln!("[excel-screenshot] terminating stalled Excel automation process pid={pid}");
        terminate_process_by_pid(pid);
    }
}

fn excel_pids_from_progress_log(progress_path: &Path) -> BTreeSet<u32> {
    let Ok(text) = fs::read_to_string(progress_path) else {
        return BTreeSet::new();
    };

    let mut pids = BTreeSet::new();
    for marker in ["excel pid=", "excel_pid="] {
        let mut rest = text.as_str();
        while let Some(index) = rest.find(marker) {
            let after_marker = &rest[index + marker.len()..];
            let digits: String = after_marker
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect();
            if let Ok(pid) = digits.parse::<u32>() {
                pids.insert(pid);
            }
            rest = &after_marker[digits.len()..];
        }
    }
    pids
}

fn terminate_process_by_pid(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .arg("/PID")
            .arg(pid.to_string())
            .arg("/F")
            .arg("/T")
            .status();
    }

    #[cfg(not(windows))]
    {
        let _ = Command::new("powershell.exe")
            .arg("-NoProfile")
            .arg("-Command")
            .arg(format!(
                "Stop-Process -Id {pid} -Force -ErrorAction SilentlyContinue"
            ))
            .status();
    }
}

fn find_power_shell() -> Option<&'static str> {
    for candidate in ["powershell.exe", "pwsh", "powershell"] {
        if Command::new(candidate)
            .arg("-NoProfile")
            .arg("-Command")
            .arg("$PSVersionTable.PSVersion.ToString()")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Some(candidate);
        }
    }
    None
}

fn power_shell_path(path: &Path) -> anyhow::Result<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    #[cfg(windows)]
    {
        return Ok(absolute.to_string_lossy().to_string());
    }

    #[cfg(not(windows))]
    {
        let text = absolute.to_string_lossy();
        if let Some(converted) = wsl_mnt_path_to_windows_path(&text) {
            return Ok(converted);
        }

        if let Ok(output) = Command::new("wslpath").arg("-w").arg(&absolute).output() {
            if output.status.success() {
                let converted = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !converted.is_empty() {
                    return Ok(converted);
                }
            }
        }

        Ok(text.to_string())
    }
}

#[cfg(not(windows))]
fn wsl_mnt_path_to_windows_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/mnt/")?;
    let (drive, tail) = rest.split_once('/')?;
    if drive.len() != 1 || !drive.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }

    Some(format!(
        "{}:\\{}",
        drive.to_ascii_uppercase(),
        tail.replace('/', "\\")
    ))
}

const RECALCULATE_AND_SAVE_SCRIPT: &str = include_str!("excel_recalculate.ps1");

const SCREENSHOT_COMPARE_SCRIPT: &str = include_str!("excel_screenshots.ps1");

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;

    #[cfg(not(windows))]
    #[test]
    fn converts_wsl_mnt_paths_for_windows_excel() {
        assert_eq!(
            wsl_mnt_path_to_windows_path("/mnt/c/Users/A/file.xlsx"),
            Some("C:\\Users\\A\\file.xlsx".to_string())
        );
    }

    #[test]
    fn compares_power_shell_manifest_with_utf8_bom() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-screenshot-compare-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let original_path = test_dir.join("original.png");
        let shrunk_path = test_dir.join("shrunk.png");
        let original = image::RgbaImage::from_pixel(3, 3, image::Rgba([255, 255, 255, 255]));
        let mut shrunk = original.clone();
        shrunk.put_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        original.save(&original_path)?;
        shrunk.save(&shrunk_path)?;

        let capture_manifest_path = test_dir.join("screenshot-captures.json");
        let report_path = test_dir.join("screenshot-differences.json");
        let capture_manifest = serde_json::json!([
            {
                "sheet_index": 1,
                "sheet_name": "Sheet1",
                "capture_range": "A1:C3",
                "capture_strategy": "test",
                "used_range_error": null,
                "original_image": original_path,
                "shrunk_image": shrunk_path,
                "capture_info": {
                    "strategy": "test",
                    "address": "A1:C3",
                    "start_row": 1,
                    "start_column": 1,
                    "rows": 3,
                    "columns": 3,
                    "used_range_error": null
                }
            }
        ]);
        let mut manifest_bytes = b"\xEF\xBB\xBF".to_vec();
        manifest_bytes.extend(serde_json::to_vec_pretty(&capture_manifest)?);
        fs::write(&capture_manifest_path, manifest_bytes)?;

        assert!(
            compare_captured_screenshots(&capture_manifest_path, &report_path, None)?
                .total_differences
                > 0
        );
        assert!(report_path.exists());
        assert!(original_path.exists());
        assert!(shrunk_path.exists());

        let report = fs::read_to_string(&report_path)?;
        assert!(report.contains("diff_pixels"));
        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn workbook_artifact_links_use_shared_hash_store() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-shared-workbooks-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        let case_dir = test_dir
            .join("regression")
            .join("tmp")
            .join("run")
            .join("case");
        fs::create_dir_all(&case_dir)?;

        let source = test_dir.join("source.xlsx");
        fs::write(&source, b"same workbook bytes")?;
        let hash = file_content_hash(&source)?;
        let expected_shared = test_dir
            .join(".tmp")
            .join("workbooks")
            .join("shared")
            .join(&hash)
            .join("workbook.xlsx");

        let linked_a = link_workbook_artifact_to_shared_cache(
            &source,
            &case_dir,
            &case_dir,
            std::ffi::OsStr::new("visible-a.xlsx"),
        )?;
        let linked_b = link_workbook_artifact_to_shared_cache(
            &source,
            &case_dir,
            &case_dir.join("nested"),
            std::ffi::OsStr::new("visible-b.xlsx"),
        )?;

        assert_eq!(fs::read(&linked_a)?, b"same workbook bytes");
        assert_eq!(fs::read(&linked_b)?, b"same workbook bytes");
        assert!(expected_shared.exists());
        assert_eq!(file_content_hash(&expected_shared)?, hash);

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn replace_workbook_with_shared_cache_link_keeps_visible_path() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-shared-workbook-replace-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        let case_dir = test_dir
            .join("regression")
            .join("tmp")
            .join("run")
            .join("case");
        fs::create_dir_all(&case_dir)?;

        let workbook = case_dir.join("output.xlsx");
        fs::write(&workbook, b"cleaned workbook bytes")?;
        let hash = file_content_hash(&workbook)?;

        let linked = replace_workbook_with_shared_cache_link(&workbook, &case_dir)?;

        assert_eq!(linked, workbook);
        assert_eq!(fs::read(&workbook)?, b"cleaned workbook bytes");
        assert!(test_dir
            .join(".tmp")
            .join("workbooks")
            .join("shared")
            .join(hash)
            .join("workbook.xlsx")
            .exists());

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn screenshot_difference_reports_vertical_suffix_reflow_proof() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-screenshot-vertical-reflow-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let original_path = test_dir.join("original.png");
        let shrunk_path = test_dir.join("shrunk.png");
        let original = test_image_from_row_colors(&[
            image::Rgba([200, 0, 0, 255]),
            image::Rgba([0, 160, 0, 255]),
            image::Rgba([255, 255, 255, 255]),
            image::Rgba([255, 255, 255, 255]),
            image::Rgba([0, 0, 200, 255]),
        ]);
        let shrunk = test_image_from_row_colors(&[
            image::Rgba([200, 0, 0, 255]),
            image::Rgba([0, 160, 0, 255]),
            image::Rgba([255, 255, 255, 255]),
            image::Rgba([0, 0, 200, 255]),
        ]);
        original.save(&original_path)?;
        shrunk.save(&shrunk_path)?;

        let capture_manifest_path = test_dir.join("screenshot-captures.json");
        let report_path = test_dir.join("screenshot-differences.json");
        let capture_manifest = serde_json::json!([
            {
                "sheet_index": 1,
                "sheet_name": "Sheet1",
                "capture_range": "A1:A5",
                "capture_strategy": "test",
                "used_range_error": null,
                "original_image": original_path,
                "shrunk_image": shrunk_path,
                "capture_info": {
                    "strategy": "test",
                    "address": "A1:A5",
                    "start_row": 1,
                    "start_column": 1,
                    "rows": 5,
                    "columns": 1,
                    "used_range_error": null
                }
            }
        ]);
        fs::write(
            &capture_manifest_path,
            serde_json::to_vec_pretty(&capture_manifest)?,
        )?;

        let summary = compare_captured_screenshots(&capture_manifest_path, &report_path, None)?;
        assert_eq!(summary.total_differences, 1);
        assert_eq!(summary.shift_only_differences, 1);
        assert_eq!(summary.entropy_changed_differences, 0);
        let report: serde_json::Value = serde_json::from_slice(&fs::read(&report_path)?)?;
        let proof = &report[0]["vertical_reflow_proof"];
        assert_eq!(proof["classification"], "vertical_suffix_reflow");
        assert_eq!(proof["reconstructs_original"], true);
        assert_eq!(proof["height_delta_px"], 1);
        assert_eq!(proof["residual_diff_pixels"], 0);
        assert_eq!(proof["operations"][0]["at_y"], 3);
        assert_eq!(proof["operations"][0]["shift_suffix_by_px"], 1);
        assert_eq!(proof["operations"][0]["direction"], "down");
        let reconstructed = proof["reconstructed_from_shrunk"]
            .as_str()
            .expect("proof should include reconstructed image path");
        assert!(Path::new(reconstructed).exists());

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn vertical_suffix_reflow_proof_handles_suffix_shift_up() {
        let original = test_image_from_row_colors(&[
            image::Rgba([200, 0, 0, 255]),
            image::Rgba([255, 255, 255, 255]),
            image::Rgba([0, 0, 200, 255]),
        ]);
        let shrunk = test_image_from_row_colors(&[
            image::Rgba([200, 0, 0, 255]),
            image::Rgba([255, 255, 255, 255]),
            image::Rgba([255, 255, 255, 255]),
            image::Rgba([0, 0, 200, 255]),
        ]);

        let result = prove_vertical_suffix_reflow(&original, &shrunk)
            .expect("extra spacer row should be provable as vertical reflow");
        assert_eq!(result.proof.residual_diff_pixels, 0);
        assert_eq!(result.proof.operations.len(), 1);
        assert_eq!(result.proof.operations[0].at_y, 2);
        assert_eq!(result.proof.operations[0].shift_suffix_by_px, -1);
        assert_eq!(result.proof.operations[0].direction, "up");
        assert_eq!(result.proof.removed_rows, 1);
        assert_eq!(result.reconstructed, original);
    }

    #[test]
    fn vertical_suffix_reflow_proof_rejects_content_changes() {
        let original = test_image_from_row_colors(&[
            image::Rgba([200, 0, 0, 255]),
            image::Rgba([0, 160, 0, 255]),
            image::Rgba([0, 0, 200, 255]),
        ]);
        let shrunk = test_image_from_row_colors(&[
            image::Rgba([200, 0, 0, 255]),
            image::Rgba([0, 160, 0, 255]),
            image::Rgba([200, 0, 200, 255]),
        ]);

        assert!(prove_vertical_suffix_reflow(&original, &shrunk).is_none());
    }

    #[test]
    fn screenshot_difference_reports_vertical_reflow_repair_residual_pixels() -> anyhow::Result<()>
    {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-screenshot-vertical-reflow-repair-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let red = vec![image::Rgba([200, 0, 0, 255]); 4];
        let content_gap = vec![
            image::Rgba([0, 160, 0, 255]),
            image::Rgba([0, 120, 220, 255]),
            image::Rgba([0, 160, 0, 255]),
            image::Rgba([0, 120, 220, 255]),
        ];
        let blue = vec![image::Rgba([0, 0, 200, 255]); 4];
        let yellow = vec![image::Rgba([220, 180, 0, 255]); 4];
        let purple = vec![image::Rgba([180, 0, 180, 255]); 4];
        let original =
            test_image_from_pixel_rows(&[red.clone(), content_gap.clone(), blue.clone(), yellow]);
        let shrunk = test_image_from_pixel_rows(&[red, blue, purple]);

        let original_path = test_dir.join("original.png");
        let shrunk_path = test_dir.join("shrunk.png");
        original.save(&original_path)?;
        shrunk.save(&shrunk_path)?;

        let capture_manifest_path = test_dir.join("screenshot-captures.json");
        let report_path = test_dir.join("screenshot-differences.json");
        let capture_manifest = serde_json::json!([
            {
                "sheet_index": 1,
                "sheet_name": "Sheet1",
                "capture_range": "A1:A4",
                "capture_strategy": "test",
                "used_range_error": null,
                "original_image": original_path,
                "shrunk_image": shrunk_path,
                "capture_info": {
                    "strategy": "test",
                    "address": "A1:A4",
                    "start_row": 1,
                    "start_column": 1,
                    "rows": 4,
                    "columns": 1,
                    "used_range_error": null
                }
            }
        ]);
        fs::write(
            &capture_manifest_path,
            serde_json::to_vec_pretty(&capture_manifest)?,
        )?;

        let summary = compare_captured_screenshots(&capture_manifest_path, &report_path, None)?;
        assert_eq!(summary.total_differences, 1);
        assert_eq!(summary.shift_only_differences, 0);
        assert_eq!(summary.entropy_changed_differences, 1);

        let report: serde_json::Value = serde_json::from_slice(&fs::read(&report_path)?)?;
        let proof = &report[0]["vertical_reflow_proof"];
        assert_eq!(
            proof["classification"],
            "vertical_suffix_reflow_partial_repair"
        );
        assert_eq!(proof["reconstructs_original"], false);
        assert_eq!(proof["non_spacer_rows"], 1);
        assert_eq!(proof["operations"][0]["spacer"]["kind"], "content_rows");
        assert!(proof["residual_diff_pixels"].as_u64().unwrap_or(0) > 0);
        let residual = proof["residual_diff_pixels_image"]
            .as_str()
            .expect("repair should include residual diff pixels image");
        assert!(Path::new(residual).exists());

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn workbook_xml_repair_marker_round_trips() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-workbook-xml-repair-marker-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        assert!(!original_workbook_xml_repair_is_marked(
            &test_dir,
            "883cf282c4066b1d"
        ));
        mark_original_workbook_xml_repair_needed(&test_dir, "883cf282c4066b1d")?;
        assert!(original_workbook_xml_repair_is_marked(
            &test_dir,
            "883cf282c4066b1d"
        ));
        assert!(!original_workbook_xml_repair_is_marked(
            &test_dir,
            "cab52f9ca5f59e7b"
        ));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn screenshot_cache_reuses_capture_by_sheet_xml_hash() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-sheet-screenshot-cache-hit-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let workbook = test_dir.join("current.xlsx");
        write_minimal_workbook(
            &workbook,
            br#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
        )?;
        let cache_root = test_dir.join("cache");
        fs::create_dir_all(&cache_root)?;
        let cache = WorkbookScreenshotCache::new(&workbook, &cache_root, "range-hash")?;
        let image_path = test_dir.join("cached.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])).save(&image_path)?;

        write_test_screenshot_cache_manifest(&cache_root, &cache, &image_path, "old-workbook")?;

        let valid = cache.valid_current_sheet_captures()?;
        assert_eq!(valid.len(), 1);
        assert!(cache.is_complete()?);
        assert_eq!(valid[&1].image.as_deref(), Some(image_path.as_path()));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn screenshot_cache_reuses_workbook_manifest_capture_at_different_sheet_index(
    ) -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-sheet-screenshot-cache-index-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let workbook = test_dir.join("current.xlsx");
        write_minimal_workbook(
            &workbook,
            br#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
        )?;
        let cache_root = test_dir.join("cache");
        fs::create_dir_all(&cache_root)?;
        let cache = WorkbookScreenshotCache::new(&workbook, &cache_root, "range-hash")?;
        let identity = cache.sheet_identities.get(&1).unwrap();
        let image_path = test_dir.join("cached.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])).save(&image_path)?;

        let old_workbook_hash = "old-workbook";
        let cache_dir = workbook_screenshot_cache_root(&cache_root).join(old_workbook_hash);
        fs::create_dir_all(&cache_dir)?;
        let manifest = WorkbookScreenshotCacheManifest {
            version: SCREENSHOT_CACHE_VERSION,
            workbook_hash: old_workbook_hash.to_string(),
            capture_range_hash: cache.capture_range_hash.clone(),
            captures: vec![WorkbookScreenshotCapture {
                sheet_index: 2,
                sheet_name: "OldSheet2".to_string(),
                sheet_file: Some("xl/worksheets/sheet2.xml".to_string()),
                sheet_xml_hash: Some(identity.sheet_xml_hash.clone()),
                capture_range: Some("A1:A1".to_string()),
                capture_strategy: Some("test".to_string()),
                used_range_error: None,
                screenshot_error: None,
                image: Some(image_path.clone()),
                image_hash: Some(file_content_hash(&image_path)?),
                background_rgba: None,
                capture_info: Some(ScreenshotCaptureInfo {
                    strategy: "test".to_string(),
                    address: "A1:A1".to_string(),
                    start_row: 1,
                    start_column: 1,
                    rows: 1,
                    columns: 1,
                    used_range_error: None,
                }),
                tiles: Vec::new(),
            }],
        };
        fs::write(
            cache_dir.join("workbook-screenshots.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;

        let valid = cache.valid_current_sheet_captures()?;
        assert_eq!(valid.len(), 1);
        assert_eq!(valid[&1].sheet_index, 1);
        assert_eq!(
            valid[&1].sheet_file.as_deref(),
            Some(identity.sheet_file.as_str())
        );
        assert_eq!(valid[&1].image.as_deref(), Some(image_path.as_path()));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn screenshot_cache_reuses_top_level_sheet_xml_manifest() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-top-level-sheet-screenshot-cache-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let workbook = test_dir.join("current.xlsx");
        write_minimal_workbook(
            &workbook,
            br#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
        )?;
        let cache_root = test_dir.join("cache");
        fs::create_dir_all(&cache_root)?;
        let cache = WorkbookScreenshotCache::new(&workbook, &cache_root, "range-hash")?;
        let identity = cache.sheet_identities.get(&1).unwrap();
        let image_path = shared_sheet_screenshot_cache_dir(&cache, 1, identity).join("cached.png");
        fs::create_dir_all(image_path.parent().unwrap())?;
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])).save(&image_path)?;

        let manifest = SheetXmlScreenshotCacheManifest {
            version: SCREENSHOT_CACHE_VERSION,
            sheet_xml_hash: identity.sheet_xml_hash.clone(),
            sheet_capture_range_hash: cache.sheet_capture_range_hash(1).to_string(),
            capture: WorkbookScreenshotCapture {
                sheet_index: 7,
                sheet_name: "CachedSheet".to_string(),
                sheet_file: Some("xl/worksheets/sheet7.xml".to_string()),
                sheet_xml_hash: Some(identity.sheet_xml_hash.clone()),
                capture_range: Some("A1:A1".to_string()),
                capture_strategy: Some("test".to_string()),
                used_range_error: None,
                screenshot_error: None,
                image: Some(image_path.clone()),
                image_hash: Some(file_content_hash(&image_path)?),
                background_rgba: None,
                capture_info: Some(ScreenshotCaptureInfo {
                    strategy: "test".to_string(),
                    address: "A1:A1".to_string(),
                    start_row: 1,
                    start_column: 1,
                    rows: 1,
                    columns: 1,
                    used_range_error: None,
                }),
                tiles: Vec::new(),
            },
        };
        fs::write(
            shared_sheet_screenshot_manifest_path(&cache, 1, identity),
            serde_json::to_vec_pretty(&manifest)?,
        )?;

        assert!(!cache.manifest_path.exists());
        let valid = cache.valid_current_sheet_captures()?;
        assert_eq!(valid.len(), 1);
        assert!(cache.is_complete()?);
        assert_eq!(valid[&1].sheet_index, 1);
        assert_eq!(
            valid[&1].sheet_file.as_deref(),
            Some(identity.sheet_file.as_str())
        );
        assert_eq!(valid[&1].image.as_deref(), Some(image_path.as_path()));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn shared_screenshot_file_name_includes_sheet_identity() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-sheet-screenshot-name-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let workbook = test_dir.join("current.xlsx");
        write_minimal_workbook(
            &workbook,
            br#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
        )?;
        let cache_root = test_dir.join("cache");
        let cache = WorkbookScreenshotCache::new(&workbook, &cache_root, "range-hash")?;
        let identity = cache.sheet_identities.get(&1).unwrap();

        let path = shared_screenshot_image_path(&cache, identity, 1, "Revenue Sheet", false, None);
        let file_name = path.file_name().and_then(OsStr::to_str).unwrap();
        assert!(file_name.contains(&identity.sheet_xml_hash));
        assert!(file_name.contains("sheet1.xml"));
        assert!(file_name.contains("Revenue_Sheet"));
        assert!(file_name.ends_with("-shrunk.png"));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn materialized_shared_screenshot_keeps_source_and_writes_png_metadata() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-sheet-screenshot-metadata-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let workbook = test_dir.join("source-workbook.xlsx");
        write_minimal_workbook(
            &workbook,
            br#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
        )?;
        let cache_root = test_dir.join("cache");
        let cache = WorkbookScreenshotCache::new(&workbook, &cache_root, "range-hash")?;
        let identity = cache.sheet_identities.get(&1).unwrap();

        let source = cache.cache_dir.join("001.png");
        fs::create_dir_all(source.parent().unwrap())?;
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])).save(&source)?;
        let source_hash_before = file_content_hash(&source)?;

        let materialized = materialize_shared_screenshot_image(
            &cache, identity, 1, "Sheet 1", true, None, &source,
        )?;

        assert!(source.exists());
        assert_eq!(file_content_hash(&source)?, source_hash_before);
        assert!(materialized.path.exists());
        assert_ne!(materialized.path, source);
        assert_eq!(
            file_content_hash(&materialized.path)?,
            materialized.image_hash
        );

        let png = fs::read(&materialized.path)?;
        let png_text = String::from_utf8_lossy(&png);
        assert!(png_text.contains(SCREENSHOT_PNG_METADATA_KEYWORD));
        assert!(png_text.contains("\"workbook_file\":\"source-workbook.xlsx\""));
        assert!(png_text.contains("\"sheet_file\":\"xl/worksheets/sheet1.xml\""));
        assert!(png_text.contains("\"sheet_name\":\"Sheet 1\""));
        assert!(png_text.contains("\"side\":\"original\""));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn screenshot_cache_rejects_changed_cached_image() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-sheet-screenshot-cache-stale-image-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let workbook = test_dir.join("current.xlsx");
        write_minimal_workbook(
            &workbook,
            br#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
        )?;
        let cache_root = test_dir.join("cache");
        fs::create_dir_all(&cache_root)?;
        let cache = WorkbookScreenshotCache::new(&workbook, &cache_root, "range-hash")?;
        let image_path = test_dir.join("cached.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])).save(&image_path)?;

        write_test_screenshot_cache_manifest(&cache_root, &cache, &image_path, "old-workbook")?;
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255])).save(&image_path)?;

        assert!(cache.valid_current_sheet_captures()?.is_empty());
        assert!(!cache.is_complete()?);

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn screenshot_cache_paths_are_shared_across_visual_profiles() {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-sheet-screenshot-profile-path-{}",
            std::process::id()
        ));
        let work_dir = test_dir
            .join("repo")
            .join("regression")
            .join("tmp")
            .join("case")
            .join("excel-automation");
        let workbook_hash = "same-xml-hash";

        let default = default_visual_profile();
        let same_default = default_visual_profile();
        let maximum = maximum_cleaning_visual_profile();
        let keep_all = keep_all_inactive_protection_visual_profile();
        let custom = isolated_column_style_visual_profile();

        let default_path =
            workbook_screenshot_cache_root(&screenshot_cache_root(&work_dir, &default))
                .join(workbook_hash);
        assert_eq!(
            default_path,
            workbook_screenshot_cache_root(&screenshot_cache_root(&work_dir, &same_default))
                .join(workbook_hash)
        );
        assert_eq!(
            default_path,
            workbook_screenshot_cache_root(&screenshot_cache_root(&work_dir, &maximum))
                .join(workbook_hash)
        );
        assert_eq!(
            default_path,
            workbook_screenshot_cache_root(&screenshot_cache_root(&work_dir, &keep_all))
                .join(workbook_hash)
        );
        assert_eq!(
            default_path,
            workbook_screenshot_cache_root(&screenshot_cache_root(&work_dir, &custom))
                .join(workbook_hash)
        );
        assert!(screenshot_cache_root(&work_dir, &custom)
            .to_string_lossy()
            .replace('\\', "/")
            .contains("/.tmp/screenshots/shared/cache"));
    }

    #[test]
    fn screenshot_cache_accepts_manifest_from_different_visual_profile() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-sheet-screenshot-cache-profile-mismatch-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let workbook = test_dir.join("current.xlsx");
        write_minimal_workbook(
            &workbook,
            br#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#,
        )?;
        let cache_root = test_dir.join("cache");
        fs::create_dir_all(&cache_root)?;
        let cache = WorkbookScreenshotCache::new(&workbook, &cache_root, "range-hash")?;
        let other_profile_cache =
            WorkbookScreenshotCache::new(&workbook, &cache_root, "range-hash")?;
        let image_path = test_dir.join("cached.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])).save(&image_path)?;

        write_test_screenshot_cache_manifest(
            &cache_root,
            &other_profile_cache,
            &image_path,
            "old-workbook",
        )?;

        assert_eq!(cache.valid_current_sheet_captures()?.len(), 1);
        assert!(cache.is_complete()?);

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn screenshot_image_load_error_is_written_as_difference() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-screenshot-load-error-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let original_path = test_dir.join("original.png");
        let shrunk_path = test_dir.join("shrunk.png");
        fs::write(&original_path, b"not a png")?;
        image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255])).save(&shrunk_path)?;

        let capture_manifest_path = test_dir.join("screenshot-captures.json");
        let report_path = test_dir.join("screenshot-differences.json");
        let capture_manifest = serde_json::json!([
            {
                "sheet_index": 1,
                "sheet_name": "Sheet1",
                "capture_range": "A1:A1",
                "capture_strategy": "test",
                "used_range_error": null,
                "screenshot_error": null,
                "original_image": original_path,
                "shrunk_image": shrunk_path,
                "capture_info": {
                    "strategy": "test",
                    "address": "A1:A1",
                    "start_row": 1,
                    "start_column": 1,
                    "rows": 1,
                    "columns": 1,
                    "used_range_error": null
                }
            }
        ]);
        fs::write(
            &capture_manifest_path,
            serde_json::to_vec_pretty(&capture_manifest)?,
        )?;

        assert!(
            compare_captured_screenshots(&capture_manifest_path, &report_path, None)?
                .total_differences
                > 0
        );
        let report = fs::read_to_string(&report_path)?;
        assert!(report.contains("failed to load original screenshot"));
        assert!(report.contains("original.png"));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn pads_smaller_screenshot_with_manifest_background_before_compare() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-screenshot-padding-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let original_path = test_dir.join("original.png");
        let shrunk_path = test_dir.join("shrunk.png");
        let content = image::Rgba([20, 30, 40, 255]);
        let background = image::Rgba([200, 210, 220, 255]);
        image::RgbaImage::from_pixel(2, 2, content).save(&original_path)?;
        let mut shrunk = image::RgbaImage::from_pixel(3, 3, background);
        for y in 0..2 {
            for x in 0..2 {
                shrunk.put_pixel(x, y, content);
            }
        }
        shrunk.save(&shrunk_path)?;

        let capture_manifest_path = test_dir.join("screenshot-captures.json");
        let report_path = test_dir.join("screenshot-differences.json");
        let capture_manifest = serde_json::json!([
            {
                "sheet_index": 1,
                "sheet_name": "Sheet1",
                "capture_range": "A1:A1",
                "capture_strategy": "test",
                "used_range_error": null,
                "screenshot_error": null,
                "original_image": original_path,
                "shrunk_image": shrunk_path,
                "original_background_rgba": { "r": 200, "g": 210, "b": 220, "a": 255 },
                "shrunk_background_rgba": { "r": 200, "g": 210, "b": 220, "a": 255 },
                "capture_info": {
                    "strategy": "test",
                    "address": "A1:A1",
                    "start_row": 1,
                    "start_column": 1,
                    "rows": 1,
                    "columns": 1,
                    "used_range_error": null
                }
            }
        ]);
        fs::write(
            &capture_manifest_path,
            serde_json::to_vec_pretty(&capture_manifest)?,
        )?;

        assert_eq!(
            compare_captured_screenshots(&capture_manifest_path, &report_path, None)?
                .total_differences,
            0
        );
        let report = fs::read_to_string(&report_path)?;
        assert_eq!(report.trim(), "[]");

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn compares_tiled_screenshots_without_full_image() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-screenshot-tiles-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let original_tile_1 = test_dir.join("original-001-tile-001.png");
        let shrunk_tile_1 = test_dir.join("shrunk-001-tile-001.png");
        let original_tile_2 = test_dir.join("original-001-tile-002.png");
        let shrunk_tile_2 = test_dir.join("shrunk-001-tile-002.png");
        let white = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255]));
        white.save(&original_tile_1)?;
        white.save(&shrunk_tile_1)?;
        white.save(&original_tile_2)?;
        let mut changed = white.clone();
        changed.put_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        changed.save(&shrunk_tile_2)?;

        let capture_manifest_path = test_dir.join("screenshot-captures.json");
        let report_path = test_dir.join("screenshot-differences.json");
        let capture_manifest = serde_json::json!([
            {
                "sheet_index": 1,
                "sheet_name": "Sheet1",
                "capture_range": "A1:A8",
                "capture_strategy": "test_tiled",
                "used_range_error": null,
                "original_image": null,
                "shrunk_image": null,
                "capture_info": {
                    "strategy": "test_tiled",
                    "address": "A1:A8",
                    "start_row": 1,
                    "start_column": 1,
                    "rows": 8,
                    "columns": 1,
                    "used_range_error": null
                },
                "tiles": [
                    {
                        "tile_index": 1,
                        "capture_range": "A1:A4",
                        "capture_info": {
                            "strategy": "test_tile",
                            "address": "A1:A4",
                            "start_row": 1,
                            "start_column": 1,
                            "rows": 4,
                            "columns": 1,
                            "used_range_error": null
                        },
                        "original_image": original_tile_1,
                        "shrunk_image": shrunk_tile_1,
                        "original_width": 4,
                        "original_height": 4,
                        "shrunk_width": 4,
                        "shrunk_height": 4
                    },
                    {
                        "tile_index": 2,
                        "capture_range": "A5:A8",
                        "capture_info": {
                            "strategy": "test_tile",
                            "address": "A5:A8",
                            "start_row": 5,
                            "start_column": 1,
                            "rows": 4,
                            "columns": 1,
                            "used_range_error": null
                        },
                        "original_image": original_tile_2,
                        "shrunk_image": shrunk_tile_2,
                        "original_width": 4,
                        "original_height": 4,
                        "shrunk_width": 4,
                        "shrunk_height": 4
                    }
                ]
            }
        ]);
        fs::write(
            &capture_manifest_path,
            serde_json::to_vec_pretty(&capture_manifest)?,
        )?;

        assert!(
            compare_captured_screenshots(&capture_manifest_path, &report_path, None)?
                .total_differences
                > 0
        );
        let report = fs::read_to_string(&report_path)?;
        assert!(report.contains("\"tile_index\": 2"));
        assert!(report.contains("\"capture_range\": \"A5:A8\""));
        assert!(report.contains("tile-002-diff-pixels"));
        assert!(!report.contains("\"tile_index\": 1"));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn screenshot_difference_writes_sheet_xml_git_diff() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-screenshot-xml-diff-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;

        let original_workbook = test_dir.join("original.xlsx");
        let shrunk_workbook = test_dir.join("shrunk.xlsx");
        write_minimal_workbook(
            &original_workbook,
            br#"<worksheet><cols><col min="1" max="2" style="1"/></cols><sheetData><row r="1" s="2"><c r="A1" s="1" t="s"><v>0</v></c></row><row r="2" s="2"><c r="A2" s="1" t="s"><v>0</v></c><c r="B2" s="1" t="s"><v>0</v></c><c r="C2" s="1" t="s"><v>0</v></c></row></sheetData></worksheet>"#,
        )?;
        write_minimal_workbook(
            &shrunk_workbook,
            br#"<worksheet><cols><col min="1" max="2" style="1"/></cols><sheetData><row r="1" s="2"><c r="A1" s="1" t="s"><v>1</v></c></row><row r="2" s="2"><c r="A2" s="1" t="s"><v>0</v></c><c r="B2" s="1" t="s"><v>1</v></c><c r="C2" s="1" t="s"><v>0</v></c></row></sheetData></worksheet>"#,
        )?;

        let original_image = test_dir.join("original.png");
        let shrunk_image = test_dir.join("shrunk.png");
        let original = image::RgbaImage::from_pixel(3, 3, image::Rgba([255, 255, 255, 255]));
        let mut shrunk = original.clone();
        shrunk.put_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        original.save(&original_image)?;
        shrunk.save(&shrunk_image)?;

        let capture_manifest_path = test_dir.join("screenshot-captures.json");
        let report_path = test_dir.join("screenshot-differences.json");
        let capture_manifest = serde_json::json!([
            {
                "sheet_index": 1,
                "sheet_name": "Sheet1",
                "capture_range": "A1:C3",
                "capture_strategy": "test",
                "used_range_error": null,
                "original_image": original_image,
                "shrunk_image": shrunk_image,
                "capture_info": {
                    "strategy": "test",
                    "address": "A1:C3",
                    "start_row": 1,
                    "start_column": 1,
                    "rows": 3,
                    "columns": 3,
                    "used_range_error": null
                }
            }
        ]);
        fs::write(
            &capture_manifest_path,
            serde_json::to_vec_pretty(&capture_manifest)?,
        )?;

        let context = SheetXmlDiffContext {
            original_workbook_path: &original_workbook,
            shrunk_workbook_path: &shrunk_workbook,
        };
        assert!(
            compare_captured_screenshots(&capture_manifest_path, &report_path, Some(&context),)?
                .total_differences
                > 0
        );

        let sheet_xml_diff = test_dir.join("001-Sheet1-sheet-xml.diff.zip");
        assert!(sheet_xml_diff.exists());
        let diff = read_test_zip_artifact(&sheet_xml_diff, "001-Sheet1-sheet-xml.diff")?;
        assert!(diff.contains("-          0"));
        assert!(diff.contains("+          1"));
        assert!(diff.contains("cell=A1"));
        assert!(diff.contains("cell=B2"));
        assert!(diff.contains("cell_s=1"));
        assert!(diff.contains("row_s=2"));
        assert!(diff.contains("column_s=1"));
        assert!(diff.contains("effective_style_xf[1]=<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFill=\"1\"/>"));
        assert!(diff.contains("row_style_xf[2]=<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/>"));
        assert!(diff.contains("column_node=<col min=\"1\" max=\"2\" style=\"1\"/>"));
        assert!(diff.contains("shared_string[0]=<si><t>old</t></si>"));
        assert!(diff.contains("shared_string[1]=<si><t>new</t></si>"));

        let sheet_xml_visual_diff = test_dir.join("001-Sheet1-sheet-xml-visual.diff.zip");
        assert!(sheet_xml_visual_diff.exists());
        let visual_diff =
            read_test_zip_artifact(&sheet_xml_visual_diff, "001-Sheet1-sheet-xml-visual.diff")?;
        assert!(visual_diff.contains("# excel-shrink visual cell range: B2:B2"));
        assert!(visual_diff.contains("cell=B2"));
        assert!(!visual_diff.contains("cell=A1"));
        assert!(!visual_diff.contains("r=\"A1\""));

        let report = fs::read_to_string(&report_path)?;
        assert!(report.contains("sheet_xml_diff"));
        assert!(report.contains("sheet_xml_visual_diff"));
        assert!(report.contains("sheet-xml.diff.zip"));
        assert!(report.contains("\"top_left\": \"B2\""));
        assert!(report.contains("\"bottom_right\": \"B2\""));
        assert!(report.contains("xl/worksheets/sheet1.xml"));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    fn write_minimal_workbook(path: &Path, sheet_xml: &[u8]) -> anyhow::Result<()> {
        use std::io::Write;
        use zip::write::FileOptions;

        let file = File::create(path)?;
        let mut writer = zip::ZipWriter::new(file);
        let options = FileOptions::default();

        writer.start_file("xl/workbook.xml", options)?;
        writer.write_all(
            br#"<workbook><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        )?;

        writer.start_file("xl/_rels/workbook.xml.rels", options)?;
        writer.write_all(
            br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#,
        )?;

        writer.start_file("xl/worksheets/sheet1.xml", options)?;
        writer.write_all(sheet_xml)?;

        writer.start_file("xl/styles.xml", options)?;
        writer.write_all(
            br#"<styleSheet><cellXfs count="3"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyFill="1"/><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs></styleSheet>"#,
        )?;

        writer.start_file("xl/sharedStrings.xml", options)?;
        writer.write_all(br#"<sst><si><t>old</t></si><si><t>new</t></si></sst>"#)?;
        writer.finish()?;

        Ok(())
    }

    fn read_test_zip_artifact(path: &Path, entry_name: &str) -> anyhow::Result<String> {
        let file = File::open(path)?;
        let mut archive = ZipArchive::new(file)?;
        let mut entry = archive.by_name(entry_name)?;
        let mut text = String::new();
        entry.read_to_string(&mut text)?;
        Ok(text)
    }

    fn test_image_from_row_colors(colors: &[image::Rgba<u8>]) -> image::RgbaImage {
        let width = 4;
        let mut image = image::RgbaImage::new(width, colors.len() as u32);
        for (y, color) in colors.iter().enumerate() {
            for x in 0..width {
                image.put_pixel(x, y as u32, *color);
            }
        }
        image
    }

    fn test_image_from_pixel_rows(rows: &[Vec<image::Rgba<u8>>]) -> image::RgbaImage {
        let width = rows.first().map_or(0, Vec::len) as u32;
        let mut image = image::RgbaImage::new(width, rows.len() as u32);
        for (y, row) in rows.iter().enumerate() {
            assert_eq!(row.len() as u32, width);
            for (x, color) in row.iter().enumerate() {
                image.put_pixel(x as u32, y as u32, *color);
            }
        }
        image
    }

    fn default_visual_profile() -> ResolvedVisualProfile {
        ResolvedVisualProfile::from_config(crate::visual_profile::VisualProfileConfig::default())
    }

    fn maximum_cleaning_visual_profile() -> ResolvedVisualProfile {
        ResolvedVisualProfile::from_config(crate::visual_profile::VisualProfileConfig {
            remove_textless_default_font_representatives: true,
            remove_textless_style_representatives: true,
            remove_textless_column_style_representatives: true,
            inactive_protection_cell_policy: crate::InactiveProtectionCellPolicyArg::Remove,
            remove_unreferenced_whitespace_cells: true,
            keep_unreferenced_hidden_content_cells: false,
        })
    }

    fn keep_all_inactive_protection_visual_profile() -> ResolvedVisualProfile {
        ResolvedVisualProfile::from_config(crate::visual_profile::VisualProfileConfig {
            inactive_protection_cell_policy: crate::InactiveProtectionCellPolicyArg::KeepAll,
            ..crate::visual_profile::VisualProfileConfig::default()
        })
    }

    fn isolated_column_style_visual_profile() -> ResolvedVisualProfile {
        ResolvedVisualProfile::from_config(crate::visual_profile::VisualProfileConfig {
            remove_textless_column_style_representatives: true,
            ..crate::visual_profile::VisualProfileConfig::default()
        })
    }

    fn write_test_screenshot_cache_manifest(
        cache_root: &Path,
        cache: &WorkbookScreenshotCache,
        image_path: &Path,
        workbook_hash: &str,
    ) -> anyhow::Result<()> {
        let identity = cache.sheet_identities.get(&1).unwrap();
        let cache_dir = workbook_screenshot_cache_root(cache_root).join(workbook_hash);
        fs::create_dir_all(&cache_dir)?;
        let manifest = WorkbookScreenshotCacheManifest {
            version: SCREENSHOT_CACHE_VERSION,
            workbook_hash: workbook_hash.to_string(),
            capture_range_hash: cache.capture_range_hash.clone(),
            captures: vec![WorkbookScreenshotCapture {
                sheet_index: 1,
                sheet_name: "Sheet1".to_string(),
                sheet_file: Some(identity.sheet_file.clone()),
                sheet_xml_hash: Some(identity.sheet_xml_hash.clone()),
                capture_range: Some("A1:A1".to_string()),
                capture_strategy: Some("test".to_string()),
                used_range_error: None,
                screenshot_error: None,
                image: Some(image_path.to_path_buf()),
                image_hash: Some(file_content_hash(image_path)?),
                background_rgba: None,
                capture_info: Some(ScreenshotCaptureInfo {
                    strategy: "test".to_string(),
                    address: "A1:A1".to_string(),
                    start_row: 1,
                    start_column: 1,
                    rows: 1,
                    columns: 1,
                    used_range_error: None,
                }),
                tiles: Vec::new(),
            }],
        };
        fs::write(
            cache_dir.join("workbook-screenshots.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn extracts_excel_pids_from_progress_log() -> anyhow::Result<()> {
        let test_dir =
            std::env::temp_dir().join(format!("excel-shrink-progress-pids-{}", std::process::id()));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;
        let progress_path = test_dir.join("screenshot-progress.log");
        fs::write(
            &progress_path,
            "2026-01-01T00:00:00Z excel pid=1234\n2026-01-01T00:00:01Z excel_pid=5678\n",
        )?;

        let pids = excel_pids_from_progress_log(&progress_path);
        assert!(pids.contains(&1234));
        assert!(pids.contains(&5678));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn failed_capture_before_original_open_triggers_workbook_repair() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-repair-decision-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;
        let progress_path = test_dir.join("screenshot-progress.log");
        let manifest_path = test_dir.join("screenshot-captures.json");
        fs::write(
            &progress_path,
            "screenshot capture started\nexcel pid=1234\n",
        )?;

        assert!(screenshot_capture_needs_original_workbook_repair(
            &ExitStatus::from_raw(1),
            &progress_path,
            &manifest_path,
        ));

        fs::write(
            &progress_path,
            "screenshot capture started\nopened original workbook 'a.xlsx'\n",
        )?;
        assert!(!screenshot_capture_needs_original_workbook_repair(
            &ExitStatus::from_raw(1),
            &progress_path,
            &manifest_path,
        ));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn screenshot_failure_details_identify_shrunk_open_failure() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-open-failure-details-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;
        let progress_path = test_dir.join("screenshot-progress.log");
        fs::write(
            &progress_path,
            "screenshot capture started\nexcel pid=1234\nopened original workbook 'original.xlsx'\n",
        )?;

        let details = screenshot_capture_failure_details(
            &progress_path,
            Path::new("original.xlsx"),
            Path::new("shrunk.xlsx"),
        );
        assert!(details.contains("likely_failed_phase=opening shrunk workbook"));
        assert!(details.contains("original_excel=original.xlsx"));
        assert!(details.contains("shrunk_excel=shrunk.xlsx"));
        assert!(details.contains("opened original workbook"));

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn cached_capture_pair_combines_cached_tiles_without_excel() {
        let capture_info = ScreenshotCaptureInfo {
            strategy: "test".to_string(),
            address: "A1:A4".to_string(),
            start_row: 1,
            start_column: 1,
            rows: 4,
            columns: 1,
            used_range_error: None,
        };
        let original = WorkbookScreenshotCapture {
            sheet_index: 7,
            sheet_name: "OriginalName".to_string(),
            sheet_file: Some("xl/worksheets/sheet7.xml".to_string()),
            sheet_xml_hash: Some("same-sheet".to_string()),
            capture_range: Some("A1:A4".to_string()),
            capture_strategy: Some("rust_bounds_tiled".to_string()),
            used_range_error: None,
            screenshot_error: None,
            image: None,
            image_hash: None,
            background_rgba: None,
            capture_info: Some(capture_info.clone()),
            tiles: vec![WorkbookScreenshotTile {
                tile_index: 1,
                capture_range: "A1:A4".to_string(),
                capture_info: capture_info.clone(),
                image: Some(PathBuf::from("original-tile.png")),
                image_hash: Some("original-hash".to_string()),
                background_rgba: Some(ScreenshotRgba {
                    r: 255,
                    g: 255,
                    b: 255,
                    a: 255,
                }),
                width: Some(10),
                height: Some(20),
            }],
        };
        let shrunk = WorkbookScreenshotCapture {
            sheet_index: 7,
            sheet_name: "ShrunkName".to_string(),
            sheet_file: Some("xl/worksheets/sheet7.xml".to_string()),
            sheet_xml_hash: Some("same-sheet".to_string()),
            capture_range: Some("A1:A4".to_string()),
            capture_strategy: Some("rust_bounds_tiled".to_string()),
            used_range_error: None,
            screenshot_error: None,
            image: None,
            image_hash: None,
            background_rgba: None,
            capture_info: Some(capture_info.clone()),
            tiles: vec![WorkbookScreenshotTile {
                tile_index: 1,
                capture_range: "A1:A4".to_string(),
                capture_info,
                image: Some(PathBuf::from("shrunk-tile.png")),
                image_hash: Some("shrunk-hash".to_string()),
                background_rgba: None,
                width: Some(10),
                height: Some(20),
            }],
        };

        let pair = cached_capture_pair(7, &original, &shrunk);
        assert_eq!(pair.sheet_index, 7);
        assert_eq!(pair.sheet_name, "OriginalName");
        assert!(pair.original_image.is_none());
        assert!(pair.shrunk_image.is_none());
        assert_eq!(pair.tiles.len(), 1);
        assert_eq!(
            pair.tiles[0].original_image,
            Some(PathBuf::from("original-tile.png"))
        );
        assert_eq!(
            pair.tiles[0].shrunk_image,
            Some(PathBuf::from("shrunk-tile.png"))
        );
    }
}
