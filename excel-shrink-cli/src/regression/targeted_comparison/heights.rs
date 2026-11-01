//! Select height checks from actual changes; unchanged fixed rows need no automatic-height scan.
use super::{xml_changes::Snapshot, Candidate};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct HeightRun {
    pub start_row: u32,
    pub end_row: u32,
    pub height: f64,
}

#[derive(Serialize)]
pub(super) struct HeightDifference {
    start_row: u32,
    end_row: u32,
    before: f64,
    after: f64,
}

#[derive(Serialize)]
pub(super) struct HeightCheck {
    pub left: String,
    pub right: String,
    pub sheet: String,
    pub checked_spans: Vec<(u32, u32)>,
    pub unchanged_fixed_rows: usize,
    pub differences: Vec<HeightDifference>,
    pub complete: bool,
}

/// Return relevant automatic-height spans, excluding only retained identical fixed definitions.
pub(super) fn selection(
    before: &Snapshot,
    after: &Snapshot,
    sheet: &str,
) -> Result<(Vec<(u32, u32)>, usize)> {
    let left = before
        .sheets
        .get(sheet)
        .ok_or_else(|| anyhow::anyhow!("Missing sheet {sheet}"))?;
    let right = after
        .sheets
        .get(sheet)
        .ok_or_else(|| anyhow::anyhow!("Missing partner sheet {sheet}"))?;
    let global = left.columns != right.columns
        || left.defaults != right.defaults
        || before.context != after.context;
    let mut wanted = BTreeSet::new();
    let mut skipped = 0;
    let fixed: BTreeSet<u32> = left
        .rows
        .keys()
        .chain(right.rows.keys())
        .copied()
        .filter(|&row| {
            left.fixed_height(row).is_some() && left.fixed_height(row) == right.fixed_height(row)
        })
        .collect();
    if global {
        // Column fonts can affect even empty rows below the populated region.
        let mut spans = Vec::new();
        let mut start = 1;
        for row in fixed {
            if start < row {
                spans.push((start, row - 1));
            }
            start = row + 1;
            skipped += 1;
        }
        if start <= 1_048_576 {
            spans.push((start, 1_048_576));
        }
        return Ok((spans, skipped));
    }
    for cell in left.cells.keys().chain(right.cells.keys()) {
        if left.cells.get(cell) != right.cells.get(cell) {
            let row = cell
                .trim_start_matches(|c: char| c.is_ascii_alphabetic())
                .parse()?;
            wanted.insert(row);
        }
    }
    for row in left.rows.keys().chain(right.rows.keys()) {
        if left.rows.get(row) != right.rows.get(row) {
            wanted.insert(*row);
        }
    }
    let mut spans: Vec<(u32, u32)> = Vec::new();
    for row in wanted {
        if fixed.contains(&row) {
            skipped += 1;
            continue;
        }
        if let Some(last) = spans.last_mut() {
            if last.1 + 1 == row {
                last.1 = row;
                continue;
            }
        }
        spans.push((row, row));
    }
    Ok((spans, skipped))
}

/// Validate exact span coverage before comparing runs; absent values never mean equality.
pub(super) fn validate(runs: &[HeightRun], spans: &[(u32, u32)]) -> Result<()> {
    let mut index = 0;
    for &(start, end) in spans {
        let mut next = start;
        while index < runs.len() && runs[index].start_row <= end {
            let run = &runs[index];
            ensure!(
                run.start_row == next
                    && run.end_row >= next
                    && run.end_row <= end
                    && run.height.is_finite()
                    && run.height >= 0.,
                "Invalid or incomplete measured height coverage"
            );
            next = run.end_row + 1;
            index += 1;
        }
        ensure!(next == end + 1, "Missing measured height rows");
    }
    ensure!(index == runs.len(), "Unexpected measured height rows");
    Ok(())
}

impl HeightCheck {
    /// Report every changed height run; select boundary images explicitly rather than a million images.
    pub(super) fn compare(
        &mut self,
        before: &[HeightRun],
        after: &[HeightRun],
    ) -> Result<BTreeSet<Candidate>> {
        validate(before, &self.checked_spans)?;
        validate(after, &self.checked_spans)?;
        let (mut i, mut j) = (0, 0);
        let mut candidates = BTreeSet::new();
        while i < before.len() && j < after.len() {
            let a = &before[i];
            let b = &after[j];
            let start = a.start_row.max(b.start_row);
            let end = a.end_row.min(b.end_row);
            if start <= end && a.height != b.height {
                self.differences.push(HeightDifference {
                    start_row: start,
                    end_row: end,
                    before: a.height,
                    after: b.height,
                });
                for row in [start, end, (end + 1).min(1_048_576)] {
                    candidates.insert(Candidate {
                        sheet: self.sheet.clone(),
                        cell: format!("A{row}"),
                    });
                }
            }
            if a.end_row <= b.end_row {
                i += 1;
            }
            if b.end_row <= a.end_row {
                j += 1;
            }
        }
        self.complete = true;
        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_height_partner_is_not_equal() {
        assert!(validate(&[], &[(1, 100)]).is_err());
    }
    #[test]
    fn fixed_height_requires_both_flag_and_retained_definition() {
        let mut sheet = super::super::xml_changes::Sheet::default();
        sheet.rows.insert(8, [("ht".into(), "40".into())].into());
        assert!(sheet.fixed_height(8).is_none());
        sheet
            .rows
            .get_mut(&8)
            .unwrap()
            .insert("customHeight".into(), "1".into());
        assert_eq!(sheet.fixed_height(8), Some(("40", false)));
    }

    fn fixture() -> Snapshot {
        let mut sheet = super::super::xml_changes::Sheet::default();
        sheet.rows.insert(
            8,
            [
                ("ht".into(), "40".into()),
                ("customHeight".into(), "1".into()),
            ]
            .into(),
        );
        Snapshot {
            sheets: [("Rows".into(), sheet)].into(),
            context: "same styles".into(),
        }
    }

    #[test]
    fn unchanged_fixed_rows_are_excluded_but_deleted_rows_are_not() {
        let mut before = fixture();
        before
            .sheets
            .get_mut("Rows")
            .unwrap()
            .cells
            .insert("B8".into(), b"empty font cell".to_vec());
        let mut after = fixture();
        assert_eq!(selection(&before, &after, "Rows").unwrap(), (vec![], 1));
        after.sheets.get_mut("Rows").unwrap().rows.clear();
        assert_eq!(
            selection(&before, &after, "Rows").unwrap(),
            (vec![(8, 8)], 0)
        );
    }

    #[test]
    fn column_change_checks_the_full_height_except_retained_fixed_rows() {
        let before = fixture();
        let mut after = fixture();
        after.sheets.get_mut("Rows").unwrap().columns = b"changed column style".to_vec();
        assert_eq!(
            selection(&before, &after, "Rows").unwrap(),
            (vec![(1, 7), (9, 1_048_576)], 1)
        );
    }

    #[test]
    fn cancelling_height_changes_are_not_hidden_by_equal_totals() {
        let mut check = HeightCheck {
            left: "O".into(),
            right: "SD".into(),
            sheet: "Rows".into(),
            checked_spans: vec![(1, 2)],
            unchanged_fixed_rows: 0,
            differences: Vec::new(),
            complete: false,
        };
        let a = [
            HeightRun {
                start_row: 1,
                end_row: 1,
                height: 15.,
            },
            HeightRun {
                start_row: 2,
                end_row: 2,
                height: 30.,
            },
        ];
        let b = [
            HeightRun {
                start_row: 1,
                end_row: 1,
                height: 30.,
            },
            HeightRun {
                start_row: 2,
                end_row: 2,
                height: 15.,
            },
        ];
        assert!(!check.compare(&a, &b).unwrap().is_empty());
        assert_eq!(check.differences.len(), 2);
    }
}
