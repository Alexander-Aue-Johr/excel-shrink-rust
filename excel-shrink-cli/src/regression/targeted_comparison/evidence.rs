//! Bounded image capture and comparison of the same regions across workbook states.
use super::capture_layout::Geometry;
use super::host::ExcelHost;
use super::image_comparison::ImageComparisons;
use super::telemetry::{LaneId, Telemetry};
use super::{Candidate, Finding};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Debug, PartialEq)]
pub(super) struct ContentVerdict {
    pub pixels_differ: bool,
    pub geometry_differs: bool,
    pub blank_margin_ignored: bool,
    pub capture_extent_ignored: bool,
    pub empty_geometry_ignored: bool,
}

#[derive(Clone)]
pub(super) struct Evidence {
    pub(super) path: Option<PathBuf>,
    pub(super) geometry: Value,
}

impl Evidence {
    /// No image is legitimate only for a measured zero-area region, never a missing response.
    pub fn validate(&self) -> Result<()> {
        let geometry: super::capture_layout::Geometry =
            serde_json::from_value(self.geometry.clone())?;
        let geometry = geometry.validate()?;
        ensure!(
            self.path.is_none() == geometry.empty(),
            "Image presence disagrees with measured geometry"
        );
        Ok(())
    }

    pub fn pixels_differ(&self, other: &Self, comparisons: &mut ImageComparisons) -> Result<bool> {
        self.validate()?;
        other.validate()?;
        match (&self.path, &other.path) {
            (Some(left), Some(right)) => comparisons.differ(left, right),
            _ => Ok(false), // The separate rendering/geometry verdict records visibility transitions.
        }
    }

    /// Compare the shared image rectangle when capture/content origins agree.
    /// Keep the historical white-margin proof distinct from an overlap-only verdict.
    pub fn compare_content(
        &self,
        other: &Self,
        comparisons: &mut ImageComparisons,
    ) -> Result<ContentVerdict> {
        let pixels_differ = self.pixels_differ(other, comparisons)?;
        let left_geometry: Geometry = serde_json::from_value(self.geometry.clone())?;
        let right_geometry: Geometry = serde_json::from_value(other.geometry.clone())?;
        let geometry_differs = left_geometry != right_geometry;
        if left_geometry.empty() && right_geometry.empty() {
            return Ok(ContentVerdict {
                pixels_differ: false,
                geometry_differs: false,
                blank_margin_ignored: false,
                capture_extent_ignored: false,
                empty_geometry_ignored: geometry_differs,
            });
        }
        let blank_pair = match (&self.path, &other.path) {
            (Some(left), Some(right)) if geometry_differs => comparisons.both_blank(left, right)?,
            _ => false,
        };
        if geometry_differs
            && (blank_pair || matching_content_origin(&self.geometry, &other.geometry))
        {
            if let (Some(left), Some(right)) = (&self.path, &other.path) {
                if !comparisons.differ_with_blank_edges(left, right)? {
                    return Ok(ContentVerdict {
                        pixels_differ: false,
                        geometry_differs: false,
                        blank_margin_ignored: true,
                        capture_extent_ignored: false,
                        empty_geometry_ignored: false,
                    });
                }
                return Ok(ContentVerdict {
                    pixels_differ: comparisons.differ_in_overlap(left, right)?,
                    geometry_differs: false,
                    blank_margin_ignored: false,
                    capture_extent_ignored: true,
                    empty_geometry_ignored: false,
                });
            }
        }
        Ok(ContentVerdict {
            pixels_differ,
            geometry_differs,
            blank_margin_ignored: false,
            capture_extent_ignored: false,
            empty_geometry_ignored: false,
        })
    }

    pub fn rendering(&self) -> &'static str {
        if self.path.is_some() {
            "captured"
        } else {
            "zero-area-range"
        }
    }
}

/// Keep the measured origins fixed and require both content cores to be captured.
/// Their sizes may differ; the separate full-image proof must establish that only
/// trailing white pixels changed. Core size alone cannot establish that proof.
pub(in crate::regression) fn matching_content_origin(left: &Value, right: &Value) -> bool {
    fn measured(value: &Value) -> Option<Geometry> {
        serde_json::from_value::<Geometry>(value.clone())
            .ok()?
            .validate()
            .ok()
    }
    let (Some(left), Some(right), Some(left_core), Some(right_core)) = (
        measured(left),
        measured(right),
        measured(&left["comparison_core"]),
        measured(&right["comparison_core"]),
    ) else {
        return false;
    };
    if left.top != right.top
        || left.left != right.left
        || left_core.top != right_core.top
        || left_core.left != right_core.left
        || left_core.empty()
        || right_core.empty()
    {
        return false;
    }
    [(left, left_core), (right, right_core)].into_iter().all(|(full, core)| {
        core.left >= full.left
            && core.top >= full.top
            && core.left + core.width <= full.left + full.width
            && core.top + core.height <= full.top + full.height
    })
}

#[derive(Serialize)]
pub(super) struct PerformanceFinding {
    sheet: String,
    cell: String,
    range: String,
    before_recalc_differs: bool,
    after_recalc_differs: bool,
    original_recalc_drift: bool,
    ordinary_save_xml_changed: bool,
    optimization_xml_changed: bool,
    images: BTreeMap<&'static str, Option<PathBuf>>,
    geometry: BTreeMap<&'static str, Value>,
}

/// Compare the third lane against the same original controls, retaining full evidence.
pub(super) fn compare_performance(
    candidates: &BTreeSet<Candidate>,
    original_before: &BTreeMap<Candidate, Evidence>,
    performance_before: &BTreeMap<Candidate, Evidence>,
    performance_after: &BTreeMap<Candidate, Evidence>,
    original_after: &BTreeMap<Candidate, Evidence>,
    save_changes: &BTreeSet<Candidate>,
    optimization_changes: &BTreeSet<Candidate>,
    comparisons: &mut ImageComparisons,
) -> Result<Vec<PerformanceFinding>> {
    candidates
        .iter()
        .map(|candidate| {
            let states = [
                ("original_before", original_before.get(candidate)),
                ("performance_before", performance_before.get(candidate)),
                ("performance_after", performance_after.get(candidate)),
                ("original_after", original_after.get(candidate)),
            ];
            let evidence: BTreeMap<_, _> = states
                .into_iter()
                .map(|(name, evidence)| {
                    evidence
                        .with_context(|| {
                            format!(
                                "Missing {name} evidence for {}!{}",
                                candidate.sheet, candidate.cell
                            )
                        })
                        .map(|evidence| (name, evidence))
                })
                .collect::<Result<_>>()?;
            let original = evidence["original_before"];
            let mut differs = |other: &Evidence| -> Result<bool> {
                Ok(original.pixels_differ(other, comparisons)?
                    || original.geometry != other.geometry)
            };
            Ok(PerformanceFinding {
                sheet: candidate.sheet.clone(),
                cell: candidate.cell.clone(),
                range: padded_range(&candidate.cell)?,
                before_recalc_differs: differs(evidence["performance_before"])?,
                after_recalc_differs: differs(evidence["performance_after"])?,
                original_recalc_drift: differs(evidence["original_after"])?,
                ordinary_save_xml_changed: save_changes.contains(candidate),
                optimization_xml_changed: optimization_changes.contains(candidate),
                images: evidence
                    .iter()
                    .map(|(&name, e)| (name, e.path.clone()))
                    .collect(),
                geometry: evidence
                    .iter()
                    .map(|(&name, e)| (name, e.geometry.clone()))
                    .collect(),
            })
        })
        .collect()
}

/// Observe the padded candidate, omitting the image only for verified zero-area geometry.
pub(super) fn capture_candidate(
    excel: &mut ExcelHost,
    lane: LaneId,
    candidate: &Candidate,
    path: PathBuf,
    telemetry: &Telemetry,
) -> Result<Evidence> {
    let range = padded_range(&candidate.cell)?;
    telemetry.update(|state| {
        state.lane(lane).current_sheet = Some(candidate.sheet.clone());
        state.lane(lane).current_range = Some(range.clone());
    })?;
    let geometry = excel.capture_range(lane, &candidate.sheet, &range, &path, telemetry)?;
    let measured: super::capture_layout::Geometry = serde_json::from_value(geometry.clone())?;
    Ok(Evidence {
        path: (!measured.validate()?.empty()).then_some(path),
        geometry,
    })
}

pub(super) fn padded_range(address: &str) -> Result<String> {
    if let Some((start, end)) = address.split_once(':') {
        let a =
            super::conditional_formatting::cell_address(start).context("Invalid fallback start")?;
        let b = super::conditional_formatting::cell_address(end).context("Invalid fallback end")?;
        ensure!(
            a.0 > 0 && a.1 > 0 && a.0 <= b.0 && a.1 <= b.1,
            "Invalid fallback rectangle"
        );
        return Ok(address.into());
    }
    let split = address
        .find(|c: char| c.is_ascii_digit())
        .context("invalid cell address")?;
    let (letters, digits) = address.split_at(split);
    let row: u32 = digits.parse()?;
    let column = letters
        .bytes()
        .try_fold(0u32, |value, digit| {
            value
                .checked_mul(26)?
                .checked_add(u32::from(digit.to_ascii_uppercase().checked_sub(b'A')? + 1))
        })
        .context("invalid column")?;
    ensure!(
        row > 0 && column > 0 && row <= 1_048_576 && column <= 16_384,
        "cell out of Excel bounds"
    );
    Ok(format!(
        "{}{}:{}{}",
        column_name(column.saturating_sub(1).max(1)),
        row.saturating_sub(1).max(1),
        column_name((column + 1).min(16_384)),
        (row + 1).min(1_048_576)
    ))
}

fn column_name(mut column: u32) -> String {
    let mut result = String::new();
    while column > 0 {
        column -= 1;
        result.insert(0, (b'A' + (column % 26) as u8) as char);
        column /= 26;
    }
    result
}

pub(super) fn compare_evidence(
    candidates: &BTreeSet<Candidate>,
    original_before: &BTreeMap<Candidate, Evidence>,
    shrink_before: &BTreeMap<Candidate, Evidence>,
    shrink_after: &BTreeMap<Candidate, Evidence>,
    original_after: &BTreeMap<Candidate, Evidence>,
    original_save_changes: &BTreeSet<Candidate>,
    comparisons: &mut ImageComparisons,
) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for candidate in candidates {
        let o0 = original_before
            .get(candidate)
            .context("Missing original-before evidence")?;
        let s0 = shrink_before
            .get(candidate)
            .context("Missing shrink-before evidence")?;
        let s1 = shrink_after
            .get(candidate)
            .context("Missing shrink-after evidence")?;
        let o1 = original_after
            .get(candidate)
            .context("Missing original-after evidence")?;
        findings.push(Finding {
            sheet: candidate.sheet.clone(),
            cell: candidate.cell.clone(),
            range: padded_range(&candidate.cell)?,
            before_recalc_differs: o0.pixels_differ(s0, comparisons)?,
            after_shrink_recalc_differs: o0.pixels_differ(s1, comparisons)?,
            also_changed_by_original_recalc: o0.pixels_differ(o1, comparisons)?
                || o0.geometry != o1.geometry,
            before_recalc_geometry_differs: o0.geometry != s0.geometry,
            after_shrink_recalc_geometry_differs: o0.geometry != s1.geometry,
            original_save_xml_changed: original_save_changes.contains(candidate),
            original_before_png: o0.path.clone(),
            shrink_before_png: s0.path.clone(),
            shrink_recalc_png: s1.path.clone(),
            original_recalc_png: o1.path.clone(),
            original_open_geometry: o0.geometry.clone(),
            shrink_open_geometry: s0.geometry.clone(),
            shrink_recalc_geometry: s1.geometry.clone(),
            original_recalc_geometry: o1.geometry.clone(),
        });
    }
    Ok(findings)
}

/// Compare decoded pixels; Office chart exports do not consistently match the documented format.
#[cfg(test)]
pub(super) fn images_differ(left: &std::path::Path, right: &std::path::Path) -> Result<bool> {
    ImageComparisons::default().differ(left, right)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_format_comes_from_bytes_instead_of_the_export_filename() -> Result<()> {
        let directory = crate::regression::artifact_cache::unique_path(
            &std::env::temp_dir(),
            "chart-image-format",
        )?;
        std::fs::create_dir_all(&directory)?;
        let path = directory.join("chart.image");
        image::RgbImage::new(2, 2).save_with_format(&path, image::ImageFormat::Png)?;
        assert!(!images_differ(&path, &path)?);
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
    #[test]
    fn missing_partner_evidence_is_an_error() {
        let candidates = BTreeSet::from([Candidate {
            sheet: "Rows".into(),
            cell: "A10".into(),
        }]);
        let empty = BTreeMap::new();
        assert!(compare_evidence(
            &candidates,
            &empty,
            &empty,
            &empty,
            &empty,
            &BTreeSet::new(),
            &mut ImageComparisons::default(),
        )
        .is_err());
    }
    #[test]
    fn missing_performance_partner_is_not_identical() {
        let candidate = Candidate {
            sheet: "Rows".into(),
            cell: "A10".into(),
        };
        let candidates = BTreeSet::from([candidate.clone()]);
        let original = BTreeMap::from([(
            candidate,
            Evidence {
                path: Some("unused.png".into()),
                geometry: Value::Null,
            },
        )]);
        let empty = BTreeMap::new();
        let error = compare_performance(
            &candidates,
            &original,
            &empty,
            &empty,
            &original,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &mut ImageComparisons::default(),
        )
        .err()
        .unwrap();
        assert!(error
            .to_string()
            .contains("Missing performance_before evidence"));
    }

    #[test]
    fn ranges_have_one_cell_margin_and_clip_at_edges() {
        assert_eq!(padded_range("A1").unwrap(), "A1:B2");
        assert_eq!(padded_range("E64").unwrap(), "D63:F65");
        assert_eq!(padded_range("XFD1048576").unwrap(), "XFC1048575:XFD1048576");
    }

    #[test]
    fn fallback_rectangles_keep_their_extent_and_reject_invalid_coordinates() {
        assert_eq!(padded_range("A1:P128").unwrap(), "A1:P128");
        for invalid in ["B2:A1", "A0:B2", "A1:XFE2", "A1:B1048577", "A:B"] {
            assert!(padded_range(invalid).is_err(), "{invalid}");
        }
    }
}

#[cfg(test)]
mod margin_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn measured_empty_surfaces_do_not_turn_hidden_geometry_into_a_visual_difference() -> Result<()> {
        let evidence = |width, height| Evidence {
            path: None,
            geometry: json!({"top":8101.5,"left":9381.75,"width":width,"height":height}),
        };
        // Data.gov LCCA retained nine zero-width tail regions whose heights
        // changed from 2000.25 to 1827 points without an image on either side.
        for (a, b) in [
            (evidence(0.0, 2000.25), evidence(0.0, 1827.0)),
            (evidence(20.0, 0.0), evidence(10.0, 0.0)),
            (evidence(0.0, 20.0), evidence(10.0, 0.0)),
        ] {
            let verdict = a.compare_content(&b, &mut ImageComparisons::default())?;
            assert!(!verdict.geometry_differs && !verdict.pixels_differ);
            assert!(verdict.empty_geometry_ignored);
        }
        let empty = evidence(0.0, 10.0);
        let visible = Evidence {
            path: Some("visible.png".into()),
            geometry: json!({"top":8101.5,"left":9381.75,"width":10.0,"height":10.0}),
        };
        let transition = empty.compare_content(&visible, &mut ImageComparisons::default())?;
        assert!(transition.geometry_differs);
        assert!(!transition.empty_geometry_ignored);
        let missing = evidence(10.0, 10.0);
        assert!(empty.compare_content(&missing, &mut ImageComparisons::default()).is_err());
        Ok(())
    }

    /// arbeitskraefte-2030218209005.xlsx, 0620T!ACC1070:AGX1166.
    /// Both retained images are white despite different far-tail column origins.
    #[test]
    #[ignore = "Requires EVIDENCE_DIAG_INPUT with a retained finding and its image paths"]
    fn retained_white_pair_ignores_far_tail_geometry() -> Result<()> {
        let input = std::fs::read(std::env::var("EVIDENCE_DIAG_INPUT")?)?;
        let finding: Value = serde_json::from_slice(&input)?;
        let evidence = |side: &str| -> Result<Evidence> {
            Ok(Evidence {
                path: Some(PathBuf::from(
                    finding[format!("{side}_png")].as_str().context("Missing image path")?,
                )),
                geometry: finding[format!("{side}_geometry")].clone(),
            })
        };
        let result = evidence("left")?
            .compare_content(&evidence("right")?, &mut ImageComparisons::default())?;
        assert_eq!(result, ContentVerdict {
            pixels_differ: false,
            geometry_differs: false,
            blank_margin_ignored: true,
            capture_extent_ignored: false,
            empty_geometry_ignored: false,
        });
        Ok(())
    }

    /// Destatis Arbeitskraefte and Baumschulerhebung retained thousands of
    /// fully white pairs whose far-tail column coordinates had shifted.
    #[test]
    fn completely_blank_pairs_ignore_shifted_origins_but_visible_pairs_do_not() -> Result<()> {
        let directory = crate::regression::artifact_cache::unique_path(
            &std::env::temp_dir(), "blank-origin",
        )?;
        std::fs::create_dir_all(&directory)?;
        for (index, color) in [image::Rgba([255; 4]), image::Rgba([240, 240, 240, 255])]
            .into_iter().enumerate()
        {
            let left = directory.join(format!("{index}-left.png"));
            let right = directory.join(format!("{index}-right.png"));
            image::RgbaImage::from_pixel(10, 10, color).save(&left)?;
            image::RgbaImage::from_pixel(20, 10, color).save(&right)?;
            let a = Evidence { path: Some(left), geometry: json!({
                "top":100,"left":200,"width":10,"height":10,
            }) };
            let b = Evidence { path: Some(right), geometry: json!({
                "top":300,"left":400,"width":20,"height":10,
            }) };
            let result = a.compare_content(&b, &mut ImageComparisons::default())?;
            assert_eq!(result.blank_margin_ignored, index == 0);
            assert_eq!(result.geometry_differs, index != 0);
            assert_eq!(result.pixels_differ, index != 0);
        }
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// The extra blank row/column in Enron Deals A1:I7 and Top 100 Exposures
    /// may change size. Body geometry and overflow ink must remain observable.
    #[test]
    fn capture_extent_changes_require_matching_origins_and_preserve_overlap_differences() -> Result<()> {
        let directory =
            crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "blank-margin")?;
        std::fs::create_dir_all(&directory)?;
        let core = json!({"top":0,"left":0,"width":8,"height":8});
        let mut comparisons = ImageComparisons::default();
        let mut evidence = Vec::new();
        for (index, (width, height)) in [(10, 10), (14, 10), (10, 14), (14, 14)]
            .into_iter()
            .enumerate()
        {
            let path = directory.join(format!("{index}.png"));
            let mut image = image::RgbaImage::from_pixel(width, height, image::Rgba([255; 4]));
            image.put_pixel(2, 3, image::Rgba([0, 0, 0, 255]));
            image.save(&path)?;
            evidence.push(Evidence {
                path: Some(path),
                geometry: json!({"top":0,"left":0,
                "width":width,"height":height,"comparison_core":core}),
            });
        }
        for other in &evidence[1..] {
            assert_eq!(
                evidence[0].compare_content(other, &mut comparisons)?,
                ContentVerdict {
                    pixels_differ: false,
                    geometry_differs: false,
                    blank_margin_ignored: true,
                    capture_extent_ignored: false,
                    empty_geometry_ignored: false,
                }
            );
        }
        let mut body = evidence[1].clone();
        body.geometry["comparison_core"]["height"] = json!(7);
        assert!(evidence[0].compare_content(&body, &mut comparisons)?.blank_margin_ignored);
        let mut unknown = evidence[1].clone();
        unknown
            .geometry
            .as_object_mut()
            .unwrap()
            .remove("comparison_core");
        assert!(
            evidence[0]
                .compare_content(&unknown, &mut comparisons)?
                .geometry_differs
        );
        let mut shifted = evidence[1].clone();
        shifted.geometry["left"] = json!(1);
        assert!(
            evidence[0]
                .compare_content(&shifted, &mut comparisons)?
                .geometry_differs
        );
        // A fresh immutable image contains overflow ink in the extra column.
        let path = directory.join("overflow.png");
        let mut image = image::open(evidence[1].path.as_ref().unwrap())?.into_rgba8();
        image.put_pixel(12, 3, image::Rgba([0, 0, 0, 255]));
        image.save(&path)?;
        let mut overflow = evidence[1].clone();
        overflow.path = Some(path);
        assert_eq!(
            evidence[0].compare_content(&overflow, &mut comparisons)?,
            ContentVerdict {
                pixels_differ: false,
                geometry_differs: false,
                blank_margin_ignored: false,
                capture_extent_ignored: true,
                empty_geometry_ignored: false,
            }
        );
        // Content outside the shared rectangle is outside this comparison's scope.
        let path = directory.join("faint-border.png");
        image.put_pixel(12, 3, image::Rgba([254, 254, 254, 255]));
        image.save(&path)?;
        overflow.path = Some(path);
        assert!(!evidence[0].compare_content(&overflow, &mut comparisons)?.pixels_differ);
        image.put_pixel(2, 3, image::Rgba([1, 0, 0, 255]));
        let changed = directory.join("changed-overlap.png");
        image.save(&changed)?;
        overflow.path = Some(changed);
        assert!(evidence[0].compare_content(&overflow, &mut comparisons)?.pixels_differ);
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
}
