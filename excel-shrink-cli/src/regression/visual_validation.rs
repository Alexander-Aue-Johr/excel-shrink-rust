use super::excel_automation::ExcelScreenshotDifferenceSummary;
use super::office_capture::{
    capture_images, check_row_heights, compare_paired_visuals, plan_pair, PairedImageTile,
};
use super::row_heights::RowHeightDifference;
use super::visual_compare::{compare_images, ImageDifference};
use super::visual_report::write_visual_report;
use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub(super) struct ExcelScreenshotReport {
    pub report_path: PathBuf,
    pub html_report_path: PathBuf,
    pub output_dir: PathBuf,
    pub differences_found: bool,
    pub difference_summary: ExcelScreenshotDifferenceSummary,
}

#[derive(Debug, Clone, Default)]
pub(super) struct VisualCheckOptions {
    pub profile: String,
    pub sheets: Option<std::collections::BTreeSet<usize>>,
    pub keep_screenshots: bool,
    pub combined_report: Option<PathBuf>,
}

pub(super) fn compare_sheet_visuals(
    original: &Path,
    shrunk: &Path,
    output_dir: &Path,
    options: &VisualCheckOptions,
) -> Result<ExcelScreenshotReport> {
    fs::create_dir_all(output_dir)?;
    let mut paired_plan = plan_pair(original, shrunk)?;
    if let Some(sheets) = &options.sheets {
        paired_plan
            .sheets
            .retain(|sheet| sheets.contains(&sheet.sheet_index));
        anyhow::ensure!(
            paired_plan.sheets.len() == sheets.len(),
            "requested visual worksheets are missing"
        );
    }
    anyhow::ensure!(
        !paired_plan.sheets.is_empty(),
        "no worksheets selected for visual comparison"
    );
    let (changed_heights, tiles, image_differences) = if options.keep_screenshots {
        capture_stored_images(&paired_plan, original, shrunk, output_dir)?
    } else {
        let result =
            compare_paired_visuals(&paired_plan, original, shrunk, &output_dir.join("images"))?;
        (result.heights, result.tiles, result.differences)
    };
    let sheets = visual_sheets(
        &paired_plan.sheets,
        &changed_heights,
        &tiles,
        &image_differences,
    )?;
    let report = json!({
        "profile": options.profile,
        "workbook": original.file_name().and_then(|name| name.to_str()).unwrap_or("Workbook"),
        "image_comparison": if changed_heights.is_empty() { "complete" } else { "complete on sheets with equal row heights; changed rows captured as evidence" },
        "sheets": sheets,
    });
    let report_path = output_dir.join("screenshot-differences.json");
    fs::write(
        &report_path,
        serde_json::to_vec_pretty(&measurement_differences(
            &changed_heights,
            &tiles,
            &image_differences,
        )?)?,
    )?;
    fs::write(
        output_dir.join("visual-result.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    let html_report_path = match &options.combined_report {
        Some(path) => path.clone(),
        None => write_visual_report(output_dir, &report)?,
    };

    let changed_rows: usize = changed_heights
        .iter()
        .map(|change| (change.end_row - change.row + 1) as usize)
        .sum();
    let total_differences = changed_rows + image_differences.len();
    let summary = ExcelScreenshotDifferenceSummary {
        total_differences,
        row_height_differences: changed_rows,
        entropy_changed_differences: image_differences.len(),
        diff_pixels: image_differences
            .iter()
            .map(|difference| difference.diff_pixels)
            .sum(),
        ..ExcelScreenshotDifferenceSummary::default()
    };
    Ok(ExcelScreenshotReport {
        report_path,
        html_report_path,
        output_dir: output_dir.to_owned(),
        differences_found: total_differences > 0,
        difference_summary: summary,
    })
}

// Full screenshot retention keeps the disk-backed diagnostic path.
fn capture_stored_images(
    paired_plan: &super::office_capture::PairedPlan,
    original: &Path,
    shrunk: &Path,
    output_dir: &Path,
) -> Result<(
    Vec<RowHeightDifference>,
    Vec<PairedImageTile>,
    Vec<ImageDifference>,
)> {
    let captures = output_dir.join("captures");
    let changed_heights = check_row_heights(
        paired_plan,
        original,
        shrunk,
        &output_dir.join("row-heights"),
    )?;
    let mut rows = BTreeMap::<usize, Vec<u32>>::new();
    for change in &changed_heights {
        rows.entry(change.sheet_index).or_default().push(change.row);
    }
    let mut tiles = Vec::new();
    let mut image_differences = Vec::new();
    // A changed height stops full-image checks on that sheet. Other sheets
    // still receive their image check and can reveal independent differences.
    for height_evidence in [false, true] {
        let mut selected = paired_plan.clone();
        selected
            .sheets
            .retain(|sheet| rows.contains_key(&sheet.sheet_index) == height_evidence);
        if selected.sheets.is_empty() {
            continue;
        }
        let selected_rows = height_evidence.then_some(&rows);
        let phase_output = output_dir.join(if height_evidence {
            "height-images"
        } else {
            "images"
        });
        let (captured, differences) = {
            let captured = capture_images(
                &selected,
                original,
                shrunk,
                &captures.join(if height_evidence { "heights" } else { "full" }),
                selected_rows,
            )?;
            let differences = if height_evidence {
                Vec::new()
            } else {
                compare_images(&captured, &phase_output.join("differences"))?
            };
            (captured, differences)
        };
        tiles.extend(captured);
        image_differences.extend(differences);
    }
    Ok((changed_heights, tiles, image_differences))
}

fn visual_sheets(
    sheets: &[super::office_capture::PairedSheetPlan],
    changed_heights: &[RowHeightDifference],
    tiles: &[PairedImageTile],
    image_differences: &[ImageDifference],
) -> Result<Vec<Value>> {
    let mut output = Vec::with_capacity(sheets.len());
    for sheet in sheets {
        let mut height_items = Vec::new();
        for change in changed_heights
            .iter()
            .filter(|change| change.sheet_index == sheet.sheet_index)
        {
            let tile = tile_for_row(tiles, sheet.sheet_index, change.row)?;
            height_items.push(json!({
                "row": change.row, "end_row": change.end_row,
                "before": change.before, "after": change.after,
                "original_image": tile.original_image,
                "shrunk_image": tile.shrunk_image,
            }));
        }
        let image_items: Vec<_> = image_differences
            .iter()
            .filter(|difference| difference.sheet_index == sheet.sheet_index)
            .map(|difference| {
                json!({
                    "range": difference.range,
                    "region": difference.region,
                    "original_image": difference.original_image,
                    "shrunk_image": difference.shrunk_image,
                    "diff_image": difference.diff_image,
                })
            })
            .collect();
        let status = if !height_items.is_empty() || !image_items.is_empty() {
            "changed"
        } else {
            "same"
        };
        output.push(json!({
            "index": sheet.sheet_index, "name": sheet.sheet_name,
            "status": status,
            "row_heights": height_items,
            "image_differences": image_items,
        }));
    }
    Ok(output)
}

fn tile_for_row(
    tiles: &[PairedImageTile],
    sheet_index: usize,
    row: u32,
) -> Result<&PairedImageTile> {
    tiles
        .iter()
        .find(|tile| {
            tile.sheet_index == sheet_index
                && range_rows(&tile.range).is_some_and(|(start, end)| start <= row && row <= end)
        })
        .ok_or_else(|| anyhow::anyhow!("no image covers changed row {row} on sheet {sheet_index}"))
}

fn range_rows(range: &str) -> Option<(u32, u32)> {
    let (first, last) = range.split_once(':')?;
    fn row(cell: &str) -> Option<u32> {
        let digits = cell.trim_start_matches(|c: char| c.is_ascii_alphabetic());
        (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| digits.parse().ok())
            .flatten()
    }
    Some((row(first)?, row(last)?))
}

fn measurement_differences(
    heights: &[RowHeightDifference],
    tiles: &[PairedImageTile],
    images: &[ImageDifference],
) -> Result<Vec<Value>> {
    let mut items = Vec::with_capacity(heights.len() + images.len());
    for change in heights {
        let tile = tile_for_row(tiles, change.sheet_index, change.row)?;
        items.push(json!({
            "kind": "row_height", "sheet_index": change.sheet_index,
            "sheet_name": change.sheet_name,
            "row": change.row, "end_row": change.end_row,
            "before": change.before, "after": change.after,
            "original_image": tile.original_image, "shrunk_image": tile.shrunk_image,
        }));
    }
    for change in images {
        items.push(json!({
            "kind": "image", "sheet_index": change.sheet_index,
            "sheet_name": change.sheet_name, "tile_index": change.tile_index,
            "capture_range": change.range, "diff_pixel_count": change.diff_pixels,
            "original_crop": change.original_image,
            "shrunk_crop": change.shrunk_image,
            "diff_pixels": change.diff_image,
        }));
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::{measurement_differences, range_rows, PairedImageTile, RowHeightDifference};
    use std::path::PathBuf;

    #[test]
    fn parses_capture_rows() {
        assert_eq!(range_rows("A1:ZZ1048576"), Some((1, 1_048_576)));
        assert_eq!(range_rows("A1:ZZ0"), Some((1, 0)));
        assert_eq!(range_rows("A1"), None);
    }

    #[test]
    fn changed_height_requires_an_image_of_its_row() {
        let change = RowHeightDifference {
            sheet_index: 1,
            sheet_name: "Sheet 1".into(),
            row: 42,
            end_row: 42,
            before: 15.0,
            after: 18.0,
        };
        let tile = PairedImageTile {
            sheet_index: 1,
            sheet_name: "Sheet 1".into(),
            range: "A41:F43".into(),
            tile_index: 0,
            original_image: PathBuf::from("before.png"),
            shrunk_image: PathBuf::from("after.png"),
        };
        assert_eq!(
            measurement_differences(&[change.clone()], &[tile.clone()], &[])
                .unwrap()
                .len(),
            1
        );
        assert!(measurement_differences(
            &[change],
            &[PairedImageTile {
                range: "A1:F3".into(),
                ..tile
            }],
            &[]
        )
        .is_err());
    }
}
