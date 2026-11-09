//! Validate immutable captures once and retain small fingerprints, never decoded bitmaps.
use anyhow::{Context, Result};
use image::GenericImageView;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Serialize)]
pub(super) struct Statistics {
    pub comparisons: usize,
    pub files_read: usize,
    pub decoded_images: usize,
    pub encoded_images_reused: usize,
    pub encoded_bytes_read: u64,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Pixels {
    dimensions: (u32, u32),
    rgba_sha256: [u8; 32],
    blank_edge_fingerprint: (u32, u32, [u8; 32]),
}

#[derive(Default)]
pub(super) struct ImageComparisons {
    files: BTreeMap<PathBuf, Pixels>,
    encoded: BTreeMap<[u8; 32], Pixels>,
    statistics: Statistics,
}

impl ImageComparisons {
    /// Captures are immutable for this workflow. Missing or invalid new images remain errors.
    /// Different encodings are decoded; their normalized pixels, not PNG bytes, decide equality.
    pub fn differ(&mut self, left: &Path, right: &Path) -> Result<bool> {
        self.statistics.comparisons += 1;
        Ok(self.fingerprint(left)? != self.fingerprint(right)?)
    }

    /// Compare top-left aligned pixels with white padding at the right/bottom only.
    /// Callers must establish matching measured origins; this is not general cropping.
    pub fn differ_with_blank_edges(&mut self, left: &Path, right: &Path) -> Result<bool> {
        self.statistics.comparisons += 1;
        Ok(self.fingerprint(left)?.blank_edge_fingerprint
            != self.fingerprint(right)?.blank_edge_fingerprint)
    }

    pub fn statistics(&self) -> Statistics {
        self.statistics.clone()
    }

    /// Empty images have no visible feature whose absolute origin could move.
    pub fn both_blank(&mut self, left: &Path, right: &Path) -> Result<bool> {
        let left = self.fingerprint(left)?.blank_edge_fingerprint;
        let right = self.fingerprint(right)?.blank_edge_fingerprint;
        Ok(left.0 == 0 && left.1 == 0 && right.0 == 0 && right.1 == 0)
    }

    /// Compare only the common rectangle, without scaling either immutable image.
    /// The caller establishes the same measured capture origin.
    pub fn differ_in_overlap(&mut self, left: &Path, right: &Path) -> Result<bool> {
        self.statistics.comparisons += 1;
        if self.fingerprint(left)? == self.fingerprint(right)? {
            return Ok(false);
        }
        self.statistics.encoded_bytes_read += fs::metadata(left)?.len() + fs::metadata(right)?.len();
        let left = image::open(left)?.into_rgba8();
        let right = image::open(right)?.into_rgba8();
        self.statistics.decoded_images += 2;
        self.statistics.files_read += 2;
        let width = left.width().min(right.width());
        let height = left.height().min(right.height());
        anyhow::ensure!(width > 0 && height > 0, "No shared capture pixels");
        Ok((0..height).any(|y| {
            (0..width).any(|x| left.get_pixel(x, y) != right.get_pixel(x, y))
        }))
    }

    fn fingerprint(&mut self, path: &Path) -> Result<Pixels> {
        if let Some(pixels) = self.files.get(path) {
            return Ok(*pixels);
        }
        let bytes =
            fs::read(path).with_context(|| format!("Read captured image {}", path.display()))?;
        self.statistics.files_read += 1;
        self.statistics.encoded_bytes_read += bytes.len() as u64;
        let encoded_sha256: [u8; 32] = Sha256::digest(&bytes).into();
        let pixels = if let Some(pixels) = self.encoded.get(&encoded_sha256) {
            self.statistics.encoded_images_reused += 1;
            *pixels
        } else {
            let image = image::ImageReader::new(Cursor::new(bytes))
                .with_guessed_format()?
                .decode()
                .with_context(|| format!("Decode captured image {}", path.display()))?;
            let dimensions = image.dimensions();
            anyhow::ensure!(dimensions.0 > 0 && dimensions.1 > 0, "Empty captured image");
            let rgba = image.into_rgba8();
            let pixels = Pixels {
                dimensions,
                rgba_sha256: Sha256::digest(rgba.as_raw()).into(),
                blank_edge_fingerprint: blank_edge_fingerprint(&rgba),
            };
            self.statistics.decoded_images += 1;
            self.encoded.insert(encoded_sha256, pixels);
            pixels
        };
        self.files.insert(path.to_path_buf(), pixels);
        Ok(pixels)
    }
}

/// Keep the origin fixed. Only opaque white or fully transparent pixels are blank;
/// antialiasing, borders and overflow text remain evidence, including in the margin.
fn blank_edge_fingerprint(image: &image::RgbaImage) -> (u32, u32, [u8; 32]) {
    let mut width = 0;
    let mut height = 0;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] != 0 && pixel.0 != [255, 255, 255, 255] {
            width = width.max(x + 1);
            height = height.max(y + 1);
        }
    }
    let mut hash = Sha256::new();
    for row in image
        .as_raw()
        .chunks_exact(image.width() as usize * 4)
        .take(height as usize)
    {
        let row = &row[..width as usize * 4];
        if row.chunks_exact(4).any(|pixel| pixel[3] == 0) {
            let mut normalized = row.to_vec();
            for pixel in normalized.chunks_exact_mut(4) {
                if pixel[3] == 0 {
                    pixel.copy_from_slice(&[255; 4]);
                }
            }
            hash.update(normalized);
        } else {
            hash.update(row);
        }
    }
    (width, height, hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{
        codecs::png::{CompressionType, FilterType, PngEncoder},
        ImageEncoder,
    };

    #[test]
    fn reuses_encoded_images_and_still_compares_different_encodings_and_pixels() -> Result<()> {
        let directory =
            crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "image-cache")?;
        fs::create_dir_all(&directory)?;
        let image = image::RgbaImage::from_pixel(30, 20, image::Rgba([8, 30, 60, 255]));
        let a = directory.join("a.png");
        let b = directory.join("b.png");
        let c = directory.join("c.png");
        let d = directory.join("d.png");
        image.save(&a)?;
        fs::copy(&a, &b)?;
        PngEncoder::new_with_quality(
            fs::File::create(&c)?,
            CompressionType::Fast,
            FilterType::NoFilter,
        )
        .write_image(image.as_raw(), 30, 20, image::ExtendedColorType::Rgba8)?;
        assert_ne!(fs::read(&a)?, fs::read(&c)?);
        let mut changed = image.clone();
        changed.put_pixel(2, 3, image::Rgba([9, 30, 60, 255]));
        changed.save(&d)?;
        let mut comparisons = ImageComparisons::default();
        assert!(!comparisons.differ(&a, &b)?);
        assert!(!comparisons.differ(&a, &c)?);
        assert!(comparisons.differ(&a, &d)?);
        assert!(comparisons.differ(&d, &a)?);
        assert_eq!(comparisons.statistics.files_read, 4);
        assert_eq!(comparisons.statistics.decoded_images, 3);
        assert_eq!(comparisons.statistics.encoded_images_reused, 1);
        let invalid = directory.join("invalid.png");
        fs::write(&invalid, b"not an image")?;
        assert!(comparisons.differ(&invalid, &invalid).is_err());
        assert!(comparisons
            .differ(&a, &directory.join("missing.png"))
            .is_err());
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
