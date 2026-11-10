//! Native object bounds qualify an explicit reference area without using selector proofs.
use super::{capture_layout::Geometry, host::ExcelHost, telemetry::Telemetry, Candidate, LaneId};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Deserialize)]
struct NativeObject {
    name: String,
    kind: String,
    rotation: f64,
    geometry: Geometry,
}

fn matches_horizontal_line(object: &NativeObject, line: &super::ole_metadata::LineFrame) -> bool {
    object.kind == "GeometricShape"
        && object.name == line.name
        && object.rotation == 0.0
        && object.geometry.left == line.geometry.left
        && object.geometry.width == line.geometry.width
        && object.geometry.height == 0.0
        && object.geometry.top.is_finite()
        && object.geometry.top >= 0.0
}

pub(super) fn verify(
    host: &mut ExcelHost,
    regions: &[Candidate],
    telemetry: &Telemetry,
    receipt: &mut Value,
    workbooks: [&Path; 2],
) -> Result<()> {
    receipt["object_coverage"] = json!([]);
    let sheets: BTreeSet<_> = regions.iter().map(|region| &region.sheet).collect();
    for (index, lane) in [LaneId::O, LaneId::SD].into_iter().enumerate() {
        for sheet in &sheets {
            let areas: Vec<_> = regions
                .iter()
                .filter(|region| &region.sheet == *sheet)
                .cloned()
                .collect();
            let measured = host.measure_ranges(&[(lane, areas)], telemetry)?;
            let inventory = host.reference_objects(lane, sheet, telemetry)?;
            // Retain the native observation even when qualification fails.
            receipt["object_coverage"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "lane":lane.key(),"sheet":sheet,"inventory":inventory,"areas":measured[0],
                }));
            let observation = receipt["object_coverage"]
                .as_array_mut()
                .unwrap()
                .last_mut()
                .unwrap();
            match super::ole_metadata::inspect(workbooks[index], sheet) {
                Ok(metadata) => observation["ole_metadata"] = serde_json::to_value(metadata)?,
                Err(error) => {
                    observation["ole_metadata_error"] = json!(format!("{error:#}"));
                    return Err(error.context("Read native reference OLE metadata"));
                }
            }
            let lines = if inventory["objects"].as_array().is_some_and(|objects| {
                objects.iter().any(|object| object["kind"] == "GeometricShape")
            }) {
                super::ole_metadata::line_frames(workbooks[index], sheet)?
            } else {
                Vec::new()
            };
            observation["line_frames"] = serde_json::to_value(&lines)?;
            validate_with_lines(&inventory, &measured[0], &lines)?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn validate(inventory: &Value, areas: &[Geometry]) -> Result<()> {
    validate_with_lines(inventory, areas, &[])
}

/// Background-tail qualification accounts for every object through the closed line family.
pub(super) fn validate_line_sheet(
    inventory: &Value,
    areas: &[Geometry],
    lines: &[super::ole_metadata::LineFrame],
) -> Result<()> {
    let objects = inventory["objects"].as_array().context("Native object inventory missing")?;
    ensure!(!lines.is_empty() && objects.len() == lines.len()
        && objects.iter().all(|object| object["kind"] == "GeometricShape"),
        "Native sheet contains objects outside the closed line family");
    validate_with_lines(inventory, areas, lines)
}

fn validate_with_lines(
    inventory: &Value,
    areas: &[Geometry],
    lines: &[super::ole_metadata::LineFrame],
) -> Result<()> {
    let objects: Vec<NativeObject> = serde_json::from_value(
        inventory
            .get("objects")
            .context("Native object inventory missing")?
            .clone(),
    )?;
    if !lines.is_empty() {
        let native_line_count = objects.iter()
            .filter(|object| object.kind == "GeometricShape")
            .count();
        let all_lines_match = lines.iter().all(|line| {
            objects.iter().filter(|object| {
                matches_horizontal_line(object, line)
            }).count() == 1
        });
        ensure!(
            lines.len() == native_line_count && all_lines_match,
            "Line XML and native identities do not correspond one-to-one"
        );
    }
    for object in &objects {
        if object.kind == "GeometricShape" && object.rotation == 0.0 {
            if let Some(line) = lines.iter().find(|line| {
                matches_horizontal_line(object, line)
            }) {
                // Include half the stroke and two points of rasterization context.
                let padding = line.stroke_points / 2.0 + 2.0;
                // Excel may resolve a row anchor at a different height than the
                // stored transform. Capture both positions, including stroke.
                let left = (line.geometry.left - padding).max(0.0);
                let top = (line.geometry.top.min(object.geometry.top) - padding).max(0.0);
                let envelope = Geometry {
                    left,
                    top,
                    width: line.geometry.left + line.geometry.width + padding - left,
                    height: line.geometry.top.max(object.geometry.top) + padding - top,
                }.validate()?;
                ensure!(
                    areas.iter().any(|area| contains(*area, envelope)),
                    "Reference line {} extends outside its padded supplied rectangle",
                    object.name
                );
                continue;
            }
        }
        // Excel also exposes chart containers as Unsupported shapes. Accept an
        // alias only when the named native chart has exactly the same rectangle.
        if object.kind == "Unsupported"
            && object.rotation == 0.0
            && objects.iter().any(|chart| {
                chart.kind == "Chart"
                    && chart.name == object.name
                    && chart.geometry == object.geometry
            })
        {
            continue;
        }
        ensure!(
            matches!(object.kind.as_str(), "Image" | "Chart"),
            "Reference object {} has unsupported kind {}",
            object.name,
            object.kind
        );
        ensure!(
            object.rotation == 0.0,
            "Rotated reference object requires a separate envelope"
        );
        let bounds = object.geometry.validate()?;
        ensure!(
            areas.iter().any(|area| contains(*area, bounds)),
            "Reference object {} extends outside a single supplied rectangle",
            object.name
        );
    }
    Ok(())
}

fn contains(area: Geometry, object: Geometry) -> bool {
    object.left >= area.left
        && object.top >= area.top
        && object.left + object.width <= area.left + area.width
        && object.top + object.height <= area.top + area.height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires LINE_DIAG_REFERENCE with the retained Abwasser native inventory"]
    fn retained_abwasser_inventory_matches_closed_line_frames() -> Result<()> {
        let path = std::path::PathBuf::from(std::env::var("LINE_DIAG_REFERENCE")?);
        let report: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let workbook = path.parent().context("Missing reference directory")?.join("original.xlsx");
        assert_eq!(
            crate::regression::artifact_cache::hash_file(&workbook)?,
            report["original_rebuilt_sha256"].as_str().context("Missing rebuilt identity")?,
        );
        let observation = &report["object_coverage"][0];
        assert_eq!(observation["sheet"], "Inhalt");
        let lines = super::super::ole_metadata::line_frames(&workbook, "Inhalt")?;
        assert_eq!(lines.len(), 2);
        let areas: Vec<Geometry> = serde_json::from_value(observation["areas"].clone())?;
        validate_with_lines(&observation["inventory"], &areas, &lines)?;
        validate_line_sheet(&observation["inventory"], &areas, &lines)?;
        Ok(())
    }

    #[test]
    fn qualified_lines_need_matching_native_frames_and_stroke_margin() -> Result<()> {
        let line = super::super::ole_metadata::LineFrame {
            name: "Line 1".into(),
            geometry: Geometry { top: 10.0, left: 10.0, width: 70.0, height: 0.0 },
            stroke_points: 0.85,
        };
        let inventory = json!({"objects":[{
            "name":"Line 1", "kind":"GeometricShape", "rotation":0,
            "geometry":line.geometry,
        }]});
        let area = Geometry { top: 0.0, left: 0.0, width: 100.0, height: 100.0 };
        validate_with_lines(&inventory, &[area], std::slice::from_ref(&line))?;
        validate_line_sheet(&inventory, &[area], std::slice::from_ref(&line))?;
        let mut extra = inventory.clone();
        extra["objects"].as_array_mut().unwrap().push(json!({"kind":"Image"}));
        assert!(validate_line_sheet(&extra, &[area], std::slice::from_ref(&line)).is_err());
        assert!(validate_line_sheet(&json!({"objects":[]}), &[area], &[]).is_err());
        assert!(validate(&inventory, &[area]).is_err());
        assert!(validate_with_lines(&inventory, &[line.geometry], std::slice::from_ref(&line)).is_err());
        let mut moved = inventory.clone();
        moved["objects"][0]["geometry"]["top"] = json!(11);
        validate_with_lines(&moved, &[area], std::slice::from_ref(&line))?;
        moved["objects"][0]["geometry"]["top"] = json!(99);
        assert!(validate_with_lines(&moved, &[area], std::slice::from_ref(&line)).is_err());
        moved["objects"][0]["geometry"]["top"] = json!(11);
        moved["objects"][0]["geometry"]["width"] = json!(71);
        assert!(validate_with_lines(&moved, &[area], std::slice::from_ref(&line)).is_err());
        let mut rotated = inventory;
        rotated["objects"][0]["rotation"] = json!(45);
        assert!(validate_with_lines(&rotated, &[area], &[line]).is_err());
        Ok(())
    }

    #[test]
    fn only_supported_unrotated_contained_objects_qualify() -> Result<()> {
        let area = Geometry {
            top: 0.0,
            left: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let picture = json!({"name":"logo","kind":"Image","rotation":0,
            "geometry":{"top":10,"left":10,"width":90,"height":90}});
        validate(&json!({"objects":[picture.clone()]}), &[area])?;
        validate(&json!({"objects":[]}), &[area])?;
        for (field, value) in [("kind", json!("Group")), ("rotation", json!(45))] {
            let mut changed = picture.clone();
            changed[field] = value;
            assert!(validate(&json!({"objects":[changed]}), &[area]).is_err());
        }
        let mut outside = picture;
        outside["geometry"]["width"] = json!(91);
        assert!(validate(&json!({"objects":[outside]}), &[area]).is_err());
        assert!(validate(&json!({}), &[area]).is_err());
        let chart = json!({"name":"plot","kind":"Chart","rotation":0,
            "geometry":{"top":10,"left":10,"width":90,"height":90}});
        let mut alias = chart.clone();
        alias["kind"] = json!("Unsupported");
        validate(&json!({"objects":[alias.clone(),chart.clone()]}), &[area])?;
        alias["name"] = json!("other-object");
        assert!(validate(&json!({"objects":[alias,chart]}), &[area]).is_err());
        Ok(())
    }
}
