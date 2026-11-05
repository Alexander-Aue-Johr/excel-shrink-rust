//! Optional diagnostic index of actual saved cell values and shared formulas.
//! Its explicit budget never limits the scientific capture or comparison path.
use super::syntax::{self, Address};
use crate::regression::{
    office_plan,
    streaming_xml::{self, Event},
    xml_views::Kind,
};
use anyhow::{ensure, Context, Result};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek},
    path::Path,
};
use zip::ZipArchive;

const MAX_CELLS: usize = 100_000;
const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub(super) struct Cell {
    pub formula: Option<String>,
    pub value: Option<String>,
    pub kind: String,
    pub shared_anchor: Option<String>,
}

impl Cell {
    pub fn same_value(&self, other: &Self) -> bool {
        if self.kind != other.kind {
            return false;
        }
        match (&self.value, &other.value) {
            (Some(a), Some(b)) if self.kind == "n" => match (a.parse::<f64>(), b.parse::<f64>()) {
                (Ok(a), Ok(b)) => a == b,
                _ => a == b,
            },
            (a, b) => a == b,
        }
    }
}

pub(super) type Workbook = BTreeMap<Address, Cell>;

#[derive(Default)]
struct Budget {
    text: usize,
}
impl Budget {
    fn add(&mut self, bytes: usize) -> Result<()> {
        self.text = self
            .text
            .checked_add(bytes)
            .context("Formula diagnostic byte count overflow")?;
        ensure!(
            self.text <= MAX_TEXT_BYTES,
            "Formula diagnostic text budget exceeded; visual result remains unchanged"
        );
        Ok(())
    }
}

/// Include hidden worksheet dependencies. No worksheet DOM is created.
pub(super) fn read(path: &Path) -> Result<Workbook> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    read_archive(&mut archive)
}

pub(super) fn read_archive<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Workbook> {
    let mut budget = Budget::default();
    let strings = read_strings(archive, &mut budget)?;
    let mut result = Workbook::new();
    for (sheet, part) in office_plan::formula_worksheet_parts(archive)? {
        read_sheet(
            archive.by_name(&part)?,
            &sheet,
            &strings,
            &mut result,
            &mut budget,
        )?;
    }
    Ok(result)
}

fn read_strings<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    budget: &mut Budget,
) -> Result<Vec<String>> {
    let stream = match archive.by_name("xl/sharedStrings.xml") {
        Ok(stream) => stream,
        Err(zip::result::ZipError::FileNotFound) => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut strings = Vec::new();
    let mut current = None;
    let mut in_text = false;
    let mut phonetic = false;
    streaming_xml::scan(stream, |event| {
        match event {
            Event::Tag(tag) => match (tag.local(), tag.kind) {
                (b"si", Kind::Start) => current = Some(Vec::new()),
                (b"rPh", kind) => phonetic = kind != Kind::End,
                (b"t", kind) => in_text = kind == Kind::Start && !phonetic,
                (b"si", Kind::End | Kind::Empty) => {
                    ensure!(
                        strings.len() < MAX_CELLS,
                        "Formula diagnostic shared-string budget exceeded"
                    );
                    strings.push(office_plan::decode_attr(
                        &current.take().unwrap_or_default(),
                    )?);
                }
                _ => {}
            },
            Event::Text(bytes) if in_text => {
                budget.add(bytes.len())?;
                current
                    .as_mut()
                    .context("String text outside an item")?
                    .extend_from_slice(bytes);
            }
            Event::Cdata(_) if in_text => anyhow::bail!("CDATA string diagnostic is unsupported"),
            _ => {}
        }
        Ok(())
    })?;
    Ok(strings)
}

#[derive(Default)]
struct Pending {
    address: Option<Address>,
    kind: String,
    has_formula: bool,
    formula: Vec<u8>,
    value: Vec<u8>,
    inline: Vec<u8>,
    shared: Option<u32>,
}

fn read_sheet(
    input: impl Read,
    sheet: &str,
    strings: &[String],
    result: &mut Workbook,
    budget: &mut Budget,
) -> Result<()> {
    let mut in_data = false;
    let mut current: Option<Pending> = None;
    let mut field = None;
    let mut shared = BTreeMap::<u32, (Address, String)>::new();
    streaming_xml::scan(input, |event| {
        match event {
            Event::Tag(tag) => {
                if tag.local() == b"sheetData" {
                    in_data = tag.kind == Kind::Start;
                }
                if !in_data {
                    return Ok(());
                }
                match (tag.local(), tag.kind) {
                    (b"c", Kind::Start | Kind::Empty) => {
                        ensure!(current.is_none(), "Nested diagnostic cells");
                        let address = office_plan::decode_attr(
                            tag.attr(b"r")?.context("Cell address missing")?,
                        )?;
                        let kind = office_plan::decode_attr(tag.attr(b"t")?.unwrap_or(b"n"))?;
                        current = Some(Pending {
                            address: Some(Address::parse(sheet, &address)?),
                            kind,
                            ..Default::default()
                        });
                    }
                    (b"f", Kind::Start | Kind::Empty) => {
                        let cell = current.as_mut().context("Formula outside a cell")?;
                        cell.has_formula = true;
                        if tag.attr(b"t")? == Some(b"shared") {
                            cell.shared = Some(
                                std::str::from_utf8(
                                    tag.attr(b"si")?.context("Shared formula index missing")?,
                                )?
                                .parse()?,
                            );
                        }
                        field = (tag.kind == Kind::Start).then_some(b'f');
                    }
                    (b"v", Kind::Start) => field = Some(b'v'),
                    (b"t", Kind::Start) if current.is_some() => field = Some(b't'),
                    (b"f" | b"v" | b"t", Kind::End | Kind::Empty) => field = None,
                    _ => {}
                }
                if tag.local() == b"c" && matches!(tag.kind, Kind::End | Kind::Empty) {
                    finish(
                        current.take().context("Unexpected diagnostic cell end")?,
                        strings,
                        &mut shared,
                        result,
                        budget,
                    )?;
                    field = None;
                }
            }
            Event::Text(bytes) if field.is_some() => {
                budget.add(bytes.len())?;
                let cell = current.as_mut().context("Diagnostic text outside a cell")?;
                match field.unwrap() {
                    b'f' => &mut cell.formula,
                    b'v' => &mut cell.value,
                    _ => &mut cell.inline,
                }
                .extend_from_slice(bytes);
            }
            Event::Cdata(_) if field.is_some() => {
                anyhow::bail!("CDATA cell diagnostic is unsupported")
            }
            _ => {}
        }
        Ok(())
    })
}

fn finish(
    pending: Pending,
    strings: &[String],
    anchors: &mut BTreeMap<u32, (Address, String)>,
    result: &mut Workbook,
    budget: &mut Budget,
) -> Result<()> {
    if !pending.has_formula && pending.value.is_empty() && pending.inline.is_empty() {
        return Ok(());
    }
    ensure!(
        result.len() < MAX_CELLS,
        "Formula diagnostic cell budget exceeded; visual result remains unchanged"
    );
    let address = pending.address.context("Diagnostic address missing")?;
    let mut cell = Cell {
        kind: pending.kind,
        ..Default::default()
    };
    if pending.has_formula {
        let mut formula = office_plan::decode_attr(&pending.formula)?;
        if let Some(index) = pending.shared {
            if formula.is_empty() {
                let (anchor, source) = anchors
                    .get(&index)
                    .context("Shared formula has no preceding anchor")?;
                formula = syntax::shifted_formula(source, anchor, &address)?;
                cell.shared_anchor = Some(anchor.cell());
            } else {
                anchors.insert(index, (address.clone(), formula.clone()));
            }
        }
        cell.formula = Some(formula);
    }
    cell.value = if cell.kind == "s" {
        let index: usize = std::str::from_utf8(&pending.value)?.parse()?;
        cell.kind = "string".into();
        Some(
            strings
                .get(index)
                .context("Shared string index outside table")?
                .clone(),
        )
    } else if cell.kind == "inlineStr" {
        cell.kind = "string".into();
        Some(office_plan::decode_attr(&pending.inline)?)
    } else if pending.value.is_empty() {
        None
    } else {
        Some(office_plan::decode_attr(&pending.value)?)
    };
    budget.add(
        cell.value.as_ref().map_or(0, String::len) + cell.formula.as_ref().map_or(0, String::len),
    )?;
    ensure!(
        result.insert(address, cell).is_none(),
        "Duplicate diagnostic cell coordinate"
    );
    Ok(())
}
