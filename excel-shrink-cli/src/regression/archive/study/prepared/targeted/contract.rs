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
            findings.len() as u64 == count,
            "Incomplete region coverage for {left}/{right}"
        );
        let mut regions = BTreeSet::new();
        for finding in findings {
            let sheet = finding["sheet"].as_str().context("Missing finding sheet")?;
            let range = finding["cell"].as_str().context("Missing finding range")?;
            ensure!(regions.insert((sheet, range)), "Duplicate region finding");
            changed |= difference_flags(finding, &["pixels_differ", "geometry_differs"])?;
            for field in ["left_png", "right_png"] {
                ensure!(
                    finding[field].as_str().is_some_and(|s| !s.is_empty()),
                    "Missing image reference: {field}"
                );
            }
        }
        if let Some(previous) = &region_set {
            ensure!(
                previous == &regions,
                "Comparison pairs cover different regions"
            );
        } else {
            region_set = Some(regions);
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
