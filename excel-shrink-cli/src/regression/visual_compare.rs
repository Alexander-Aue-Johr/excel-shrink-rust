use super::office_capture::PairedImageTile;
use anyhow::{ensure, Context, Result};
use image::{imageops, Rgba, RgbaImage};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_IMAGE_PIXELS: u64 = 32_000_000;
const CROP_MARGIN: u32 = 16;

#[derive(Debug, Clone, Serialize)]
pub(super) struct ImageDifference {
    pub sheet_index: usize,
    pub sheet_name: String,
    pub range: String,
    pub tile_index: usize,
    pub original_image: PathBuf,
    pub shrunk_image: PathBuf,
    pub diff_image: PathBuf,
    pub diff_pixels: u64,
}

#[derive(Clone, Copy)]
struct Band {
    top: u32,
    bottom: u32,
    left: u32,
    right: u32,
    pixels: u64,
}

pub(super) fn compare_images(
    tiles: &[PairedImageTile],
    output_dir: &Path,
) -> Result<Vec<ImageDifference>> {
    fs::create_dir_all(output_dir)?;
    let mut differences = Vec::new();
    for tile in tiles {
        let original = read_image(&tile.original_image)?;
        let shrunk = read_image(&tile.shrunk_image)?;
        if original.dimensions() == shrunk.dimensions() && original.as_raw() == shrunk.as_raw() {
            continue;
        }
        let width = original.width().max(shrunk.width());
        let height = original.height().max(shrunk.height());
        ensure!(
            u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS,
            "paired image exceeds pixel limit at {}!{}",
            tile.sheet_name,
            tile.range
        );

        let bands = difference_bands(&original, &shrunk, width, height);
        for (region, band) in bands.into_iter().enumerate() {
            let x = band.left.saturating_sub(CROP_MARGIN);
            let y = band.top.saturating_sub(CROP_MARGIN);
            let right = band.right.saturating_add(CROP_MARGIN).min(width - 1);
            let bottom = band.bottom.saturating_add(CROP_MARGIN).min(height - 1);
            let crop_width = right - x + 1;
            let crop_height = bottom - y + 1;
            let stem = format!(
                "sheet-{}-tile-{}-region-{}",
                tile.sheet_index, tile.tile_index, region
            );
            let original_path = output_dir.join(format!("{stem}-original.png"));
            let shrunk_path = output_dir.join(format!("{stem}-shrunk.png"));
            let diff_path = output_dir.join(format!("{stem}-diff.png"));
            cropped_canvas(&original, x, y, crop_width, crop_height).save(&original_path)?;
            cropped_canvas(&shrunk, x, y, crop_width, crop_height).save(&shrunk_path)?;
            let mut mask =
                RgbaImage::from_pixel(crop_width, crop_height, Rgba([255, 255, 255, 255]));
            for yy in y..=bottom {
                for xx in x..=right {
                    if pixel(&original, xx, yy) != pixel(&shrunk, xx, yy) {
                        mask.put_pixel(xx - x, yy - y, Rgba([188, 48, 74, 255]));
                    }
                }
            }
            mask.save(&diff_path)?;
            differences.push(ImageDifference {
                sheet_index: tile.sheet_index,
                sheet_name: tile.sheet_name.clone(),
                range: tile.range.clone(),
                tile_index: tile.tile_index,
                original_image: original_path,
                shrunk_image: shrunk_path,
                diff_image: diff_path,
                diff_pixels: band.pixels,
            });
        }
    }
    Ok(differences)
}

fn read_image(path: &Path) -> Result<RgbaImage> {
    let image = image::open(path)
        .with_context(|| format!("cannot decode capture {}", path.display()))?
        .to_rgba8();
    ensure!(
        image.width() > 0
            && image.height() > 0
            && u64::from(image.width()) * u64::from(image.height()) <= MAX_IMAGE_PIXELS,
        "invalid capture dimensions: {}",
        path.display()
    );
    Ok(image)
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> Option<&Rgba<u8>> {
    (x < image.width() && y < image.height()).then(|| image.get_pixel(x, y))
}

fn difference_bands(
    original: &RgbaImage,
    shrunk: &RgbaImage,
    width: u32,
    height: u32,
) -> Vec<Band> {
    let mut bands: Vec<Band> = Vec::new();
    for y in 0..height {
        let mut left = width;
        let mut right = 0;
        let mut pixels = 0;
        for x in 0..width {
            if pixel(original, x, y) != pixel(shrunk, x, y) {
                left = left.min(x);
                right = x;
                pixels += 1;
            }
        }
        if pixels == 0 {
            continue;
        }
        match bands.last_mut() {
            Some(band) if y - band.bottom <= CROP_MARGIN => {
                band.bottom = y;
                band.left = band.left.min(left);
                band.right = band.right.max(right);
                band.pixels += pixels;
            }
            _ => bands.push(Band {
                top: y,
                bottom: y,
                left,
                right,
                pixels,
            }),
        }
    }
    bands
}

fn cropped_canvas(image: &RgbaImage, x: u32, y: u32, width: u32, height: u32) -> RgbaImage {
    let mut canvas = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));
    if x < image.width() && y < image.height() {
        let available_width = width.min(image.width() - x);
        let available_height = height.min(image.height() - y);
        let part = imageops::crop_imm(image, x, y, available_width, available_height).to_image();
        imageops::replace(&mut canvas, &part, 0, 0);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_distant_pixel_changes() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("visual-compare-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let before = dir.join("before.png");
        let after = dir.join("after.png");
        RgbaImage::from_pixel(32, 100, Rgba([255, 255, 255, 255])).save(&before)?;
        let mut changed = RgbaImage::from_pixel(32, 100, Rgba([255, 255, 255, 255]));
        changed.put_pixel(3, 2, Rgba([0, 0, 0, 255]));
        changed.put_pixel(20, 90, Rgba([0, 0, 0, 255]));
        changed.save(&after)?;
        let tile = PairedImageTile {
            sheet_index: 1,
            sheet_name: "Sheet 1".into(),
            range: "A1:B100".into(),
            tile_index: 0,
            original_image: before,
            shrunk_image: after,
        };
        let differences = compare_images(&[tile], &dir.join("diffs"))?;
        assert_eq!(differences.len(), 2);
        assert_eq!(
            differences.iter().map(|item| item.diff_pixels).sum::<u64>(),
            2
        );
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
