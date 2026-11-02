//! Invert local A1 dependencies without evaluating conditional-formatting formulas.
use super::{ranges, Rect};

#[derive(Clone, Copy, Debug)]
pub(super) struct Reference {
    row: u32,
    column: u32,
    fixed_row: bool,
    fixed_column: bool,
}

/// Parse one complete A1 reference; reject names, ranges and external references.
pub(super) fn parse(text: &str) -> Option<Reference> {
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
    Some(Reference {
        row,
        column,
        fixed_row,
        fixed_column,
    })
}

fn inverse_axis(
    input: u32,
    reference: u32,
    fixed: bool,
    anchor: u32,
    low: u32,
    high: u32,
) -> Option<(u32, u32)> {
    if fixed {
        return (input == reference).then_some((low, high));
    }
    let target = i64::from(input) - i64::from(reference) + i64::from(anchor);
    (target >= i64::from(low) && target <= i64::from(high))
        .then_some((target as u32, target as u32))
}

impl Reference {
    /// Return dependent target rectangles, never enumerate an entire worksheet grid.
    /// For disjoint sqref areas include both shared and per-area anchors conservatively.
    pub(super) fn targets(self, input: (u32, u32), areas: &[Rect], extent: Rect) -> Vec<Rect> {
        let mut result = Vec::new();
        for area in areas {
            for anchor in [areas[0], *area] {
                let Some((top, bottom)) = inverse_axis(
                    input.0,
                    self.row,
                    self.fixed_row,
                    anchor.top,
                    area.top,
                    area.bottom,
                ) else {
                    continue;
                };
                let Some((left, right)) = inverse_axis(
                    input.1,
                    self.column,
                    self.fixed_column,
                    anchor.left,
                    area.left,
                    area.right,
                ) else {
                    continue;
                };
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
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        for invalid in ["A0", "XFE1", "A1048577", "Other!A1", "A1:B2", "NamedRange"] {
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
