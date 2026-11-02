//! Finite worksheet rectangles and capture-grid intersections; never expand cells in memory.
use serde::Serialize;

pub(in crate::regression::targeted_comparison) const CAPTURE_ROWS: u32 = 128;
pub(in crate::regression::targeted_comparison) const CAPTURE_COLUMNS: u32 = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(in crate::regression::targeted_comparison) struct Rect {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

pub(in crate::regression::targeted_comparison) fn cell(address: &str) -> Option<(u32, u32)> {
    let mut column = 0u32;
    let mut row = 0u32;
    let mut digits = false;
    for byte in address.bytes().filter(|&b| b != b'$') {
        if byte.is_ascii_digit() {
            digits = true;
            row = row.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
        } else if !digits && byte.is_ascii_uppercase() {
            column = column
                .checked_mul(26)?
                .checked_add(u32::from(byte - b'A') + 1)?;
        } else {
            return None;
        }
    }
    (row <= 1048576 && column <= 16384 && (row > 0 || column > 0)).then_some((row, column))
}

pub(in crate::regression::targeted_comparison) fn areas(sqref: &str) -> Option<Vec<Rect>> {
    if sqref.trim().is_empty() {
        return None;
    }
    sqref
        .split_ascii_whitespace()
        .map(|area| {
            let (a, b) = area.split_once(':').unwrap_or((area, area));
            let (mut top, mut left) = cell(a)?;
            let (mut bottom, mut right) = cell(b)?;
            if top == 0 && bottom == 0 {
                top = 1;
                bottom = 1048576;
            }
            if left == 0 && right == 0 {
                left = 1;
                right = 16384;
            }
            (top > 0 && left > 0 && top <= bottom && left <= right).then_some(Rect {
                top,
                left,
                bottom,
                right,
            })
        })
        .collect()
}

pub(in crate::regression::targeted_comparison) fn address(row: u32, mut column: u32) -> String {
    let mut letters = String::new();
    while column > 0 {
        column -= 1;
        letters.insert(0, (b'A' + (column % 26) as u8) as char);
        column /= 26;
    }
    format!("{letters}{row}")
}

impl Rect {
    pub fn contains(self, row: u32, column: u32) -> bool {
        (self.top..=self.bottom).contains(&row) && (self.left..=self.right).contains(&column)
    }
    pub fn intersect(self, other: Self) -> Option<Self> {
        let result = Self {
            top: self.top.max(other.top),
            left: self.left.max(other.left),
            bottom: self.bottom.min(other.bottom),
            right: self.right.min(other.right),
        };
        (result.top <= result.bottom && result.left <= result.right).then_some(result)
    }
    pub fn address(self) -> String {
        format!(
            "{}:{}",
            address(self.top, self.left),
            address(self.bottom, self.right)
        )
    }

    /// Partition the finite comparison extent; the image API still enforces its own geometry limits.
    pub fn tiles(self) -> impl Iterator<Item = Self> {
        (self.top..=self.bottom)
            .step_by(CAPTURE_ROWS as usize)
            .flat_map(move |top| {
                (self.left..=self.right)
                    .step_by(CAPTURE_COLUMNS as usize)
                    .map(move |left| Self {
                        top,
                        left,
                        bottom: (top + CAPTURE_ROWS - 1).min(self.bottom),
                        right: (left + CAPTURE_COLUMNS - 1).min(self.right),
                    })
            })
    }
}
