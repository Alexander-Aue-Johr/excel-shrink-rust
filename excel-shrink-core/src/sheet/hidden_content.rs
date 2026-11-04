//! Streaming, row-local admission for conservative hidden constant tails.
//! Ordinary hidden tables never become deferred deletion candidates. Original
//! cells, including protected/formula cells, contribute before style cleanup.
use super::cells::ParsedCell;
use crate::models::{CleanSheetMetrics, CleanupCause, HiddenContentSpan};
use crate::xml::{attributes::Attributes, find_bytes};

const LAST_COLUMN: u32 = 16_384;
const MIN_TAIL_CELLS: u32 = 4_096;

#[derive(Default)]
pub(super) struct HiddenTail {
    start: u32,
    last: u32,
    value: Option<i64>,
    rejected: bool,
}

impl HiddenTail {
    pub(super) fn observe(&mut self, cell: &ParsedCell<'_, '_>, hidden: bool, content: bool) {
        if !hidden {
            *self = Self::default();
            return;
        }
        if self.start == 0 {
            self.start = cell.col_num;
        } else if cell.col_num != self.last + 1 {
            self.rejected = true;
        }
        self.last = cell.col_num;
        if self.rejected {
            return;
        }
        let value = content.then(|| integer_constant(cell)).flatten();
        match (self.value, value) {
            (_, None) => self.rejected = true,
            (Some(before), Some(after)) if before != after => self.rejected = true,
            (_, Some(value)) => self.value = Some(value),
        }
    }

    /// Drop rejected Hidden candidates before workbook reference scanning, and
    /// replace a fully eligible suffix with one span. Missing eligibility for
    /// any source cell (font/protection/geometry guards) retains the whole run.
    pub(super) fn finish(self, metrics: &mut CleanSheetMetrics, begin: usize, row: u32) {
        let admitted = !self.rejected
            && self.value.is_some()
            && self.start > 0
            && self.last == LAST_COLUMN
            && self.last - self.start + 1 >= MIN_TAIL_CELLS;
        let count = metrics.candidates[begin..]
            .iter()
            .filter(|c| {
                c.cause == CleanupCause::UnreferencedHiddenContentCells
                    && c.row == row
                    && self.start <= c.col
                    && c.col <= self.last
            })
            .count();
        let mut write = begin;
        for read in begin..metrics.candidates.len() {
            if metrics.candidates[read].cause != CleanupCause::UnreferencedHiddenContentCells {
                metrics.candidates.swap(write, read);
                write += 1;
            }
        }
        metrics.candidates.truncate(write);
        if admitted && count == (self.last - self.start + 1) as usize {
            metrics.hidden_content_spans.push(HiddenContentSpan {
                row,
                start_col: self.start,
                end_col: self.last,
            });
        }
    }
}

fn integer_constant(cell: &ParsedCell<'_, '_>) -> Option<i64> {
    for attr in Attributes::new(cell.attr) {
        let attr = attr.ok()?;
        if attr.name == b"t" && attr.value != b"n" {
            return None;
        }
    }
    if find_bytes(cell.raw, b"<f").is_some() {
        return None;
    }
    let start = find_bytes(cell.raw, b"<v>")? + 3;
    let tail = &cell.raw[start..];
    let end = find_bytes(tail, b"</v>")?;
    let value = &tail[..end];
    // Constant zero is the common path. Reject unsupported decimal spellings,
    // overflow and strings rather than approximating values or decoding SSTs.
    if value == b"0" {
        return Some(0);
    }
    if value.is_empty() || value.len() > 20 {
        return None;
    }
    std::str::from_utf8(value).ok()?.parse().ok()
}
