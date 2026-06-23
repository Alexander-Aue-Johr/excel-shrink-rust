use super::xlsx_value_compare::WorksheetCaptureRange;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

const SCREENSHOT_CACHE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct ExcelSavedWorkbook {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExcelScreenshotReport {
    pub report_path: PathBuf,
    pub output_dir: PathBuf,
    pub differences_found: bool,
    pub kept_screenshots: bool,
    pub screenshot_cache_root: PathBuf,
    pub original_cache_dir: PathBuf,
    pub shrunk_cache_dir: PathBuf,
    pub original_cache_hit: bool,
    pub shrunk_cache_hit: bool,
    pub range_limits: Vec<WorksheetCaptureRange>,
}

pub fn save_recalculated_copy(
    input_path: &Path,
    output_path: &Path,
    work_dir: &Path,
) -> anyhow::Result<ExcelSavedWorkbook> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir_all(work_dir)?;

    let script_path = work_dir.join("excel_recalculate_and_save.ps1");
    fs::write(&script_path, RECALCULATE_AND_SAVE_SCRIPT)?;

    run_power_shell_script(
        &script_path,
        &[
            ("InputPath", power_shell_path(input_path)?),
            ("OutputPath", power_shell_path(output_path)?),
        ],
    )?;

    anyhow::ensure!(
        output_path.exists(),
        "Excel did not create recalculated workbook: {}",
        output_path.display()
    );

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
) -> anyhow::Result<ExcelScreenshotReport> {
    eprintln!(
        "[excel-screenshot] original={} shrunk={} output_dir={} range_limits={}",
        original_path.display(),
        shrunk_path.display(),
        output_dir.display(),
        range_limits.len()
    );

    fs::create_dir_all(output_dir)?;
    fs::create_dir_all(work_dir)?;

    let cache_root = screenshot_cache_root(work_dir);
    fs::create_dir_all(&cache_root)?;
    let capture_range_hash = capture_range_hash(range_limits)?;
    let original_cache =
        WorkbookScreenshotCache::new(original_path, &cache_root, &capture_range_hash)?;
    let shrunk_cache = WorkbookScreenshotCache::new(shrunk_path, &cache_root, &capture_range_hash)?;

    let script_path = work_dir.join("excel_compare_screenshots.ps1");
    fs::write(&script_path, SCREENSHOT_COMPARE_SCRIPT)?;
    let range_limit_path = work_dir.join("screenshot-range-limits.json");
    fs::write(
        &range_limit_path,
        serde_json::to_string_pretty(range_limits)?,
    )?;

    let original_excel_path = copy_workbook_with_suffix(original_path, work_dir, "original")?;
    let shrunk_excel_path = copy_workbook_with_suffix(shrunk_path, work_dir, "shrunk")?;

    let capture_manifest_path = output_dir.join("screenshot-captures.json");
    let report_path = output_dir.join("screenshot-differences.json");
    let original_cache_hit = original_cache.is_complete()?;
    let shrunk_cache_hit = shrunk_cache.is_complete()?;
    eprintln!(
        "[excel-screenshot] cache original_hash={} hit={} shrunk_hash={} hit={} root={}",
        original_cache.workbook_hash,
        original_cache_hit,
        shrunk_cache.workbook_hash,
        shrunk_cache_hit,
        cache_root.display()
    );

    if original_cache_hit && shrunk_cache_hit {
        write_pair_manifest_from_cache(&original_cache, &shrunk_cache, &capture_manifest_path)?;
    } else {
        fs::create_dir_all(&original_cache.cache_dir)?;
        fs::create_dir_all(&shrunk_cache.cache_dir)?;
        let status = run_power_shell_script_with_status(
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
                ("OriginalAlreadyCached", original_cache_hit.to_string()),
                ("ShrunkAlreadyCached", shrunk_cache_hit.to_string()),
            ],
        )?;

        anyhow::ensure!(
            capture_manifest_path.exists(),
            "Excel screenshot capture did not create manifest: {}",
            capture_manifest_path.display()
        );

        anyhow::ensure!(
            status.success(),
            "Excel screenshot capture failed; manifest: {}",
            capture_manifest_path.display()
        );
        update_workbook_screenshot_cache(&capture_manifest_path, &original_cache, true)?;
        update_workbook_screenshot_cache(&capture_manifest_path, &shrunk_cache, false)?;
    }

    let differences_found = compare_captured_screenshots(&capture_manifest_path, &report_path)?;

    eprintln!(
        "[excel-screenshot] done differences_found={} report={}",
        differences_found,
        report_path.display()
    );

    Ok(ExcelScreenshotReport {
        report_path,
        output_dir: output_dir.to_path_buf(),
        differences_found,
        kept_screenshots: true,
        screenshot_cache_root: cache_root,
        original_cache_dir: original_cache.cache_dir,
        shrunk_cache_dir: shrunk_cache.cache_dir,
        original_cache_hit,
        shrunk_cache_hit,
        range_limits: range_limits.to_vec(),
    })
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
    capture_info: Option<ScreenshotCaptureInfo>,
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

#[derive(Debug, Clone)]
struct WorkbookScreenshotCache {
    workbook_hash: String,
    capture_range_hash: String,
    cache_dir: PathBuf,
    manifest_path: PathBuf,
    marker_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkbookScreenshotCacheManifest {
    version: u32,
    workbook_hash: String,
    capture_range_hash: String,
    captures: Vec<WorkbookScreenshotCapture>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkbookScreenshotCapture {
    sheet_index: usize,
    sheet_name: String,
    capture_range: Option<String>,
    capture_strategy: Option<String>,
    used_range_error: Option<String>,
    screenshot_error: Option<String>,
    image: Option<PathBuf>,
    capture_info: Option<ScreenshotCaptureInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkbookScreenshotCacheMarker {
    version: u32,
    workbook_hash: String,
    capture_range_hash: String,
    capture_count: usize,
}

impl WorkbookScreenshotCache {
    fn new(
        workbook_path: &Path,
        cache_root: &Path,
        capture_range_hash: &str,
    ) -> anyhow::Result<Self> {
        let workbook_hash = workbook_content_hash(workbook_path)?;
        let cache_dir = cache_root.join(&workbook_hash);
        Ok(Self {
            manifest_path: cache_dir.join("workbook-screenshots.json"),
            marker_path: cache_dir.join("complete.json"),
            workbook_hash,
            capture_range_hash: capture_range_hash.to_string(),
            cache_dir,
        })
    }

    fn is_complete(&self) -> anyhow::Result<bool> {
        if !self.marker_path.exists() || !self.manifest_path.exists() {
            return Ok(false);
        }

        let Ok(marker) = read_json_file::<WorkbookScreenshotCacheMarker>(&self.marker_path) else {
            return Ok(false);
        };
        if marker.version != SCREENSHOT_CACHE_VERSION
            || marker.workbook_hash != self.workbook_hash
            || marker.capture_range_hash != self.capture_range_hash
        {
            return Ok(false);
        }

        let Ok(manifest) = self.read_manifest() else {
            return Ok(false);
        };
        if manifest.version != SCREENSHOT_CACHE_VERSION
            || manifest.workbook_hash != self.workbook_hash
            || manifest.capture_range_hash != self.capture_range_hash
            || manifest.captures.len() != marker.capture_count
        {
            return Ok(false);
        }

        for capture in &manifest.captures {
            if capture.screenshot_error.is_some() {
                return Ok(false);
            }
            let Some(image) = capture.image.as_ref().map(normalize_automation_path) else {
                return Ok(false);
            };
            if !image.exists() {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn read_manifest(&self) -> anyhow::Result<WorkbookScreenshotCacheManifest> {
        read_json_file(&self.manifest_path)
    }
}

#[derive(Debug, Clone, Serialize)]
struct ScreenshotDifference {
    sheet_index: usize,
    sheet_name: String,
    capture_range: Option<String>,
    capture_strategy: Option<String>,
    used_range_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screenshot_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_box: Option<DiffBox>,
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
    original_image: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shrunk_image: Option<PathBuf>,
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
struct CropRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

fn compare_captured_screenshots(
    capture_manifest_path: &Path,
    report_path: &Path,
) -> anyhow::Result<bool> {
    let captures: Vec<ScreenshotCapture> = read_json_file(capture_manifest_path)?;

    let mut differences = Vec::new();

    for capture in captures {
        let original_image = capture
            .original_image
            .as_ref()
            .map(normalize_automation_path);
        let shrunk_image = capture.shrunk_image.as_ref().map(normalize_automation_path);

        if capture.screenshot_error.is_some() {
            differences.push(ScreenshotDifference {
                sheet_index: capture.sheet_index,
                sheet_name: capture.sheet_name,
                capture_range: capture.capture_range,
                capture_strategy: capture.capture_strategy,
                used_range_error: capture.used_range_error,
                screenshot_error: capture.screenshot_error,
                diff_box: None,
                approximate_cells: None,
                approximate_cells_error: None,
                original_crop: None,
                shrunk_crop: None,
                diff_pixels: None,
                original_image,
                shrunk_image,
            });
            continue;
        }

        let Some(original_image) = original_image else {
            continue;
        };
        let Some(shrunk_image) = shrunk_image else {
            continue;
        };

        let original_pixels = load_screenshot_image(&original_image)?;
        let shrunk_pixels = load_screenshot_image(&shrunk_image)?;
        let diff_box = find_diff_box(&original_pixels, &shrunk_pixels);
        if let Some(diff_box) = diff_box {
            let safe_name = sanitize_sheet_name(&capture.sheet_name);
            let output_dir = report_path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("report path has no parent"))?;
            let original_crop = output_dir.join(format!(
                "{}-{}-diff-original.png",
                capture.sheet_index, safe_name
            ));
            let shrunk_crop = output_dir.join(format!(
                "{}-{}-diff-shrunk.png",
                capture.sheet_index, safe_name
            ));
            let diff_pixels = output_dir.join(format!(
                "{}-{}-diff-pixels.png",
                capture.sheet_index, safe_name
            ));

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

            save_diff_crop(&original_pixels, &original_crop, diff_box, &capture_info)?;
            save_diff_crop(&shrunk_pixels, &shrunk_crop, diff_box, &capture_info)?;
            save_diff_pixels(
                &original_pixels,
                &shrunk_pixels,
                &diff_pixels,
                diff_box,
                &capture_info,
            )?;

            let approximate_cells = approximate_cells(diff_box, &capture_info);
            differences.push(ScreenshotDifference {
                sheet_index: capture.sheet_index,
                sheet_name: capture.sheet_name,
                capture_range: capture.capture_range,
                capture_strategy: capture.capture_strategy,
                used_range_error: capture.used_range_error,
                screenshot_error: None,
                diff_box: Some(diff_box),
                approximate_cells: Some(approximate_cells),
                approximate_cells_error: None,
                original_crop: Some(original_crop),
                shrunk_crop: Some(shrunk_crop),
                diff_pixels: Some(diff_pixels),
                original_image: None,
                shrunk_image: None,
            });
        }
    }

    fs::write(report_path, serde_json::to_vec_pretty(&differences)?)?;
    Ok(!differences.is_empty())
}

fn write_pair_manifest_from_cache(
    original_cache: &WorkbookScreenshotCache,
    shrunk_cache: &WorkbookScreenshotCache,
    capture_manifest_path: &Path,
) -> anyhow::Result<()> {
    let original_manifest = original_cache.read_manifest()?;
    let shrunk_manifest = shrunk_cache.read_manifest()?;
    let original_by_index: BTreeMap<_, _> = original_manifest
        .captures
        .into_iter()
        .map(|capture| (capture.sheet_index, capture))
        .collect();
    let shrunk_by_index: BTreeMap<_, _> = shrunk_manifest
        .captures
        .into_iter()
        .map(|capture| (capture.sheet_index, capture))
        .collect();
    let max_sheet_index = original_by_index
        .keys()
        .chain(shrunk_by_index.keys())
        .copied()
        .max()
        .unwrap_or(0);
    let mut captures = Vec::new();

    for sheet_index in 1..=max_sheet_index {
        let original = original_by_index.get(&sheet_index);
        let shrunk = shrunk_by_index.get(&sheet_index);
        let screenshot_error = match (original, shrunk) {
            (None, None) => Some("missing cached original and shrunk screenshots".to_string()),
            (None, Some(_)) => Some("missing cached original screenshot".to_string()),
            (Some(_), None) => Some("missing cached shrunk screenshot".to_string()),
            (Some(original), Some(shrunk)) => original
                .screenshot_error
                .clone()
                .or_else(|| shrunk.screenshot_error.clone()),
        };
        let preferred = shrunk.or(original);
        let sheet_name = original
            .or(shrunk)
            .map(|capture| capture.sheet_name.clone())
            .unwrap_or_else(|| format!("Sheet {sheet_index}"));

        captures.push(ScreenshotCapture {
            sheet_index,
            sheet_name,
            capture_range: preferred.and_then(|capture| capture.capture_range.clone()),
            capture_strategy: preferred.and_then(|capture| capture.capture_strategy.clone()),
            used_range_error: preferred.and_then(|capture| capture.used_range_error.clone()),
            screenshot_error,
            original_image: original.and_then(|capture| capture.image.clone()),
            shrunk_image: shrunk.and_then(|capture| capture.image.clone()),
            capture_info: preferred.and_then(|capture| capture.capture_info.clone()),
        });
    }

    fs::write(capture_manifest_path, serde_json::to_vec_pretty(&captures)?)?;
    Ok(())
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

        if pair.screenshot_error.is_some() || image.as_ref().is_none_or(|image| !image.exists()) {
            complete = false;
        }

        captures.push(WorkbookScreenshotCapture {
            sheet_index: pair.sheet_index,
            sheet_name: pair.sheet_name,
            capture_range: pair.capture_range,
            capture_strategy: pair.capture_strategy,
            used_range_error: pair.used_range_error,
            screenshot_error: pair.screenshot_error,
            image,
            capture_info: pair.capture_info,
        });
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

fn load_screenshot_image(path: &Path) -> anyhow::Result<image::RgbaImage> {
    Ok(image::open(path)
        .map_err(|error| anyhow::anyhow!("failed to open image '{}': {error}", path.display()))?
        .to_rgba8())
}

fn read_json_file<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let bytes = fs::read(path)?;
    let json = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    serde_json::from_slice(json)
        .map_err(|error| anyhow::anyhow!("failed to parse JSON '{}': {error}", path.display()))
}

fn workbook_content_hash(path: &Path) -> anyhow::Result<String> {
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

fn capture_range_hash(range_limits: &[WorksheetCaptureRange]) -> anyhow::Result<String> {
    let bytes = serde_json::to_vec(range_limits)?;
    Ok(format!(
        "{:016x}",
        fnv_hash_update(fnv_offset_basis(), &bytes)
    ))
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

fn screenshot_cache_root(work_dir: &Path) -> PathBuf {
    for ancestor in work_dir.ancestors() {
        if ancestor.file_name() == Some(std::ffi::OsStr::new("regression")) {
            if let Some(repo_root) = ancestor.parent() {
                return repo_root.join(".tmp").join("excel-screenshot-cache");
            }
        }
    }

    std::env::current_dir()
        .unwrap_or_else(|_| work_dir.to_path_buf())
        .join(".tmp")
        .join("excel-screenshot-cache")
}

fn find_diff_box(left: &image::RgbaImage, right: &image::RgbaImage) -> Option<DiffBox> {
    if left.width() != right.width() || left.height() != right.height() {
        return Some(DiffBox {
            min_x: 0,
            min_y: 0,
            max_x: left.width().max(right.width()).saturating_sub(1),
            max_y: left.height().max(right.height()).saturating_sub(1),
            image_width: right.width(),
            image_height: right.height(),
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

fn approximate_cells(diff_box: DiffBox, capture_info: &ScreenshotCaptureInfo) -> ApproximateCells {
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

    ApproximateCells {
        top_left: format!("{}{}", column_name(left_col), top_row),
        bottom_right: format!("{}{}", column_name(right_col), bottom_row),
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

fn copy_workbook_with_suffix(
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
    let copied_path = work_dir.join(format!("{stem}.{suffix}.{extension}"));
    fs::copy(input_path, &copied_path).map_err(|e| {
        anyhow::anyhow!(
            "failed to copy workbook '{}' to '{}': {e}",
            input_path.display(),
            copied_path.display()
        )
    })?;
    Ok(copied_path)
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

    command
        .status()
        .map_err(|e| anyhow::anyhow!("failed to start PowerShell automation: {e}"))
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

const RECALCULATE_AND_SAVE_SCRIPT: &str = r#"
param(
    [Parameter(Mandatory=$true)][string]$InputPath,
    [Parameter(Mandatory=$true)][string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$excel = $null
$workbook = $null

try {
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    $excel.AskToUpdateLinks = $false
    $excel.AutomationSecurity = 3

    $workbook = $excel.Workbooks.Open($InputPath, 0, $false)
    $workbook.ForceFullCalculation = $true
    $excel.CalculateFullRebuild()
    $workbook.SaveAs($OutputPath, 51)
}
finally {
    if ($workbook -ne $null) {
        $workbook.Close($false)
    }
    if ($excel -ne $null) {
        $excel.Quit()
    }
}
"#;

const SCREENSHOT_COMPARE_SCRIPT: &str = r#"
param(
    [Parameter(Mandatory=$true)][string]$OriginalPath,
    [Parameter(Mandatory=$true)][string]$ShrunkPath,
    [Parameter(Mandatory=$true)][string]$OutputDir,
    [Parameter(Mandatory=$true)][string]$CaptureManifestPath,
    [Parameter(Mandatory=$false)][string]$RangeLimitPath = '',
    [Parameter(Mandatory=$true)][string]$OriginalCacheDir,
    [Parameter(Mandatory=$true)][string]$ShrunkCacheDir,
    [Parameter(Mandatory=$false)][string]$OriginalAlreadyCached = 'false',
    [Parameter(Mandatory=$false)][string]$ShrunkAlreadyCached = 'false'
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

function Write-ProgressLine([string]$message) {
    $timestamp = Get-Date -Format o
    Add-Content -Path $ProgressPath -Value "$timestamp $message"
}

function File-Size([string]$path) {
    if (Test-Path $path) {
        return (Get-Item $path).Length
    }
    return 0
}

function Range-Address($range) {
    try {
        $startRow = [int]$range.Row
        $startColumn = [int]$range.Column
        $endRow = $startRow + [int]$range.Rows.Count - 1
        $endColumn = $startColumn + [int]$range.Columns.Count - 1
        return "{0}{1}:{2}{3}" -f (Column-Name $startColumn), $startRow, (Column-Name $endColumn), $endRow
    }
    catch {
        return '<unknown>'
    }
}

function Column-Name([int]$index) {
    $name = ''
    while ($index -gt 0) {
        $index--
        $name = [char](65 + ($index % 26)) + $name
        $index = [Math]::Floor($index / 26)
    }
    return $name
}

function Matching-WorksheetRange($worksheet, $sourceRange) {
    $startRow = [int]$sourceRange.Row
    $startColumn = [int]$sourceRange.Column
    $endRow = $startRow + [int]$sourceRange.Rows.Count - 1
    $endColumn = $startColumn + [int]$sourceRange.Columns.Count - 1
    return ,$worksheet.Range($worksheet.Cells.Item($startRow, $startColumn), $worksheet.Cells.Item($endRow, $endColumn))
}

function RangeLimit-Address($rangeLimit, $range) {
    if ($null -ne $rangeLimit) {
        $maxCell = [string]$rangeLimit.max_cell
        if (-not [string]::IsNullOrWhiteSpace($maxCell) -and $maxCell -match '^[A-Za-z]+[0-9]+$') {
            return "A1:$maxCell"
        }
    }

    return Range-Address $range
}

function Resolve-CaptureRange($worksheet, $rangeLimit) {
    if ($null -ne $rangeLimit) {
        $maxRow = [int]$rangeLimit.max_row
        $maxColumn = [int]$rangeLimit.max_column
        if ($maxRow -gt 0 -and $maxColumn -gt 0) {
            return ,$worksheet.Range($worksheet.Cells.Item(1, 1), $worksheet.Cells.Item($maxRow, $maxColumn))
        }

        $maxCell = [string]$rangeLimit.max_cell
        if (-not [string]::IsNullOrWhiteSpace($maxCell) -and $maxCell -match '^[A-Za-z]+[0-9]+$') {
            return ,$worksheet.Range("A1:$maxCell")
        }
    }

    return ,$worksheet.UsedRange
}

function Hide-ActiveGridlines($worksheet, [string]$label) {
    try {
        $window = $worksheet.Application.ActiveWindow
        if ($null -ne $window) {
            $window.DisplayGridlines = $false
        }
    }
    catch {
        Write-ProgressLine "hide gridlines failed $label error='$($_.Exception.Message)'"
    }
}

function Save-RangeImage($worksheet, $range, [string]$path, [string]$label, [string]$addressLabel) {
    $address = if (-not [string]::IsNullOrWhiteSpace($addressLabel)) { $addressLabel } else { Range-Address $range }
    Write-ProgressLine "capture start $label worksheet='$($worksheet.Name)' range='$address'"

    $image = $null
    $lastError = $null
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        try {
            [void]$worksheet.Parent.Activate()
            [void]$worksheet.Activate()
            Hide-ActiveGridlines $worksheet $label
            [void]$range.Select()
            Start-Sleep -Milliseconds 100
            Write-ProgressLine "clipboard clear $label attempt=$attempt"
            [System.Windows.Forms.Clipboard]::Clear()
            Write-ProgressLine "copy picture $label attempt=$attempt"
            $selection = $worksheet.Application.Selection
            [void]$selection.CopyPicture(1, 2)
        }
        catch {
            $lastError = $_.Exception.Message
            Write-ProgressLine "copy failed $label attempt=$attempt error='$lastError'"
            Start-Sleep -Milliseconds 250
            continue
        }

        Start-Sleep -Milliseconds 250
        Write-ProgressLine "clipboard read $label attempt=$attempt"
        $image = [System.Windows.Forms.Clipboard]::GetImage()
        if ($null -ne $image) {
            break
        }
    }

    if ($null -eq $image) {
        $address = Range-Address $range
        if ($lastError -ne $null) {
            throw "CopyPicture failed for worksheet '$($worksheet.Name)' range '$address': $lastError"
        }
        throw "Clipboard did not contain an image for worksheet '$($worksheet.Name)' range '$address'"
    }
    $image.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-ProgressLine "capture image $label width=$($image.Width) height=$($image.Height)"
    $image.Dispose()
    [System.Windows.Forms.Clipboard]::Clear()
    Write-ProgressLine "capture saved $label path='$path' bytes=$(File-Size $path)"
}

function Use-CachedRangeImage([string]$path, [string]$label) {
    if (-not (Test-Path $path)) {
        throw "cached screenshot missing for $label path='$path'"
    }
    Write-ProgressLine "capture cache hit $label path='$path' bytes=$(File-Size $path)"
}

function Range-CaptureInfo($range, [string]$strategy, [string]$address, [string]$usedRangeError) {
    return [pscustomobject]@{
        strategy = $strategy
        address = $address
        start_row = [int]$range.Row
        start_column = [int]$range.Column
        rows = [Math]::Max(1, [int]$range.Rows.Count)
        columns = [Math]::Max(1, [int]$range.Columns.Count)
        used_range_error = $usedRangeError
    }
}

$excel = $null
$original = $null
$shrunk = $null
$captures = New-Object System.Collections.ArrayList
$rangeLimitsByIndex = @{}
$ProgressPath = Join-Path $OutputDir "screenshot-progress.log"
$originalAlreadyCachedEnabled = $OriginalAlreadyCached -match '^(1|true|yes)$'
$shrunkAlreadyCachedEnabled = $ShrunkAlreadyCached -match '^(1|true|yes)$'

if (-not [string]::IsNullOrWhiteSpace($RangeLimitPath) -and (Test-Path $RangeLimitPath)) {
    $rangeLimits = Get-Content -Path $RangeLimitPath -Raw | ConvertFrom-Json
    foreach ($rangeLimit in @($rangeLimits)) {
        $rangeLimitsByIndex[[int]$rangeLimit.sheet_index] = $rangeLimit
    }
}

try {
    New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
    New-Item -ItemType Directory -Path $OriginalCacheDir -Force | Out-Null
    New-Item -ItemType Directory -Path $ShrunkCacheDir -Force | Out-Null
    Set-Content -Path $ProgressPath -Value "screenshot capture started $(Get-Date -Format o)"
    Write-ProgressLine "input original='$OriginalPath'"
    Write-ProgressLine "input shrunk='$ShrunkPath'"
    Write-ProgressLine "output_dir='$OutputDir'"
    Write-ProgressLine "original_cache_dir='$OriginalCacheDir' already_cached=$OriginalAlreadyCached"
    Write-ProgressLine "shrunk_cache_dir='$ShrunkCacheDir' already_cached=$ShrunkAlreadyCached"
    Write-ProgressLine "capture_manifest_path='$CaptureManifestPath'"
    Write-ProgressLine "range_limit_path='$RangeLimitPath' loaded_count=$($rangeLimitsByIndex.Count)"
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $true
    $excel.ScreenUpdating = $true
    $excel.DisplayAlerts = $false
    $excel.AskToUpdateLinks = $false
    $excel.AutomationSecurity = 3

    $original = $excel.Workbooks.Open($OriginalPath, 0, $false)
    if ($null -eq $original) {
        throw "Excel did not open original workbook '$OriginalPath'"
    }
    Write-ProgressLine "opened original workbook '$OriginalPath'"

    $shrunk = $excel.Workbooks.Open($ShrunkPath, 0, $false)
    if ($null -eq $shrunk) {
        throw "Excel did not open shrunk workbook '$ShrunkPath'"
    }
    Write-ProgressLine "opened shrunk workbook '$ShrunkPath'"
    $sheetCount = [Math]::Min($original.Worksheets.Count, $shrunk.Worksheets.Count)
    Write-ProgressLine "sheet count $sheetCount"

    for ($i = 1; $i -le $sheetCount; $i++) {
        $origSheet = $original.Worksheets.Item($i)
        $shrunkSheet = $shrunk.Worksheets.Item($i)
        $rangeLimit = $rangeLimitsByIndex[$i]
        $imageName = "{0:D3}.png" -f $i
        $origImage = Join-Path $OriginalCacheDir $imageName
        $shrunkImage = Join-Path $ShrunkCacheDir $imageName
        $captureRange = $null
        $captureInfo = $null
        $usedRangeError = $null

        Write-ProgressLine "sheet start index=$i name='$($origSheet.Name)'"

        try {
            $primaryStrategy = if ($null -ne $rangeLimit) { "rust_bounds" } else { "used_range" }
            try {
                $shrunkRange = Resolve-CaptureRange $shrunkSheet $rangeLimit
                $origRange = Matching-WorksheetRange $origSheet $shrunkRange
                $captureRange = RangeLimit-Address $rangeLimit $shrunkRange
                $captureInfo = Range-CaptureInfo $shrunkRange $primaryStrategy $captureRange $null
                Write-ProgressLine "sheet primary range index=$i strategy=$primaryStrategy range='$captureRange' start_row=$($captureInfo.start_row) start_column=$($captureInfo.start_column) rows=$($captureInfo.rows) columns=$($captureInfo.columns)"
                if ($originalAlreadyCachedEnabled) {
                    Use-CachedRangeImage $origImage "original sheet $i $primaryStrategy"
                }
                else {
                    Save-RangeImage $origSheet $origRange $origImage "original sheet $i $primaryStrategy" $captureRange
                }
                if ($shrunkAlreadyCachedEnabled) {
                    Use-CachedRangeImage $shrunkImage "shrunk sheet $i $primaryStrategy"
                }
                else {
                    Save-RangeImage $shrunkSheet $shrunkRange $shrunkImage "shrunk sheet $i $primaryStrategy" $captureRange
                }
            }
            catch {
                $usedRangeError = $_.Exception.Message
                Write-ProgressLine "primary range failed index=$i strategy=$primaryStrategy error='$usedRangeError'"
                if (-not $originalAlreadyCachedEnabled) {
                    Remove-Item $origImage -Force -ErrorAction SilentlyContinue
                }
                if (-not $shrunkAlreadyCachedEnabled) {
                    Remove-Item $shrunkImage -Force -ErrorAction SilentlyContinue
                }

                $shrunkRange = $shrunkSheet.UsedRange
                $origRange = Matching-WorksheetRange $origSheet $shrunkRange
                $captureRange = Range-Address $shrunkRange
                $captureInfo = Range-CaptureInfo $shrunkRange "used_range_after_primary_failure" $captureRange $usedRangeError
                Write-ProgressLine "sheet used range fallback index=$i range='$captureRange' start_row=$($captureInfo.start_row) start_column=$($captureInfo.start_column) rows=$($captureInfo.rows) columns=$($captureInfo.columns)"
                if ($originalAlreadyCachedEnabled) {
                    Use-CachedRangeImage $origImage "original sheet $i used range fallback"
                }
                else {
                    Save-RangeImage $origSheet $origRange $origImage "original sheet $i used range fallback" $captureRange
                }
                if ($shrunkAlreadyCachedEnabled) {
                    Use-CachedRangeImage $shrunkImage "shrunk sheet $i used range fallback"
                }
                else {
                    Save-RangeImage $shrunkSheet $shrunkRange $shrunkImage "shrunk sheet $i used range fallback" $captureRange
                }
            }

            [void]$captures.Add([pscustomobject]@{
                sheet_index = $i
                sheet_name = $origSheet.Name
                capture_range = $captureRange
                capture_strategy = $captureInfo.strategy
                used_range_error = $usedRangeError
                capture_info = $captureInfo
                original_image = $origImage
                shrunk_image = $shrunkImage
            })
            Write-ProgressLine "sheet captured index=$i strategy=$($captureInfo.strategy) range='$captureRange'"
            Write-ProgressLine "sheet done index=$i"
        }
        catch {
            $capturedOriginalImage = if (Test-Path $origImage) { $origImage } else { $null }
            $capturedShrunkImage = if (Test-Path $shrunkImage) { $shrunkImage } else { $null }
            $errorCaptureStrategy = if ($null -ne $captureInfo) { $captureInfo.strategy } else { $null }
            [void]$captures.Add([pscustomobject]@{
                sheet_index = $i
                sheet_name = $origSheet.Name
                capture_range = $captureRange
                capture_strategy = $errorCaptureStrategy
                used_range_error = $usedRangeError
                screenshot_error = $_.Exception.Message
                capture_info = $captureInfo
                original_image = $capturedOriginalImage
                shrunk_image = $capturedShrunkImage
            })
            Write-ProgressLine "sheet error index=$i error='$($_.Exception.Message)'"
        }
    }
}
finally {
    if ($original -ne $null) { $original.Close($false) }
    if ($shrunk -ne $null) { $shrunk.Close($false) }
    if ($excel -ne $null) { $excel.Quit() }
}

if ($captures.Count -eq 0) {
    '[]' | Set-Content -Path $CaptureManifestPath -Encoding UTF8
}
else {
    @($captures) | ConvertTo-Json -Depth 8 | Set-Content -Path $CaptureManifestPath -Encoding UTF8
}
Write-ProgressLine "screenshot capture finished captures=$($captures.Count) manifest='$CaptureManifestPath'"
"#;

#[cfg(test)]
mod tests {
    use super::*;

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

        assert!(compare_captured_screenshots(
            &capture_manifest_path,
            &report_path
        )?);
        assert!(report_path.exists());
        assert!(original_path.exists());
        assert!(shrunk_path.exists());

        let report = fs::read_to_string(&report_path)?;
        assert!(report.contains("diff_pixels"));
        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }
}
