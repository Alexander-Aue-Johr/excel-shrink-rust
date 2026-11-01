//! Bounded image capture and comparison of the same regions across workbook states.
use super::host::ExcelHost;
use super::telemetry::{LaneId, Telemetry};
use super::{Candidate, Finding};
use anyhow::{ensure, Context, Result};
use image::GenericImageView;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub(super) struct Evidence {
    pub(super) path: PathBuf,
    pub(super) geometry: Value,
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
    images: BTreeMap<&'static str, PathBuf>,
    geometry: BTreeMap<&'static str, Value>,
}

impl PerformanceFinding {
    pub(super) fn differs(&self) -> bool {
        self.before_recalc_differs || self.after_recalc_differs
    }
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
            let differs = |other: &Evidence| -> Result<bool> {
                Ok(images_differ(&original.path, &other.path)?
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

/// Measure the candidate cell and capture its padded visual neighborhood.
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
    let geometry = excel.measure_range(lane, &candidate.sheet, &candidate.cell, telemetry)?;
    excel.capture_range(lane, &candidate.sheet, &range, &path, telemetry)?;
    Ok(Evidence { path, geometry })
}

fn padded_range(address: &str) -> Result<String> {
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
            before_recalc_differs: images_differ(&o0.path, &s0.path)?,
            after_shrink_recalc_differs: images_differ(&o0.path, &s1.path)?,
            also_changed_by_original_recalc: images_differ(&o0.path, &o1.path)?
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
pub(super) fn images_differ(left: &Path, right: &Path) -> Result<bool> {
    let left = image::ImageReader::open(left)?
        .with_guessed_format()?
        .decode()?;
    let right = image::ImageReader::open(right)?
        .with_guessed_format()?
        .decode()?;
    Ok(left.dimensions() != right.dimensions() || left.to_rgba8() != right.to_rgba8())
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
            &BTreeSet::new()
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
                path: "unused.png".into(),
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
