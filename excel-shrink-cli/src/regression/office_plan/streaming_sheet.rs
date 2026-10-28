//! Capture bounds from the decompressed worksheet stream. Keep the current row,
//! current cell and column definitions, never a whole sheet or cell body.
use super::{CaptureColumnSpan, Coordinate, SheetXmlSummary, MAX_COLUMNS, MAX_ROWS, parse_column_width};
use crate::regression::{
    streaming_xml::{self, Event},
    xml_views::{decimal, truth, Kind, Tag},
};
use anyhow::{ensure, Context, Result};
use std::io::Read;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Other,
    Columns,
    SheetData,
    MergeCells,
}

struct Cell {
    coordinate: Coordinate,
    text_type: bool,
    content: bool,
    inline_text: bool,
    value_depth: Option<usize>,
    inline_depth: Option<usize>,
    text_depth: Option<usize>,
}

struct SheetScan {
    summary: SheetXmlSummary,
    section: Section,
    current_row: u32,
    current_column: u32,
    in_row: bool,
    cell: Option<Cell>,
}

impl SheetScan {
    fn new() -> Self {
        Self {
            summary: SheetXmlSummary { max_row: 1, max_column: 1, ..Default::default() },
            section: Section::Other,
            current_row: 0,
            current_column: 0,
            in_row: false,
            cell: None,
        }
    }

    fn event(&mut self, event: Event<'_>) -> Result<()> {
        match event {
            Event::Text(bytes) => {
                if let Some(cell) = &mut self.cell {
                    if (cell.value_depth.is_some() || cell.text_depth.is_some())
                        && bytes.iter().any(|byte| !byte.is_ascii_whitespace())
                    {
                        cell.content = true;
                        cell.inline_text |= cell.text_depth.is_some();
                    }
                }
            }
            Event::Tag(tag) => self.tag(tag)?,
        }
        Ok(())
    }

    fn tag(&mut self, tag: Tag<'_>) -> Result<()> {
        let local = tag.local();
        if tag.kind == Kind::End {
            if tag.depth == 3 && local == b"c" {
                if let Some(cell) = self.cell.take() {
                    if cell.content {
                        self.summary.max_row = self.summary.max_row.max(cell.coordinate.row);
                        self.summary.max_column = self.summary.max_column.max(cell.coordinate.column);
                        self.summary.has_text |= cell.inline_text || cell.text_type;
                    }
                }
            } else if let Some(cell) = &mut self.cell {
                if cell.value_depth == Some(tag.depth) {
                    cell.value_depth = None;
                }
                if cell.text_depth == Some(tag.depth) {
                    cell.text_depth = None;
                }
                if cell.inline_depth == Some(tag.depth) {
                    cell.inline_depth = None;
                }
            }
            if tag.depth == 2 && local == b"row" {
                self.in_row = false;
            } else if tag.depth == 1 {
                self.section = Section::Other;
            }
            return Ok(());
        }
        if tag.depth == 0 {
            ensure!(local == b"worksheet", "expected worksheet root for capture bounds");
            return Ok(());
        }
        if tag.depth == 1 {
            self.section = if tag.kind == Kind::Start {
                match local {
                    b"cols" => Section::Columns,
                    b"sheetData" => Section::SheetData,
                    b"mergeCells" => Section::MergeCells,
                    _ => Section::Other,
                }
            } else {
                Section::Other
            };
            if local == b"sheetFormatPr" {
                if let Some(width) = tag.attr(b"defaultColWidth")? {
                    self.summary.columns.default_width = parse_column_width(width)?;
                }
            }
            // The declared dimension can include formatting-only bloat. Bounds
            // still come from real contents/merges, as in the byte-view scanner.
            return Ok(());
        }
        if self.section == Section::Columns && local == b"col" {
            let min = decimal(tag.attr(b"min")?.context("column min missing")?)?;
            let max = decimal(tag.attr(b"max")?.context("column max missing")?)?;
            ensure!(min > 0 && min <= max && max <= MAX_COLUMNS, "invalid column range");
            ensure!(
                self.summary.columns.spans.len() < MAX_COLUMNS as usize,
                "too many column definitions for bounded capture planning"
            );
            self.summary.columns.spans.push(CaptureColumnSpan {
                min,
                max,
                width: tag.attr(b"width")?.map(parse_column_width).transpose()?,
                hidden: truth(tag.attr(b"hidden")?),
            });
        } else if self.section == Section::MergeCells && tag.depth == 2 && local == b"mergeCell" {
            if let Some(reference) = tag.attr(b"ref")? {
                for cell in reference.split(|byte| *byte == b':') {
                    let coordinate = Coordinate::parse(cell)?;
                    self.summary.max_row = self.summary.max_row.max(coordinate.row);
                    self.summary.max_column = self.summary.max_column.max(coordinate.column);
                }
            }
        } else if self.section == Section::SheetData {
            self.sheet_data(tag)?;
        }
        Ok(())
    }

    fn sheet_data(&mut self, tag: Tag<'_>) -> Result<()> {
        let local = tag.local();
        if tag.depth == 2 && local == b"row" {
            self.current_row = match tag.attr(b"r")? {
                Some(value) => decimal(value)?,
                None => self.current_row.checked_add(1).context("row overflow")?,
            };
            ensure!((1..=MAX_ROWS).contains(&self.current_row), "row outside XLSX grid");
            self.current_column = 0;
            if truth(tag.attr(b"hidden")?) {
                // Preserve the existing visual-tail heuristic: omitted rows do
                // not cancel an explicit hidden suffix; explicit visible rows do.
                self.summary.trailing_hidden_row_start.get_or_insert(self.current_row);
            } else {
                self.summary.trailing_hidden_row_start = None;
            }
            self.in_row = tag.kind == Kind::Start;
        } else if self.in_row && tag.depth == 3 && local == b"c" {
            let coordinate = match tag.attr(b"r")? {
                Some(value) => Coordinate::parse(value)?,
                None => Coordinate {
                    row: self.current_row,
                    column: self.current_column.checked_add(1).context("column overflow")?,
                },
            };
            ensure!(
                coordinate.row == self.current_row && coordinate.column <= MAX_COLUMNS,
                "cell reference disagrees with row"
            );
            self.current_column = coordinate.column;
            if tag.kind == Kind::Start {
                self.cell = Some(Cell {
                    coordinate,
                    text_type: matches!(tag.attr(b"t")?.unwrap_or(b"n"), b"s" | b"inlineStr" | b"str"),
                    content: false,
                    inline_text: false,
                    value_depth: None,
                    inline_depth: None,
                    text_depth: None,
                });
            }
        } else if let Some(cell) = &mut self.cell {
            // Preserve conservative nonempty-value handling for nested markup.
            if cell.value_depth.is_some() || cell.text_depth.is_some() {
                cell.content = true;
                cell.inline_text |= cell.text_depth.is_some();
            }
            if tag.depth == 4 {
                match local {
                    b"f" => cell.content = true,
                    b"v" if tag.kind == Kind::Start => cell.value_depth = Some(tag.depth),
                    b"is" if tag.kind == Kind::Start => cell.inline_depth = Some(tag.depth),
                    _ => {}
                }
            } else if cell.inline_depth.is_some() && local == b"t" && tag.kind == Kind::Start {
                cell.text_depth = Some(tag.depth);
            }
        }
        Ok(())
    }
}

pub(super) fn scan(input: impl Read) -> Result<SheetXmlSummary> {
    let mut sheet = SheetScan::new();
    streaming_xml::scan(input, |event| sheet.event(event))?;
    Ok(sheet.summary)
}

#[cfg(test)]
#[path = "streaming_sheet_tests.rs"]
mod tests;
