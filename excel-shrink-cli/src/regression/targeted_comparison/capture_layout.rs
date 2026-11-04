//! Preserve the selected cell coverage; partition images by measured visible geometry.
use super::{content_bounds::Rectangle, telemetry::LaneId, Candidate};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

// Conservatively allow two export pixels per point. Native exports are checked
// again against these limits; an unsupported scale fails instead of losing evidence.
pub(super) const PIXELS_PER_POINT: f64 = 2.0;
pub(super) const MAX_EDGE: u32 = 30_000;
pub(super) const MAX_PIXELS: u64 = 32_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub(super) struct Geometry {
    pub top: f64,
    pub left: f64,
    pub width: f64,
    pub height: f64,
}

impl Geometry {
    pub fn validate(self) -> Result<Self> {
        ensure!(
            [self.top, self.left, self.width, self.height]
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0),
            "Invalid measured capture geometry"
        );
        Ok(self)
    }

    pub fn empty(self) -> bool {
        self.width == 0.0 || self.height == 0.0
    }

    pub fn fits(self) -> bool {
        if self.empty() {
            return true;
        }
        let width = (self.width * PIXELS_PER_POINT).ceil() + 2.0;
        let height = (self.height * PIXELS_PER_POINT).ceil() + 2.0;
        width <= f64::from(MAX_EDGE)
            && height <= f64::from(MAX_EDGE)
            && width * height <= MAX_PIXELS as f64
    }
}

/// Merge adjacent grid cells without filling holes or changing selected coverage.
/// The grid remains an internal candidate index, never an image-size decision.
pub(super) fn coalesce(selected: &BTreeSet<Candidate>) -> Result<BTreeSet<Candidate>> {
    let mut bands: BTreeMap<(String, u32, u32), Vec<Rectangle>> = BTreeMap::new();
    for candidate in selected {
        let rect = Rectangle::parse(&candidate.cell)?;
        bands
            .entry((candidate.sheet.clone(), rect.top, rect.bottom))
            .or_default()
            .push(rect);
    }
    let mut columns: BTreeMap<(String, u32, u32), Vec<Rectangle>> = BTreeMap::new();
    for ((sheet, _, _), mut band) in bands {
        band.sort_by_key(|rect| rect.left);
        let mut merged: Vec<Rectangle> = Vec::new();
        for rect in band {
            if let Some(last) = merged.last_mut().filter(|last| last.right + 1 == rect.left) {
                last.right = rect.right;
            } else {
                merged.push(rect);
            }
        }
        for rect in merged {
            columns
                .entry((sheet.clone(), rect.left, rect.right))
                .or_default()
                .push(rect);
        }
    }
    let mut result = BTreeSet::new();
    for ((sheet, _, _), mut column) in columns {
        column.sort_by_key(|rect| rect.top);
        let mut merged: Vec<Rectangle> = Vec::new();
        for rect in column {
            if let Some(last) = merged.last_mut().filter(|last| last.bottom + 1 == rect.top) {
                last.bottom = rect.bottom;
            } else {
                merged.push(rect);
            }
        }
        result.extend(merged.into_iter().map(|rect| Candidate {
            sheet: sheet.clone(),
            cell: rect.address(),
        }));
    }
    Ok(result)
}

/// Use one cell partition for every live state, including later rebuild copies.
/// Bisect cell coordinates and measure each child once. Every accepted leaf must fit
/// the image budget in every state; zero-area leaves remain explicit.
pub(super) fn plan(
    selected: &BTreeSet<Candidate>,
    mut measure: impl FnMut(&Candidate) -> Result<BTreeMap<LaneId, Geometry>>,
) -> Result<BTreeMap<Candidate, BTreeMap<LaneId, Geometry>>> {
    let mut pending: Vec<_> = coalesce(selected)?.into_iter().collect();
    let mut leaves = BTreeMap::new();
    while let Some(candidate) = pending.pop() {
        let geometries = measure(&candidate)?;
        ensure!(!geometries.is_empty(), "No live capture states to measure");
        for geometry in geometries.values() {
            geometry.validate()?;
        }
        let oversized = geometries
            .iter()
            .filter(|(_, g)| !g.fits())
            .max_by(|(_, a), (_, b)| (a.width * a.height).total_cmp(&(b.width * b.height)));
        let Some((_, &geometry)) = oversized else {
            leaves.insert(candidate, geometries);
            continue;
        };
        let rect = Rectangle::parse(&candidate.cell)?;
        let vertical = if rect.left == rect.right {
            true
        } else if rect.top == rect.bottom {
            false
        } else {
            geometry.height >= geometry.width
        };
        let (start, end) = if vertical {
            (rect.top, rect.bottom)
        } else {
            (rect.left, rect.right)
        };
        ensure!(
            start < end,
            "A single cell exceeds the safe image export budget: {}!{}",
            candidate.sheet,
            candidate.cell
        );
        // Native geometry round trips dominate large plans. Do not run a binary
        // boundary search for every output image: a balanced split needs only
        // the two child observations and preserves the exact selected cell union.
        let low = start + (end - start) / 2;
        let mut first = rect;
        let mut second = rect;
        if vertical {
            first.bottom = low;
            second.top = low + 1;
        } else {
            first.right = low;
            second.left = low + 1;
        }
        pending.push(Candidate {
            sheet: candidate.sheet.clone(),
            cell: second.address(),
        });
        pending.push(Candidate {
            sheet: candidate.sheet,
            cell: first.address(),
        });
    }
    Ok(leaves)
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use super::*;
    fn candidate(cell: &str) -> Candidate {
        Candidate {
            sheet: "Data".into(),
            cell: cell.into(),
        }
    }

    #[test]
    fn merging_preserves_holes_and_separates_sheets() -> Result<()> {
        let selected = ["A1:P128", "Q1:AF128", "A129:P256"].map(candidate).into();
        let merged = coalesce(&selected)?;
        assert_eq!(
            merged,
            [candidate("A1:AF128"), candidate("A129:P256")].into()
        );
        Ok(())
    }

    #[test]
    fn tall_visible_plan_measures_each_tree_node_once() -> Result<()> {
        let selected = [candidate("A1:P30000")].into();
        let mut calls = 0;
        let leaves = plan(&selected, |candidate| {
            calls += 1;
            let rect = Rectangle::parse(&candidate.cell)?;
            Ok([(
                LaneId::O,
                Geometry {
                    top: 0.,
                    left: 0.,
                    width: 1000.,
                    height: f64::from(rect.bottom - rect.top + 1) * 15.,
                },
            )]
            .into())
        })?;
        assert_eq!(calls, 2 * leaves.len() - 1);
        assert!(leaves
            .values()
            .all(|states| states.values().all(|geometry| geometry.fits())));
        let rows: u32 = leaves
            .keys()
            .map(|candidate| {
                let rect = Rectangle::parse(&candidate.cell).unwrap();
                rect.bottom - rect.top + 1
            })
            .sum();
        assert_eq!(rows, 30000);
        Ok(())
    }
    #[test]
    fn hidden_million_row_extent_needs_one_geometry_observation() -> Result<()> {
        let selected = [candidate("A1:P1048576")].into();
        let mut calls = 0;
        let result = plan(&selected, |_| {
            calls += 1;
            Ok([(
                LaneId::O,
                Geometry {
                    top: 15.0,
                    left: 0.0,
                    width: 1000.0,
                    height: 0.0,
                },
            )]
            .into())
        })?;
        assert_eq!(calls, 1);
        assert_eq!(result.len(), 1);
        Ok(())
    }

    #[test]
    fn invalid_or_indivisible_oversized_geometry_fails_instead_of_skipping() {
        let selected = [candidate("A1:A1")].into();
        assert!(plan(&selected, |_| Ok([(
            LaneId::O,
            Geometry {
                top: 0.0,
                left: 0.0,
                width: 100000.0,
                height: 100000.0,
            }
        )]
        .into()))
        .is_err());
        assert!(plan(&selected, |_| Ok([(
            LaneId::O,
            Geometry {
                top: f64::NAN,
                left: 0.0,
                width: 10.0,
                height: 0.0,
            }
        )]
        .into()))
        .is_err());
    }

    #[test]
    fn every_state_controls_splitting_and_no_rows_are_lost() -> Result<()> {
        let selected = [candidate("A1:P1000000")].into();
        let result = plan(&selected, |candidate| {
            let r = Rectangle::parse(&candidate.cell)?;
            // Only the last 1,000 rows are visible. Rebuilt Default is taller.
            let visible = r.bottom.saturating_sub(r.top.max(999001) - 1) as f64;
            Ok([
                (
                    LaneId::O,
                    Geometry {
                        top: 0.0,
                        left: 0.0,
                        width: 100.0,
                        height: 0.0,
                    },
                ),
                (
                    LaneId::SDR,
                    Geometry {
                        top: 0.0,
                        left: 0.0,
                        width: 100.0,
                        height: visible * 30.0,
                    },
                ),
            ]
            .into())
        })?;
        let mut rectangles = result
            .keys()
            .map(|c| Rectangle::parse(&c.cell))
            .collect::<Result<Vec<_>>>()?;
        rectangles.sort_by_key(|r| r.top);
                // A sparse visible suffix can add logarithmically many zero-area leaves;
        // these retain coverage without generating image exports.
        assert!(rectangles.len() > 1 && rectangles.len() < 32);
        assert!(result.values().filter(|states| !states[&LaneId::SDR].empty()).count() <= 4);
        assert_eq!(rectangles[0].top, 1);
        assert_eq!(rectangles.last().unwrap().bottom, 1_000_000);
        for pair in rectangles.windows(2) {
            assert_eq!(pair[0].bottom + 1, pair[1].top);
        }
        assert!(result
            .values()
            .all(|states| states.values().all(|g| g.fits())));
        Ok(())
    }

    #[test]
    #[ignore = "read-only Excel experiment; requires CAPTURE_GEOMETRY_WORKBOOK and CAPTURE_GEOMETRY_OUTPUT"]
    fn native_frpp_visible_geometry_plan() -> Result<()> {
        use std::{
            io::{BufRead, BufReader, Write},
            process::{Command, Stdio},
            time::Instant,
        };
        ensure!(
            std::env::var_os("CAPTURE_GEOMETRY_WORKBOOK").is_some(),
            "Inspection workbook is required"
        );
        let output = std::path::PathBuf::from(std::env::var("CAPTURE_GEOMETRY_OUTPUT")?);
        let mut child = Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(crate::regression::repo_root().join(
                "docs/visual-validation/experiments/adaptive-capture/measure-existing-workbook.ps1",
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut input = child.stdin.take().context("Missing probe stdin")?;
        let mut responses = BufReader::new(child.stdout.take().context("Missing probe stdout")?);
        let mut calls = 0;
        let started = Instant::now();
        let result = plan(&[candidate("A1:DN307921")].into(), |candidate| {
            serde_json::to_writer(&mut input, &serde_json::json!({"range":candidate.cell}))?;
            input.write_all(b"\n")?;
            input.flush()?;
            let mut line = String::new();
            ensure!(
                responses.read_line(&mut line)? > 0,
                "Geometry probe exited without evidence"
            );
            calls += 1;
            Ok([(
                LaneId::O,
                serde_json::from_str::<Geometry>(&line)?.validate()?,
            )]
            .into())
        });
        let elapsed = started.elapsed().as_secs_f64();
        drop(input);
        let status = child.wait()?;
        let result = result?;
        ensure!(status.success(), "Geometry probe failed");
        let report = serde_json::json!({"geometry_calls":calls,"elapsed_seconds":elapsed,
            "regions":result.iter().map(|(candidate,geometry)| serde_json::json!({"range":candidate.cell,"geometry":geometry})).collect::<Vec<_>>()});
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
        println!(
            "Native FRPP plan: {} regions, {calls} geometry calls, {elapsed:.3} seconds",
            result.len()
        );
        assert!(result.len() < 100);
        Ok(())
    }
}
