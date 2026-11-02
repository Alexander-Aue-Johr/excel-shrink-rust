//! Preserve original comparison verdicts while bounding retained image storage.
use super::*;
use crate::regression::targeted_comparison::{CAPTURE_COLUMNS, CAPTURE_ROWS};
use std::collections::BTreeMap;

type Tile = (String, u32, u32);
type Dimensions = (u32, u32);

/// Reduce only confirmed-equal regions after every state has been compared at full resolution.
/// Differences and their eight captured neighbors remain full-resolution evidence.
pub(super) fn apply(
    report: &mut Value,
    directory: &Path,
    preview_width: u32,
) -> Result<Vec<PathBuf>> {
    ensure!(
        report["evidence_retention"].is_null(),
        "Evidence retention has already been applied"
    );
    let mut regions = BTreeMap::<Tile, BTreeSet<PathBuf>>::new();
    let mut protected = BTreeSet::new();
    for pair in report["comparisons"]
        .as_array()
        .context("Missing comparisons for evidence retention")?
    {
        for finding in pair["findings"]
            .as_array()
            .context("Missing region findings")?
        {
            let tile = tile(finding)?;
            if finding["pixels_differ"] == true || finding["geometry_differs"] == true {
                for row_offset in -1..=1 {
                    for column_offset in -1..=1 {
                        if let (Some(row), Some(column)) = (
                            tile.1.checked_add_signed(row_offset),
                            tile.2.checked_add_signed(column_offset),
                        ) {
                            protected.insert((tile.0.clone(), row, column));
                        }
                    }
                }
            }
            let paths = regions.entry(tile).or_default();
            for key in ["left_png", "right_png"] {
                let path = PathBuf::from(
                    finding[key]
                        .as_str()
                        .context("Missing captured image path")?,
                );
                ensure!(
                    path.starts_with(directory),
                    "Image is outside this comparison's evidence directory"
                );
                paths.insert(path);
            }
        }
    }
    let mut canonical = BTreeMap::<(String, u32), (PathBuf, Dimensions, Dimensions)>::new();
    let mut files = Vec::new();
    let mut aliases = BTreeMap::new();
    let mut original_paths = BTreeSet::new();
    let mut retained_paths = BTreeSet::new();
    let mut deduplicated = 0;
    let previews = directory.join("equal-previews");
    fs::create_dir_all(&previews)?;
    for (tile, paths) in regions {
        let width = if protected.contains(&tile) {
            0
        } else {
            preview_width
        };
        for path in paths {
            original_paths.insert(path.clone());
            let original_sha256 = artifact_cache::hash_file(&path)?;
            let key = (original_sha256.clone(), width);
            let (retained_path, original_pixels, retained_pixels) =
                if let Some((first, original, retained)) = canonical.get(&key) {
                    deduplicated += 1;
                    (first.clone(), *original, *retained)
                } else {
                    let pixels = image::open(&path)?;
                    let original = (pixels.width(), pixels.height());
                    let (retained_path, retained) = if width > 0 && pixels.width() > width {
                        let height = ((u64::from(pixels.height()) * u64::from(width))
                            / u64::from(pixels.width()))
                        .max(1) as u32;
                        let preview = previews.join(format!("{original_sha256}-{width}.png"));
                        pixels
                            .resize_exact(width, height, image::imageops::FilterType::Triangle)
                            .save_with_format(&preview, image::ImageFormat::Png)?;
                        (preview, (width, height))
                    } else {
                        (path.clone(), original)
                    };
                    canonical.insert(key, (retained_path.clone(), original, retained));
                    (retained_path, original, retained)
                };
            if path != retained_path {
                aliases.insert(
                    path.to_string_lossy().into_owned(),
                    retained_path.to_string_lossy().into_owned(),
                );
            }
            retained_paths.insert(retained_path.clone());
            files.push(json!({"file":retained_path,
                "capture_name":path.file_name().map(|name| name.to_string_lossy().into_owned()),
                "capture_state":path.parent().and_then(Path::file_name).map(|name| name.to_string_lossy().into_owned()),
                "sheet":tile.0,"grid_row":tile.1,"grid_column":tile.2,
                "original_png_sha256":original_sha256,
                "original_pixels":original_pixels,"retained_pixels":retained_pixels,
                "full_resolution_required":protected.contains(&tile) || preview_width == 0}));
        }
    }
    replace_alias_paths(report, &aliases);
    report["evidence_retention"] = json!({"comparison_resolution":"original",
        "equal_preview_width":preview_width,"full_resolution_context_tiles":1,
        "deduplicated_images":deduplicated,"files":files});
    Ok(original_paths
        .difference(&retained_paths)
        .cloned()
        .collect())
}

/// Call only after both completed reports reference the durable retained files.
pub(super) fn discard_originals(directory: &Path, obsolete: &[PathBuf]) -> Result<()> {
    for path in obsolete {
        ensure!(
            path.starts_with(directory) && path.extension().is_some_and(|ext| ext == "png"),
            "Refusing to discard an image outside this attempt"
        );
        fs::remove_file(path)
            .with_context(|| format!("Discard superseded image {}", path.display()))?;
    }
    Ok(())
}

fn replace_alias_paths(value: &mut Value, aliases: &BTreeMap<String, String>) {
    match value {
        Value::Object(fields) => {
            for child in fields.values_mut() {
                replace_alias_paths(child, aliases);
            }
        }
        Value::Array(values) => {
            for child in values {
                replace_alias_paths(child, aliases);
            }
        }
        Value::String(path) => {
            if let Some(canonical) = aliases.get(path) {
                *path = canonical.clone();
            }
        }
        _ => {}
    }
}

/// Decode only the normalized A1 start emitted by the shared capture grid.
fn tile(finding: &Value) -> Result<Tile> {
    let sheet = finding["sheet"].as_str().context("Missing capture sheet")?;
    let range = finding["cell"].as_str().context("Missing capture range")?;
    let start = range.split(':').next().context("Missing range start")?;
    let split = start
        .find(|c: char| c.is_ascii_digit())
        .context("Invalid capture address")?;
    let row: u32 = start[split..].parse()?;
    let column = start[..split]
        .bytes()
        .try_fold(0u32, |value, byte| {
            if !byte.is_ascii_uppercase() {
                return None;
            }
            value
                .checked_mul(26)?
                .checked_add(u32::from(byte - b'A') + 1)
        })
        .context("Invalid capture column")?;
    ensure!(
        (1..=1_048_576).contains(&row) && (1..=16_384).contains(&column),
        "Capture outside Excel grid"
    );
    Ok((
        sheet.into(),
        (row - 1) / CAPTURE_ROWS,
        (column - 1) / CAPTURE_COLUMNS,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(ranges: &[(&str, bool)]) -> Result<(PathBuf, Value)> {
        let directory = artifact_cache::unique_path(&std::env::temp_dir(), "targeted-retention")?;
        fs::create_dir_all(&directory)?;
        let mut findings = Vec::new();
        for (index, &(range, changed)) in ranges.iter().enumerate() {
            let left = directory.join(format!("{index}-original.png"));
            let right = directory.join(format!("{index}-default.png"));
            image::RgbaImage::from_pixel(256, 128, image::Rgba([10, 20, 30, 255])).save(&left)?;
            if changed {
                image::RgbaImage::from_pixel(256, 128, image::Rgba([200, 20, 30, 255]))
                    .save(&right)?;
            } else {
                fs::copy(&left, &right)?;
            }
            findings.push(json!({"sheet":"Data","cell":range,"pixels_differ":changed,
                "geometry_differs":false,"left_png":left,"right_png":right}));
        }
        Ok((
            directory,
            json!({"comparisons":[{"label":"direct Default","findings":findings}]}),
        ))
    }

    #[test]
    fn previews_preserve_difference_context_original_hashes_and_verdicts() -> Result<()> {
        let (directory, mut report) = fixture(&[
            ("A1:P128", true),
            ("Q129:AF256", false),
            ("A1025:P1152", false),
        ])?;
        let verdicts: Vec<_> = report["comparisons"][0]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| finding["pixels_differ"].clone())
            .collect();
        let far = directory.join("2-original.png");
        let original_hash = artifact_cache::hash_file(&far)?;
        let obsolete = apply(&mut report, &directory, 16)?;
        assert_eq!(
            report["comparisons"][0]["findings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|finding| finding["pixels_differ"].clone())
                .collect::<Vec<_>>(),
            verdicts
        );
        assert_eq!(
            image::image_dimensions(directory.join("0-original.png"))?,
            (256, 128)
        );
        assert_eq!(
            image::image_dimensions(directory.join("1-default.png"))?,
            (256, 128)
        );
        let preview = PathBuf::from(
            report["comparisons"][0]["findings"][2]["left_png"]
                .as_str()
                .unwrap(),
        );
        assert_eq!(image::image_dimensions(&preview)?, (16, 8));
        assert_eq!(
            image::image_dimensions(&far)?,
            (256, 128),
            "Raw evidence must survive until report publication"
        );
        let metadata = report["evidence_retention"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["capture_name"] == "2-original.png")
            .unwrap();
        assert_eq!(metadata["original_png_sha256"], original_hash);
        assert_eq!(metadata["original_pixels"], json!([256, 128]));
        assert_eq!(metadata["retained_pixels"], json!([16, 8]));
        assert_eq!(
            report["evidence_retention"]["deduplicated_images"]
                .as_u64()
                .unwrap(),
            3
        );
        assert_eq!(
            report["comparisons"][0]["findings"][2]["left_png"],
            report["comparisons"][0]["findings"][2]["right_png"]
        );
        discard_originals(&directory, &obsolete)?;
        assert!(!far.exists());
        for finding in report["comparisons"][0]["findings"].as_array().unwrap() {
            assert!(Path::new(finding["left_png"].as_str().unwrap()).is_file());
            assert!(Path::new(finding["right_png"].as_str().unwrap()).is_file());
        }
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn zero_preview_width_preserves_full_resolution_and_encoded_bytes() -> Result<()> {
        let (directory, mut report) = fixture(&[("A1025:P1152", false)])?;
        let path = directory.join("0-original.png");
        let before = fs::read(&path)?;
        apply(&mut report, &directory, 0)?;
        assert_eq!(image::image_dimensions(&path)?, (256, 128));
        assert_eq!(fs::read(path)?, before);
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn paths_outside_the_attempt_are_never_resized_or_discarded() -> Result<()> {
        let (directory, mut report) = fixture(&[("A1:P128", false)])?;
        let outside = directory.parent().unwrap().join("not-owned.png");
        report["comparisons"][0]["findings"][0]["left_png"] = json!(outside);
        assert!(apply(&mut report, &directory, 16)
            .err()
            .unwrap()
            .to_string()
            .contains("outside"));
        assert!(discard_originals(&directory, &[outside]).is_err());
        assert_eq!(
            image::image_dimensions(directory.join("0-original.png"))?,
            (256, 128)
        );
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
