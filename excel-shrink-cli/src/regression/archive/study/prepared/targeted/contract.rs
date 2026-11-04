//! Reject incomplete successful-looking reports before classifying or deleting evidence.
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::collections::BTreeSet;

/// Every requested pair must cover the same unique selected regions with explicit verdicts.
pub(super) fn has_differences(report: &Value, skip_performance: bool) -> Result<bool> {
    ensure!(
        report["status"] == "completed_with_limitations",
        "Targeted report is incomplete"
    );
    let inplace = report["execution_strategy"] == "default-inplace-rebuild-v1";
    let count = report["candidate_count"]
        .as_u64()
        .context("Missing candidate count")?;
    let pairs = report["comparisons"]
        .as_array()
        .context("Missing comparison results")?;
    let mut expected = BTreeSet::from([
        ("O", "SD"),
        ("O", "SDR"),
        ("SD", "SDR"),
        ("O", "OR"),
        ("OR", "SDR"),
    ]);
    if !skip_performance {
        expected.extend([("O", "CP"), ("O", "CPR"), ("CP", "CPR"), ("OR", "CPR")]);
    }
    if matches!(
        report["capture_policy"].as_str(),
        Some(
            "targeted-content-grid-v3"
                | "targeted-content-grid-v4"
                | "targeted-content-grid-v5"
                | "targeted-content-grid-v6"
                | "targeted-content-grid-v7"
        )
    ) {
        expected = super::execution_contract::expected_pairs(report, skip_performance)?;
    }
    ensure!(
        pairs.len() == expected.len(),
        "Incomplete comparison pair coverage"
    );
    let mut region_set = None;
    let mut changed = false;
    for pair in pairs {
        let left = pair["left"].as_str().context("Missing comparison source")?;
        let right = pair["right"]
            .as_str()
            .context("Missing comparison target")?;
        ensure!(
            expected.remove(&(left, right)),
            "Duplicate or unexpected comparison pair: {left}/{right}"
        );
        let findings = pair["findings"]
            .as_array()
            .context("Missing region findings")?;
        ensure!(
            findings.len() as u64
                == if inplace {
                    pair["selected_region_count"]
                        .as_u64()
                        .context("Missing phase-specific region count")?
                } else {
                    count
                },
            "Incomplete region coverage for {left}/{right}"
        );
        let mut regions = BTreeSet::new();
        for finding in findings {
            let sheet = finding["sheet"].as_str().context("Missing finding sheet")?;
            let range = finding["cell"].as_str().context("Missing finding range")?;
            ensure!(regions.insert((sheet, range)), "Duplicate region finding");
            changed |= difference_flags(finding, &["pixels_differ", "geometry_differs"])?;
            if report["capture_partition"] == "visible-geometry-v1" {
                changed |= validate_rendering(finding)?;
            } else {
                for field in ["left_png", "right_png"] {
                    ensure!(
                        finding[field].as_str().is_some_and(|s| !s.is_empty()),
                        "Missing image reference: {field}"
                    );
                }
            }
        }
        if !inplace {
            if let Some(previous) = &region_set {
                ensure!(
                    previous == &regions,
                    "Comparison pairs cover different regions"
                );
            } else {
                region_set = Some(regions);
            }
        }
        let charts = pair["charts"]
            .as_array()
            .context("Missing chart findings")?;
        let left_charts = report["capture_counts"][format!("{left}:charts")]
            .as_u64()
            .context("Missing left chart capture count")?;
        let right_charts = report["capture_counts"][format!("{right}:charts")]
            .as_u64()
            .context("Missing right chart capture count")?;
        ensure!(
            charts.len() as u64 >= left_charts.max(right_charts),
            "Missing rendered chart findings"
        );
        for chart in charts {
            changed |= difference_flags(
                chart,
                &[
                    "pixels_differ",
                    "geometry_differs",
                    "presence_differs",
                    "rendering_differs",
                ],
            )?;
        }
    }
    Ok(changed)
}

/// An absent PNG is valid only with explicit zero-area geometry, never missing evidence.
fn validate_rendering(finding: &Value) -> Result<bool> {
    let mut captured = Vec::new();
    for side in ["left", "right"] {
        let geometry = &finding[format!("{side}_geometry")];
        let mut dimensions = Vec::new();
        for field in ["top", "left", "width", "height"] {
            dimensions.push(
                geometry[field]
                    .as_f64()
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .context("Invalid zero-area/capture geometry")?,
            );
        }
        let empty = dimensions[2] == 0.0 || dimensions[3] == 0.0;
        let rendering = finding[format!("{side}_rendering")]
            .as_str()
            .context("Missing rendering state")?;
        ensure!(
            rendering == if empty { "zero-area-range" } else { "captured" },
            "Rendering state contradicts geometry"
        );
        let image = &finding[format!("{side}_png")];
        ensure!(
            if empty {
                image.is_null()
            } else {
                image.as_str().is_some_and(|s| !s.is_empty())
            },
            "Image reference contradicts visible area"
        );
        captured.push(!empty);
    }
    let rendering_differs = captured[0] != captured[1];
    ensure!(
        finding["pixels_compared"].as_bool() == Some(captured[0] && captured[1]),
        "Invalid pixel comparison state"
    );
    ensure!(
        finding["rendering_differs"].as_bool() == Some(rendering_differs),
        "Invalid rendering verdict"
    );
    if !(captured[0] && captured[1]) {
        ensure!(
            finding["pixels_differ"] == false,
            "Unperformed pixel comparison has a pixel verdict"
        );
    }
    let left = &finding["left_geometry"];
    let right = &finding["right_geometry"];
    let raw_geometry_differs = ["top", "left", "width", "height"]
        .iter()
        .any(|field| left[*field].as_f64() != right[*field].as_f64());
    let ignored_margin = finding["blank_margin_ignored"] == true;
    if ignored_margin {
        ensure!(
            raw_geometry_differs && captured[0] && captured[1]
                && finding["pixels_differ"] == false
                && crate::regression::targeted_comparison::matching_content_geometry(left, right),
            "Blank-margin exclusion contradicts measured content evidence"
        );
    }
    ensure!(
        finding["geometry_differs"].as_bool() == Some(raw_geometry_differs && !ignored_margin),
        "Geometry verdict contradicts measured evidence"
    );
    Ok(rendering_differs)
}

fn difference_flags(finding: &Value, fields: &[&str]) -> Result<bool> {
    let mut changed = false;
    for field in fields {
        changed |= finding[field]
            .as_bool()
            .with_context(|| format!("Missing boolean verdict: {field}"))?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[ignore = "read-only audit; requires TARGETED_REPORT_REVIEW_ROOT"]
    fn retained_completed_reports_satisfy_the_contract() -> Result<()> {
        let root = std::path::PathBuf::from(std::env::var("TARGETED_REPORT_REVIEW_ROOT")?);
        let queue: Value =
            serde_json::from_slice(&std::fs::read(root.join("control-room/queue.json"))?)?;
        let mut reviewed = 0;
        for group in queue["groups"].as_array().context("Missing queue groups")? {
            if !matches!(group["status"].as_str(), Some("unchanged" | "differences")) {
                continue;
            }
            let id = group["id"].as_str().context("Missing group identity")?;
            let record: Value = serde_json::from_slice(&std::fs::read(
                root.join("records").join(format!("{id}.json")),
            )?)?;
            let Some(path) = record["targeted_comparison"]["report"].as_str() else {
                continue;
            };
            let report: Value = serde_json::from_slice(&std::fs::read(path)?)?;
            let changed = has_differences(
                &report,
                record["targeted_comparison"]["skip_check_performance"] == true,
            )?;
            ensure!(
                changed == (record["status"] == "differences"),
                "Stored verdict differs for {id}"
            );
            reviewed += 1;
        }
        ensure!(reviewed > 0, "No completed reports were reviewed");
        println!("Validated {reviewed} retained reports without modifying them");
        Ok(())
    }

    #[test]
    fn inplace_phases_allow_late_regions_but_reject_incomplete_phase_evidence() -> Result<()> {
        let mut report = report();
        report["capture_policy"] = json!("targeted-content-grid-v3");
        report["execution_strategy"] = json!("default-inplace-rebuild-v1");
        report["input_identity"] = json!({"status":"different"});
        report["state_decisions"] = json!({
            "O":{"status":"observed","method":"open-save"},
            "SD":{"status":"observed","method":"open-save"},
            "SDR":{"status":"observed","method":"inplace-rebuild"},
            "OR":{"status":"disabled"},"CP":{"status":"disabled"},"CPR":{"status":"disabled"}
        });
        report["excel_pids"] = json!({"O":10,"SD":20,"SDR":20});
        report["comparisons"].as_array_mut().unwrap().truncate(2);
        report["comparisons"][0]["selected_region_count"] = json!(1);
        report["comparisons"][1]["selected_region_count"] = json!(2);
        let mut late = report["comparisons"][1]["findings"][0].clone();
        late["cell"] = json!("C3:D4");
        late["pixels_differ"] = json!(true);
        report["comparisons"][1]["findings"]
            .as_array_mut()
            .unwrap()
            .push(late);
        report["candidate_count"] = json!(2);
        assert!(has_differences(&report, true)?);
        report["comparisons"][1]["findings"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(has_differences(&report, true).is_err());
        Ok(())
    }
    fn report() -> Value {
        let pairs = [
            ("O", "SD"),
            ("O", "SDR"),
            ("SD", "SDR"),
            ("O", "OR"),
            ("OR", "SDR"),
        ];
        json!({"status":"completed_with_limitations","candidate_count":1,
            "capture_counts":{"O:charts":0,"SD:charts":0,"SDR:charts":0,"OR:charts":0},
            "comparisons":pairs.iter().map(|(left,right)| json!({
            "left":left,"right":right,"charts":[],"findings":[{"sheet":"Data","cell":"A1:B2",
            "pixels_differ":false,"geometry_differs":false,"left_png":"left.png","right_png":"right.png"}]
        })).collect::<Vec<_>>()})
    }

    #[test]
    fn zero_area_observations_are_explicit_and_visibility_changes_are_differences() -> Result<()> {
        let mut report = report();
        report["capture_partition"] = json!("visible-geometry-v1");
        for pair in report["comparisons"].as_array_mut().unwrap() {
            let f = &mut pair["findings"][0];
            for side in ["left", "right"] {
                f[format!("{side}_png")] = Value::Null;
                f[format!("{side}_rendering")] = json!("zero-area-range");
                f[format!("{side}_geometry")] = json!({"top":15,"left":0,"width":100,"height":0});
            }
            f["pixels_compared"] = json!(false);
            f["rendering_differs"] = json!(false);
        }
        assert!(!has_differences(&report, true)?);
        let mut invalid = report.clone();
        invalid["comparisons"][0]["findings"][0]["right_geometry"]["height"] = json!(10);
        assert!(has_differences(&invalid, true).is_err());
        let f = &mut report["comparisons"][0]["findings"][0];
        f["right_geometry"]["height"] = json!(10);
        f["right_rendering"] = json!("captured");
        f["right_png"] = json!("visible.png");
        f["geometry_differs"] = json!(true);
        f["rendering_differs"] = json!(true);
        assert!(has_differences(&report, true)?);
        Ok(())
    }

    #[test]
    fn blank_margin_exclusions_require_unchanged_measured_content() -> Result<()> {
        let mut report = report();
        report["capture_partition"] = json!("visible-geometry-v1");
        for pair in report["comparisons"].as_array_mut().unwrap() {
            let f = &mut pair["findings"][0];
            let core = json!({"top":0,"left":0,"width":90,"height":15});
            f["left_geometry"] = json!({"top":0,"left":0,"width":100,"height":30,"comparison_core":core});
            f["right_geometry"] = json!({"top":0,"left":0,"width":100,"height":35,"comparison_core":core});
            f["blank_margin_ignored"] = json!(true);
            f["pixels_compared"] = json!(true);
            f["rendering_differs"] = json!(false);
            f["left_rendering"] = json!("captured");
            f["right_rendering"] = json!("captured");
        }
        assert!(!has_differences(&report, true)?);
        for field in ["top", "left", "width", "height"] {
            let mut invalid = report.clone();
            invalid["comparisons"][0]["findings"][0]["right_geometry"]["comparison_core"][field] = json!(7);
            assert!(has_differences(&invalid, true).is_err());
        }
        let mut invalid = report.clone();
        invalid["comparisons"][0]["findings"][0]["pixels_differ"] = json!(true);
        assert!(has_differences(&invalid, true).is_err());
        report["comparisons"][0]["findings"][0]["blank_margin_ignored"] = json!(false);
        assert!(has_differences(&report, true).is_err());
        Ok(())
    }

    #[test]
    fn missing_or_partial_evidence_is_never_unchanged() -> Result<()> {
        assert!(has_differences(&json!({"status":"completed_with_limitations"}), true).is_err());
        let good = report();
        assert!(!has_differences(&good, true)?);
        assert!(has_differences(&good, false).is_err());
        let mut changed = good.clone();
        changed["comparisons"][0]["findings"][0]["pixels_differ"] = json!(true);
        assert!(has_differences(&changed, true)?);
        for field in ["pixels_differ", "geometry_differs", "left_png", "right_png"] {
            let mut broken = good.clone();
            broken["comparisons"][0]["findings"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(
                has_differences(&broken, true).is_err(),
                "accepted missing {field}"
            );
        }
        let mut broken = good.clone();
        broken["capture_counts"]["O:charts"] = json!(1);
        assert!(has_differences(&broken, true).is_err());
        let mut broken = good.clone();
        broken["comparisons"][0]["findings"] = json!([]);
        assert!(has_differences(&broken, true).is_err());
        let mut broken = good.clone();
        broken["comparisons"][1] = broken["comparisons"][0].clone();
        assert!(has_differences(&broken, true).is_err());
        let mut broken = good;
        broken["comparisons"][1]["findings"][0]["cell"] = json!("C1:D2");
        assert!(has_differences(&broken, true).is_err());
        Ok(())
    }
}
