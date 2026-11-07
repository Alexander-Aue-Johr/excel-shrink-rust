//! Live package sizes for targeted geometry reads and image captures.
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::{fs, path::Path};

pub(crate) const DEFAULT_GEOMETRY_RANGES: usize = 64;
pub(crate) const MAX_GEOMETRY_RANGES: usize = 4096;
pub(crate) const DEFAULT_CAPTURE_RANGES: usize = 4;
pub(crate) const MAX_CAPTURE_RANGES: usize = 64;
pub(crate) const DEFAULT_CAPTURE_BATCH_MEGAPIXELS: usize = 32;
pub(crate) const MAX_CAPTURE_BATCH_MEGAPIXELS: usize = 256;

/// A package may hold several individually bounded images; this is not a per-image allowance.
pub(crate) fn capture_batch_megapixels(value: &Value) -> Result<usize> {
    let budget = batch_size(value, DEFAULT_CAPTURE_BATCH_MEGAPIXELS, MAX_CAPTURE_BATCH_MEGAPIXELS)?;
    ensure!(budget >= DEFAULT_CAPTURE_BATCH_MEGAPIXELS, "Capture package budget must be 32..=256 MP");
    Ok(budget)
}

/// Validate the same numeric contract at API admission and before worker dispatch.
pub(crate) fn batch_size(value: &Value, default: usize, maximum: usize) -> Result<usize> {
    if value.is_null() {
        return Ok(default);
    }
    let count = value
        .as_u64()
        .context("Batch size must be a positive integer")?;
    ensure!(
        count > 0 && count <= maximum as u64,
        "Batch size must be 1..={maximum}"
    );
    Ok(count as usize)
}

/// Reload controls at package boundaries; standalone probes retain the defaults.
pub(crate) fn read_batch_size(
    path: Option<&Path>,
    key: &str,
    default: usize,
    maximum: usize,
) -> Result<usize> {
    let Some(path) = path else {
        return Ok(default);
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(default),
        Err(error) => return Err(error).context("Read capture package controls"),
    };
    let controls: Value = serde_json::from_slice(&bytes)?;
    batch_size(&controls[key], default, maximum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn capture_pixel_budget_is_independent_and_bounded() -> Result<()> {
        assert_eq!(capture_batch_megapixels(&Value::Null)?, 32);
        for allowed in [32, 64, 128, 256] {
            assert_eq!(capture_batch_megapixels(&json!(allowed))?, allowed);
        }
        for invalid in [json!(0), json!(31), json!(257), json!(64.5), json!("64")] {
            assert!(capture_batch_megapixels(&invalid).is_err());
        }
        Ok(())
    }

    #[test]
    fn independent_settings_are_reloaded_without_restarting_a_comparison() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("capture-controls-{}.json", uuid::Uuid::new_v4()));
        fs::write(
            &path,
            br#"{"geometry_batch_ranges":128,"capture_batch_ranges":2}"#,
        )?;
        assert_eq!(
            read_batch_size(Some(&path), "geometry_batch_ranges", 64, 4096)?,
            128
        );
        assert_eq!(
            read_batch_size(Some(&path), "capture_batch_ranges", 4, 64)?,
            2
        );
        fs::write(
            &path,
            br#"{"geometry_batch_ranges":256,"capture_batch_ranges":3}"#,
        )?;
        assert_eq!(
            read_batch_size(Some(&path), "geometry_batch_ranges", 64, 4096)?,
            256
        );
        assert_eq!(
            read_batch_size(Some(&path), "capture_batch_ranges", 4, 64)?,
            3
        );
        for invalid in [json!(0), json!(-1), json!(1.5), json!(65), json!("2")] {
            assert!(batch_size(&invalid, 4, 64).is_err());
        }
        fs::remove_file(path)?;
        Ok(())
    }
}
