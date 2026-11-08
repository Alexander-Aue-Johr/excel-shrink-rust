//! Discard verified-equal image files only after durable reports retain comparison receipts.
use super::*;
use std::collections::BTreeSet;

const OBSERVATIONS: [(&str, [&str; 2]); 2] = [
    ("findings", ["left_png", "right_png"]),
    ("charts", ["left_image", "right_image"]),
];

fn confirmed_equal(finding: &Value) -> bool {
    finding["pixels_compared"] == true
        && finding["capture_extent_ignored"] != true
        && finding["left_rendering"] == "captured"
        && finding["right_rendering"] == "captured"
        && finding["pixels_differ"] == false
        && finding["geometry_differs"] == false
        && finding["rendering_differs"] == false
        && (finding["presence_differs"].is_null() || finding["presence_differs"] == false)
}

/// Validate resolved ownership too, so parent components or junctions cannot escape the attempt.
pub(super) fn validate_owned_image(directory: &Path, path: &Path) -> Result<()> {
    ensure!(
        path.extension()
            .is_some_and(|ext| ext == "png" || ext == "image"),
        "Refusing to discard a non-image artifact"
    );
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && fs::canonicalize(path)?.starts_with(fs::canonicalize(directory)?),
        "Refusing to discard an image outside this attempt"
    );
    Ok(())
}

/// An image needed by any differing or unperformed comparison remains full-resolution evidence.
fn protected_images(report: &Value) -> Result<BTreeSet<PathBuf>> {
    let mut protected = BTreeSet::new();
    for pair in report["comparisons"]
        .as_array()
        .context("Missing comparisons")?
    {
        for (field, keys) in OBSERVATIONS {
            for finding in pair[field].as_array().context("Missing observation list")? {
                if confirmed_equal(finding) {
                    continue;
                }
                for key in keys {
                    if let Some(path) = finding[key].as_str() {
                        protected.insert(PathBuf::from(path));
                    }
                }
            }
        }
    }
    Ok(protected)
}

pub(super) fn apply(report: &mut Value, directory: &Path) -> Result<Vec<PathBuf>> {
    let protected = protected_images(report)?;
    let mut retained = Vec::new();
    for path in &protected {
        validate_owned_image(directory, path)?;
        let pixels = image::ImageReader::open(path)?
            .with_guessed_format()?
            .into_dimensions()?;
        retained.push(
            json!({"file":path,"original_image_sha256":artifact_cache::hash_file(path)?,
            "original_pixels":pixels,"retained_pixels":pixels,"full_resolution_required":true}),
        );
    }
    let mut discarded = BTreeMap::<PathBuf, Value>::new();
    for pair in report["comparisons"]
        .as_array_mut()
        .context("Missing comparisons")?
    {
        for (field, keys) in OBSERVATIONS {
            for finding in pair[field]
                .as_array_mut()
                .context("Missing observation list")?
            {
                if !confirmed_equal(finding) {
                    continue;
                }
                for (side, key) in ["left", "right"].into_iter().zip(keys) {
                    let Some(path) = finding[key].as_str().map(PathBuf::from) else {
                        continue;
                    };
                    if protected.contains(&path) {
                        continue;
                    }
                    if !discarded.contains_key(&path) {
                        validate_owned_image(directory, &path)?;
                        let pixels = image::ImageReader::open(&path)?
                            .with_guessed_format()?
                            .into_dimensions()?;
                        let sha256 = artifact_cache::hash_file(&path)?;
                        discarded.insert(path.clone(), json!({"sha256":sha256,"pixels":pixels,
                            "encoded_bytes":fs::metadata(&path)?.len(),"reason":"verified-equal-image-discarded"}));
                    }
                    // Receipts deliberately contain no file path: portable publishing must not
                    // recreate or export an image that the selected policy is about to discard.
                    finding[format!("{side}_image_receipt")] = discarded[&path].clone();
                    finding[format!("{side}_image_retained")] = json!(false);
                    finding[key] = Value::Null;
                }
            }
        }
    }
    report["evidence_retention"] = json!({"retain_equal_images":false,"comparison_resolution":"original",
        "discarded_equal_images":discarded.len(),"discarded_encoded_bytes":discarded.values()
            .filter_map(|receipt| receipt["encoded_bytes"].as_u64()).sum::<u64>(),
        "equal_neighbors_retained":false,"files":retained,
        "meaning":"Equal-image receipts retain hashes, dimensions and verdicts. Images needed by differences stay full resolution. Byte counts are logical file lengths, not released disk clusters."});
    Ok(discarded.into_keys().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_only_equality_keeps_the_original_capture_evidence() {
        let finding = json!({"pixels_compared":true,"pixels_differ":false,
            "geometry_differs":false,"rendering_differs":false,
            "left_rendering":"captured","right_rendering":"captured",
            "capture_extent_ignored":true});
        assert!(!confirmed_equal(&finding));
    }

    /// Apply the storage policy to real native output without deleting its original evidence.
    #[test]
    #[ignore = "Requires NATIVE_IMAGE_REPORT from a completed native chart experiment"]
    fn native_equal_image_receipts_preserve_the_result_contract() -> Result<()> {
        let path = PathBuf::from(std::env::var("NATIVE_IMAGE_REPORT")?);
        let directory = path.parent().context("Missing native attempt directory")?;
        let mut report: Value = serde_json::from_slice(&fs::read(&path)?)?;
        let before = super::super::super::contract::has_differences(&report, true)?;
        let obsolete = apply(&mut report, directory)?;
        assert_eq!(
            before,
            super::super::super::contract::has_differences(&report, true)?
        );
        assert!(obsolete.iter().all(|p| p.is_file()));
        let mut record = json!({});
        super::super::super::publish_portable_evidence(
            directory,
            directory.parent().unwrap(),
            report,
            &mut record,
        )?;
        if !before {
            assert!(record["targeted_comparison"]["evidence_images"]
                .as_array()
                .unwrap()
                .is_empty());
        }
        Ok(())
    }

    #[test]
    fn equal_images_are_unlinked_from_reports_but_not_deleted_before_publication() -> Result<()> {
        let root = std::env::temp_dir().join(format!("discard-equal-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let a = root.join("a.png");
        let b = root.join("b.png");
        image::RgbaImage::new(2, 3).save(&a)?;
        fs::copy(&a, &b)?;
        let finding = json!({"pixels_compared":true,"pixels_differ":false,"geometry_differs":false,"left_rendering":"captured","right_rendering":"captured",
            "rendering_differs":false,"left_png":a,"right_png":b});
        let mut report = json!({"comparisons":[{"findings":[finding],"charts":[]}]});
        let obsolete = apply(&mut report, &root)?;
        assert_eq!(obsolete.len(), 2);
        assert!(a.is_file() && b.is_file());
        assert!(report["comparisons"][0]["findings"][0]["left_png"].is_null());
        assert_eq!(
            report["comparisons"][0]["findings"][0]["left_image_receipt"]["pixels"],
            json!([2, 3])
        );
        super::super::discard_originals(&root, &obsolete)?;
        assert!(!a.exists() && !b.exists());
        fs::remove_dir(root)?;
        Ok(())
    }

    #[test]
    fn differing_charts_protect_shared_images_and_unperformed_observations() -> Result<()> {
        let root = std::env::temp_dir().join(format!("keep-different-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let a = root.join("a.png");
        image::RgbaImage::new(2, 3).save(&a)?;
        let equal = json!({"pixels_compared":true,"pixels_differ":false,"geometry_differs":false,"left_rendering":"captured","right_rendering":"captured",
            "rendering_differs":false,"left_png":a,"right_png":a});
        let chart = json!({"pixels_compared":true,"pixels_differ":true,"geometry_differs":false,"left_rendering":"captured","right_rendering":"captured",
            "presence_differs":false,"rendering_differs":false,"left_image":a,"right_image":a});
        let mut report = json!({"comparisons":[{"findings":[equal],"charts":[chart]}]});
        assert!(apply(&mut report, &root)?.is_empty());
        assert!(report["comparisons"][0]["findings"][0]["left_png"].is_string());
        report["comparisons"][0]["charts"][0]["pixels_differ"] = json!(false);
        report["comparisons"][0]["charts"][0]["pixels_compared"] = json!(false);
        assert!(apply(&mut report, &root)?.is_empty());
        fs::remove_file(a)?;
        fs::remove_dir(root)?;
        Ok(())
    }
}
