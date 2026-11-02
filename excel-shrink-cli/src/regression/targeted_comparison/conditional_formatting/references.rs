//! Invert local A1 dependencies without evaluating conditional-formatting formulas.
use super::{ranges, Rect};

#[derive(Clone, Copy, Debug)]
pub(super) struct Reference {
    start: Address,
    end: Address,
}

#[derive(Clone, Copy, Debug)]
struct Address {
    row: u32,
    column: u32,
    fixed_row: bool,
    fixed_column: bool,
}

/// Parse a local cell or rectangular range; endpoint dollar axes remain independent.
pub(super) fn parse(text: &str) -> Option<Reference> {
    let (start, end) = text.split_once(':').unwrap_or((text, text));
    Some(Reference {
        start: address(start)?,
        end: address(end)?,
    })
}

fn address(text: &str) -> Option<Address> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let fixed_column = bytes.get(index) == Some(&b'$');
    index += usize::from(fixed_column);
    let start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
        index += 1;
    }
    if start == index {
        return None;
    }
    let fixed_row = bytes.get(index) == Some(&b'$');
    index += usize::from(fixed_row);
    let start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    if start == index || index != bytes.len() {
        return None;
    }
    let (row, column) = ranges::cell(&text.to_ascii_uppercase())?;
    if row == 0 || column == 0 {
        return None;
    }
    Some(Address {
        row,
        column,
        fixed_row,
        fixed_column,
    })
}

fn inverse_axis(
    input: u32,
    start: (u32, bool),
    end: (u32, bool),
    anchor: u32,
    low: u32,
    high: u32,
) -> Vec<(u32, u32)> {
    let mut intervals = Vec::new();
    // Mixed fixed/relative endpoints can cross as the target moves. Include both orders.
    for (first, last) in [(start, end), (end, start)] {
        let mut lower = i64::from(low);
        let mut upper = i64::from(high);
        if first.1 {
            if input < first.0 {
                continue;
            }
        } else {
            upper = upper.min(i64::from(input) - i64::from(first.0) + i64::from(anchor));
        }
        if last.1 {
            if input > last.0 {
                continue;
            }
        } else {
            lower = lower.max(i64::from(input) - i64::from(last.0) + i64::from(anchor));
        }
        if lower <= upper {
            let interval = (lower as u32, upper as u32);
            if !intervals.contains(&interval) {
                intervals.push(interval);
            }
        }
    }
    intervals
}

impl Reference {
    /// Return dependent target rectangles, never enumerate an entire worksheet grid.
    /// For disjoint sqref areas include both shared and per-area anchors conservatively.
    pub(super) fn targets(self, input: (u32, u32), areas: &[Rect], extent: Rect) -> Vec<Rect> {
        let mut result = Vec::new();
        for area in areas {
            for anchor in [areas[0], *area] {
                let rows = inverse_axis(
                    input.0,
                    (self.start.row, self.start.fixed_row),
                    (self.end.row, self.end.fixed_row),
                    anchor.top,
                    area.top,
                    area.bottom,
                );
                let columns = inverse_axis(
                    input.1,
                    (self.start.column, self.start.fixed_column),
                    (self.end.column, self.end.fixed_column),
                    anchor.left,
                    area.left,
                    area.right,
                );
                for &(top, bottom) in &rows {
                    for &(left, right) in &columns {
                        if let Some(target) = (Rect {
                            top,
                            bottom,
                            left,
                            right,
                        })
                        .intersect(extent)
                        {
                            result.push(target);
                        }
                    }
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_inversion_matches_forward_mapping_for_mixed_and_reversed_endpoints() {
        let area = Rect {
            top: 2,
            left: 2,
            bottom: 6,
            right: 6,
        };
        for text in [
            "A1:B2",
            "$A$1:$B$3",
            "$A1:B$4",
            "A$5:$B2",
            "B2:A1",
            "$A$1:A1",
        ] {
            let reference = parse(text).unwrap();
            for row in 1..=7 {
                for column in 1..=7 {
                    let targets = reference.targets((row, column), &[area], area);
                    for target_row in area.top..=area.bottom {
                        for target_column in area.left..=area.right {
                            let forward = |address: Address| {
                                let r = i64::from(address.row)
                                    + if address.fixed_row {
                                        0
                                    } else {
                                        i64::from(target_row) - i64::from(area.top)
                                    };
                                let c = i64::from(address.column)
                                    + if address.fixed_column {
                                        0
                                    } else {
                                        i64::from(target_column) - i64::from(area.left)
                                    };
                                (r, c)
                            };
                            let a = forward(reference.start);
                            let b = forward(reference.end);
                            let expected = (a.0.min(b.0)..=a.0.max(b.0)).contains(&i64::from(row))
                                && (a.1.min(b.1)..=a.1.max(b.1)).contains(&i64::from(column));
                            assert_eq!(
                                targets
                                    .iter()
                                    .any(|r| r.contains(target_row, target_column)),
                                expected,
                                "{text}; input {row},{column}; target {target_row},{target_column}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn inverse_mapping_respects_both_dollar_axes() {
        let area = Rect {
            top: 2,
            left: 2,
            bottom: 5,
            right: 4,
        };
        for (reference, input, expected) in [
            ("A1", (2, 2), Some("C3:C3")),
            ("$A1", (2, 1), Some("B3:D3")),
            ("A$1", (1, 2), Some("C2:C5")),
            ("$A$1", (1, 1), Some("B2:D5")),
            ("$A$1", (2, 1), None),
        ] {
            let targets = parse(reference).unwrap().targets(input, &[area], area);
            assert_eq!(
                targets.first().map(|r| r.address()),
                expected.map(str::to_owned)
            );
        }
        for invalid in [
            "A0",
            "XFE1",
            "A1048577",
            "Other!A1",
            "A1:B2:C3",
            "NamedRange",
        ] {
            assert!(parse(invalid).is_none());
        }
    }

    #[test]
    fn million_row_targets_remain_rectangles_and_clip_at_the_grid_boundary() {
        let area = Rect {
            top: 2,
            left: 1,
            bottom: 1_048_576,
            right: 1,
        };
        let last = parse("A1").unwrap().targets((1_048_575, 1), &[area], area);
        assert_eq!(last[0].address(), "A1048576:A1048576");
        assert!(parse("A1")
            .unwrap()
            .targets((1_048_576, 1), &[area], area)
            .is_empty());
        let all = parse("$A$1").unwrap().targets((1, 1), &[area], area);
        assert!(all.len() <= 2);
        assert_eq!(all[0], area);
    }
}
