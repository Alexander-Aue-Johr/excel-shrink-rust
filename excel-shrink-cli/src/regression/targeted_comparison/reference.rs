//! Native reference observations from an explicit plan independent of candidate selection.

use super::{content_bounds::Rectangle, telemetry::LaneId, Candidate, Comparison};
use anyhow::{ensure, Context, Result};
use clap::Parser;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::PathBuf, time::Instant};
use super::reference_objects as objects;

#[derive(Debug, Parser)]
pub(crate) struct Args {
    #[arg(long)]
    original: PathBuf,
    #[arg(long)]
    default: PathBuf,
    /// JSON array of {sheet, range}, frozen independently of candidate selection.
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..))]
    operation_timeout_seconds: u64,
    /// Require supported native picture/chart bounds to fit inside the supplied areas.
    #[arg(long)]
    verify_objects: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Region {
    sheet: String,
    range: String,
}

/// Rebuild both working copies and render exactly the supplied reference areas.
/// Production selection, content clipping, CF proofs and chart exclusion never
/// choose these ranges. The plan author owns their asserted full-area coverage.
pub(crate) fn run(input: Args) -> Result<()> {
    let plan_bytes = fs::read(&input.plan)?;
    let regions = parse_plan(&plan_bytes)?;
    let args = super::Args {
        original: input.original,
        default: input.default,
        output: input.output,
        host_script: None,
        observatory_group_dir: None,
        workbook_label: Some("Independent reference comparison".into()),
        operation_timeout_seconds: input.operation_timeout_seconds,
        skip_check_performance: true,
        check_performance: false,
        serial_captures: false,
        chart_capture_mode: Default::default(),
        prepare_artifacts_only: false,
    };
    let mut receipt = json!({
        "version": 1, "status": "running", "regions": [],
        "selection": "explicit_independent_reference_plan",
        "plan": serde_json::from_slice::<Value>(&plan_bytes)?,
        "plan_sha256": digest(&plan_bytes),
        "original_sha256": deltasieve_engine::reuse::file_sha256(&args.original)?,
        "default_sha256": deltasieve_engine::reuse::file_sha256(&args.default)?,
        "producer_sha256": deltasieve_engine::reuse::file_sha256(&std::env::current_exe()?)?,
        "started_at": chrono::Utc::now().to_rfc3339(),
        "limitations": [
            "Coverage is exactly the supplied areas; the command does not establish their completeness.",
            "Both workbooks are rebuilt and saved in the native renderer before reference captures.",
            "Oversized images and failed commands remain failed reference cases."
        ]
    });
    let started = Instant::now();
    let mut comparison = Comparison::prepare(&args)?;
    comparison.telemetry.update(|state| {
        state.capture_policy = "explicit-reference-plan-v1";
        state.execution_strategy = "reference-paired-rebuild-v1";
    })?;
    receipt["verify_objects"] = json!(input.verify_objects);
    if input.verify_objects {
        receipt["limitations"].as_array_mut().unwrap().push(json!(
            "Object qualification covers native image/chart rectangles, not decorative ink outside their reported bounds."
        ));
    }
    let result = observe(
        &mut comparison,
        &regions,
        &mut receipt,
        input.verify_objects,
    );
    let cleanup = comparison.release_excel_and_wait_for_exit();
    let error = result.as_ref().err().or(cleanup.as_ref().err());
    receipt["status"] = json!(if error.is_none() {
        "completed"
    } else {
        "failed"
    });
    receipt["error"] = json!(error.map(|error| format!("{error:#}")));
    receipt["elapsed_seconds"] = json!(started.elapsed().as_secs_f64());
    receipt["finished_at"] = json!(chrono::Utc::now().to_rfc3339());
    publish(&args.output, &receipt)?;
    comparison.telemetry.update(|state| {
        state.status = receipt["status"].as_str().unwrap().into();
        state.finished_at = Some(chrono::Utc::now().timestamp_millis());
    })?;
    result?;
    cleanup
}

fn parse_plan(bytes: &[u8]) -> Result<Vec<Candidate>> {
    let input: Vec<Region> = serde_json::from_slice(bytes)?;
    ensure!(!input.is_empty(), "Reference plan is empty");
    let mut unique = BTreeSet::new();
    let mut regions = Vec::new();
    for region in input {
        ensure!(
            !region.sheet.trim().is_empty(),
            "Reference sheet name is empty"
        );
        let rectangle = Rectangle::parse(&region.range)?;
        ensure!(
            rectangle.top > 0
                && rectangle.left > 0
                && rectangle.bottom <= 1_048_576
                && rectangle.right <= 16_384,
            "Reference range exceeds Excel's grid"
        );
        let candidate = Candidate {
            sheet: region.sheet,
            cell: rectangle.address(),
        };
        ensure!(unique.insert(candidate.clone()), "Duplicate reference area");
        regions.push(candidate);
    }
    Ok(regions)
}

fn observe(
    comparison: &mut Comparison<'_>,
    regions: &[Candidate],
    receipt: &mut Value,
    verify_objects: bool,
) -> Result<()> {
    publish(&comparison.args.output, receipt)?;
    comparison.open_workbooks(true)?;
    let host = comparison
        .excel
        .as_mut()
        .context("Reference Excel host is missing")?;
    for lane in [LaneId::O, LaneId::SD] {
        host.recalculate_workbook(lane, &comparison.telemetry)?;
        host.save_workbook(lane, &comparison.telemetry)?;
    }
    receipt["original_rebuilt_sha256"] = json!(deltasieve_engine::reuse::file_sha256(
        &comparison.paths.original
    )?);
    receipt["default_rebuilt_sha256"] = json!(deltasieve_engine::reuse::file_sha256(
        &comparison.paths.default
    )?);
    if verify_objects {
        objects::verify(host, regions, &comparison.telemetry, receipt)?;
        publish(&comparison.args.output, receipt)?;
    }
    for (index, region) in regions.iter().enumerate() {
        let measured = host.measure_ranges(
            &[
                (LaneId::O, vec![region.clone()]),
                (LaneId::SD, vec![region.clone()]),
            ],
            &comparison.telemetry,
        )?;
        let geometries = [measured[0][0], measured[1][0]];
        ensure!(
            geometries.iter().all(|geometry| geometry.fits()),
            "Reference area exceeds image limits; split its frozen plan independently"
        );
        let mut images = Vec::new();
        let mut retained = Vec::new();
        for (lane, geometry) in [LaneId::O, LaneId::SD].into_iter().zip(geometries) {
            if geometry.empty() {
                images.push(None);
                retained.push(Value::Null);
                continue;
            }
            let path = comparison
                .args
                .output
                .join(format!("reference-{index}-{}.png", lane.key()));
            let observed = host.capture_range(
                lane,
                &region.sheet,
                &region.cell,
                &path,
                &comparison.telemetry,
            )?;
            let observed: super::capture_layout::Geometry = serde_json::from_value(observed)?;
            ensure!(
                observed == geometry,
                "Reference geometry changed during capture"
            );
            images.push(Some(read_image(&path)?));
            retained.push(json!({
                "file": path.file_name().context("Reference image has no filename")?.to_string_lossy(),
                "sha256": deltasieve_engine::reuse::file_sha256(&path)?,
                "bytes": fs::metadata(&path)?.len()
            }));
        }
        let pixels_differ = match (&images[0], &images[1]) {
            (Some(left), Some(right)) => Some(left != right),
            _ => None,
        };
        receipt["regions"].as_array_mut().unwrap().push(json!({
            "sheet": region.sheet, "range": region.cell,
            "original_geometry": geometries[0], "default_geometry": geometries[1],
            "pixels_differ": pixels_differ,
            "pixels_compared": pixels_differ.is_some(),
            "rendering_differs": images[0].is_some() != images[1].is_some(),
            "original_rendered": images[0].is_some(), "default_rendered": images[1].is_some(),
            "geometry_differs": geometries[0] != geometries[1],
            "images": retained,
            "image_dimensions": images.iter().map(|image| image.as_ref().map(|image|
                [image.width(), image.height()])).collect::<Vec<_>>()
        }));
        publish(&comparison.args.output, receipt)?;
    }
    for lane in [LaneId::O, LaneId::SD] {
        comparison.close_workbook(lane)?;
    }
    Ok(())
}

fn read_image(path: &std::path::Path) -> Result<image::RgbaImage> {
    let (width, height) = image::image_dimensions(path)?;
    ensure!(
        width <= super::capture_layout::MAX_EDGE
            && height <= super::capture_layout::MAX_EDGE
            && u64::from(width) * u64::from(height) <= super::capture_layout::MAX_PIXELS,
        "Actual reference image exceeds its decode budget"
    );
    Ok(image::open(path)?.into_rgba8())
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn publish(root: &std::path::Path, receipt: &Value) -> Result<()> {
    crate::regression::write_atomic_artifact(
        &root.join("reference.json"),
        &serde_json::to_vec_pretty(receipt)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_plan_preserves_explicit_areas_and_rejects_ambiguous_input() -> Result<()> {
        let plan = parse_plan(br#"[{"sheet":"Data","range":"B3:D9"}]"#)?;
        assert_eq!(plan[0].cell, "B3:D9");
        for text in [
            "[]",
            r#"[{"sheet":"","range":"A1"}]"#,
            r#"[{"sheet":"Data","range":"A0:B2"}]"#,
            r#"[{"sheet":"Data","range":"A1:XFE2"}]"#,
            r#"[{"sheet":"Data","range":"B3:A1"}]"#,
            r#"[{"sheet":"Data","range":"A1","unknown":true}]"#,
            r#"[{"sheet":"Data","range":"A1"},{"sheet":"Data","range":"A1:A1"}]"#,
        ] {
            assert!(parse_plan(text.as_bytes()).is_err(), "Accepted {text}");
        }
        Ok(())
    }
}
