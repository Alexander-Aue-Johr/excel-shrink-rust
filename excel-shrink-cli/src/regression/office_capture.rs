use super::office_plan::build_workbook_visual_plan;
use super::row_heights::{self, RowHeightDifference, SheetExtent, SheetHeights};
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

mod paired_capture;
pub(super) use paired_capture::compare_paired_visuals;

#[cfg(test)]
#[test]
fn preserves_numeric_browser_error_codes_and_messages() {
    let result: OfficeResult = serde_json::from_value(json!({"run_id":"test", "heights":[], "captures":[],
        "error":{"code":20,"message":"The operation was aborted"}})).unwrap();
    let error = result.error.unwrap();
    assert_eq!(error.code, 20);
    assert_eq!(error.message, "The operation was aborted");
}

const MAX_CAPTURE_COLUMN: u32 = 16_383;
const MAX_CAPTURE_ROW: u32 = 1_048_575;
const PROBE_VISIBLE_COLUMNS: u32 = 64;

#[derive(Debug, Clone)]
pub(super) struct PairedSheetPlan {
    pub sheet_index: usize,
    pub sheet_name: String,
    pub bottom_row: u32,
    row_height_bottom_row: u32,
    pub capture_right_column: u32,
    original_config: Value,
    shrunk_config: Value,
}

#[derive(Debug, Clone)]
pub(super) struct PairedPlan {
    pub sheets: Vec<PairedSheetPlan>,
    original_workbook: String,
    shrunk_workbook: String,
}

#[derive(Debug, Clone)]
pub(super) struct PairedImageTile {
    pub sheet_index: usize,
    pub sheet_name: String,
    pub range: String,
    pub tile_index: usize,
    pub original_image: PathBuf,
    pub shrunk_image: PathBuf,
}

#[derive(Debug, Deserialize)]
struct OfficeResult {
    run_id: String,
    heights: Vec<SheetHeights>,
    captures: Vec<ImageTile>,
    #[serde(default)]
    error: Option<OfficeError>,
}

#[derive(Debug, Deserialize)]
struct OfficeError {
    #[serde(default)]
    code: Value,
    message: String,
}

#[derive(Debug)]
struct ImageBudgetError(String);

impl fmt::Display for ImageBudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for ImageBudgetError {}

#[derive(Debug, Clone, Deserialize)]
struct ImageTile {
    sheet_index: usize,
    sheet_name: String,
    range: String,
    tile_index: usize,
    image_path: PathBuf,
}

type ImageTiles = BTreeMap<(usize, usize), ImageTile>;

pub(super) fn plan_pair(original: &Path, shrunk: &Path) -> Result<PairedPlan> {
    let left = build_workbook_visual_plan(original)?;
    let right = build_workbook_visual_plan(shrunk)?;
    ensure!(
        !left.sheets.is_empty(),
        "workbook has no visible worksheets"
    );
    ensure!(
        left.sheets.len() == right.sheets.len(),
        "visible worksheet count changed"
    );
    let mut sheets = Vec::with_capacity(left.sheets.len());
    for (a, b) in left.sheets.iter().zip(&right.sheets) {
        ensure!(
            a.sheet_index == b.sheet_index && a.sheet == b.sheet,
            "worksheet identity changed at index {}",
            a.sheet_index
        );
        let bottom_row = a.bottom_row.max(b.bottom_row);
        ensure!(
            bottom_row <= MAX_CAPTURE_ROW,
            "worksheet {} exceeds row capture limit",
            a.sheet
        );
        let mut capture_right_column = a.base_right_column.max(b.base_right_column);
        for side in [a, b] {
            if side.horizontal_overflow_required {
                capture_right_column = capture_right_column.max(probe_end(
                    side.base_right_column,
                    side.columns.default_width,
                    side.columns
                        .spans
                        .iter()
                        .map(|span| (span.min, span.max, span.width, span.hidden)),
                )?);
            }
        }
        ensure!(
            capture_right_column <= MAX_CAPTURE_COLUMN,
            "worksheet capture exceeds column limit"
        );
        let original_config = json!({
            "index": a.sheet_index,
            "name": a.sheet,
            "bottom_row": bottom_row,
            "base_right_column": a.base_right_column,
            "capture_right_column": capture_right_column,
            "horizontal_overflow_required": a.horizontal_overflow_required,
            "drawing_fallback_required": a.drawing_fallback_required,
            "columns": a.columns,
        });
        let shrunk_config = json!({
            "index": b.sheet_index,
            "name": b.sheet,
            "bottom_row": bottom_row,
            "base_right_column": b.base_right_column,
            "capture_right_column": capture_right_column,
            "horizontal_overflow_required": b.horizontal_overflow_required,
            "drawing_fallback_required": b.drawing_fallback_required,
            "columns": b.columns,
        });
        sheets.push(PairedSheetPlan {
            sheet_index: a.sheet_index,
            sheet_name: a.sheet.clone(),
            bottom_row,
            row_height_bottom_row: a.row_height_bottom_row.max(b.row_height_bottom_row),
            capture_right_column,
            original_config,
            shrunk_config,
        });
    }
    Ok(PairedPlan {
        sheets,
        original_workbook: left.workbook,
        shrunk_workbook: right.workbook,
    })
}

fn probe_end(
    base: u32,
    default_width: f64,
    spans: impl IntoIterator<Item = (u32, u32, Option<f64>, bool)>,
) -> Result<u32> {
    ensure!(
        default_width.is_finite() && default_width >= 0.0,
        "invalid default column width"
    );
    let mut widths = vec![default_width; MAX_CAPTURE_COLUMN as usize + 1];
    for (min, max, width, hidden) in spans {
        ensure!(
            min > 0 && min <= max && max <= MAX_CAPTURE_COLUMN + 1,
            "invalid capture column span"
        );
        let width = if hidden {
            0.0
        } else {
            width.unwrap_or(default_width)
        };
        ensure!(
            width.is_finite() && width >= 0.0,
            "invalid capture column width"
        );
        widths[(min - 1) as usize..max as usize].fill(width);
    }
    let mut visible = 0;
    for column in base.saturating_add(1)..=MAX_CAPTURE_COLUMN {
        if widths[column as usize] > 0.0 {
            visible += 1;
        }
        if visible == PROBE_VISIBLE_COLUMNS {
            return Ok(column);
        }
    }
    ensure!(
        visible > 0,
        "no visible columns remain after capture boundary"
    );
    Ok(MAX_CAPTURE_COLUMN)
}

pub(super) fn check_row_heights(
    plan: &PairedPlan,
    original: &Path,
    shrunk: &Path,
    output: &Path,
) -> Result<Vec<RowHeightDifference>> {
    let left = run_side(plan, original, output, "heights", "original", None, 32)?;
    let right = run_side(plan, shrunk, output, "heights", "shrunk", None, 32)?;
    ensure!(
        left.captures.is_empty() && right.captures.is_empty(),
        "height result contains images"
    );
    let expected = plan
        .sheets
        .iter()
        .map(|sheet| SheetExtent {
            index: sheet.sheet_index,
            name: sheet.sheet_name.clone(),
            last_row: sheet.bottom_row + 1,
            last_checked_row: sheet.row_height_bottom_row + 1,
        })
        .collect::<Vec<_>>();
    row_heights::compare_row_heights(&left.heights, &right.heights, &expected)
}

pub(super) fn capture_images(
    plan: &PairedPlan,
    original: &Path,
    shrunk: &Path,
    output: &Path,
    rows: Option<&BTreeMap<usize, Vec<u32>>>,
) -> Result<Vec<PairedImageTile>> {
    match capture_images_once(plan, original, shrunk, output, rows, 32) {
        Err(first) if first.downcast_ref::<ImageBudgetError>().is_some() => {
            capture_images_once(plan, original, shrunk, output, rows, 16)
                .with_context(|| format!("32 MP capture failed: {first:#}"))
        }
        result => result,
    }
}

fn capture_images_once(
    plan: &PairedPlan,
    original: &Path,
    shrunk: &Path,
    output: &Path,
    rows: Option<&BTreeMap<usize, Vec<u32>>>,
    tile_megapixels: u32,
) -> Result<Vec<PairedImageTile>> {
    let (mut left_tiles, mut right_tiles) =
        capture_tile_pair(plan, original, shrunk, output, rows, tile_megapixels)?;
    if !same_tile_ranges(&left_tiles, &right_tiles) {
        // Different widths in empty margins can change the pixel budget's row split.
        // Split at every boundary from either side so each shared tile fits both.
        let ranges = shared_tile_rows(&left_tiles, &right_tiles)?;
        let mut shared = plan.clone();
        shared.sheets.retain(|sheet| {
            let on_sheet = |tile: &&ImageTile| tile.sheet_index == sheet.sheet_index;
            let left = left_tiles.values().filter(on_sheet).map(|tile| &tile.range);
            let right = right_tiles
                .values()
                .filter(on_sheet)
                .map(|tile| &tile.range);
            !left.eq(right)
        });
        for sheet in &mut shared.sheets {
            if let Some(tile_rows) = ranges.get(&sheet.sheet_index) {
                sheet.original_config["tile_rows"] = json!(tile_rows);
                sheet.shrunk_config["tile_rows"] = json!(tile_rows);
            }
        }
        let selected_rows = rows.map(|rows| {
            shared
                .sheets
                .iter()
                .map(|sheet| (sheet.sheet_index, rows[&sheet.sheet_index].clone()))
                .collect::<BTreeMap<_, _>>()
        });
        let (left, right) = capture_tile_pair(
            &shared,
            original,
            shrunk,
            output,
            selected_rows.as_ref(),
            tile_megapixels,
        )?;
        let recaptured = shared
            .sheets
            .iter()
            .map(|sheet| sheet.sheet_index)
            .collect::<BTreeSet<_>>();
        left_tiles.retain(|(index, _), _| !recaptured.contains(index));
        right_tiles.retain(|(index, _), _| !recaptured.contains(index));
        left_tiles.extend(left);
        right_tiles.extend(right);
    }
    ensure!(
        same_tile_ranges(&left_tiles, &right_tiles),
        "paired image geometry differs after using shared row boundaries"
    );
    let mut paired = Vec::with_capacity(left_tiles.len());
    for (key, a) in left_tiles {
        let b = &right_tiles[&key];
        paired.push(PairedImageTile {
            sheet_index: a.sheet_index,
            sheet_name: a.sheet_name,
            range: a.range,
            tile_index: a.tile_index,
            original_image: a.image_path,
            shrunk_image: b.image_path.clone(),
        });
    }
    Ok(paired)
}

fn same_tile_ranges(left: &ImageTiles, right: &ImageTiles) -> bool {
    left.len() == right.len()
        && left.iter().all(|(key, a)| {
            right
                .get(key)
                .is_some_and(|b| a.range == b.range && a.sheet_name == b.sheet_name)
        })
}

fn shared_tile_rows(
    left: &ImageTiles,
    right: &ImageTiles,
) -> Result<BTreeMap<usize, Vec<[u32; 2]>>> {
    let mut spans = BTreeMap::<usize, [Vec<(u32, u32)>; 2]>::new();
    for (side, tiles) in [left, right].into_iter().enumerate() {
        for tile in tiles.values() {
            let (first, last, _, _) = parse_range(&tile.range)?;
            spans.entry(tile.sheet_index).or_default()[side].push((first - 1, last));
        }
    }
    let mut shared = BTreeMap::new();
    for (index, sides) in spans {
        let boundaries = sides
            .iter()
            .flatten()
            .flat_map(|&(first, last)| [first, last])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut ranges = Vec::new();
        for pair in boundaries.windows(2) {
            let covered = |side: usize| {
                sides[side]
                    .iter()
                    .any(|&(a, b)| a <= pair[0] && pair[1] <= b)
            };
            ensure!(
                covered(0) == covered(1),
                "paired image row coverage differs on sheet {index}"
            );
            if covered(0) {
                ranges.push([pair[0], pair[1]]);
            }
        }
        shared.insert(index, ranges);
    }
    Ok(shared)
}

fn capture_tile_pair(
    plan: &PairedPlan,
    original: &Path,
    shrunk: &Path,
    output: &Path,
    rows: Option<&BTreeMap<usize, Vec<u32>>>,
    tile_megapixels: u32,
) -> Result<(ImageTiles, ImageTiles)> {
    let selected = selected_sheets(plan, rows)?;
    let left = run_side(
        plan,
        original,
        output,
        "capture",
        "original",
        rows,
        tile_megapixels,
    )?;
    let right = run_side(
        plan,
        shrunk,
        output,
        "capture",
        "shrunk",
        rows,
        tile_megapixels,
    )?;
    ensure!(
        left.heights.is_empty() && right.heights.is_empty(),
        "image result contains row heights"
    );
    let left_dir = left.output_dir;
    let right_dir = right.output_dir;
    let left_tiles = validate_tiles(left.captures, &selected, rows, &left_dir)?;
    let right_tiles = validate_tiles(right.captures, &selected, rows, &right_dir)?;
    Ok((left_tiles, right_tiles))
}

fn selected_sheets<'a>(
    plan: &'a PairedPlan,
    rows: Option<&BTreeMap<usize, Vec<u32>>>,
) -> Result<Vec<&'a PairedSheetPlan>> {
    if let Some(rows) = rows {
        ensure!(!rows.is_empty(), "no rows selected for capture");
        for (&index, selected) in rows {
            let sheet = plan
                .sheets
                .iter()
                .find(|s| s.sheet_index == index)
                .with_context(|| format!("unknown selected sheet {index}"))?;
            ensure!(
                !selected.is_empty()
                    && selected
                        .iter()
                        .all(|&row| row >= 1 && row <= sheet.bottom_row + 1),
                "invalid selected rows for sheet {index}"
            );
        }
        Ok(plan
            .sheets
            .iter()
            .filter(|s| rows.contains_key(&s.sheet_index))
            .collect())
    } else {
        Ok(plan.sheets.iter().collect())
    }
}

struct SideResult {
    output_dir: PathBuf,
    heights: Vec<SheetHeights>,
    captures: Vec<ImageTile>,
}

struct CaptureTransfer(PathBuf);

impl CaptureTransfer {
    fn new(workbook: &Path, config: &Path) -> Result<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::var_os("EXCEL_SHRINK_TRANSFER_ROOT")
            .map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let path = root.join(format!("es-capture-{}-{stamp}", std::process::id()));
        fs::create_dir(&path)?;
        let transfer = Self(path);
        fs::copy(workbook, transfer.0.join("workbook.xlsx"))?;
        fs::copy(config, transfer.0.join("config.json"))?;
        fs::create_dir(transfer.0.join("output"))?;
        Ok(transfer)
    }

    fn publish(&self, destination: &Path) -> Result<()> {
        let source = self.0.join("output");
        let images = source.join("images");
        if images.is_dir() {
            let target = destination.join("images");
            fs::create_dir_all(&target)?;
            for image in fs::read_dir(images)? {
                let image = image?;
                fs::copy(image.path(), target.join(image.file_name()))?;
            }
        }
        for name in [
            "host-build.stdout.log",
            "host-build.stderr.log",
            "host.stdout.log",
            "host.stderr.log",
            "result.json",
        ] {
            let path = source.join(name);
            if path.is_file() {
                fs::copy(path, destination.join(name))?;
            }
        }
        Ok(())
    }
}

impl Drop for CaptureTransfer {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_side(
    plan: &PairedPlan,
    workbook: &Path,
    output_root: &Path,
    mode: &str,
    side: &str,
    rows: Option<&BTreeMap<usize, Vec<u32>>>,
    tile_megapixels: u32,
) -> Result<SideResult> {
    let selected = selected_sheets(plan, rows)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let output_dir = output_root.join(format!("{mode}-{side}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&output_dir)?;
    let mut sheets = Vec::with_capacity(selected.len());
    for sheet in selected {
        let mut config = if side == "original" {
            sheet.original_config.clone()
        } else {
            sheet.shrunk_config.clone()
        };
        if let Some(rows) = rows {
            let mut selected_rows = rows[&sheet.sheet_index].clone();
            selected_rows.sort_unstable();
            selected_rows.dedup();
            config["rows"] = json!(selected_rows);
        }
        sheets.push(config);
    }
    let workbook_name = if side == "original" {
        &plan.original_workbook
    } else {
        &plan.shrunk_workbook
    };
    let config_path = output_dir.join("config.json");
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&json!({
            "mode": mode,
            "tile_megapixels": tile_megapixels,
            "workbook": workbook_name,
            "sheets": sheets,
        }))?,
    )?;
    let launched = run_power_shell(workbook, &config_path, &output_dir);
    let result_path = output_dir.join("result.json");
    if !result_path.exists() {
        launched?;
        bail!("capture result missing: {}", result_path.display());
    }
    let result: OfficeResult = serde_json::from_slice(
        &fs::read(&result_path)
            .with_context(|| format!("capture result missing: {}", result_path.display()))?,
    )?;
    ensure!(
        !result.run_id.is_empty(),
        "capture result has no run identifier"
    );
    if let Some(error) = result.error {
        if mode == "capture"
            && error.code == "IMAGE_BUDGET"
            && (error
                .message
                .starts_with("Image exceeds the capture budget")
                || error.message.starts_with("Image exceeds the tile budget"))
        {
            return Err(ImageBudgetError(error.message).into());
        }
        bail!("worksheet capture failed: {}", error.message);
    }
    launched?;
    Ok(SideResult {
        output_dir,
        heights: result.heights,
        captures: result.captures,
    })
}

fn run_power_shell(workbook: &Path, config: &Path, output: &Path) -> Result<()> {
    let script = super::repo_root().join("excel-shrink-cli/office-capture/run.ps1");
    ensure!(
        script.is_file(),
        "worksheet capture launcher missing: {}",
        script.display()
    );
    // PowerShell 5 file operations reject long UNC paths. Only Rust accesses
    // the report directory; Windows receives short transfer paths.
    let transfer = CaptureTransfer::new(workbook, config)?;
    let result = Command::new("powershell.exe")
        .args([
            "-STA",
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(windows_path(&script)?)
        .arg("-WorkbookPath")
        .arg(windows_path(&transfer.0.join("workbook.xlsx"))?)
        .arg("-ConfigPath")
        .arg(windows_path(&transfer.0.join("config.json"))?)
        .arg("-OutputDir")
        .arg(windows_path(&transfer.0.join("output"))?)
        .arg("-PublishedOutputDir")
        .arg(windows_path(output)?)
        .args(capture_timeout_args())
        .output()
        .context("start Windows PowerShell for worksheet capture")?;
    transfer
        .publish(output)
        .context("copy worksheet capture output")?;
    if !result.status.success() {
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        bail!(
            "worksheet capture failed ({}): {} {}",
            result.status,
            stdout.chars().take(4000).collect::<String>(),
            stderr.chars().take(4000).collect::<String>()
        );
    }
    Ok(())
}

fn windows_path(path: &Path) -> Result<String> {
    #[cfg(windows)]
    {
        super::excel_automation::power_shell_path(path)
    }
    #[cfg(not(windows))]
    {
        let path = path.canonicalize()?;
        let output = Command::new("wslpath")
            .arg("-w")
            .arg(&path)
            .output()
            .context("convert capture path for Windows")?;
        ensure!(
            output.status.success(),
            "wslpath could not convert {}",
            path.display()
        );
        let converted = String::from_utf8(output.stdout)?.trim().to_owned();
        ensure!(!converted.is_empty(), "wslpath returned an empty path");
        Ok(converted)
    }
}

fn validate_image_path(
    path: &Path,
    output_dir: &Path,
    sheet_index: usize,
    tile_index: usize,
) -> Result<PathBuf> {
    let receipt = path
        .to_str()
        .context("capture image path is not UTF-8")?
        .replace('\\', "/");
    let drive_path = receipt
        .as_bytes()
        .get(0)
        .is_some_and(u8::is_ascii_alphabetic)
        && receipt.as_bytes().get(1) == Some(&b':')
        && receipt.as_bytes().get(2) == Some(&b'/');
    ensure!(
        receipt.starts_with("//") || drive_path,
        "capture image path is not an absolute Windows path"
    );
    let parts = receipt
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    ensure!(
        parts.iter().all(|part| *part != "." && *part != ".."),
        "invalid capture image path"
    );
    ensure!(
        parts.len() >= 3 && parts[parts.len() - 2] == "images",
        "invalid capture image directory"
    );
    let name = parts[parts.len() - 1];
    ensure!(
        name == format!("s{sheet_index:04}-t{tile_index:05}.png"),
        "capture image filename does not match its sheet and tile"
    );
    let root = fs::canonicalize(output_dir)?;
    let images = fs::canonicalize(output_dir.join("images"))?;
    ensure!(
        images.starts_with(&root),
        "capture image directory is outside its output directory"
    );
    let actual = fs::canonicalize(output_dir.join("images").join(name))
        .with_context(|| format!("captured image missing: {name}"))?;
    ensure!(
        actual.starts_with(&images),
        "captured image is outside its output directory"
    );
    ensure!(
        fs::metadata(&actual)?.len() <= 160 * 1024 * 1024,
        "captured image is too large"
    );
    let reader = image::ImageReader::open(&actual)?.with_guessed_format()?;
    ensure!(
        reader.format() == Some(image::ImageFormat::Png),
        "captured image is not PNG"
    );
    let decoded = reader.decode().context("decode captured PNG")?;
    let width = decoded.width();
    let height = decoded.height();
    ensure!(
        width > 0
            && height > 0
            && width <= 30_000
            && height <= 30_000
            && u64::from(width) * u64::from(height) <= 32_000_000,
        "captured image exceeds image bounds"
    );
    Ok(actual)
}

fn validate_tiles(
    captures: Vec<ImageTile>,
    selected: &[&PairedSheetPlan],
    rows: Option<&BTreeMap<usize, Vec<u32>>>,
    output_dir: &Path,
) -> Result<BTreeMap<(usize, usize), ImageTile>> {
    let sheets: BTreeMap<_, _> = selected
        .iter()
        .map(|sheet| (sheet.sheet_index, *sheet))
        .collect();
    let mut tiles = BTreeMap::new();
    for mut capture in captures {
        let sheet = sheets
            .get(&capture.sheet_index)
            .with_context(|| format!("unexpected image sheet {}", capture.sheet_index))?;
        ensure!(
            capture.sheet_name == sheet.sheet_name,
            "captured sheet name differs"
        );
        let (first_row, last_row, first_column, last_column) = parse_range(&capture.range)?;
        ensure!(
            first_column == 1
                && last_column == sheet.capture_right_column + 1
                && first_row >= 1
                && first_row <= last_row
                && last_row <= sheet.bottom_row + 1,
            "captured range exceeds planned worksheet bounds"
        );
        capture.image_path = validate_image_path(
            &capture.image_path,
            output_dir,
            capture.sheet_index,
            capture.tile_index,
        )?;
        ensure!(
            tiles
                .insert((capture.sheet_index, capture.tile_index), capture)
                .is_none(),
            "duplicate image tile"
        );
    }
    validate_tile_coverage(&tiles, selected, rows)?;
    Ok(tiles)
}

fn validate_tile_coverage(
    tiles: &ImageTiles,
    selected: &[&PairedSheetPlan],
    rows: Option<&BTreeMap<usize, Vec<u32>>>,
) -> Result<()> {
    for sheet in selected {
        let mut next_index = 0;
        let mut previous_row = 0;
        let mut covered = Vec::new();
        for ((index, tile_index), tile) in tiles
            .iter()
            .filter(|((index, _), _)| *index == sheet.sheet_index)
        {
            ensure!(
                *index == sheet.sheet_index && *tile_index == next_index,
                "image tile index gap"
            );
            let (first, last, _, _) = parse_range(&tile.range)?;
            ensure!(first > previous_row, "image tiles overlap or are unordered");
            if rows.is_none() {
                ensure!(first == previous_row + 1, "image tile coverage has a gap");
            }
            previous_row = last;
            covered.push((first, last));
            next_index += 1;
        }
        ensure!(
            next_index > 0,
            "no image tiles for sheet {}",
            sheet.sheet_index
        );
        if let Some(rows) = rows {
            for &row in &rows[&sheet.sheet_index] {
                ensure!(
                    covered
                        .iter()
                        .any(|&(first, last)| first <= row && row <= last),
                    "selected row {row} was not captured on sheet {}",
                    sheet.sheet_index
                );
            }
        } else {
            ensure!(
                previous_row == sheet.bottom_row + 1,
                "image tiles do not reach the last row"
            );
        }
    }
    Ok(())
}

fn parse_range(text: &str) -> Result<(u32, u32, u32, u32)> {
    let (first, last) = text
        .split_once(':')
        .context("capture range has no end cell")?;
    ensure!(!last.contains(':'), "capture range has multiple separators");
    let (first_row, first_column) = parse_cell(first)?;
    let (last_row, last_column) = parse_cell(last)?;
    ensure!(
        first_row <= last_row && first_column <= last_column,
        "capture range is reversed"
    );
    Ok((first_row, last_row, first_column, last_column))
}

fn parse_cell(cell: &str) -> Result<(u32, u32)> {
    let mut column = 0u32;
    let mut offset = 0;
    for byte in cell.bytes() {
        if !byte.is_ascii_uppercase() {
            break;
        }
        column = column
            .checked_mul(26)
            .and_then(|value| value.checked_add(u32::from(byte - b'A' + 1)))
            .context("capture column overflow")?;
        offset += 1;
    }
    ensure!(
        offset > 0 && offset < cell.len() && column <= 16_384,
        "invalid capture column"
    );
    let row_text = &cell[offset..];
    ensure!(
        !row_text.starts_with('0') && row_text.bytes().all(|byte| byte.is_ascii_digit()),
        "invalid capture row"
    );
    let row = row_text.parse::<u32>()?;
    ensure!(
        (1..=1_048_576).contains(&row),
        "capture row outside worksheet"
    );
    Ok((row, column))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiles(ranges: &[&str]) -> ImageTiles {
        ranges
            .iter()
            .enumerate()
            .map(|(index, range)| {
                (
                    (8, index),
                    ImageTile {
                        sheet_index: 8,
                        sheet_name: "AL-Beschäftigung".into(),
                        range: (*range).into(),
                        tile_index: index,
                        image_path: PathBuf::new(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn shared_tiles_split_at_boundaries_from_both_workbooks() {
        let left = tiles(&["A1:BT386", "A387:BT766", "A767:BT873"]);
        let right = tiles(&["A1:BT387", "A388:BT768", "A769:BT873"]);
        assert!(!same_tile_ranges(&left, &right));
        assert_eq!(
            shared_tile_rows(&left, &right).unwrap()[&8],
            vec![[0, 386], [386, 387], [387, 766], [766, 768], [768, 873]]
        );
    }

    #[test]
    fn shared_tiles_keep_gaps_between_selected_row_groups() {
        let left = tiles(&["A2:C5", "A100:C106"]);
        let right = tiles(&["A2:C3", "A4:C5", "A100:C106"]);
        assert_eq!(
            shared_tile_rows(&left, &right).unwrap()[&8],
            vec![[1, 3], [3, 5], [99, 106]]
        );
        assert!(same_tile_ranges(&left, &left));
    }

    #[test]
    fn shared_tiles_reject_different_row_coverage() {
        assert!(shared_tile_rows(&tiles(&["A1:C2"]), &tiles(&["A1:C3"])).is_err());
    }

    #[test]
    fn capture_transfer_keeps_windows_io_out_of_long_report_paths() -> Result<()> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!("capture-transfer-test-{stamp}"));
        fs::create_dir(&root)?;
        let _cleanup = CaptureTransfer(root.clone());
        let directory = root.join("profile".repeat(20)).join("case".repeat(30));
        fs::create_dir_all(&directory)?;
        let workbook = directory.join("source.xlsx");
        let config = directory.join("config.json");
        fs::write(&workbook, b"workbook bytes")?;
        fs::write(&config, b"{\"mode\":\"capture\"}")?;
        let destination = directory.join("captures");
        fs::create_dir(&destination)?;
        assert!(destination.as_os_str().len() > 260);

        let transfer = CaptureTransfer::new(&workbook, &config)?;
        assert!(!transfer.0.starts_with(&directory));
        assert_eq!(
            fs::read(transfer.0.join("workbook.xlsx"))?,
            b"workbook bytes"
        );
        assert_eq!(
            fs::read(transfer.0.join("config.json"))?,
            fs::read(&config)?
        );
        let output = transfer.0.join("output");
        fs::write(output.join("host.stderr.log"), b"host diagnostic")?;
        transfer.publish(&destination)?;
        assert_eq!(
            fs::read(destination.join("host.stderr.log"))?,
            b"host diagnostic"
        );
        assert!(!destination.join("result.json").exists());

        fs::create_dir(output.join("images"))?;
        fs::write(output.join("images/s0001-t00000.png"), b"image bytes")?;
        fs::write(output.join("result.json"), b"{\"run_id\":\"completed\"}")?;
        transfer.publish(&destination)?;
        assert_eq!(
            fs::read(destination.join("images/s0001-t00000.png"))?,
            b"image bytes"
        );
        assert_eq!(
            fs::read(destination.join("result.json"))?,
            b"{\"run_id\":\"completed\"}"
        );
        let transfer_path = transfer.0.clone();
        drop(transfer);
        assert!(!transfer_path.exists());
        assert_eq!(fs::read(&workbook)?, b"workbook bytes");
        Ok(())
    }

    #[test]
    fn capture_range_requires_a1_cells_within_the_grid() {
        assert_eq!(
            parse_range("A1:XFD1048576").unwrap(),
            (1, 1_048_576, 1, 16_384)
        );
        for range in ["A0:B1", "A1:XFE1", "a1:B2", "A1:A1:B2", "B2:A1", "A01:B2"] {
            assert!(parse_range(range).is_err(), "{range}");
        }
    }

    #[test]
    fn selected_tiles_cover_every_requested_row() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let output =
            std::env::temp_dir().join(format!("office-capture-{}-{stamp}", std::process::id()));
        let images = output.join("images");
        fs::create_dir_all(&images).unwrap();
        let name = "s0001-t00000.png";
        image::RgbImage::new(1, 1).save(images.join(name)).unwrap();
        let capture = |range: &str| ImageTile {
            sheet_index: 1,
            sheet_name: "Sheet 1".into(),
            range: range.into(),
            tile_index: 0,
            image_path: PathBuf::from(format!(r"\\wsl.localhost\Ubuntu\images\{name}")),
        };
        let sheet = PairedSheetPlan {
            sheet_index: 1,
            sheet_name: "Sheet 1".into(),
            bottom_row: 2,
            row_height_bottom_row: 2,
            capture_right_column: 0,
            original_config: Value::Null,
            shrunk_config: Value::Null,
        };
        let selected = [&sheet];
        assert!(validate_tiles(vec![capture("A1:A3")], &selected, None, &output).is_ok());
        assert!(validate_tiles(vec![capture("A1:A2")], &selected, None, &output).is_err());
        let rows = BTreeMap::from([(1, vec![3])]);
        assert!(validate_tiles(vec![capture("A1:A2")], &selected, Some(&rows), &output).is_err());
        assert!(validate_tiles(vec![capture("A2:A3")], &selected, Some(&rows), &output).is_ok());
        fs::remove_dir_all(output).unwrap();
    }
}

fn capture_timeout_args() -> Vec<String> {
    let mut args = Vec::new();
    for (variable, flag, low, high) in [
        ("EXCEL_SHRINK_CAPTURE_STARTUP_SECONDS", "-StartupTimeoutSeconds", 10, 600),
        ("EXCEL_SHRINK_CAPTURE_TIMEOUT_SECONDS", "-CaptureTimeoutSeconds", 10, 7200),
    ] {
        if let Some(seconds) = std::env::var(variable).ok().and_then(|v| v.parse::<u64>().ok()) {
            args.extend([flag.to_owned(), seconds.clamp(low, high).to_string()]);
        }
    }
    args
}
