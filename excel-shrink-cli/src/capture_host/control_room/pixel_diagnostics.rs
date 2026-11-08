//! Lazy, cached inspection of immutable evidence. These numbers describe image
//! differences; they never change the scientific comparison verdict.
use super::*;
use image::{ImageEncoder, Rgba, RgbaImage};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

static DIAGNOSTICS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
const BLOCK_SIZE: u32 = 64;

#[derive(serde::Deserialize)]
pub(super) struct Options {
    region: Option<String>,
}

pub(super) async fn inspect(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
    Query(options): Query<Options>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    // Waiting requests retain identifiers until their report can be inspected.
    let _permit = DIAGNOSTICS
        .acquire()
        .await
        .context("Pixel diagnostics stopped")?;
    let Json(report) = targeted::report(State(room.clone()), RoutePath(id.clone())).await?;
    let study = room
        .root
        .parent()
        .context("Missing study directory")?
        .to_owned();
    let mut identity = serde_json::to_vec(&report)?;
    identity.extend_from_slice(options.region.as_deref().unwrap_or("summary").as_bytes());
    let hash = format!("{:x}", Sha256::digest(identity));
    let cache = study
        .join(".diagnostics/pixel-v1")
        .join(format!("{hash}.json"));
    if cache.is_file() {
        return Ok(Json(read_object(&cache, SNAPSHOT_LIMIT).await?));
    }
    let result = tokio::task::spawn_blocking(move || {
        inspect_report(
            &study,
            &id,
            &hash,
            &cache,
            report,
            options.region.as_deref(),
        )
    })
    .await
    .context("Inspect retained pixels")??;
    Ok(Json(result))
}

fn inspect_report(
    study: &Path,
    id: &str,
    hash: &str,
    cache: &Path,
    report: Value,
    selected: Option<&str>,
) -> Result<Value> {
    if cache.is_file() {
        return Ok(serde_json::from_reader(std::fs::File::open(cache)?)?);
    }
    let mut regions = Vec::new();
    let mut totals = Totals::default();
    let mut measured: HashMap<(String, String), Value> = HashMap::new();
    for (pair_index, pair) in report["comparisons"]
        .as_array()
        .context("Missing comparison pairs")?
        .iter()
        .enumerate()
    {
        for kind in ["findings", "charts"] {
            for (finding_index, finding) in pair[kind].as_array().into_iter().flatten().enumerate()
            {
                let region_key = format!("{pair_index}:{kind}:{finding_index}");
                if selected.is_some_and(|key| key != region_key) {
                    continue;
                }
                if ![
                    "pixels_differ",
                    "geometry_differs",
                    "presence_differs",
                    "rendering_differs",
                ]
                .iter()
                .any(|key| finding[*key] == true)
                {
                    continue;
                }
                let left = finding["left_png"]
                    .as_str()
                    .or(finding["left_image"].as_str());
                let right = finding["right_png"]
                    .as_str()
                    .or(finding["right_image"].as_str());
                let (Some(left), Some(right)) = (left, right) else {
                    totals.unmeasured_regions += 1;
                    continue;
                };
                let key = (left.to_owned(), right.to_owned());
                let metrics = if let Some(metrics) = measured.get(&key) {
                    metrics.clone()
                } else {
                    let left = evidence_path(study, id, left)?;
                    let right = evidence_path(study, id, right)?;
                    let (metrics, overlay) = compare_pixels(
                        &image::open(left)?.to_rgba8(),
                        &image::open(right)?.to_rgba8(),
                        selected.is_some(),
                    );
                    let mut metrics = metrics;
                    if let Some(overlay) = overlay {
                        let relative =
                            format!("evidence/{id}/diagnostics/{hash}-{}.png", measured.len());
                        let path = confined_path(study, &relative)?;
                        std::fs::create_dir_all(
                            path.parent().context("Missing overlay directory")?,
                        )?;
                        let mut png = Vec::new();
                        image::codecs::png::PngEncoder::new(&mut png).write_image(
                            overlay.as_raw(),
                            overlay.width(),
                            overlay.height(),
                            image::ExtendedColorType::Rgba8,
                        )?;
                        crate::regression::write_atomic_artifact(&path, &png)?;
                        metrics["overlay_url"] = json!(format!("/api/{}", relative));
                    }
                    measured.insert(key, metrics.clone());
                    metrics
                };
                totals.add(&metrics);
                if selected.is_some() {
                    regions.push(json!({"pair_index":pair_index,"kind":kind,"finding_index":finding_index,"metrics":metrics}));
                }
            }
        }
    }
    let result = json!({"version":1,"report_key":hash,"report_id":report["_evidence_id"],"regions":regions,"summary":totals});
    std::fs::create_dir_all(
        cache
            .parent()
            .context("Missing diagnostic cache directory")?,
    )?;
    crate::regression::write_atomic_artifact(cache, &serde_json::to_vec(&result)?)?;
    Ok(result)
}

fn evidence_path(study: &Path, id: &str, relative: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        relative.starts_with(&format!("evidence/{id}/")) && relative.ends_with(".png"),
        "Pixel diagnostics require retained evidence from this workbook"
    );
    confined_path(study, relative)
}

#[derive(Default, serde::Serialize)]
struct Totals {
    unmeasured_regions: u64,
    changed_pixels: u64,
    unmatched_pixels: u64,
    unmatched_nonwhite_pixels: u64,
    max_channel_delta: u64,
    total_channel_delta: u64,
}

impl Totals {
    fn add(&mut self, metrics: &Value) {
        self.changed_pixels += metrics["changed_pixels"].as_u64().unwrap_or(0);
        self.unmatched_pixels += metrics["unmatched_pixels"].as_u64().unwrap_or(0);
        self.unmatched_nonwhite_pixels +=
            metrics["unmatched_nonwhite_pixels"].as_u64().unwrap_or(0);
        self.max_channel_delta = self
            .max_channel_delta
            .max(metrics["max_channel_delta"].as_u64().unwrap_or(0));
        self.total_channel_delta += metrics["total_channel_delta"].as_u64().unwrap_or(0);
    }
}

/// A tight pixel bound within one display block, not a capture selection rule.
struct Hotspot {
    x: u32,
    y: u32,
    right: u32,
    bottom: u32,
    pixels: u64,
    delta_sum: u64,
    unpaired: bool,
}

impl Hotspot {
    fn new(x: u32, y: u32, unpaired: bool) -> Self {
        Self {
            x,
            y,
            right: x,
            bottom: y,
            pixels: 0,
            delta_sum: 0,
            unpaired,
        }
    }

    fn observe(&mut self, x: u32, y: u32, delta: u64) {
        self.x = self.x.min(x);
        self.y = self.y.min(y);
        self.right = self.right.max(x);
        self.bottom = self.bottom.max(y);
        self.pixels += 1;
        self.delta_sum += delta;
    }
}

/// Compare aligned overlapping RGBA pixels. Exterior pixels stay a separate
/// measurement: a differently sized capture is not a changed common pixel.
pub(crate) fn compare_pixels(
    left: &RgbaImage,
    right: &RgbaImage,
    details: bool,
) -> (Value, Option<RgbaImage>) {
    let width = left.width().min(right.width());
    let height = left.height().min(right.height());
    let mut totals = Totals::default();
    let mut blocks: BTreeMap<(bool, u32, u32), Hotspot> = BTreeMap::new();
    let mut overlay = None;
    for y in 0..height {
        for x in 0..width {
            let a = left.get_pixel(x, y);
            let b = right.get_pixel(x, y);
            let deltas = std::array::from_fn::<_, 4, _>(|channel| {
                u64::from(a[channel].abs_diff(b[channel]))
            });
            let maximum = *deltas.iter().max().unwrap();
            if maximum == 0 {
                continue;
            }
            let sum: u64 = deltas.iter().sum();
            totals.changed_pixels += 1;
            totals.max_channel_delta = totals.max_channel_delta.max(maximum);
            totals.total_channel_delta += sum;
            if details {
                blocks
                    .entry((false, x / BLOCK_SIZE, y / BLOCK_SIZE))
                    .or_insert_with(|| Hotspot::new(x, y, false))
                    .observe(x, y, sum);
                overlay
                    .get_or_insert_with(|| RgbaImage::new(width, height))
                    .put_pixel(x, y, Rgba([255, 25, 25, 220]));
            }
        }
    }
    totals.unmatched_pixels = u64::from(left.width()) * u64::from(left.height())
        + u64::from(right.width()) * u64::from(right.height())
        - 2 * u64::from(width) * u64::from(height);
    for image in [left, right] {
        for y in 0..image.height() {
            let start = if y < height { width } else { 0 };
            for x in start..image.width() {
                if image.get_pixel(x, y).0 != [255; 4] {
                    totals.unmatched_nonwhite_pixels += 1;
                    if details {
                        blocks
                            .entry((true, x / BLOCK_SIZE, y / BLOCK_SIZE))
                            .or_insert_with(|| Hotspot::new(x, y, true))
                            .observe(x, y, 0);
                    }
                }
            }
        }
    }
    let mut hotspots: Vec<_> = blocks.into_values().collect();
    hotspots.sort_by_key(|block| std::cmp::Reverse((block.delta_sum, block.pixels)));
    let hotspot_count = hotspots.len();
    let hotspots: Vec<_> = hotspots
        .into_iter()
        .take(64)
        .map(|block| {
            json!({
                "x":block.x, "y":block.y, "width":block.right-block.x+1,
                "height":block.bottom-block.y+1, "pixels":block.pixels,
                "delta_sum":block.delta_sum, "unpaired":block.unpaired
            })
        })
        .collect();
    let mut result = serde_json::to_value(totals).expect("Integer metrics serialize");
    result["hotspots"] = json!(hotspots);
    result["hotspot_count"] = json!(hotspot_count);
    result["overlap_width"] = json!(width);
    result["overlap_height"] = json!(height);
    result["left_width"] = json!(left.width());
    result["left_height"] = json!(left.height());
    result["right_width"] = json!(right.width());
    result["right_height"] = json!(right.height());
    (result, overlay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_alpha_and_preserves_exact_channel_deltas() {
        let left = RgbaImage::from_pixel(2, 2, Rgba([255; 4]));
        let mut right = left.clone();
        right.put_pixel(1, 1, Rgba([253, 255, 255, 254]));
        let (metrics, overlay) = compare_pixels(&left, &right, true);
        assert_eq!(metrics["changed_pixels"], 1);
        assert_eq!(metrics["max_channel_delta"], 2);
        assert_eq!(metrics["total_channel_delta"], 3);
        assert_eq!(metrics["unmatched_pixels"], 0);
        assert_eq!(overlay.unwrap().get_pixel(1, 1).0[3], 220);
    }

    #[test]
    fn exterior_white_area_is_not_counted_as_changed_overlap() {
        let left = RgbaImage::from_pixel(3, 2, Rgba([255; 4]));
        let right = RgbaImage::from_pixel(2, 2, Rgba([255; 4]));
        let (metrics, overlay) = compare_pixels(&left, &right, true);
        assert_eq!(metrics["changed_pixels"], 0);
        assert_eq!(metrics["unmatched_pixels"], 2);
        assert_eq!(metrics["unmatched_nonwhite_pixels"], 0);
        assert!(overlay.is_none());
    }

    #[test]
    fn alpha_only_changes_are_measured_without_allocating_summary_overlays() {
        let left = RgbaImage::from_pixel(1, 1, Rgba([12, 34, 56, 255]));
        let right = RgbaImage::from_pixel(1, 1, Rgba([12, 34, 56, 253]));
        let (metrics, overlay) = compare_pixels(&left, &right, false);
        assert_eq!(metrics["changed_pixels"], 1);
        assert_eq!(metrics["total_channel_delta"], 2);
        assert_eq!(metrics["hotspot_count"], 0);
        assert!(overlay.is_none());
    }

    #[test]
    fn exterior_content_has_separate_hotspots_and_no_invented_pixel_delta() {
        let mut left = RgbaImage::from_pixel(3, 2, Rgba([255; 4]));
        left.put_pixel(2, 1, Rgba([0, 0, 0, 255]));
        let right = RgbaImage::from_pixel(2, 2, Rgba([255; 4]));
        let (metrics, overlay) = compare_pixels(&left, &right, true);
        assert_eq!(metrics["changed_pixels"], 0);
        assert_eq!(metrics["unmatched_nonwhite_pixels"], 1);
        assert_eq!(metrics["hotspots"][0]["unpaired"], true);
        assert_eq!(metrics["hotspots"][0]["x"], 2);
        assert_eq!(metrics["total_channel_delta"], 0);
        assert!(overlay.is_none());
    }

    #[test]
    fn diagnostics_reject_evidence_from_another_workbook() {
        let root = std::env::temp_dir();
        assert!(evidence_path(&root, "one", "evidence/two/image.png").is_err());
        assert!(evidence_path(&root, "one", "evidence/one/../image.png").is_err());
        assert!(evidence_path(&root, "one", "file:///image.png").is_err());
    }
}
