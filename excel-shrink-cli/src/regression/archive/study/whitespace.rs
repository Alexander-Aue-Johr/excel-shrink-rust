//! Matched controls: keep whitespace cells, with every other profile option unchanged.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const VERSION: u64 = 1;

pub(super) fn needed(record: &Value) -> bool {
    record["complete"] == true
        && record["status"] == "differences"
        && (record["whitespace_diagnosis"]["version"] != VERSION
            || record["whitespace_diagnosis"]["status"] == "running")
}

fn items<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or_default()
}

fn image_sheets(profile: &Value) -> BTreeSet<usize> {
    items(profile, "sheets")
        .iter()
        .filter(|s| !items(s, "image_differences").is_empty() && items(s, "row_heights").is_empty())
        .filter_map(|s| s["index"].as_u64().map(|i| i as usize))
        .collect()
}

fn shrink(
    job: &MemberJob,
    original: &Path,
    name: &str,
    whitespace: bool,
    log_level: LogLevel,
) -> Result<PathBuf> {
    let mut config = predefined_visual_profiles()
        .iter()
        .find(|p| p.name == name)
        .context("unknown whitespace control profile")?
        .config;
    config.remove_unreferenced_whitespace_cells = whitespace;
    let directory = job.work.join("whitespace").join(name).join(if whitespace {
        "full"
    } else {
        "keep-whitespace"
    });
    let case = serde_json::from_value(json!({"id":"workbook", "input_path":original}))?;
    let result = run_case(
        "whitespace diagnosis",
        &repo_root(),
        &directory,
        None,
        None,
        &config.cli_args(),
        false,
        false,
        &visual_validation::VisualCheckOptions::default(),
        log_level,
        &case,
    )?;
    ensure!(
        result.failures.is_empty(),
        "control shrink failed: {}",
        result.failures.join("; ")
    );
    artifact_cache::assert_shrink_invariants(
        original,
        &result.output_path,
        &directory.join("audit"),
    )?;
    Ok(result.output_path)
}

fn compare(
    job: &MemberJob,
    original: &Path,
    control: &Path,
    profile: &str,
    sheets: &BTreeSet<usize>,
    phase: &str,
) -> Result<Value> {
    let output = job.work.join("whitespace").join(profile).join(phase);
    let result = visual_validation::compare_sheet_visuals(
        original,
        control,
        &output,
        &visual_validation::VisualCheckOptions {
            profile: format!("{profile}: {phase}"),
            sheets: Some(sheets.clone()),
            keep_screenshots: false,
            combined_report: Some(job.report_root.join("report.html")),
        },
    )?;
    let mut report: Value =
        serde_json::from_slice(&fs::read(result.output_dir.join("visual-result.json"))?)?;
    report["checks_complete"] = json!(true);
    Ok(report)
}

// Legacy records have no output fingerprints. Reproduce their actual before/after
// pixels, not just a matching range or bounding box, before attributing their cause.
fn evidence_matches(previous: &Value, fresh: &Value, report_root: &Path) -> Result<bool> {
    fn fingerprint(sheet: &Value, root: &Path) -> Result<BTreeMap<String, [String; 2]>> {
        let mut result = BTreeMap::new();
        for difference in items(sheet, "image_differences") {
            let mut hashes = Vec::new();
            for key in ["original_image", "shrunk_image"] {
                let path =
                    PathBuf::from(difference[key].as_str().context("missing image evidence")?);
                let path = if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                };
                let image = image::open(path)?.to_rgba8();
                let mut bytes = image.width().to_le_bytes().to_vec();
                bytes.extend(image.height().to_le_bytes());
                bytes.extend(image.as_raw());
                hashes.push(artifact_cache::sha256(&bytes));
            }
            let key = serde_json::to_string(&json!([difference["range"], difference["region"]]))?;
            ensure!(
                !result.contains_key(&key),
                "duplicate image evidence region"
            );
            result.insert(key, [hashes.remove(0), hashes.remove(0)]);
        }
        Ok(result)
    }
    Ok(items(fresh, "row_heights").is_empty()
        && fingerprint(previous, report_root)? == fingerprint(fresh, report_root)?)
}

pub(super) fn diagnose(job: &MemberJob, record: &mut Value, log_level: LogLevel) -> Result<()> {
    let producer = artifact_cache::hash_file(&std::env::current_exe()?)?;
    record["whitespace_diagnosis"] = json!({"version":VERSION,"status":"running", "producer":producer,
        "method":"matched profile with whitespace deletion disabled", "profiles":[]});
    write_record(&job.record, record)?;
    let profiles = items(record, "results").to_vec();
    let candidates: Vec<_> = profiles
        .iter()
        .filter(|p| {
            matches!(
                p["profile"].as_str(),
                Some("maximum-cleaning" | "too-aggressive-cleaning")
            ) && p["checks_complete"] == true
                && !image_sheets(p).is_empty()
        })
        .collect();
    // Height changes are not evidence of a completed pixel check. No control is
    // needed to preserve that warning, and Default never deletes whitespace.
    if candidates.is_empty() {
        record["whitespace_diagnosis"]["status"] = json!("complete");
        return Ok(());
    }
    let original = job.work.join("original.xlsx");
    if !original.exists() {
        let source = worker::extract(job)?;
        ensure!(
            record["source_sha256"] == artifact_cache::hash_file(&source)?,
            "source fingerprint changed"
        );
        let source = if source
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("xls"))
        {
            let converted = job.work.join("converted.xlsx");
            excel_automation::convert_legacy_workbook(
                &source,
                &converted,
                &job.work.join("conversion"),
            )?;
            converted
        } else {
            source
        };
        let baseline = artifact_cache::prepare_baseline(&source)?;
        fs::copy(baseline.snapshot.path, &original)?;
    }
    let baseline_hash = artifact_cache::hash_file(&original)?;
    ensure!(
        record["baseline_sha256"] == baseline_hash,
        "baseline fingerprint changed"
    );
    record["whitespace_diagnosis"]["baseline_sha256"] = json!(baseline_hash);
    let mut controls = BTreeMap::<(String, Vec<usize>), Value>::new();
    let mut reproductions = BTreeMap::<(String, Vec<usize>), Value>::new();
    for profile in candidates {
        let name = profile["profile"].as_str().unwrap();
        let selected = image_sheets(profile);
        eprintln!("[whitespace] {name}: checking {} sheet(s)", selected.len());
        // A newly compared profile already has its full output. A backfill must
        // reproduce both the output and the stored image evidence first.
        let full = job.work.join(format!("{name}.xlsx"));
        let fresh = if full.exists() {
            None
        } else {
            let full = shrink(job, &original, name, true, log_level)?;
            let key = (
                artifact_cache::hash_file(&full)?,
                selected.iter().copied().collect(),
            );
            if !reproductions.contains_key(&key) {
                reproductions.insert(
                    key.clone(),
                    compare(
                        job,
                        &original,
                        &full,
                        name,
                        &selected,
                        "reproduce-original-comparison",
                    )?,
                );
            }
            Some(reproductions[&key].clone())
        };
        let control = shrink(job, &original, name, false, log_level)?;
        let control_sha = artifact_cache::hash_file(&control)?;
        let key = (control_sha.clone(), selected.iter().copied().collect());
        let reused_control = controls.contains_key(&key);
        if !reused_control {
            controls.insert(
                key.clone(),
                compare(
                    job,
                    &original,
                    &control,
                    name,
                    &selected,
                    "without-whitespace-deletion",
                )?,
            );
        }
        let mut residual = controls[&key].clone();
        let mut classifications = Vec::new();
        for &index in &selected {
            let previous = items(profile, "sheets")
                .iter()
                .find(|s| s["index"] == index)
                .unwrap();
            let sheet = items(&residual, "sheets")
                .iter()
                .find(|s| s["index"] == index)
                .context("control did not check the selected worksheet")?;
            let reproduced = if let Some(fresh) = &fresh {
                let sheet = items(fresh, "sheets")
                    .iter()
                    .find(|s| s["index"] == index)
                    .context("reproduction did not check the selected worksheet")?;
                evidence_matches(previous, sheet, &job.report_root)?
            } else {
                true
            };
            let category = if !reproduced {
                "unresolved"
            } else if !items(sheet, "row_heights").is_empty() {
                "height_unchecked"
            } else if sheet["status"] == "same" {
                "whitespace_explained"
            } else {
                "other"
            };
            classifications.push(json!({"index":index, "category":category,
                "checks_complete":true, "original_evidence_reproduced":reproduced}));
        }
        // Equal controls need no image evidence; keep every selected sheet status.
        residual["profile"] = json!(format!("{name} without whitespace deletion"));
        let base_config = predefined_visual_profiles()
            .iter()
            .find(|p| p.name == name)
            .unwrap()
            .config;
        let mut control_config = base_config;
        control_config.remove_unreferenced_whitespace_cells = false;
        record["whitespace_diagnosis"]["profiles"].as_array_mut().unwrap().push(json!({
            "profile":name, "base_config_hash":base_config.config_hash(),
            "control_config_hash":control_config.config_hash(), "control_flags":control_config.cli_args(),
            "control_sha256":control_sha, "reused_identical_control":reused_control,
            "sheets":classifications, "control":residual,
        }));
        write_record(&job.record, record)?;
    }
    record["whitespace_diagnosis"]["status"] = json!("complete");
    Ok(())
}

pub(super) fn run(job: &MemberJob, record: &mut Value, log_level: LogLevel) {
    if let Err(error) = diagnose(job, record, log_level) {
        record["whitespace_diagnosis"]["version"] = json!(VERSION);
        record["whitespace_diagnosis"]["status"] = json!("failed");
        record["whitespace_diagnosis"]["error"] = json!(format!("{error:#}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn followups_do_not_invent_completed_comparisons_or_retry_failures_forever() {
        let mut record = json!({"complete":true,"status":"differences"});
        assert!(needed(&record));
        record["whitespace_diagnosis"] = json!({"version":VERSION,"status":"failed"});
        assert!(!needed(&record));
        record["complete"] = json!(false);
        record["whitespace_diagnosis"] = Value::Null;
        assert!(!needed(&record));
        let profile = json!({"sheets":[{"index":1,"image_differences":[{}]},
            {"index":2,"row_heights":[{}],"image_differences":[{}]}]});
        assert_eq!(image_sheets(&profile), BTreeSet::from([1]));
    }

    #[test]
    fn matching_regions_with_different_pixel_values_do_not_reproduce_evidence() -> Result<()> {
        let root = super::super::tests::test_directory("whitespace-evidence")?;
        for (name, color) in [("before", 0), ("after", 255), ("different", 128)] {
            image::RgbaImage::from_pixel(2, 2, image::Rgba([color, 0, 0, 255]))
                .save(root.join(format!("{name}.png")))?;
        }
        let previous = json!({"image_differences":[{"range":"A1:B2","region":[0,0,2,2],
            "original_image":"before.png","shrunk_image":"after.png"}]});
        assert!(evidence_matches(&previous, &previous, &root)?);
        let mut changed = previous.clone();
        changed["image_differences"][0]["shrunk_image"] = json!("different.png");
        assert!(!evidence_matches(&previous, &changed, &root)?);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
