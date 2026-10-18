//! The common path owns only ZIP/XML buffers, the SST index, column intervals
//! and small per-sheet reports. Cells and values are borrowed stack views.
use anyhow::{Result, ensure, Context};
use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeSet, HashMap, HashSet}, fs::File, io::Read, path::{Path, PathBuf}, ops::Range};
use zip::ZipArchive;
use super::{CellValueDifference, CellDifferenceKind, MissingCellPolicy, WorkbookComparisonReport, WorksheetCaptureRange};
use super::super::{artifact_cache as cache, xml_views::{self as xml, Reader, Kind, TextBytes}};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) struct Coordinate { pub row: u32, pub column: u32 }
impl Coordinate {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut p = 0;
        if bytes.get(p) == Some(&b'$') { p += 1; }
        let mut column = 0u32;
        while let Some(&b) = bytes.get(p) {
            if !b.is_ascii_alphabetic() { break; }
            column = column.checked_mul(26).and_then(|n| n.checked_add(u32::from(b.to_ascii_uppercase() - b'A' + 1)))
                .context("column coordinate overflow")?;
            p += 1;
        }
        if bytes.get(p) == Some(&b'$') { p += 1; }
        let row = xml::decimal(&bytes[p..])?;
        ensure!((1..=1_048_576).contains(&row) && (1..=16_384).contains(&column), "cell reference outside XLSX grid");
        Ok(Self { row, column })
    }
    fn name(self) -> String {
        let mut out = [0u8; 3]; let mut n = self.column; let mut p = out.len();
        while n > 0 { n -= 1; p -= 1; out[p] = b'A' + (n % 26) as u8; n /= 26; }
        format!("{}{}", std::str::from_utf8(&out[p..]).unwrap(), self.row)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Dimension { first: Coordinate, last: Coordinate }
impl Dimension {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let mut parts = bytes.split(|&b| b == b':');
        let first = Coordinate::parse(parts.next().context("missing dimension")?)?;
        let last = match parts.next() { Some(p) => Coordinate::parse(p)?, None => first };
        ensure!(parts.next().is_none() && first.row <= last.row && first.column <= last.column, "invalid worksheet dimension");
        Ok(Self { first, last })
    }
    fn include(&mut self, c: Coordinate) {
        self.first.row = self.first.row.min(c.row); self.first.column = self.first.column.min(c.column);
        self.last.row = self.last.row.max(c.row); self.last.column = self.last.column.max(c.column);
    }
    fn contains(&self, v: Self) -> bool {
        self.first.row <= v.first.row && self.first.column <= v.first.column
            && self.last.row >= v.last.row && self.last.column >= v.last.column
    }
    fn excessive(&self, v: Self) -> bool {
        self.last.row.saturating_sub(v.last.row) > v.last.row - v.first.row + 1
            || self.last.column.saturating_sub(v.last.column) > v.last.column - v.first.column + 1
    }
}

#[derive(Clone, Copy)]
pub(super) struct TextView<'a> { raw: &'a [u8], runs: bool }
struct TextIterator<'a> { reader: Option<Reader<'a, 16>>, bytes: TextBytes<'a> }
impl<'a> TextView<'a> {
    fn bytes(self) -> Result<TextIterator<'a>> {
        Ok(TextIterator { reader: if self.runs { Some(Reader::<16>::new(self.raw)?) } else { None },
            bytes: TextBytes::new(if self.runs { &[] } else { self.raw }) })
    }
    fn fingerprint(self) -> Result<([u8; 32], bool, bool)> {
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 1024]; let mut used = 0;
        let mut whitespace = true; let mut content = false;
        for byte in self.bytes()? {
            let byte = byte?; content = true;
            whitespace &= matches!(byte, b' ' | b'\t' | b'\n' | b'\r');
            buffer[used] = byte; used += 1;
            if used == buffer.len() { hash.update(&buffer); used = 0; }
        }
        hash.update(&buffer[..used]);
        Ok((hash.finalize().into(), whitespace, content))
    }
    fn equal(self, other: Self) -> Result<bool> {
        if self.runs == other.runs && self.raw == other.raw { return Ok(true); }
        let mut a = self.bytes()?; let mut b = other.bytes()?;
        loop {
            match (a.next().transpose()?, b.next().transpose()?) {
                (None, None) => return Ok(true),
                (Some(a), Some(b)) if a == b => {},
                _ => return Ok(false),
            }
        }
    }
    fn report(self) -> Result<String> {
        let bytes: Result<Vec<_>> = self.bytes()?.collect();
        Ok(String::from_utf8(bytes?)?)
    }
}
impl<'a> Iterator for TextIterator<'a> {
    type Item = Result<u8>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(byte) = self.bytes.next() { return Some(byte); }
            let reader = self.reader.as_mut()?;
            let tag = match reader.next() { Ok(Some(t)) => t, Ok(None) => return None, Err(e) => return Some(Err(e)) };
            if tag.kind == Kind::End { continue; }
            if tag.local() == b"rPh" || tag.local() == b"rPr" || tag.local() == b"phoneticPr" {
                if let Err(e) = reader.element(tag) { return Some(Err(e)); }
                continue;
            }
            if tag.local() == b"t" {
                match reader.element(tag) {
                    Ok(element) if !element.body.contains(&b'<') => self.bytes = TextBytes::new(element.body),
                    Ok(_) => return Some(Err(anyhow::anyhow!("nested markup inside spreadsheet text"))),
                    Err(e) => return Some(Err(e)),
                }
            }
        }
    }
}

pub(super) struct SharedEntry { body: Range<usize>, digest: [u8; 32], whitespace: bool, content: bool }
pub(super) struct SharedStrings { xml: Vec<u8>, entries: Vec<SharedEntry> }
impl SharedStrings {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let mut entries = Vec::new();
        if !bytes.is_empty() {
            std::str::from_utf8(&bytes)?;
            let mut reader = Reader::<128>::new(&bytes)?;
            let mut root = false;
            while let Some(tag) = reader.next()? {
                if tag.kind == Kind::End { continue; }
                if tag.depth == 0 { ensure!(!root && tag.local() == b"sst", "expected one shared-string table root"); root = true; }
                if tag.local() == b"si" && tag.depth == 1 {
                    let element = reader.element(tag)?;
                    let view = TextView { raw: element.body, runs: true };
                    let (digest, whitespace, content) = view.fingerprint()?;
                    let start = element.body.as_ptr() as usize - bytes.as_ptr() as usize;
                    entries.push(SharedEntry { body: start..start + element.body.len(), digest, whitespace, content });
                }
            }
            ensure!(root, "missing SST root");
        }
        Ok(Self { xml: bytes, entries })
    }
    fn get(&self, index_bytes: &[u8]) -> Result<Value<'_>> {
        // Checked ASCII -> integer conversion does not allocate. A vector avoids
        // hashing decimal keys and avoids owning one decimal String per SST item.
        let index = xml::decimal(index_bytes)? as usize;
        let item = self.entries.get(index).context("shared-string index is out of bounds")?;
        Ok(Value { kind: ValueKind::Text, raw: &self.xml[item.body.clone()], runs: true, shared: Some(item) })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum ValueKind { Empty = 0, Number = 1, Boolean = 2, Error = 3, Date = 4, Text = 5 }
#[derive(Clone, Copy)]
pub(super) struct Value<'a> { kind: ValueKind, raw: &'a [u8], runs: bool, shared: Option<&'a SharedEntry> }
impl<'a> Value<'a> {
    fn empty() -> Self { Self { kind: ValueKind::Empty, raw: &[], runs: false, shared: None } }
    fn text(self) -> TextView<'a> { TextView { raw: self.raw, runs: self.runs } }
    fn is_empty(self) -> bool { self.kind == ValueKind::Empty }
    fn has_content(self) -> Result<bool> {
        if let Some(s) = self.shared { return Ok(s.content); }
        if self.kind == ValueKind::Text { return Ok(self.text().bytes()?.next().transpose()?.is_some()); }
        Ok(!self.is_empty() && !self.raw.is_empty())
    }
    fn is_whitespace(self) -> Result<bool> {
        if let Some(s) = self.shared { return Ok(s.whitespace); }
        if self.kind == ValueKind::Text { return Ok(self.text().fingerprint()?.1); }
        Ok(self.raw.iter().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r')))
    }
    fn hash_into(self, hash: &mut Sha256) -> Result<()> {
        hash.update([self.kind as u8]);
        if self.kind == ValueKind::Text {
            let digest = match self.shared { Some(s) => s.digest, None => self.text().fingerprint()?.0 };
            hash.update(digest);
        } else { hash.update((self.raw.len() as u64).to_le_bytes()); hash.update(self.raw); }
        Ok(())
    }
    fn equal(self, other: Self) -> Result<bool> {
        if self.kind != other.kind { return Ok(false); }
        if self.kind == ValueKind::Text { return self.text().equal(other.text()); }
        Ok(self.raw == other.raw)
    }
    fn report(self) -> Result<String> {
        Ok(if self.kind == ValueKind::Text { self.text().report()? }
           else { format!("{:?}:{}", self.kind, String::from_utf8_lossy(self.raw)) })
    }
}

#[derive(Clone, Copy)]
pub(super) struct CellView<'a> {
    pub coordinate: Coordinate,
    value: Value<'a>,
    formula: Option<&'a [u8]>,
    hidden: bool,
}
#[cfg(test)]
impl CellView<'_> {
    pub(super) fn borrowed_value_bytes(&self) -> &[u8] { self.value.raw }
}

#[derive(Clone, Copy)]
enum Source<'a> { Reference(&'a [u8]), Coordinate(Coordinate) }
impl Source<'_> { fn name(self) -> String { match self { Self::Reference(s) => String::from_utf8_lossy(s).into_owned(), Self::Coordinate(c) => c.name() } } }
struct Bounds<'a> { row: u32, column: u32, row_source: Option<Source<'a>>, column_source: Option<Source<'a>> }
impl<'a> Bounds<'a> {
    fn new() -> Self { Self { row: 1, column: 1, row_source: None, column_source: None } }
    fn include(&mut self, coordinate: Coordinate, source: Source<'a>) {
        if coordinate.row > self.row { self.row = coordinate.row; self.row_source = Some(source); }
        if coordinate.column > self.column { self.column = coordinate.column; self.column_source = Some(source); }
    }
    fn report(&self, index: usize, name: &str) -> WorksheetCaptureRange {
        WorksheetCaptureRange { sheet_index: index, sheet_file: name.to_owned(),
            max_cell: Coordinate { row: self.row, column: self.column }.name(), max_row: self.row, max_column: self.column,
            max_row_source: self.row_source.map(Source::name), max_column_source: self.column_source.map(Source::name) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureColumnSpan { pub min: u32, pub max: u32, pub width: Option<f64>, pub hidden: bool }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureColumns { pub default_width: f64, pub spans: Vec<CaptureColumnSpan> }
impl Default for CaptureColumns {
    fn default() -> Self { Self { default_width: 8.43, spans: Vec::new() } }
}
fn column_width(bytes: &[u8]) -> Result<f64> {
    let width: f64 = std::str::from_utf8(bytes)?.parse()?;
    ensure!(width.is_finite() && (0.0..=255.0).contains(&width), "invalid column width");
    Ok(width)
}

pub(super) struct Cells<'a> {
    reader: Reader<'a>, shared: &'a SharedStrings,
    row: u32, column: u32, row_hidden: bool, in_row: bool, in_data: bool,
    root_seen: bool, data_seen: bool, dimension: Option<Dimension>,
    hidden_columns: Vec<(u32, u32)>, capture_columns: CaptureColumns,
    pub ordered: bool, previous: Option<Coordinate>, bounds: Bounds<'a>,
}
impl<'a> Cells<'a> {
    pub fn new(bytes: &'a [u8], shared: &'a SharedStrings) -> Result<Self> {
        std::str::from_utf8(bytes)?;
        Ok(Self { reader: Reader::<128>::new(bytes)?, shared, row: 0, column: 0, row_hidden: false, in_row: false,
            in_data: false, root_seen: false, data_seen: false, dimension: None, hidden_columns: Vec::new(), capture_columns: CaptureColumns::default(), ordered: true,
            previous: None, bounds: Bounds::new() })
    }
    pub fn next(&mut self) -> Result<Option<CellView<'a>>> {
        while let Some(tag) = self.reader.next()? {
            let name = tag.local();
            if tag.kind == Kind::End {
                if name == b"row" && tag.depth == 2 { self.in_row = false; }
                if name == b"sheetData" && tag.depth == 1 { self.in_data = false; }
                continue;
            }
            if tag.depth == 0 {
                ensure!(!self.root_seen && name == b"worksheet", "expected exactly one worksheet root");
                self.root_seen = true;
                continue;
            }
            if tag.depth == 1 && name == b"dimension" {
                ensure!(self.dimension.is_none(), "duplicate worksheet dimension");
                self.dimension = Some(Dimension::parse(tag.attr(b"ref")?.context("dimension ref missing")?)?);
                self.reader.element(tag)?;
                continue;
            }
            if tag.depth == 1 && name == b"sheetData" {
                ensure!(!self.data_seen, "duplicate sheetData"); self.data_seen = true;
                self.in_data = tag.kind == Kind::Start; continue;
            }
            if tag.depth == 1 && name == b"sheetFormatPr" {
                if let Some(width) = tag.attr(b"defaultColWidth")? {
                    self.capture_columns.default_width = column_width(width)?;
                }
                self.reader.element(tag)?;
                continue;
            }
            if tag.depth == 1 && name == b"cols" {
                ensure!(!self.data_seen, "cols after sheetData is unsupported");
                let element = self.reader.element(tag)?;
                let mut cols = Reader::<128>::new(element.body)?;
                while let Some(col) = cols.next()? {
                    if col.local() == b"col" && col.kind != Kind::End {
                        let lo = xml::decimal(col.attr(b"min")?.context("column min is missing")?)?;
                        let hi = xml::decimal(col.attr(b"max")?.context("column max is missing")?)?;
                        ensure!(lo > 0 && hi >= lo && hi <= 16_384, "invalid column range");
                        let hidden = xml::truth(col.attr(b"hidden")?);
                        let width = col.attr(b"width")?.map(column_width).transpose()?;
                        if hidden { self.hidden_columns.push((lo, hi)); }
                        self.capture_columns.spans.push(CaptureColumnSpan { min: lo, max: hi, width, hidden });
                    }
                }
                continue;
            }
            if self.in_data && tag.depth == 2 && name == b"row" {
                self.row = match tag.attr(b"r")? { Some(r) => xml::decimal(r)?, None => self.row.checked_add(1).context("row overflow")? };
                ensure!(self.row > 0 && self.row <= 1_048_576, "row outside XLSX grid");
                self.column = 0; self.row_hidden = xml::truth(tag.attr(b"hidden")?); self.in_row = tag.kind == Kind::Start;
                continue;
            }
            if self.in_row && tag.depth == 3 && name == b"c" {
                let element = self.reader.element(tag)?;
                let reference = tag.attr(b"r")?;
                let coordinate = match reference {
                    Some(reference) => Coordinate::parse(reference)?,
                    None => Coordinate { row: self.row, column: self.column.checked_add(1).context("column overflow")? },
                };
                ensure!(coordinate.row == self.row && coordinate.column <= 16_384, "cell reference disagrees with its row");
                self.column = coordinate.column;
                if let Some(previous) = self.previous {
                    ensure!(previous != coordinate, "duplicate cell address {}", coordinate.name());
                    self.ordered &= previous < coordinate;
                }
                self.previous = Some(coordinate);
                let mut value_bytes = None; let mut inline = None; let mut formula = None;
                if !element.body.is_empty() {
                    let mut children = Reader::<16>::new(element.body)?;
                    while let Some(child) = children.next()? {
                        if child.kind == Kind::End { continue; }
                        let e = children.element(child)?;
                        match child.local() {
                            b"v" => { ensure!(value_bytes.is_none() && !e.body.contains(&b'<'), "duplicate or nested cell value"); value_bytes = Some(e.body); },
                            b"is" => { ensure!(inline.is_none(), "duplicate inline string"); inline = Some(e.body); },
                            b"f" => { ensure!(formula.is_none(), "duplicate formula"); formula = Some(e.raw); },
                            b"extLst" => {},
                            _ => anyhow::bail!("unsupported child of cell {}", coordinate.name()),
                        }
                    }
                }
                ensure!(value_bytes.is_none() || inline.is_none(), "cell contains both value and inline string");
                let cell_type = tag.attr(b"t")?.unwrap_or(b"n");
                let value = if cell_type == b"inlineStr" {
                    ensure!(value_bytes.is_none(), "inline string cell contains v");
                    Value { kind: ValueKind::Text, raw: inline.context("inline string cell has no is element")?, runs: true, shared: None }
                } else {
                    ensure!(inline.is_none(), "is element without inlineStr cell type");
                    match value_bytes {
                        None => { ensure!(matches!(cell_type, b"n" | b"b" | b"e" | b"d" | b"str" | b"s"), "unsupported cell type"); Value::empty() },
                        Some(raw) if cell_type == b"s" => self.shared.get(raw)?,
                        Some(raw) => {
                            let kind = match cell_type { b"n" => ValueKind::Number, b"b" => ValueKind::Boolean,
                                b"e" => ValueKind::Error, b"d" => ValueKind::Date, b"str" => ValueKind::Text,
                                _ => anyhow::bail!("unsupported cell type"), };
                            Value { kind, raw, runs: false, shared: None }
                        }
                    }
                };
                if value.has_content()? || formula.is_some() {
                    self.bounds.include(coordinate, reference.map(Source::Reference).unwrap_or(Source::Coordinate(coordinate)));
                }
                let hidden = self.row_hidden || self.hidden_columns.iter().any(|&(a, b)| a <= coordinate.column && coordinate.column <= b);
                return Ok(Some(CellView { coordinate, value, formula, hidden }));
            }
            if tag.depth == 1 && name == b"mergeCells" {
                let element = self.reader.element(tag)?;
                let mut merges = Reader::<128>::new(element.body)?;
                while let Some(merge) = merges.next()? {
                    if merge.local() == b"mergeCell" && merge.kind != Kind::End {
                        let reference = merge.attr(b"ref")?.context("mergeCell has no ref")?;
                        for cell in reference.split(|&b| b == b':') {
                            self.bounds.include(Coordinate::parse(cell)?, Source::Reference(reference));
                        }
                    }
                }
                continue;
            }
            // Unknown sibling containers are not cell data; consume explicitly.
            if tag.depth == 1 { self.reader.element(tag)?; }
            else { anyhow::bail!("unexpected worksheet element {}", String::from_utf8_lossy(name)); }
        }
        ensure!(self.root_seen && self.data_seen, "worksheet or sheetData missing");
        Ok(None)
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Summary {
    key: String, values_sha256: String, formulas_sha256: String, ordered: bool,
    value_count: usize, formula_count: usize, bounds: WorksheetCaptureRange,
    dimension: Option<Dimension>, value_extent: Option<Dimension>, columns: CaptureColumns,
}
fn summary_key(sheet_hash: &str, sst_hash: &str) -> String {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let version = VERSION.get_or_init(|| {
        let mut h = Sha256::new(); h.update(include_bytes!("value_views.rs")); h.update(include_bytes!("xml_views.rs")); cache::hex(&h.finalize())
    });
    cache::sha256(format!("values-v9/{version}/{sheet_hash}/{sst_hash}").as_bytes())
}
fn add_coordinate(hash: &mut Sha256, coordinate: Coordinate) {
    hash.update(coordinate.row.to_le_bytes()); hash.update(coordinate.column.to_le_bytes());
}
fn summarize(bytes: &[u8], sst: &SharedStrings, name: &str, index: usize, key: String) -> Result<Summary> {
    let _phase = super::super::visual_diagnostics::phase("values.cells_and_bounds", name.to_owned());
    let mut cells = Cells::new(bytes, sst)?;
    let mut values = Sha256::new(); values.update(b"typed-values-v9\0");
    let mut formulas = Sha256::new(); formulas.update(b"formula-preservation-v9\0");
    let mut value_count = 0; let mut formula_count = 0;
    let mut value_extent: Option<Dimension> = None;
    while let Some(cell) = cells.next()? {
        if !cell.value.is_empty() || cell.formula.is_some() {
            match &mut value_extent {
                Some(v) => v.include(cell.coordinate),
                None => value_extent = Some(Dimension { first: cell.coordinate, last: cell.coordinate }),
            }
        }
        if !cell.value.is_empty() { add_coordinate(&mut values, cell.coordinate); cell.value.hash_into(&mut values)?; value_count += 1; }
        if let Some(formula) = cell.formula {
            add_coordinate(&mut formulas, cell.coordinate); formulas.update((formula.len() as u64).to_le_bytes()); formulas.update(formula); formula_count += 1;
        }
    }
    Ok(Summary { key, values_sha256: cache::hex(&values.finalize()), formulas_sha256: cache::hex(&formulas.finalize()),
        ordered: cells.ordered, value_count, formula_count, dimension: cells.dimension, value_extent, columns: cells.capture_columns.clone(), bounds: cells.bounds.report(index, name) })
}

impl Summary {
    fn visual_dimension_bounds(&self, path: &Path) -> Result<WorksheetCaptureRange> {
        let name = &self.bounds.sheet_file;
        let d = self.dimension.with_context(|| format!("saved workbook {} sheet {name}: dimension missing", path.display()))?;
        if let Some(v) = self.value_extent {
            ensure!(d.contains(v), "saved workbook {} sheet {name}: dimension {}:{} does not contain values {}:{}",
                path.display(), d.first.name(), d.last.name(), v.first.name(), v.last.name());
            if d.excessive(v) {
                tracing::warn!("[dimension] workbook={} sheet={name} dimension={}:{} values={}:{}: overhang exceeds value-range size; check shrink cleanup",
                    path.display(), d.first.name(), d.last.name(), v.first.name(), v.last.name());
            }
        } else {
            tracing::info!("[dimension] workbook={} sheet={name}: no values/formulas; using dimension {}:{}", path.display(), d.first.name(), d.last.name());
        }
        let mut b = self.bounds.clone();
        if d.last.row > b.max_row { b.max_row = d.last.row; b.max_row_source = Some("dimension".into()); }
        if d.last.column > b.max_column { b.max_column = d.last.column; b.max_column_source = Some("dimension".into()); }
        b.max_cell = Coordinate { row: b.max_row, column: b.max_column }.name();
        tracing::info!("[dimension] workbook={} sheet={name} dimension={}:{} capture={}", path.display(), d.first.name(), d.last.name(), b.max_cell);
        Ok(b)
    }
}

struct Workbook {
    zip: ZipArchive<File>, manifest: cache::PackageManifest, order: Vec<String>, sst: Option<SharedStrings>, path: PathBuf,
}
impl Workbook {
    fn open(path: &Path) -> Result<Self> {
        let manifest = cache::manifest(path)?;
        let mut zip = ZipArchive::new(File::open(path)?)?;
        let order = read_order(&mut zip, false)?;
        Ok(Self { zip, manifest, order, sst: None, path: path.to_owned() })
    }
    fn ensure_sst(&mut self) -> Result<()> {
        if self.sst.is_none() {
            self.sst = Some(SharedStrings::new(read_optional(&mut self.zip, "xl/sharedStrings.xml")?.unwrap_or_default())?);
        }
        Ok(())
    }
    fn read_sheet(&mut self, name: &str, buffer: &mut Vec<u8>) -> Result<()> {
        let _phase = super::super::visual_diagnostics::phase("values.inflate_sheet", name.to_owned());
        buffer.clear(); self.zip.by_name(name)?.read_to_end(buffer)?;
        let expected = self.manifest.parts.get(name).context("worksheet missing from manifest")?;
        ensure!(cache::sha256(buffer) == expected.sha256, "worksheet changed after package manifest was read");
        Ok(())
    }
    fn summary(&mut self, name: &str, buffer: &mut Vec<u8>, root: &Path, use_cache: bool) -> Result<Summary> {
        let sst_hash = self.manifest.parts.get("xl/sharedStrings.xml").map(|p| p.sha256.as_str()).unwrap_or("absent");
        let key = summary_key(&self.manifest.parts.get(name).context("missing sheet")?.sha256, sst_hash);
        let file = root.join("values").join(format!("{key}.json"));
        let index = self.order.iter().position(|n| n == name).context("worksheet absent from workbook order")? + 1;
        if use_cache {
            if let Some(mut saved) = cache::read_record::<Summary>(&file) {
                if saved.key == key {
                    saved.bounds.sheet_index = index; saved.bounds.sheet_file = name.to_owned();
                    tracing::debug!("[values] digest cache hit sheet={name}");
                    return Ok(saved);
                }
            }
        }
        self.ensure_sst()?; self.read_sheet(name, buffer)?;
        let summary = summarize(buffer, self.sst.as_ref().unwrap(), name, index, key)?;
        if use_cache { cache::write_record(&file, &summary)?; }
        Ok(summary)
    }
    fn check_unchanged(&self) -> Result<()> {
        ensure!(cache::hash_file(&self.path)? == self.manifest.archive_sha256, "workbook changed during value validation"); Ok(())
    }
}

fn read_optional(zip: &mut ZipArchive<File>, name: &str) -> Result<Option<Vec<u8>>> {
    match zip.by_name(name) {
        Ok(mut file) => { let mut bytes = Vec::new(); file.read_to_end(&mut bytes)?; Ok(Some(bytes)) },
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn decode_attr(raw: &[u8]) -> Result<String> { TextView { raw, runs: false }.report() }
fn target_name(target: &str) -> Result<String> {
    let raw = if target.starts_with('/') { target.trim_start_matches('/').to_owned() } else { format!("xl/{target}") };
    let mut parts = Vec::new();
    for part in raw.split('/') {
        match part { "" | "." => {}, ".." => { ensure!(parts.pop().is_some(), "relationship path escapes ZIP"); },
            _ => { ensure!(!part.contains('\\'), "backslash in OPC relationship"); parts.push(part); } }
    }
    Ok(parts.join("/"))
}
fn read_order(zip: &mut ZipArchive<File>, visual: bool) -> Result<Vec<String>> {
    let wb = read_optional(zip, "xl/workbook.xml")?;
    let rels = read_optional(zip, "xl/_rels/workbook.xml.rels")?;
    let mut order = Vec::new(); let mut charts = Vec::new();
    if let (Some(wb), Some(rels)) = (wb, rels) {
        let mut reader = Reader::<128>::new(&rels)?; let mut targets = HashMap::new();
        while let Some(tag) = reader.next()? {
            if tag.local() == b"Relationship" && tag.kind != Kind::End {
                if tag.attr(b"TargetMode")? == Some(b"External".as_slice()) { continue; }
                let id = decode_attr(tag.attr(b"Id")?.context("relationship Id missing")?)?;
                let target = target_name(&decode_attr(tag.attr(b"Target")?.context("relationship Target missing")?)?)?;
                ensure!(targets.insert(id, target).is_none(), "duplicate relationship Id");
            }
        }
        let mut reader = Reader::<128>::new(&wb)?;
        while let Some(tag) = reader.next()? {
            if tag.local() == b"sheet" && tag.kind != Kind::End {
                let id = decode_attr(tag.attr(b"r:id")?.context("sheet relationship Id missing")?)?;
                let target = targets.get(&id).context("unresolved sheet relationship")?;
                if cache::is_sheet(target) { order.push(target.clone()); }
                else if target.starts_with("xl/chartsheets/") && visual { charts.push(target.clone()); }
            }
        }
    }
    let mut listed = BTreeSet::new();
    for name in &order { ensure!(listed.insert(name.clone()), "worksheet listed twice in workbook"); }
    let mut remaining = Vec::new();
    for i in 0..zip.len() {
        let file = zip.by_index(i)?; let name = file.name();
        if cache::is_sheet(name) && !listed.contains(name) { remaining.push(name.to_owned()); }
    }
    remaining.sort_by_key(|n| (n.strip_prefix("xl/worksheets/sheet").and_then(|n| n.strip_suffix(".xml")).and_then(|n| n.parse::<u32>().ok()).unwrap_or(u32::MAX), n.clone()));
    order.extend(remaining); order.extend(charts);
    Ok(order)
}
pub fn workbook_visual_sheet_order(path: &Path) -> Result<Vec<String>> { read_order(&mut ZipArchive::new(File::open(path)?)?, true) }

/// Reuse the summaries produced by the value pass, including column XML metadata.
pub fn workbook_capture_columns(path: &Path) -> Result<std::collections::BTreeMap<usize, CaptureColumns>> {
    let _phase = super::super::visual_diagnostics::phase("values.capture_columns", path.display().to_string());
    let mut book = Workbook::open(path)?;
    let root = cache::root()?;
    let use_cache = !std::env::var("EXCEL_SHRINK_VALUE_CACHE").is_ok_and(|v| v == "0");
    let mut buffer = Vec::new(); let mut columns = std::collections::BTreeMap::new();
    for name in book.order.clone() {
        let summary = book.summary(&name, &mut buffer, &root, use_cache)?;
        columns.insert(summary.bounds.sheet_index, summary.columns);
    }
    book.check_unchanged()?;
    Ok(columns)
}

pub fn compare_workbook_values(left: &Path, right: &Path, policy: MissingCellPolicy, only: Option<&HashSet<String>>) -> Result<WorkbookComparisonReport> {
    compare(left, right, policy, only, false, false)
}
pub fn compare_workbook_preservation(left: &Path, right: &Path, policy: MissingCellPolicy, only: Option<&HashSet<String>>) -> Result<WorkbookComparisonReport> {
    compare(left, right, policy, only, true, false)
}
pub fn compare_saved_visual_values(left: &Path, right: &Path) -> Result<WorkbookComparisonReport> {
    compare(left, right, MissingCellPolicy::strict(), None, false, true)
}
fn compare(left: &Path, right: &Path, policy: MissingCellPolicy, only: Option<&HashSet<String>>, formulas: bool, saved_dimensions: bool) -> Result<WorkbookComparisonReport> {
    let _phase = super::super::visual_diagnostics::phase("values.views_compare", format!("{} <-> {}", left.display(), right.display()));
    let mut a = Workbook::open(left)?; let mut b = Workbook::open(right)?;
    let names: BTreeSet<_> = a.order.iter().chain(&b.order).filter(|n| only.is_none_or(|set| set.contains(*n))).cloned().collect();
    let root = cache::root()?;
    let use_cache = !std::env::var("EXCEL_SHRINK_VALUE_CACHE").is_ok_and(|v| v == "0");
    let verify = std::env::var("EXCEL_SHRINK_VALUE_VERIFY").is_ok_and(|v| v == "1");
    let mut report = WorkbookComparisonReport { compared_sheets: names.len(), difference_count: 0, differences: Vec::new(), right_capture_ranges: Vec::new(), left_capture_ranges: Vec::new() };
    let mut left_buffer = Vec::new(); let mut right_buffer = Vec::new();
    for name in names {
        let sa = if a.manifest.parts.contains_key(&name) { Some(a.summary(&name, &mut left_buffer, &root, use_cache)?) } else { None };
        let sb = if b.manifest.parts.contains_key(&name) { Some(b.summary(&name, &mut right_buffer, &root, use_cache)?) } else { None };
        if let Some(s) = &sa { report.left_capture_ranges.push(s.bounds.clone()); }
        if let Some(s) = &sb { report.right_capture_ranges.push(if saved_dimensions { s.visual_dimension_bounds(right)? } else { s.bounds.clone() }); }
        let hashes_equal = match (&sa, &sb) {
            (Some(x), Some(y)) => x.ordered && y.ordered && x.values_sha256 == y.values_sha256 && x.value_count == y.value_count
                && (!formulas || (x.formulas_sha256 == y.formulas_sha256 && x.formula_count == y.formula_count)),
            _ => false,
        };
        if hashes_equal && !verify { tracing::debug!("[values] equal digests sheet={name}; detailed scan skipped"); continue; }
        a.ensure_sst()?; b.ensure_sst()?;
        if sa.is_some() { a.read_sheet(&name, &mut left_buffer)?; } else { left_buffer = b"<worksheet><sheetData/></worksheet>".to_vec(); }
        if sb.is_some() { b.read_sheet(&name, &mut right_buffer)?; } else { right_buffer = b"<worksheet><sheetData/></worksheet>".to_vec(); }
        let before = report.differences.len();
        let unordered = sa.as_ref().is_some_and(|s| !s.ordered) || sb.as_ref().is_some_and(|s| !s.ordered);
        compare_sheet_views(&name, &left_buffer, &right_buffer, a.sst.as_ref().unwrap(), b.sst.as_ref().unwrap(), policy, formulas, unordered, &mut report.differences)?;
        if sa.is_none() || sb.is_none() {
            // Even an empty missing sheet is a structural difference.
            report.differences.push(CellValueDifference { sheet_file: name.clone(), cell_ref: "<worksheet>".into(),
                kind: if sa.is_none() { CellDifferenceKind::ExtraRight } else { CellDifferenceKind::MissingRight },
                left_value: sa.as_ref().map(|_| "worksheet present".into()), right_value: sb.as_ref().map(|_| "worksheet present".into()) });
        }
        ensure!(!hashes_equal || report.differences.len() == before, "digest verification disagrees with byte-view comparator for {name}");
    }
    a.check_unchanged()?; b.check_unchanged()?;
    report.differences.sort_by(|a, b| (&a.sheet_file, &a.cell_ref).cmp(&(&b.sheet_file, &b.cell_ref)));
    report.difference_count = report.differences.len();
    report.left_capture_ranges.sort_by_key(|r| r.sheet_index); report.right_capture_ranges.sort_by_key(|r| r.sheet_index);
    Ok(report)
}

fn missing_allowed(cell: CellView<'_>, policy: MissingCellPolicy) -> Result<bool> {
    Ok(cell.value.is_empty() || (policy.allow_whitespace_values && cell.value.is_whitespace()?)
        || (policy.allow_hidden_rows_or_columns && cell.hidden))
}
fn difference(name: &str, coordinate: Coordinate, left: Option<Value<'_>>, right: Option<Value<'_>>) -> Result<CellValueDifference> {
    Ok(CellValueDifference { sheet_file: name.to_owned(), cell_ref: coordinate.name(),
        kind: match (left, right) { (Some(_), None) => CellDifferenceKind::MissingRight, (None, Some(_)) => CellDifferenceKind::ExtraRight, _ => CellDifferenceKind::Changed },
        left_value: left.map(Value::report).transpose()?, right_value: right.map(Value::report).transpose()? })
}
fn compare_cell(name: &str, a: Option<CellView<'_>>, b: Option<CellView<'_>>, policy: MissingCellPolicy, formulas: bool, out: &mut Vec<CellValueDifference>) -> Result<()> {
    let coordinate = a.or(b).unwrap().coordinate;
    let av = a.map(|c| c.value).unwrap_or(Value::empty()); let bv = b.map(|c| c.value).unwrap_or(Value::empty());
    if !av.equal(bv)? {
        if bv.is_empty() { if let Some(cell) = a { if !missing_allowed(cell, policy)? { out.push(difference(name, coordinate, Some(av), None)?); } } }
        else { out.push(difference(name, coordinate, if av.is_empty() { None } else { Some(av) }, Some(bv))?); }
    }
    if formulas {
        let af = a.and_then(|c| c.formula); let bf = b.and_then(|c| c.formula);
        // Formula preservation is strict even if the cached value is unchanged.
        let allowed_removal = af.is_some() && bf.is_none() && bv.is_empty() && match a {
            Some(cell) => (policy.allow_hidden_rows_or_columns && cell.hidden)
                || (policy.allow_whitespace_values && !cell.value.is_empty() && cell.value.is_whitespace()?),
            None => false,
        };
        if af != bf && !allowed_removal {
            out.push(CellValueDifference { sheet_file: name.to_owned(), cell_ref: coordinate.name(), kind: CellDifferenceKind::Changed,
                left_value: af.map(|s| format!("formula:{}", String::from_utf8_lossy(s))), right_value: bf.map(|s| format!("formula:{}", String::from_utf8_lossy(s))) });
        }
    }
    Ok(())
}

pub(super) fn compare_sheet_views(name: &str, left: &[u8], right: &[u8], left_sst: &SharedStrings, right_sst: &SharedStrings,
    policy: MissingCellPolicy, formulas: bool, unordered: bool, differences: &mut Vec<CellValueDifference>) -> Result<()> {
    let mut a = Cells::new(left, left_sst)?; let mut b = Cells::new(right, right_sst)?;
    if unordered {
        // Rare non-row-major input: one contiguous vector of borrowed views, no
        // cloned cell addresses or values. Reject duplicate addresses explicitly.
        let mut aa = Vec::new(); let mut bb = Vec::new();
        while let Some(c) = a.next()? { aa.push(c); } while let Some(c) = b.next()? { bb.push(c); }
        aa.sort_unstable_by_key(|c| c.coordinate); bb.sort_unstable_by_key(|c| c.coordinate);
        ensure!(aa.windows(2).all(|p| p[0].coordinate != p[1].coordinate) && bb.windows(2).all(|p| p[0].coordinate != p[1].coordinate), "duplicate cell addresses");
        let mut ai = aa.into_iter().peekable(); let mut bi = bb.into_iter().peekable();
        while ai.peek().is_some() || bi.peek().is_some() {
            let take_a = bi.peek().is_none() || ai.peek().zip(bi.peek()).is_some_and(|(a,b)| a.coordinate <= b.coordinate);
            let take_b = ai.peek().is_none() || ai.peek().zip(bi.peek()).is_some_and(|(a,b)| b.coordinate <= a.coordinate);
            compare_cell(name, if take_a { ai.next() } else { None }, if take_b { bi.next() } else { None }, policy, formulas, differences)?;
        }
    } else {
        let mut av = a.next()?; let mut bv = b.next()?;
        while av.is_some() || bv.is_some() {
            let take_a = bv.is_none() || av.zip(bv).is_some_and(|(a,b)| a.coordinate <= b.coordinate);
            let take_b = av.is_none() || av.zip(bv).is_some_and(|(a,b)| b.coordinate <= a.coordinate);
            compare_cell(name, if take_a { av } else { None }, if take_b { bv } else { None }, policy, formulas, differences)?;
            if take_a { av = a.next()?; } if take_b { bv = b.next()?; }
        }
        ensure!(a.ordered && b.ordered, "row-major fast path received unordered cells");
    }
    Ok(())
}

#[cfg(test)]
mod dimension_tests {
    use super::*;
    fn sample(reference: &str) -> Summary {
        let bytes = format!("<worksheet><dimension ref=\"{reference}\"/><sheetData><row r=\"3\"><c r=\"B3\"><v>7</v></c></row></sheetData></worksheet>");
        summarize(bytes.as_bytes(), &SharedStrings::new(Vec::new()).unwrap(), "sheet1.xml", 1, "test".into()).unwrap()
    }
    #[test]
    fn dimension_equal_and_larger_extend_capture() {
        let equal = sample("A1:B3").visual_dimension_bounds(Path::new("saved.xlsx")).unwrap();
        assert_eq!((equal.max_row, equal.max_column), (3,2));
        let larger = sample("A1:C22").visual_dimension_bounds(Path::new("saved.xlsx")).unwrap();
        assert_eq!((larger.max_row, larger.max_column), (22,3));
        assert_eq!(larger.max_row_source.as_deref(), Some("dimension"));
    }
    #[test]
    fn dimension_must_contain_value_rectangle_on_both_axes() {
        for d in ["A1:A3", "A1:B2", "C1:D5", "A4:D5"] {
            assert!(sample(d).visual_dimension_bounds(Path::new("saved.xlsx")).is_err(), "{d}");
        }
    }
    #[test]
    fn dimension_warning_uses_extent_not_absolute_coordinate() {
        let v = Dimension::parse(b"B100:C109").unwrap();
        assert!(!Dimension::parse(b"A1:E119").unwrap().excessive(v));
        assert!(Dimension::parse(b"A1:E120").unwrap().excessive(v));
        assert!(Dimension::parse(b"A1:F119").unwrap().excessive(v));
        assert!(Dimension::parse(b"C5:B1").is_err());
        assert!(Dimension::parse(b"A1:B2:C3").is_err());
    }
    #[test]
    fn dimension_missing_fails_visual_validation_but_not_value_summary() {
        let s = summarize(b"<worksheet><sheetData/></worksheet>", &SharedStrings::new(Vec::new()).unwrap(), "sheet", 1, "test".into()).unwrap();
        assert!(s.visual_dimension_bounds(Path::new("saved.xlsx")).is_err());
        let empty = summarize(b"<worksheet><dimension ref='A1:C22'/><sheetData/></worksheet>", &SharedStrings::new(Vec::new()).unwrap(), "sheet", 1, "test".into()).unwrap();
        assert_eq!(empty.visual_dimension_bounds(Path::new("saved.xlsx")).unwrap().max_row, 22);
    }
}
