//! Stream cell fingerprints through owned scratch files instead of an in-memory cell index.
use crate::regression::{streaming_xml::Event, xml_views::Kind};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::PathBuf,
    sync::Arc,
};

const RECORD_BYTES: usize = 40;

#[derive(Clone, Default)]
pub(super) struct Cells(Option<Arc<Records>>);

struct Records {
    path: PathBuf,
    digest: [u8; 32],
    has_formulas: bool,
}

impl Drop for Records {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path) {
            tracing::warn!(path=%self.path.display(),%error,"Cannot remove cell fingerprint scratch file");
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Cell {
    pub row: u32,
    pub column: u16,
    pub digest: [u8; 32],
    pub visible: bool,
    pub has_formula: bool,
}
impl Cell {
    fn key(&self) -> (u32, u16) {
        (self.row, self.column)
    }
    pub fn address(&self) -> String {
        let mut column = u32::from(self.column);
        let mut letters = String::new();
        while column > 0 {
            column -= 1;
            letters.insert(0, (b'A' + (column % 26) as u8) as char);
            column /= 26;
        }
        format!("{letters}{}", self.row)
    }
}

pub(super) struct CellChange {
    pub before: Option<Cell>,
    pub after: Option<Cell>,
}
impl CellChange {
    pub fn coordinate(&self) -> Cell {
        self.before
            .or(self.after)
            .expect("merge emits at least one side")
    }
    pub fn visible(&self) -> bool {
        self.before.is_some_and(|c| c.visible) || self.after.is_some_and(|c| c.visible)
    }
}

pub(super) struct Writer {
    output: Option<BufWriter<File>>,
    path: PathBuf,
    complete: bool,
    current: Option<(Cell, Sha256)>,
    last: Option<(u32, u16)>,
    digest: Sha256,
    has_formulas: bool,
}

impl Writer {
    pub fn new() -> Result<Self> {
        let path =
            std::env::temp_dir().join(format!("excel-shrink-cells-{}.bin", uuid::Uuid::new_v4()));
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self {
            output: Some(BufWriter::with_capacity(128 * 1024, output)),
            path,
            complete: false,
            current: None,
            last: None,
            digest: Sha256::new(),
            has_formulas: false,
        })
    }

    /// Hash text incrementally; even a large cell body is never accumulated in a Vec.
    pub fn event(&mut self, event: &Event<'_>) -> Result<()> {
        match event {
            Event::Tag(tag) => {
                if tag.local() == b"c" && matches!(tag.kind, Kind::Start | Kind::Empty) {
                    ensure!(
                        self.current.is_none(),
                        "Nested worksheet cells are unsupported"
                    );
                    let address = tag.attr(b"r")?.context("Worksheet cell has no address")?;
                    let split = address
                        .iter()
                        .position(u8::is_ascii_digit)
                        .context("Invalid cell address")?;
                    let mut column = 0u32;
                    for &letter in &address[..split] {
                        ensure!(letter.is_ascii_uppercase(), "Invalid cell column");
                        column = column
                            .checked_mul(26)
                            .and_then(|v| v.checked_add(u32::from(letter - b'A') + 1))
                            .context("Cell column overflow")?;
                    }
                    let row = std::str::from_utf8(&address[split..])?.parse::<u32>()?;
                    ensure!(
                        (1..=1_048_576).contains(&row) && (1..=16384).contains(&column),
                        "Cell outside Excel grid"
                    );
                    self.current = Some((
                        Cell {
                            row,
                            column: column as u16,
                            digest: [0; 32],
                            visible: false,
                            has_formula: false,
                        },
                        Sha256::new(),
                    ));
                }
                if let Some((cell, hash)) = &mut self.current {
                    hash.update([
                        0,
                        match tag.kind {
                            Kind::Start => 1,
                            Kind::Empty => 2,
                            Kind::End => 3,
                        },
                    ]);
                    hash.update(tag.name);
                    hash.update([0]);
                    hash.update(tag.attrs);
                    hash.update([0]);
                    cell.visible |= matches!(tag.local(), b"v" | b"f" | b"is" | b"t");
                    cell.has_formula |= tag.local() == b"f";
                }
                if tag.local() == b"c" && matches!(tag.kind, Kind::End | Kind::Empty) {
                    self.finish_cell()?;
                }
            }
            Event::Text(bytes) | Event::Cdata(bytes) => {
                if let Some((_, hash)) = &mut self.current {
                    hash.update(bytes);
                }
            }
        }
        Ok(())
    }

    fn finish_cell(&mut self) -> Result<()> {
        let (mut cell, hash) = self.current.take().context("Unexpected cell end")?;
        ensure!(
            self.last.is_none_or(|last| last < cell.key()),
            "Worksheet cells must have unique ascending numeric coordinates"
        );
        self.last = Some(cell.key());
        cell.digest = hash.finalize().into();
        self.write_cell(cell)
    }

    fn write_cell(&mut self, cell: Cell) -> Result<()> {
        self.has_formulas |= cell.has_formula;
        let mut record = [0u8; RECORD_BYTES];
        record[..4].copy_from_slice(&cell.row.to_le_bytes());
        record[4..6].copy_from_slice(&cell.column.to_le_bytes());
        record[6] = u8::from(cell.visible);
        record[7] = u8::from(cell.has_formula);
        record[8..].copy_from_slice(&cell.digest);
        self.output
            .as_mut()
            .context("Cell writer closed")?
            .write_all(&record)?;
        self.digest.update(record);
        Ok(())
    }

    pub fn finish(mut self) -> Result<Cells> {
        ensure!(self.current.is_none(), "Unclosed worksheet cell");
        self.output.as_mut().unwrap().flush()?;
        self.output.take();
        let records = Records {
            path: self.path.clone(),
            digest: self.digest.clone().finalize().into(),
            has_formulas: self.has_formulas,
        };
        self.complete = true;
        Ok(Cells(Some(Arc::new(records))))
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.output.take();
        if !self.complete {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn next_cell(reader: &mut Option<BufReader<File>>) -> Result<Option<Cell>> {
    let Some(reader) = reader else {
        return Ok(None);
    };
    if reader.fill_buf()?.is_empty() {
        return Ok(None);
    }
    let mut record = [0; RECORD_BYTES];
    reader.read_exact(&mut record)?;
    Ok(Some(Cell {
        row: u32::from_le_bytes(record[..4].try_into().unwrap()),
        column: u16::from_le_bytes(record[4..6].try_into().unwrap()),
        visible: record[6] != 0,
        has_formula: record[7] != 0,
        digest: record[8..].try_into().unwrap(),
    }))
}

impl Cells {
    /// Formula XML/cache equality alone cannot establish nonvolatile conditional-format inputs.
    pub fn formula_cells(&self, mut emit: impl FnMut(Cell) -> Result<()>) -> Result<()> {
        if !self.0.as_ref().is_some_and(|records| records.has_formulas) {
            return Ok(());
        }
        let mut reader = self
            .0
            .as_ref()
            .map(|records| File::open(&records.path).map(BufReader::new))
            .transpose()?;
        while let Some(cell) = next_cell(&mut reader)? {
            if cell.has_formula {
                emit(cell)?;
            }
        }
        Ok(())
    }
    /// Merge two immutable disk streams in numeric grid order; emit only actual differences.
    pub fn changes(
        &self,
        other: &Self,
        mut emit: impl FnMut(CellChange) -> Result<()>,
    ) -> Result<()> {
        if self
            .0
            .as_ref()
            .zip(other.0.as_ref())
            .is_some_and(|(a, b)| a.digest == b.digest)
        {
            return Ok(());
        }
        let open = |cells: &Self| -> Result<_> {
            cells
                .0
                .as_ref()
                .map(|r| File::open(&r.path).map(BufReader::new))
                .transpose()
                .map_err(Into::into)
        };
        let (mut a, mut b) = (open(self)?, open(other)?);
        let (mut left, mut right) = (next_cell(&mut a)?, next_cell(&mut b)?);
        while left.is_some() || right.is_some() {
            match (left, right) {
                (Some(x), Some(y)) if x.key() == y.key() => {
                    if x.digest != y.digest {
                        emit(CellChange {
                            before: left,
                            after: right,
                        })?;
                    }
                    left = next_cell(&mut a)?;
                    right = next_cell(&mut b)?;
                }
                (Some(x), Some(y)) if x.key() > y.key() => {
                    emit(CellChange {
                        before: None,
                        after: right,
                    })?;
                    right = next_cell(&mut b)?;
                }
                (Some(_), _) => {
                    emit(CellChange {
                        before: left,
                        after: None,
                    })?;
                    left = next_cell(&mut a)?;
                }
                (_, Some(_)) => {
                    emit(CellChange {
                        before: None,
                        after: right,
                    })?;
                    right = next_cell(&mut b)?;
                }
                _ => break,
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn empty_font_cell() -> Self {
        let mut writer = Writer::new().unwrap();
        writer
            .write_cell(Cell {
                row: 8,
                column: 2,
                digest: [1; 32],
                visible: false,
                has_formula: false,
            })
            .unwrap();
        writer.finish().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regression::streaming_xml;

    fn cells(xml: &[u8]) -> Result<Cells> {
        let mut writer = Writer::new()?;
        streaming_xml::scan(xml, |event| writer.event(&event))?;
        writer.finish()
    }

    #[test]
    fn merge_preserves_numeric_order_and_deleted_cells() -> Result<()> {
        let before =
            cells(br#"<row><c r="Z9"><v>1</v></c><c r="AA9"/><c r="A10"><v>3</v></c></row>"#)?;
        let after =
            cells(br#"<row><c r="Z9"><v>2</v></c><c r="AB9"/><c r="A10"><v>3</v></c></row>"#)?;
        let mut found = Vec::new();
        before.changes(&after, |change| {
            found.push((
                change.coordinate().address(),
                change.before.is_some(),
                change.after.is_some(),
            ));
            Ok(())
        })?;
        assert_eq!(
            found,
            vec![
                ("Z9".into(), true, true),
                ("AA9".into(), true, false),
                ("AB9".into(), false, true)
            ]
        );
        Ok(())
    }

    #[test]
    fn scratch_file_lives_until_the_last_snapshot_is_dropped() -> Result<()> {
        let original = cells(br#"<c r="A1"/>"#)?;
        let path = original.0.as_ref().unwrap().path.clone();
        let retained = original.clone();
        drop(original);
        assert!(path.exists());
        drop(retained);
        assert!(!path.exists());
        Ok(())
    }
}
