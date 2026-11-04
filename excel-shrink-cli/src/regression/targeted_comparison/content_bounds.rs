//! Restrict cell captures to real content retained across all comparison states.
use super::{
    conditional_formatting::cell_address, evidence::padded_range, xml_changes::Snapshot, Candidate,
};
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// One-based inclusive coordinates. Formatting-only cells do not extend this envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Rectangle {
    pub(super) top: u32,
    pub(super) left: u32,
    pub(super) bottom: u32,
    pub(super) right: u32,
}

impl Rectangle {
    pub(super) fn parse(address: &str) -> Result<Self> {
        let (start, end) = address.split_once(':').unwrap_or((address, address));
        let (top, left) = cell_address(start).context("Invalid capture start")?;
        let (bottom, right) = cell_address(end).context("Invalid capture end")?;
        anyhow::ensure!(top <= bottom && left <= right, "Reversed capture bounds");
        Ok(Self {
            top,
            left,
            bottom,
            right,
        })
    }

    fn padded(self) -> Self {
        Self {
            top: self.top.saturating_sub(1).max(1),
            left: self.left.saturating_sub(1).max(1),
            bottom: (self.bottom + 1).min(1_048_576),
            right: (self.right + 1).min(16_384),
        }
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let result = Self {
            top: self.top.max(other.top),
            left: self.left.max(other.left),
            bottom: self.bottom.min(other.bottom),
            right: self.right.min(other.right),
        };
        (result.top <= result.bottom && result.left <= result.right).then_some(result)
    }

    pub(super) fn address(self) -> String {
        fn column(mut value: u32) -> String {
            let mut letters = String::new();
            while value > 0 {
                value -= 1;
                letters.insert(0, (b'A' + (value % 26) as u8) as char);
                value /= 26;
            }
            letters
        }
        format!(
            "{}{}:{}{}",
            column(self.left),
            self.top,
            column(self.right),
            self.bottom
        )
    }

    /// Use the existing CF capture grid; many changed cells in one tile need one image.
    fn capture_tiles(self) -> impl Iterator<Item = Self> {
        use super::{CAPTURE_COLUMNS, CAPTURE_ROWS};
        let first_row = ((self.top - 1) / CAPTURE_ROWS) * CAPTURE_ROWS + 1;
        let first_column = ((self.left - 1) / CAPTURE_COLUMNS) * CAPTURE_COLUMNS + 1;
        (first_row..=self.bottom)
            .step_by(CAPTURE_ROWS as usize)
            .flat_map(move |top| {
                (first_column..=self.right)
                    .step_by(CAPTURE_COLUMNS as usize)
                    .map(move |left| Self {
                        top,
                        left,
                        bottom: (top + CAPTURE_ROWS - 1).min(1_048_576),
                        right: (left + CAPTURE_COLUMNS - 1).min(16_384),
                    })
            })
    }
}

pub(super) fn include(bounds: &mut Option<Rectangle>, row: u32, column: u32) {
    *bounds = Some(match *bounds {
        Some(previous) => Rectangle {
            top: previous.top.min(row),
            left: previous.left.min(column),
            bottom: previous.bottom.max(row),
            right: previous.right.max(column),
        },
        None => Rectangle {
            top: row,
            left: column,
            bottom: row,
            right: column,
        },
    });
}

/// Counts describe selection, never a successful pixel comparison of skipped regions.
#[derive(Serialize)]
pub(super) struct Coverage {
    sheet: String,
    reason: &'static str,
    content_envelope_with_one_cell_margin: Option<Rectangle>,
    requested_candidates: usize,
    skipped_outside_content: usize,
    clipped_to_content: usize,
    selected_capture_regions: usize,
    selected_grid_tiles: usize,
}

#[derive(Default)]
pub(super) struct ContentBounds(BTreeMap<String, Rectangle>);

impl ContentBounds {
    /// Account for both the candidate's neighborhood and the content margin before allocating a change.
    pub fn intersects_cell(&self, sheet: &str, row: u32, column: u32) -> bool {
        self.0.get(sheet).is_some_and(|bounds| {
            Rectangle {
                top: row,
                bottom: row,
                left: column,
                right: column,
            }
            .padded()
            .intersection(bounds.padded())
            .is_some()
        })
    }
    /// Include raw originals as well as saved/rebuilt variants: deleted real data stays covered.
    pub fn include(&mut self, snapshot: &Snapshot) {
        for (name, sheet) in &snapshot.sheets {
            if let Some(bounds) = sheet.cells.content_bounds() {
                self.0
                    .entry(name.clone())
                    .and_modify(|previous| {
                        previous.top = previous.top.min(bounds.top);
                        previous.left = previous.left.min(bounds.left);
                        previous.bottom = previous.bottom.max(bounds.bottom);
                        previous.right = previous.right.max(bounds.right);
                    })
                    .or_insert(bounds);
            }
        }
    }

    /// The unpadded content intersection anchors comparison of extra blank edge cells.
    pub fn core_range(&self, candidate: &Candidate) -> Result<Option<String>> {
        let rectangle = Rectangle::parse(&candidate.cell)?;
        Ok(self
            .0
            .get(&candidate.sheet)
            .and_then(|bounds| bounds.intersection(rectangle))
            .map(Rectangle::address))
    }

    /// Heights above visible content affect its position; rows below its padded envelope do not.
    pub fn last_relevant_row(&self, sheet: &str) -> u32 {
        self.0.get(sheet).map_or(0, |bounds| bounds.padded().bottom)
    }

    /// Recompute after late discoveries; retained workbooks backfill newly eligible evidence.
    pub fn select(
        &self,
        requested: &BTreeSet<Candidate>,
    ) -> Result<(BTreeSet<Candidate>, Vec<Coverage>)> {
        let mut selected = BTreeSet::new();
        let mut coverage = BTreeMap::new();
        for candidate in requested {
            let bounds = self.0.get(&candidate.sheet).copied().map(Rectangle::padded);
            let entry = coverage
                .entry(candidate.sheet.clone())
                .or_insert_with(|| Coverage {
                    sheet: candidate.sheet.clone(),
                    reason: "outside-content-envelope",
                    content_envelope_with_one_cell_margin: bounds,
                    requested_candidates: 0,
                    skipped_outside_content: 0,
                    clipped_to_content: 0,
                    selected_capture_regions: 0,
                    selected_grid_tiles: 0,
                });
            entry.requested_candidates += 1;
            let region = Rectangle::parse(&padded_range(&candidate.cell)?)?;
            let Some(clipped) = bounds.and_then(|bounds| region.intersection(bounds)) else {
                entry.skipped_outside_content += 1;
                continue;
            };
            entry.clipped_to_content += usize::from(region != clipped);
            for tile in clipped.capture_tiles() {
                let tile = tile
                    .intersection(bounds.expect("clipped region has content bounds"))
                    .expect("tile overlaps content");
                if selected.insert(Candidate {
                    sheet: candidate.sheet.clone(),
                    cell: tile.address(),
                }) {
                    entry.selected_capture_regions += 1;
                    entry.selected_grid_tiles += 1;
                }
            }
        }
        Ok((selected, coverage.into_values().collect()))
    }
}

/// Keep internal grid accounting separate from the final image partition.
pub(super) fn record_partition(coverage: &mut [Coverage], candidates: &BTreeSet<Candidate>) {
    for sheet in coverage {
        sheet.selected_capture_regions =
            candidates.iter().filter(|c| c.sheet == sheet.sheet).count();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_height_candidates_reach_content_starting_far_from_column_a() -> Result<()> {
        let bounds = ContentBounds(BTreeMap::from([(
            "Data".into(),
            Rectangle::parse("T10:W20")?,
        )]));
        let requested = BTreeSet::from([Candidate {
            sheet: "Data".into(),
            cell: "A12:XFD12".into(),
        }]);
        let (selected, coverage) = bounds.select(&requested)?;
        assert_eq!(
            selected,
            BTreeSet::from([Candidate {
                sheet: "Data".into(),
                cell: "S9:X21".into()
            }])
        );
        assert_eq!(coverage[0].skipped_outside_content, 0);
        Ok(())
    }

    #[test]
    fn dense_changes_share_tiles_without_dropping_grid_boundary_neighbors() -> Result<()> {
        let bounds = ContentBounds(BTreeMap::from([(
            "Data".into(),
            Rectangle::parse("A1:Z1000")?,
        )]));
        let requested = BTreeSet::from([Candidate {
            sheet: "Data".into(),
            cell: "P128".into(),
        }]);
        let (selected, _) = bounds.select(&requested)?;
        assert_eq!(
            selected
                .iter()
                .map(|candidate| candidate.cell.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["A1:P128", "A129:P256", "Q1:AA128", "Q129:AA256"])
        );
        let requested = (1..=1000)
            .flat_map(|row| {
                (b'A'..=b'J').map(move |column| Candidate {
                    sheet: "Data".into(),
                    cell: format!("{}{row}", column as char),
                })
            })
            .collect();
        let (selected, _) = bounds.select(&requested)?;
        assert_eq!(selected.len(), 8);
        assert!(selected
            .iter()
            .all(|candidate| Rectangle::parse(&candidate.cell).unwrap().bottom <= 1001));
        Ok(())
    }

    #[test]
    fn deleted_blank_tail_does_not_expand_captures() -> Result<()> {
        let bounds = ContentBounds(BTreeMap::from([(
            "Data".into(),
            Rectangle::parse("C1:F10")?,
        )]));
        let requested = (1..=100_010)
            .map(|row| Candidate {
                sheet: "Data".into(),
                cell: format!("D{row}"),
            })
            .collect();
        let (selected, coverage) = bounds.select(&requested)?;
        assert_eq!(selected.len(), 1);
        assert_eq!(coverage[0].skipped_outside_content, 99_998);
        assert!(selected
            .iter()
            .all(|candidate| Rectangle::parse(&candidate.cell).unwrap().bottom <= 11));
        Ok(())
    }

    #[test]
    fn rectangles_are_intersected_and_empty_sheets_are_explicitly_skipped() -> Result<()> {
        let bounds = ContentBounds(BTreeMap::from([(
            "Data".into(),
            Rectangle::parse("C5:F10")?,
        )]));
        let requested = BTreeSet::from([
            Candidate {
                sheet: "Data".into(),
                cell: "A1:Z100000".into(),
            },
            Candidate {
                sheet: "Empty".into(),
                cell: "A1".into(),
            },
        ]);
        let (selected, coverage) = bounds.select(&requested)?;
        assert_eq!(
            selected,
            BTreeSet::from([Candidate {
                sheet: "Data".into(),
                cell: "B4:G11".into()
            }])
        );
        assert_eq!(coverage[0].clipped_to_content, 1);
        assert_eq!(coverage[1].skipped_outside_content, 1);
        Ok(())
    }
}
