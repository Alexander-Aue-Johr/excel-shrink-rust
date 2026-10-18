use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct HeightRun {
    pub start_row: u32,
    pub end_row: u32,
    pub height: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct SheetHeights {
    pub sheet_index: usize,
    pub sheet_name: String,
    pub runs: Vec<HeightRun>,
}

#[derive(Debug, Clone)]
pub(super) struct SheetExtent {
    pub index: usize,
    pub name: String,
    pub last_row: u32,
    pub last_checked_row: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub(super) struct RowHeightDifference {
    pub sheet_index: usize,
    pub sheet_name: String,
    pub row: u32,
    pub end_row: u32,
    pub before: f64,
    pub after: f64,
}

fn validated_sheets<'a>(
    sheets: &'a [SheetHeights],
    expected: &[SheetExtent],
) -> Result<BTreeMap<usize, &'a SheetHeights>> {
    ensure!(
        sheets.len() == expected.len(),
        "row-height sheet count differs"
    );
    let mut by_index = BTreeMap::new();
    for sheet in sheets {
        ensure!(
            by_index.insert(sheet.sheet_index, sheet).is_none(),
            "duplicate row-height sheet index {}",
            sheet.sheet_index
        );
    }
    for item in expected {
        ensure!(
            (1..=item.last_row).contains(&item.last_checked_row),
            "invalid row-height check boundary for sheet {}",
            item.index
        );
        let sheet = by_index
            .get(&item.index)
            .with_context(|| format!("missing row heights for sheet {}", item.index))?;
        ensure!(
            sheet.sheet_name == item.name,
            "row-height sheet name differs at index {}",
            item.index
        );
        let mut next = 1u32;
        for run in &sheet.runs {
            ensure!(
                run.start_row == next
                    && run.end_row >= next
                    && run.height.is_finite()
                    && run.height >= 0.0,
                "invalid row-height coverage for sheet {} at row {}",
                item.index,
                next
            );
            next = run
                .end_row
                .checked_add(1)
                .context("row-height row overflow")?;
        }
        ensure!(
            next == item.last_row.checked_add(1).context("sheet row overflow")?,
            "incomplete row-height coverage for sheet {}",
            item.index
        );
    }
    Ok(by_index)
}

pub(super) fn compare_row_heights(
    original: &[SheetHeights],
    shrunk: &[SheetHeights],
    expected: &[SheetExtent],
) -> Result<Vec<RowHeightDifference>> {
    let left = validated_sheets(original, expected)?;
    let right = validated_sheets(shrunk, expected)?;
    let mut differences: Vec<RowHeightDifference> = Vec::new();

    for item in expected {
        let before = &left[&item.index].runs;
        let after = &right[&item.index].runs;
        let (mut a, mut b) = (0usize, 0usize);
        while a < before.len() && b < after.len() {
            let first = before[a].start_row.max(after[b].start_row);
            let last = before[a]
                .end_row
                .min(after[b].end_row)
                .min(item.last_checked_row);
            if before[a].height != after[b].height {
                match differences.last_mut() {
                    Some(previous)
                        if previous.sheet_index == item.index
                            && previous.end_row.checked_add(1) == Some(first)
                            && previous.before == before[a].height
                            && previous.after == after[b].height =>
                    {
                        previous.end_row = last;
                    }
                    _ => differences.push(RowHeightDifference {
                        sheet_index: item.index,
                        sheet_name: item.name.clone(),
                        row: first,
                        end_row: last,
                        before: before[a].height,
                        after: after[b].height,
                    }),
                }
            }
            if last == item.last_checked_row {
                break;
            }
            if before[a].end_row == last {
                a += 1;
            }
            if after[b].end_row == last {
                b += 1;
            }
        }
    }
    Ok(differences)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(runs: &[(u32, u32, f64)]) -> SheetHeights {
        SheetHeights {
            sheet_index: 1,
            sheet_name: "Sheet 1".into(),
            runs: runs
                .iter()
                .map(|&(start_row, end_row, height)| HeightRun {
                    start_row,
                    end_row,
                    height,
                })
                .collect(),
        }
    }

    fn extent() -> Vec<SheetExtent> {
        vec![SheetExtent {
            index: 1,
            name: "Sheet 1".into(),
            last_row: 1_048_576,
            last_checked_row: 1_048_576,
        }]
    }

    #[test]
    fn compares_large_uniform_spans_without_expanding_rows() {
        let before = sheet(&[(1, 1_048_576, 15.0)]);
        let after = sheet(&[
            (1, 500_000, 15.0),
            (500_001, 500_100, 20.0),
            (500_101, 1_048_576, 15.0),
        ]);
        assert_eq!(
            compare_row_heights(&[before], &[after], &extent()).unwrap(),
            vec![RowHeightDifference {
                sheet_index: 1,
                sheet_name: "Sheet 1".into(),
                row: 500_001,
                end_row: 500_100,
                before: 15.0,
                after: 20.0
            }]
        );
    }

    #[test]
    fn ignores_capture_margin_heights_but_checks_the_last_visual_row() {
        let expected = [SheetExtent {
            index: 1,
            name: "Sheet 1".into(),
            last_row: 14,
            last_checked_row: 13,
        }];
        let before = sheet(&[(1, 14, 15.75)]);
        let margin_changed = sheet(&[(1, 13, 15.75), (14, 14, 12.0)]);
        assert!(compare_row_heights(&[before.clone()], &[margin_changed], &expected)
            .unwrap()
            .is_empty());
        let content_changed = sheet(&[(1, 12, 15.75), (13, 14, 12.0)]);
        let differences = compare_row_heights(&[before], &[content_changed], &expected).unwrap();
        assert_eq!(differences.len(), 1);
        assert_eq!((differences[0].row, differences[0].end_row), (13, 13));
    }

    #[test]
    fn rejects_missing_rows_and_duplicate_sheets() {
        let missing = sheet(&[(1, 1_048_575, 15.0)]);
        let complete = sheet(&[(1, 1_048_576, 15.0)]);
        assert!(compare_row_heights(&[missing], &[complete.clone()], &extent()).is_err());
        assert!(compare_row_heights(
            &[complete.clone(), complete.clone()],
            &[complete],
            &extent()
        )
        .is_err());
    }
}
