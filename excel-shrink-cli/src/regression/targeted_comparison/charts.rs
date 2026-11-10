//! Keep dependency exclusions distinct from whole-chart pixel observations.
use super::{
    host::ExcelHost,
    image_comparison::ImageComparisons,
    telemetry::{LaneId, Telemetry},
};
use anyhow::{ensure, Context, Result};
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
    pub index: Option<usize>,
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

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Inventory {
    #[serde(flatten)]
    pub chart: Chart,
    pub geometry: Option<serde_json::Value>,
}

impl Inventory {
    /// Old workers or chart sheets without complete live geometry cannot prove equality.
    pub fn valid_geometry(&self) -> Result<super::capture_layout::Geometry> {
        serde_json::from_value::<super::capture_layout::Geometry>(
            self.geometry.clone().context("Missing live chart geometry")?,
        )?.validate()
    }
}

pub(super) struct Capture {
    geometry: serde_json::Value,
    image: ChartImage,
}

enum ChartImage {
    Rendered(PathBuf),
    ZeroArea,
    UnchangedDependencies,
}

impl Capture {
    fn path(&self) -> Option<&PathBuf> {
        match &self.image {
            ChartImage::Rendered(path) => Some(path),
            ChartImage::ZeroArea => None,
            ChartImage::UnchangedDependencies => None,
        }
    }
    fn rendering(&self) -> &'static str {
        match self.image {
            ChartImage::Rendered(_) => "captured",
            ChartImage::ZeroArea => "zero-area-chart",
            ChartImage::UnchangedDependencies => "unchanged-chart-dependencies",
        }
    }
}

pub(super) type Captures = BTreeMap<Chart, Capture>;

#[derive(Serialize)]
pub(super) struct Finding {
    pub sheet: String,
    pub chart: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart_index: Option<usize>,
    pub kind: ChartKind,
    pub presence_differs: bool,
    pub pixels_differ: bool,
    pub geometry_differs: bool,
    pub pixels_compared: bool,
    pub dependencies_unchanged: bool,
    pub rendering_differs: bool,
    pub left_rendering: Option<&'static str>,
    pub right_rendering: Option<&'static str>,
    pub left_image: Option<PathBuf>,
    pub right_image: Option<PathBuf>,
    pub left_geometry: Option<serde_json::Value>,
    pub right_geometry: Option<serde_json::Value>,
}

/// Observe every chart; export positive-area charts and reject unexplained missing images.
pub(super) fn capture(
    excel: &mut ExcelHost,
    lane: LaneId,
    directory: &Path,
    telemetry: &Telemetry,
    inventory: Vec<Inventory>,
    skipped: &BTreeSet<Chart>,
) -> Result<Captures> {
    fs::create_dir_all(directory)?;
    let mut captures = BTreeMap::new();
    for entry in inventory {
        let chart = entry.chart;
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
        let capture = if skipped.contains(&chart) {
            Capture { geometry: entry.geometry.context("Skipped chart requires live geometry")?,
                image: ChartImage::UnchangedDependencies }
        } else {
            export(excel, lane, &chart, path, telemetry)?
        };
        captures.insert(chart, capture);
    }
    Ok(captures)
}

fn export(
    excel: &mut ExcelHost,
    lane: LaneId,
    chart: &Chart,
    path: PathBuf,
    telemetry: &Telemetry,
) -> Result<Capture> {
    let response = excel.capture_chart(lane, chart, &path, telemetry)?;
    let geometry = response["geometry"].clone();
    if response["non_rendering"] == true {
        let width = geometry["width"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0);
        let height = geometry["height"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0);
        ensure!(
            width.is_some() && height.is_some() && (width == Some(0.0) || height == Some(0.0)),
            "Missing chart image is not explained by measured zero area"
        );
        telemetry.progress(|state| state.capture_progress_sequence += 1)?;
        return Ok(Capture {
            geometry,
            image: ChartImage::ZeroArea,
        });
    }
    let image = image::ImageReader::open(&path)?
        .with_guessed_format()?
        .decode()?;
    ensure!(
        image.width() > 0 && image.height() > 0,
        "Empty chart export"
    );
    telemetry.update(|state| {
        state.capture_success_sequence += 1;
        state.capture_progress_sequence += 1;
        state.lane(lane).record_capture(&chart.sheet, &format!("Chart: {}", chart.name), &path, &geometry);
    })?;
    Ok(Capture {
        geometry,
        image: ChartImage::Rendered(path),
    })
}

pub(super) fn image_count(captures: &Captures) -> usize {
    captures
        .values()
        .filter(|capture| capture.path().is_some())
        .count()
}

/// Compare the union: an added/deleted chart is a difference, never identical evidence.
pub(super) fn compare(
    left: &Captures,
    right: &Captures,
    comparisons: &mut ImageComparisons,
) -> Result<Vec<Finding>> {
    let charts: BTreeSet<_> = left.keys().chain(right.keys()).collect();
    charts
        .into_iter()
        .map(|chart| {
            let a = left.get(chart);
            let b = right.get(chart);
            let (pixels_differ, geometry_differs, pixels_compared, rendering_differs) = match (a, b)
            {
                (Some(a), Some(b)) => {
                    let pixels = match (a.path(), b.path()) {
                        (Some(left), Some(right)) => Some(comparisons.differ(left, right)?),
                        _ => None,
                    };
                    (
                        pixels.unwrap_or(false),
                        a.geometry != b.geometry,
                        pixels.is_some(),
                        a.rendering() != b.rendering(),
                    )
                }
                _ => (false, false, false, false),
            };
            Ok(Finding {
                sheet: chart.sheet.clone(),
                chart: chart.name.clone(),
                chart_index: chart.index,
                kind: chart.kind,
                presence_differs: a.is_none() || b.is_none(),
                pixels_differ,
                geometry_differs,
                pixels_compared,
                dependencies_unchanged: a.zip(b).is_some_and(|(a, b)|
                    matches!(a.image, ChartImage::UnchangedDependencies)
                    && matches!(b.image, ChartImage::UnchangedDependencies)),
                rendering_differs,
                left_rendering: a.map(Capture::rendering),
                right_rendering: b.map(Capture::rendering),
                left_image: a.and_then(Capture::path).cloned(),
                right_image: b.and_then(Capture::path).cloned(),
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
    fn dependency_exclusions_do_not_claim_pixel_comparisons() -> Result<()> {
        let captures = BTreeMap::from([(
            Chart { sheet: "Data".into(), name: "Chart 1".into(), index: Some(0), kind: ChartKind::Embedded },
            Capture { geometry: json!({"top":0,"left":0,"width":400,"height":200}),
                image: ChartImage::UnchangedDependencies },
        )]);
        let findings = compare(&captures, &captures, &mut ImageComparisons::default())?;
        assert!(findings[0].dependencies_unchanged);
        assert!(!findings[0].pixels_compared && !findings[0].pixels_differ);
        assert!(findings[0].left_image.is_none() && findings[0].right_image.is_none());
        assert_eq!(image_count(&captures), 0);
        Ok(())
    }

    #[test]
    fn removed_chart_is_reported_without_inventing_partner_evidence() -> Result<()> {
        let left = BTreeMap::from([(
            Chart {
                sheet: "Data".into(),
                name: "Chart 1".into(),
                index: None,
                kind: ChartKind::Embedded,
            },
            Capture {
                geometry: json!({"visibility": 0, "page_chart_size": 1}),
                image: ChartImage::Rendered("original.jpg".into()),
            },
        )]);
        let findings = compare(&left, &BTreeMap::new(), &mut ImageComparisons::default())?;
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
                index: None,
                kind: ChartKind::Embedded,
            },
            Capture {
                geometry: json!({}),
                image: ChartImage::Rendered("missing-chart-export.jpg".into()),
            },
        )]);
        assert!(compare(&captures, &captures, &mut ImageComparisons::default()).is_err());
    }

    #[test]
    fn a_collapsed_chart_is_a_geometry_change_without_a_fabricated_pixel_comparison() -> Result<()>
    {
        let chart = Chart {
            sheet: "Data".into(),
            name: "Chart 11".into(),
            index: None,
            kind: ChartKind::Embedded,
        };
        let before = BTreeMap::from([(
            chart.clone(),
            Capture {
                geometry: json!({"width":1072.0,"height":400.0}),
                image: ChartImage::Rendered("already-validated.png".into()),
            },
        )]);
        let after = BTreeMap::from([(
            chart,
            Capture {
                geometry: json!({"width":1072.0,"height":0.0}),
                image: ChartImage::ZeroArea,
            },
        )]);
        let findings = compare(&before, &after, &mut ImageComparisons::default())?;
        assert!(findings[0].geometry_differs && findings[0].rendering_differs);
        assert!(!findings[0].pixels_compared);
        assert_eq!(findings[0].right_rendering, Some("zero-area-chart"));
        assert!(findings[0].right_image.is_none());
        assert_eq!(image_count(&after), 0);
        let same = compare(&after, &after, &mut ImageComparisons::default())?;
        assert!(
            !same[0].pixels_compared && !same[0].geometry_differs && !same[0].rendering_differs
        );
        Ok(())
    }

    #[test]
    fn duplicate_names_remain_separate_and_a_missing_object_is_reported() -> Result<()> {
        let chart = |index| Chart {
            sheet: "Summary".into(),
            name: "Chart 1".into(),
            index: Some(index),
            kind: ChartKind::Embedded,
        };
        let capture = || Capture {
            geometry: json!({"width": 0, "height": 80}),
            image: ChartImage::ZeroArea,
        };
        let before = BTreeMap::from([(chart(0), capture()), (chart(1), capture())]);
        let after = BTreeMap::from([(chart(0), capture())]);
        let findings = compare(&before, &after, &mut ImageComparisons::default())?;
        assert_eq!(findings.len(), 2);
        assert!(!findings[0].presence_differs);
        assert!(findings[1].presence_differs);
        assert_eq!(findings[1].chart_index, Some(1));
        let legacy: Chart = serde_json::from_value(json!({"sheet":"Summary", "name":"Chart 1"}))?;
        assert_eq!(legacy.index, None);
        Ok(())
    }

}
