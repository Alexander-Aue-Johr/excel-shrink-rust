//! Native object bounds qualify an explicit reference area without using selector proofs.
use super::{capture_layout::Geometry, host::ExcelHost, telemetry::Telemetry, Candidate, LaneId};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Deserialize)]
struct NativeObject {
    name: String,
    kind: String,
    rotation: f64,
    geometry: Geometry,
}

pub(super) fn verify(
    host: &mut ExcelHost,
    regions: &[Candidate],
    telemetry: &Telemetry,
    receipt: &mut Value,
) -> Result<()> {
    receipt["object_coverage"] = json!([]);
    let sheets: BTreeSet<_> = regions.iter().map(|region| &region.sheet).collect();
    for lane in [LaneId::O, LaneId::SD] {
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
            validate(&inventory, &measured[0])?;
        }
    }
    Ok(())
}

fn validate(inventory: &Value, areas: &[Geometry]) -> Result<()> {
    let objects: Vec<NativeObject> = serde_json::from_value(
        inventory
            .get("objects")
            .context("Native object inventory missing")?
            .clone(),
    )?;
    for object in &objects {
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
