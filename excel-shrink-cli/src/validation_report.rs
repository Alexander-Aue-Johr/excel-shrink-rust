//! Durable comparison evidence and a static Observatory view.
//!
//! Scientific verdicts remain those of the targeted comparison. Packaging owns
//! image retention and relative paths; the viewer has no dependency on Excel,
//! processing programs, live telemetry, or the original workbook files.

mod evidence;
mod publication;
mod replay;
mod site;
mod merge;
mod atlas;
mod inventory;

pub(crate) use inventory::read as read_workbook_inventory;
pub(crate) use atlas::label_atlas;
pub(crate) use atlas::set_cohort;
pub(crate) use atlas::live_asset;

pub(crate) use site::{export_selected_site, export_site, write_pair_page, write_site};
pub(crate) use merge::merge_reports;

use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const FORMAT_VERSION: u32 = 1;
const MAX_REPORT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_DIAGNOSTIC_PIXELS: u64 = 32_000_000;
const MAX_CACHED_DIAGNOSTIC_PAIRS: usize = 128;
static PIXEL_INSPECTION: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One independently retained comparison. Counts describe observed regions,
/// including separate state pairs; they are not a whole-workbook verdict.
#[derive(Debug)]
pub(crate) struct PackagedComparison {
    pub report_path: PathBuf,
    pub image_count: usize,
    pub different_regions: usize,
}

/// Retain a targeted result in a new directory before its working copies are
/// removed. Only explicitly referenced images and the report are copied.
///
/// A completed report takes precedence over the last intermediate snapshot;
/// an explicit failure selects the partial report. Missing or out-of-tree images fail publication, including for
/// partial reports. An already packaged comparison is verified before copying.
pub(crate) fn package_comparison(
    source_directory: &Path,
    destination: &Path,
) -> Result<PackagedComparison> {
    if source_directory.join("bundle.json").is_file() {
        return evidence::copy_verified_package(source_directory, destination);
    }
    let (mut report, source_name) = read_source_report(source_directory)?;
    let mut images = evidence::ImageStore::create(source_directory, destination)?;
    images.retain_report_images(&mut report)?;
    let diagnostics = inspect_differences(&report, &mut images)?;
    report["portable_report"] = json!({
        "version": FORMAT_VERSION,
        "source_report": source_name,
        "different_regions": difference_count(&report),
        "diagnostics": diagnostics,
        "failure": read_failure(source_directory)?,
        "note": "Pixel diagnostics describe retained captures. They do not change the scientific comparison verdict or extend its coverage."
    });
    images.publish(&report)
}

/// Verify every retained image and the immutable report against its package
/// manifest. Use this before reusing a previous comparison or deleting scratch
/// evidence. This does not revalidate the scientific selection policy.
pub(crate) fn verify_comparison(directory: &Path) -> Result<()> {
    evidence::verify_package(directory).map(|_| ())
}

fn read_source_report(directory: &Path) -> Result<(Value, &'static str)> {
    let failed = directory.join("failure.json").try_exists()?;
    let partial_exists = directory.join("partial-report.json").is_file();
    // The legacy writer can be interrupted while writing the final JSON. An
    // explicit failure selects its intact partial snapshot without parsing that
    // abandoned final file; a damaged successful report still fails closed.
    let completed = if !(failed && partial_exists) && directory.join("report.json").is_file() {
        Some(read_json_file(&directory.join("report.json"))?)
    } else {
        None
    };
    let name = if completed
        .as_ref()
        .is_some_and(|report| report["status"] == "completed_with_limitations")
        && !failed
    {
        "report.json"
    } else if partial_exists {
        "partial-report.json"
    } else {
        "report.json"
    };
    let report = if name == "report.json" {
        completed.context("Comparison report is missing")?
    } else {
        read_json_file(&directory.join(name))?
    };
    ensure!(report.is_object(), "Comparison report must be an object");
    ensure!(
        report["status"].is_string(),
        "Comparison report has no status"
    );
    ensure!(
        report["comparisons"].is_array(),
        "Comparison report has no comparison list"
    );
    Ok((report, name))
}

fn read_failure(directory: &Path) -> Result<Option<Value>> {
    let path = directory.join("failure.json");
    if path.try_exists()? {
        Ok(Some(read_json_file(&path)?))
    } else {
        Ok(None)
    }
}

fn read_json_file(path: &Path) -> Result<Value> {
    let file = fs::File::open(path).with_context(|| format!("Open report {}", path.display()))?;
    ensure!(
        file.metadata()?.len() <= MAX_REPORT_BYTES,
        "Report exceeds the 128 MiB reading bound"
    );
    serde_json::from_reader(file).with_context(|| format!("Read report {}", path.display()))
}

fn is_difference(finding: &Value) -> bool {
    [
        "pixels_differ",
        "geometry_differs",
        "presence_differs",
        "rendering_differs",
    ]
    .iter()
    .any(|key| finding[*key] == true)
}

fn difference_count(report: &Value) -> usize {
    report["comparisons"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|pair| {
            ["findings", "charts"]
                .into_iter()
                .flat_map(move |kind| pair[kind].as_array().into_iter().flatten())
        })
        .filter(|finding| is_difference(finding))
        .count()
}

/// Decode at most one image pair at a time and reuse the Observatory's exact
/// diagnostic algorithm. Its masks are display evidence, never new verdicts.
fn inspect_differences(report: &Value, images: &mut evidence::ImageStore) -> Result<Value> {
    let mut diagnostics = serde_json::Map::new();
    let mut measured = BTreeMap::<(String, String), Value>::new();
    for (pair_index, pair) in report["comparisons"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        for kind in ["findings", "charts"] {
            for (finding_index, finding) in pair[kind].as_array().into_iter().flatten().enumerate()
            {
                if !is_difference(finding) {
                    continue;
                }
                let key = format!("{pair_index}:{kind}:{finding_index}");
                let left = finding["left_png"]
                    .as_str()
                    .or(finding["left_image"].as_str());
                let right = finding["right_png"]
                    .as_str()
                    .or(finding["right_image"].as_str());
                let (Some(left), Some(right)) = (left, right) else {
                    diagnostics.insert(key, json!({"unavailable": "This observation has no rendered image pair; use its presence, rendering and geometry evidence."}));
                    continue;
                };
                let paths = (left.to_owned(), right.to_owned());
                let result = if let Some(metrics) = measured.get(&paths) {
                    metrics.clone()
                } else {
                    let value = match measure_pair(images, left, right) {
                        Ok(metrics) => json!({"metrics": metrics}),
                        Err(error) => {
                            json!({"unavailable": format!("Retained captures remain available; pixel diagnostics could not be generated: {error:#}")})
                        }
                    };
                    if measured.len() >= MAX_CACHED_DIAGNOSTIC_PAIRS {
                        measured.clear();
                    }
                    measured.insert(paths, value.clone());
                    value
                };
                diagnostics.insert(key, result);
            }
        }
    }
    Ok(Value::Object(diagnostics))
}

fn measure_pair(images: &mut evidence::ImageStore, left: &str, right: &str) -> Result<Value> {
    // Processing and validation may finish concurrently. Diagnostic exports
    // must not multiply the large decoded-image working set by that parallelism.
    let _permit = PIXEL_INSPECTION
        .lock()
        .map_err(|_| anyhow::anyhow!("Pixel inspection was interrupted"))?;
    let left = read_bounded_image(&images.path(left)?)?;
    let right = read_bounded_image(&images.path(right)?)?;
    let (mut metrics, overlay) =
        crate::capture_host::control_room::pixel_diagnostics::compare_pixels(&left, &right, true);
    if let Some(overlay) = overlay {
        metrics["overlay_url"] = json!(images.retain_overlay(&overlay)?);
    }
    Ok(metrics)
}

fn read_bounded_image(path: &Path) -> Result<image::RgbaImage> {
    let (width, height) = image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_dimensions()?;
    ensure!(
        u64::from(width) * u64::from(height) <= MAX_DIAGNOSTIC_PIXELS,
        "Capture exceeds the 32 million pixel diagnostic budget"
    );
    Ok(image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?
        .to_rgba8())
}

#[cfg(test)]
mod tests;
