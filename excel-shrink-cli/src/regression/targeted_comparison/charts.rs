//! Whole-chart evidence is unconditional and independent of changed worksheet cells.
use super::{
    evidence::{images_differ, Evidence},
    host::ExcelHost,
    telemetry::{LaneId, Telemetry},
};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(super) struct Chart {
    pub sheet: String,
    pub name: String,
    #[serde(default)]
    pub kind: ChartKind,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ChartKind {
    #[default]
    Embedded,
    ChartSheet,
}

pub(super) type Captures = BTreeMap<Chart, Evidence>;

#[derive(Serialize)]
pub(super) struct Finding {
    pub sheet: String,
    pub chart: String,
    pub kind: ChartKind,
    pub presence_differs: bool,
    pub pixels_differ: bool,
    pub geometry_differs: bool,
    pub left_image: Option<PathBuf>,
    pub right_image: Option<PathBuf>,
    pub left_geometry: Option<serde_json::Value>,
    pub right_geometry: Option<serde_json::Value>,
}

/// Export every chart once per retained state; failed inventory/export is an error.
pub(super) fn capture(
    excel: &mut ExcelHost,
    lane: LaneId,
    directory: &Path,
    telemetry: &Telemetry,
) -> Result<Captures> {
    fs::create_dir_all(directory)?;
    let mut captures = BTreeMap::new();
    for chart in excel.charts(lane, telemetry)? {
        ensure!(
            !captures.contains_key(&chart),
            "Duplicate chart identity: {} / {}",
            chart.sheet,
            chart.name
        );
        telemetry.update(|state| {
            state.lane(lane).current_sheet = Some(chart.sheet.clone());
            state.lane(lane).current_range = Some(format!("Chart: {}", chart.name));
        })?;
        let name = format!("{:x}", Sha256::digest(serde_json::to_vec(&chart)?));
        let path = directory.join(format!("{name}.image"));
        let geometry = excel.capture_chart(lane, &chart, &path, telemetry)?;
        let image = image::ImageReader::open(&path)?
            .with_guessed_format()?
            .decode()?;
        ensure!(
            image.width() > 0 && image.height() > 0,
            "Empty chart export"
        );
        captures.insert(chart, Evidence { path, geometry });
    }
    Ok(captures)
}

/// Compare the union: an added/deleted chart is a difference, never identical evidence.
pub(super) fn compare(left: &Captures, right: &Captures) -> Result<Vec<Finding>> {
    let charts: BTreeSet<_> = left.keys().chain(right.keys()).collect();
    charts
        .into_iter()
        .map(|chart| {
            let a = left.get(chart);
            let b = right.get(chart);
            let (pixels_differ, geometry_differs) = match (a, b) {
                (Some(a), Some(b)) => (images_differ(&a.path, &b.path)?, a.geometry != b.geometry),
                _ => (false, false),
            };
            Ok(Finding {
                sheet: chart.sheet.clone(),
                chart: chart.name.clone(),
                kind: chart.kind,
                presence_differs: a.is_none() || b.is_none(),
                pixels_differ,
                geometry_differs,
                left_image: a.map(|e| e.path.clone()),
                right_image: b.map(|e| e.path.clone()),
                left_geometry: a.map(|e| e.geometry.clone()),
                right_geometry: b.map(|e| e.geometry.clone()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn removed_chart_is_reported_without_inventing_partner_evidence() -> Result<()> {
        let left = BTreeMap::from([(
            Chart {
                sheet: "Data".into(),
                name: "Chart 1".into(),
                kind: ChartKind::Embedded,
            },
            Evidence {
                path: "original.jpg".into(),
                geometry: json!({"visibility": 0, "page_chart_size": 1}),
            },
        )]);
        let findings = compare(&left, &BTreeMap::new())?;
        assert_eq!(findings.len(), 1);
        assert!(findings[0].presence_differs);
        assert!(findings[0].right_image.is_none());
        assert_eq!(
            findings[0].left_geometry,
            Some(json!({"visibility": 0, "page_chart_size": 1}))
        );
        assert!(findings[0].right_geometry.is_none());
        Ok(())
    }

    #[test]
    fn missing_export_is_an_error_when_both_inventories_contain_the_chart() {
        let captures = BTreeMap::from([(
            Chart {
                sheet: "Data".into(),
                name: "Chart 1".into(),
                kind: ChartKind::Embedded,
            },
            Evidence {
                path: "missing-chart-export.jpg".into(),
                geometry: json!({}),
            },
        )]);
        assert!(compare(&captures, &captures).is_err());
    }
}
