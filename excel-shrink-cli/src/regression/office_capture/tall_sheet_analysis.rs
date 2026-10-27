//! Conservative row indicators for abbreviated, unusually tall worksheets.
//! XML buffers are borrowed, one sheet pair at a time; no per-cell maps or DOM.
use crate::regression::xml_views::{decimal, Attributes, Kind, Reader, TextBytes};
use anyhow::{bail, ensure, Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use zip::ZipArchive;

const MAX_ROWS: u32 = 1_048_576;
const MAX_XML_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Default)]
pub(super) struct SheetIndicators {
    /// Zero-based, half-open rows; never clipped to populated cells.
    pub rows: Vec<[u32; 2]>,
    pub reasons: Vec<String>,
    pub full_capture_reason: Option<String>,
}

impl SheetIndicators {
    fn add(&mut self, rows: [u32; 2], reason: &str) {
        // Row cursors normally visit adjacent rows. Compact those immediately
        // instead of allocating a million entries for a uniformly styled sheet.
        if let Some(last) = self
            .rows
            .last_mut()
            .filter(|last| rows[0] <= last[1] && rows[1] >= last[0])
        {
            last[0] = last[0].min(rows[0]);
            last[1] = last[1].max(rows[1]);
        } else {
            self.rows.push(rows);
        }
        if !self.reasons.iter().any(|r| r == reason) {
            self.reasons.push(reason.into());
        }
    }
    fn full(&mut self, reason: impl Into<String>) {
        self.full_capture_reason
            .get_or_insert_with(|| reason.into());
    }
    fn merge(&mut self) {
        self.rows.sort_unstable();
        let mut result: Vec<[u32; 2]> = Vec::new();
        for range in self.rows.drain(..) {
            if let Some(last) = result.last_mut().filter(|last| range[0] <= last[1]) {
                last[1] = last[1].max(range[1]);
            } else {
                result.push(range);
            }
        }
        self.rows = result;
    }
}

fn decode(raw: &[u8]) -> Result<String> {
    Ok(String::from_utf8(
        TextBytes::new(raw).collect::<Result<Vec<_>>>()?,
    )?)
}

fn read_part(zip: &mut ZipArchive<File>, name: &str) -> Result<Option<Vec<u8>>> {
    let mut part = match zip.by_name(name) {
        Ok(part) => part,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        part.size() <= MAX_XML_BYTES,
        "XML analysis memory limit exceeded for {name}"
    );
    let mut bytes = Vec::with_capacity(part.size() as usize);
    part.by_ref()
        .take(MAX_XML_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_XML_BYTES,
        "XML analysis memory limit exceeded for {name}"
    );
    Ok(Some(bytes))
}

fn required_part(zip: &mut ZipArchive<File>, name: &str) -> Result<Vec<u8>> {
    read_part(zip, name)?.with_context(|| format!("missing OOXML part {name}"))
}

fn resolve(source: &str, target: &str) -> Result<String> {
    ensure!(
        !target.contains('\\') && !target.contains('#'),
        "unsupported relationship target"
    );
    let raw = if target.starts_with('/') {
        target.to_owned()
    } else {
        format!("{}/{}", source.rsplit_once('/').map_or("", |p| p.0), target)
    };
    let mut parts = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                ensure!(parts.pop().is_some(), "relationship escapes package");
            }
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

#[derive(Debug)]
struct Relationship {
    id: String,
    target: String,
    kind: String,
    external: bool,
}

fn relationships(bytes: &[u8]) -> Result<Vec<Relationship>> {
    let mut result = Vec::new();
    let mut reader = Reader::<64>::new(bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"Relationship" {
            continue;
        }
        result.push(Relationship {
            id: decode(tag.attr(b"Id")?.context("relationship Id missing")?)?,
            target: decode(
                tag.attr(b"Target")?
                    .context("relationship target missing")?,
            )?,
            kind: decode(tag.attr(b"Type")?.context("relationship type missing")?)?,
            external: tag
                .attr(b"TargetMode")?
                .is_some_and(|v| v.eq_ignore_ascii_case(b"External")),
        });
    }
    Ok(result)
}

fn descriptors(zip: &mut ZipArchive<File>) -> Result<BTreeMap<usize, (String, String)>> {
    let workbook = required_part(zip, "xl/workbook.xml")?;
    let rels = relationships(&required_part(zip, "xl/_rels/workbook.xml.rels")?)?;
    let mut result = BTreeMap::new();
    let mut reader = Reader::<64>::new(&workbook)?;
    let mut index = 0;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"sheet" {
            continue;
        }
        index += 1;
        let name = decode(tag.attr(b"name")?.context("sheet name missing")?)?;
        let id = Attributes::new(tag.attrs)
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .find(|(name, _)| crate::regression::xml_views::local_name(name) == b"id")
            .map(|(_, value)| decode(value))
            .transpose()?
            .context("sheet relationship missing")?;
        let rel = rels
            .iter()
            .find(|rel| rel.id == id && !rel.external)
            .context("sheet target missing")?;
        ensure!(
            rel.kind.ends_with("/worksheet"),
            "non-worksheet workbook member"
        );
        result.insert(index, (name, resolve("xl/workbook.xml", &rel.target)?));
    }
    Ok(result)
}

/// Stable workbook indexes without decompressing any worksheet body.
pub(super) fn sheet_paths(path: &Path) -> Result<BTreeMap<usize, String>> {
    let mut zip = ZipArchive::new(File::open(path)?)?;
    Ok(descriptors(&mut zip)?
        .into_iter()
        .map(|(index, (_, file))| (index, file))
        .collect())
}

/// Number/date formats alone are normal in flat data. Decorations are not.
fn decorated_styles(bytes: Option<&[u8]>) -> Result<Vec<bool>> {
    let Some(bytes) = bytes else {
        return Ok(vec![false]);
    };
    let decorated_default = default_resources_decorated(bytes)?;
    let plain_fonts = plain_font_resources(bytes)?;
    let mut reader = Reader::<64>::new(bytes)?;
    let mut styles = Vec::new();
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"cellXfs" {
            continue;
        }
        let element = reader.element(tag)?;
        let mut xfs = Reader::<64>::new(element.body)?;
        while let Some(xf) = xfs.next()? {
            if xf.kind == Kind::End || xf.local() != b"xf" {
                continue;
            }
            // A separate font record can merely change Calibri's point size.
            // It does not paint the empty cells of an entire column. Unknown
            // resources and visible font decorations remain conservative.
            let decorated_font = match xf.attr(b"fontId")? {
                None | Some(b"0") => false,
                Some(id) => !plain_fonts
                    .get(decimal(id)? as usize)
                    .copied()
                    .unwrap_or(false),
            };
            let decorated = [b"fillId".as_slice(), b"borderId", b"xfId"]
                .iter()
                .any(|attr| xf.attr(attr).map_or(true, |v| v.is_some_and(|v| v != b"0")));
            let body = xfs.element(xf)?;
            // Alignment can alter wrapping, text direction or rotation.
            let mut children = Reader::<64>::new(body.body)?;
            let mut alignment = false;
            while let Some(child) = children.next()? {
                alignment |= child.kind != Kind::End && child.local() == b"alignment";
            }
            styles.push(decorated_default || decorated_font || decorated || alignment);
        }
    }
    ensure!(!styles.is_empty(), "cell style table unavailable");
    Ok(styles)
}

fn plain_font_resources(bytes: &[u8]) -> Result<Vec<bool>> {
    let mut reader = Reader::<64>::new(bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"fonts" {
            continue;
        }
        let collection = reader.element(tag)?;
        let mut fonts = Reader::<64>::new(collection.body)?;
        let mut plain = Vec::new();
        while let Some(font) = fonts.next()? {
            if font.kind == Kind::End || font.local() != b"font" {
                continue;
            }
            let font = fonts.element(font)?;
            let mut fields = Reader::<64>::new(font.body)?;
            let mut regular = true;
            while let Some(field) = fields.next()? {
                if field.kind == Kind::End {
                    continue;
                }
                regular &= match field.local() {
                    b"name" | b"sz" | b"family" | b"charset" | b"scheme" => true,
                    b"color" => !color_decorated(field.attrs)?,
                    b"b" | b"i" | b"u" | b"strike" | b"outline" | b"shadow" | b"vertAlign" => field
                        .attr(b"val")?
                        .is_some_and(|v| matches!(v, b"0" | b"false" | b"none" | b"baseline")),
                    _ => false,
                };
            }
            plain.push(regular);
        }
        return Ok(plain);
    }
    Ok(Vec::new())
}

fn color_decorated(attrs: &[u8]) -> Result<bool> {
    for attr in Attributes::new(attrs) {
        let (name, value) = attr?;
        let plain = match name {
            b"rgb" => value.eq_ignore_ascii_case(b"FF000000") || value == b"00000000",
            b"theme" => value == b"1",
            b"indexed" => matches!(value, b"8" | b"64"),
            b"auto" => matches!(value, b"1" | b"true"),
            b"tint" => value == b"0",
            _ => false,
        };
        if !plain {
            return Ok(true);
        }
    }
    Ok(false)
}

// A workbook can put bold text or a background in resource zero itself.
// Resource IDs alone must not certify that such a sheet is visually plain.
fn default_resources_decorated(bytes: &[u8]) -> Result<bool> {
    let mut reader = Reader::<64>::new(bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || !matches!(tag.local(), b"fonts" | b"fills" | b"borders") {
            continue;
        }
        let collection = reader.element(tag)?;
        let mut items = Reader::<64>::new(collection.body)?;
        let Some(first) = items.next()? else {
            continue;
        };
        let first = items.element(first)?;
        let mut fields = Reader::<64>::new(first.raw)?;
        while let Some(field) = fields.next()? {
            if field.kind == Kind::End {
                continue;
            }
            if matches!(
                field.local(),
                b"b" | b"i" | b"u" | b"strike" | b"outline" | b"shadow" | b"vertAlign"
            ) && !field
                .attr(b"val")?
                .is_some_and(|v| matches!(v, b"0" | b"false" | b"none" | b"baseline"))
            {
                return Ok(true);
            }
            if field.local() == b"patternFill"
                && field.attr(b"patternType")?.is_some_and(|v| v != b"none")
            {
                return Ok(true);
            }
            if field.local() == b"gradientFill" || field.attr(b"style")?.is_some() {
                return Ok(true);
            }
            if field.local() == b"color" && color_decorated(field.attrs)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn endpoint(raw: &str) -> Result<(Option<u32>, Option<u32>)> {
    let raw: String = raw.chars().filter(|c| *c != '$').collect();
    let split = raw
        .bytes()
        .position(|b| !b.is_ascii_alphabetic())
        .unwrap_or(raw.len());
    let (letters, digits) = raw.split_at(split);
    let mut column = 0u32;
    for byte in letters.bytes() {
        column = column
            .checked_mul(26)
            .and_then(|c| c.checked_add(u32::from(byte.to_ascii_uppercase() - b'A' + 1)))
            .context("column overflow")?;
    }
    ensure!(column <= 16_384, "column outside worksheet");
    let row = if digits.is_empty() {
        None
    } else {
        Some(decimal(digits.as_bytes())?)
    };
    ensure!(
        row.is_none_or(|r| (1..=MAX_ROWS).contains(&r)),
        "row outside worksheet"
    );
    ensure!(column > 0 || row.is_some(), "empty range endpoint");
    Ok((row, (column > 0).then_some(column)))
}

fn range_rows(raw: &[u8]) -> Result<Vec<[u32; 2]>> {
    let text = decode(raw)?;
    let mut result = Vec::new();
    for range in text.split_whitespace() {
        let (start, end) = range.split_once(':').unwrap_or((range, range));
        let a = endpoint(start)?;
        let b = endpoint(end)?;
        ensure!(
            a.0.is_some() == b.0.is_some() && a.1.is_some() == b.1.is_some(),
            "inconsistent range endpoints"
        );
        let rows = match (a.0, b.0) {
            (Some(a), Some(b)) => {
                ensure!(a <= b, "reversed row range");
                [a - 1, b]
            }
            (None, None) => [0, MAX_ROWS],
            _ => unreachable!(),
        };
        ensure!(a.1.zip(b.1).is_none_or(|(a, b)| a <= b), "reversed columns");
        result.push(rows);
    }
    ensure!(!result.is_empty(), "empty capture indicator range");
    Ok(result)
}

fn add_ranges(output: &mut SheetIndicators, raw: &[u8], reason: &str) -> Result<()> {
    for range in range_rows(raw)? {
        output.add(range, reason);
    }
    Ok(())
}

fn inspect_row(raw: &[u8], row: u32, styles: &[bool], out: &mut SheetIndicators) -> Result<()> {
    let mut reader = Reader::<32>::new(raw)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || !matches!(tag.local(), b"row" | b"c") {
            continue;
        }
        if let Some(style) = tag.attr(b"s")? {
            let index = decimal(style)? as usize;
            if *styles.get(index).context("unknown cell style index")? {
                out.add([row - 1, row], "decorated_cells_or_rows");
                return Ok(());
            }
        }
    }
    Ok(())
}

struct RowCursor<'a> {
    reader: Reader<'a>,
    styles: &'a [bool],
    last_row: u32,
    geometry: Vec<Vec<u8>>,
    drawing_ids: BTreeSet<String>,
}
impl<'a> RowCursor<'a> {
    fn new(bytes: &'a [u8], styles: &'a [bool]) -> Result<Self> {
        Ok(Self {
            reader: Reader::new(bytes)?,
            styles,
            last_row: 0,
            geometry: Vec::new(),
            drawing_ids: BTreeSet::new(),
        })
    }
    fn next(&mut self, out: &mut SheetIndicators) -> Result<Option<(u32, &'a [u8])>> {
        while let Some(tag) = self.reader.next()? {
            if tag.kind == Kind::End {
                continue;
            }
            match tag.local() {
                b"row" => {
                    let row = decimal(tag.attr(b"r")?.context("row index missing")?)?;
                    ensure!(
                        row > self.last_row && row <= MAX_ROWS,
                        "unordered or invalid worksheet row"
                    );
                    self.last_row = row;
                    let row_xml = self.reader.element(tag)?;
                    inspect_row(row_xml.raw, row, self.styles, out)?;
                    return Ok(Some((row, row_xml.raw)));
                }
                b"conditionalFormatting" => {
                    let body = self.reader.element(tag)?;
                    if let Some(sqref) = tag.attr(b"sqref")? {
                        add_ranges(out, sqref, "conditional_formatting")?;
                    } else {
                        let mut ranges = Reader::<64>::new(body.body)?;
                        let mut found = false;
                        while let Some(child) = ranges.next()? {
                            if child.kind != Kind::End && child.local() == b"sqref" {
                                add_ranges(
                                    out,
                                    ranges.element(child)?.body,
                                    "conditional_formatting",
                                )?;
                                found = true;
                            }
                        }
                        ensure!(found, "conditional formatting range missing");
                    }
                }
                b"mergeCell" => add_ranges(
                    out,
                    tag.attr(b"ref")?.context("merge reference missing")?,
                    "merged_cells",
                )?,
                b"col" => {
                    if let Some(style) = tag.attr(b"style")? {
                        if *self
                            .styles
                            .get(decimal(style)? as usize)
                            .context("unknown column style")?
                        {
                            out.add([0, MAX_ROWS], "decorated_columns");
                        }
                    }
                    self.geometry.push(self.reader.element(tag)?.raw.to_vec());
                }
                b"sheetFormatPr" | b"sheetPr" | b"sheetViews" => {
                    self.geometry.push(self.reader.element(tag)?.raw.to_vec())
                }
                b"drawing" => {
                    let mut found = false;
                    for attribute in Attributes::new(tag.attrs) {
                        let (name, value) = attribute?;
                        if crate::regression::xml_views::local_name(name) == b"id" {
                            self.drawing_ids.insert(decode(value)?);
                            found = true;
                        }
                    }
                    ensure!(found, "drawing relationship id missing");
                }
                b"legacyDrawing" | b"legacyDrawingHF" | b"picture" | b"oleObjects"
                | b"controls" | b"tableParts" | b"sparklineGroups" => {
                    out.full("unbounded worksheet visual object")
                }
                b"ext" => {
                    let uri = tag.attr(b"uri")?.unwrap_or_default();
                    // Standard x14 conditional-formatting extension. Other
                    // extensions may render information outside ordinary cells.
                    if uri != b"{78C0D931-6437-407D-A8EE-F0AAD7539E65}" {
                        out.full("unsupported worksheet extension");
                    }
                }
                // Other changed top-level settings are not assumed harmless.
                _ if tag.depth == 1
                    && !matches!(
                        tag.local(),
                        b"sheetData"
                            | b"cols"
                            | b"mergeCells"
                            | b"drawing"
                            | b"extLst"
                            | b"dimension"
                    ) =>
                {
                    self.geometry.push(self.reader.element(tag)?.raw.to_vec());
                }
                _ => {}
            }
        }
        Ok(None)
    }
}

fn scan_pair(
    left: &[u8],
    right: &[u8],
    left_styles: &[bool],
    right_styles: &[bool],
    out: &mut SheetIndicators,
) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    // Implicit style zero also paints cells/rows absent from sheetData. A
    // decorated workbook default cannot be bounded by explicit cell records.
    if left_styles.first() == Some(&true) || right_styles.first() == Some(&true) {
        out.add([0, MAX_ROWS], "decorated_default_style");
    }
    let mut a = RowCursor::new(left, left_styles)?;
    let mut b = RowCursor::new(right, right_styles)?;
    let mut ar = a.next(out)?;
    let mut br = b.next(out)?;
    while ar.is_some() || br.is_some() {
        match (ar, br) {
            (Some((ai, ax)), Some((bi, bx))) if ai == bi => {
                if ax != bx {
                    out.add([ai - 1, ai], "worksheet_xml_changed");
                }
                ar = a.next(out)?;
                br = b.next(out)?;
            }
            (Some((ai, _)), Some((bi, _))) if ai < bi => {
                out.add([ai - 1, ai], "worksheet_xml_changed");
                ar = a.next(out)?;
            }
            (Some((ai, _)), None) => {
                out.add([ai - 1, ai], "worksheet_xml_changed");
                ar = a.next(out)?;
            }
            (_, Some((bi, _))) => {
                out.add([bi - 1, bi], "worksheet_xml_changed");
                br = b.next(out)?;
            }
            _ => unreachable!(),
        }
    }
    if a.geometry != b.geometry {
        out.full("column or worksheet geometry changed");
    }
    Ok((a.drawing_ids, b.drawing_ids))
}

fn drawing_rows(bytes: &[u8], out: &mut SheetIndicators) -> Result<()> {
    let mut reader = Reader::<128>::new(bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() == b"wsDr" {
            continue;
        }
        if tag.local() != b"twoCellAnchor" {
            out.full("drawing extent cannot be bounded by cell anchors");
            break;
        }
        let anchor = reader.element(tag)?;
        if [
            b"rot=".as_slice(),
            b"effectLst",
            b"effectDag",
            b"AlternateContent",
        ]
        .iter()
        .any(|s| memchr::memmem::find(anchor.raw, s).is_some())
        {
            out.full("drawing effects extend beyond known anchors");
        }
        let mut markers = Reader::<64>::new(anchor.body)?;
        let mut endpoints = Vec::new();
        while let Some(marker) = markers.next()? {
            if marker.kind == Kind::End {
                continue;
            }
            if marker.depth == 0 && matches!(marker.local(), b"from" | b"to") {
                let body = markers.element(marker)?;
                let mut fields = Reader::<32>::new(body.body)?;
                while let Some(field) = fields.next()? {
                    if field.kind != Kind::End && field.local() == b"row" {
                        endpoints.push(decimal(fields.element(field)?.body)?);
                    }
                }
            } else if marker.depth == 0 {
                let _ = markers.element(marker)?;
            }
        }
        ensure!(
            endpoints.len() == 2 && endpoints.iter().all(|r| *r < MAX_ROWS),
            "invalid drawing anchor rows"
        );
        out.add(
            [
                endpoints[0].min(endpoints[1]).saturating_sub(1),
                endpoints[0]
                    .max(endpoints[1])
                    .saturating_add(2)
                    .min(MAX_ROWS),
            ],
            "drawing_or_chart",
        );
    }
    Ok(())
}

fn inspect_drawings(
    zip: &mut ZipArchive<File>,
    sheet: &str,
    expected: &BTreeSet<String>,
    out: &mut SheetIndicators,
) -> Result<()> {
    let (directory, name) = sheet.rsplit_once('/').context("invalid sheet path")?;
    let Some(bytes) = read_part(zip, &format!("{directory}/_rels/{name}.rels"))? else {
        ensure!(
            expected.is_empty(),
            "worksheet drawing has no relationships part"
        );
        return Ok(());
    };
    let rels = relationships(&bytes)?;
    ensure!(
        expected.iter().all(|id| rels
            .iter()
            .any(|rel| &rel.id == id && rel.kind.ends_with("/drawing"))),
        "unresolved worksheet drawing relationship"
    );
    for rel in rels {
        match rel.kind.rsplit('/').next().unwrap_or("") {
            "drawing" if !rel.external => {
                drawing_rows(&required_part(zip, &resolve(sheet, &rel.target)?)?, out)?
            }
            "drawing" | "vmlDrawing" | "oleObject" | "control" => {
                out.full("unsupported or external drawing")
            }
            _ => {}
        }
    }
    Ok(())
}

/// Only requested giant sheets are scanned; workbook indexes match Office plans.
/// Failures are propagated so callers can explicitly fall back to full capture.
pub(super) fn analyze(
    original: &Path,
    variants: &[PathBuf],
    wanted: &BTreeSet<usize>,
) -> Result<BTreeMap<usize, SheetIndicators>> {
    let mut baseline = ZipArchive::new(File::open(original)?)?;
    let sheets = descriptors(&mut baseline)?;
    let styles = read_part(&mut baseline, "xl/styles.xml")?;
    let original_styles = decorated_styles(styles.as_deref())?;
    let strings = read_part(&mut baseline, "xl/sharedStrings.xml")?;
    let mut result: BTreeMap<_, _> = wanted
        .iter()
        .map(|&index| (index, SheetIndicators::default()))
        .collect();
    if original_styles.first() == Some(&true) {
        for out in result.values_mut() {
            out.full("default cell appearance is decorated");
        }
        return Ok(result);
    }
    for path in variants {
        let mut variant = ZipArchive::new(File::open(path)?)?;
        let other_sheets = descriptors(&mut variant)?;
        let other_styles = read_part(&mut variant, "xl/styles.xml")?;
        let other_strings = read_part(&mut variant, "xl/sharedStrings.xml")?;
        if styles != other_styles || strings != other_strings || sheets != other_sheets {
            for out in result.values_mut() {
                out.full("styles, shared strings or sheet mapping changed");
            }
            continue;
        }
        for (&index, out) in &mut result {
            if out.full_capture_reason.is_some() {
                continue;
            }
            let file = &sheets.get(&index).context("requested sheet missing")?.1;
            let scanned = (|| -> Result<()> {
                let left = required_part(&mut baseline, file)?;
                let right = required_part(&mut variant, file)?;
                let (left_drawings, right_drawings) =
                    scan_pair(&left, &right, &original_styles, &original_styles, out)?;
                inspect_drawings(&mut baseline, file, &left_drawings, out)?;
                inspect_drawings(&mut variant, file, &right_drawings, out)?;
                Ok(())
            })();
            if let Err(error) = scanned {
                out.full(format!("XML indicators unavailable: {error:#}"));
            }
            out.merge();
        }
    }
    if variants.is_empty() {
        bail!("capture indicators require at least one comparison workbook");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additions_deletions_and_changes_keep_original_rows() {
        let a = br#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="100"><c r="A100"><v>2</v></c></row></sheetData></worksheet>"#;
        let b = br#"<worksheet><sheetData><row r="1"><c r="A1"><v>3</v></c></row><row r="200"/></sheetData></worksheet>"#;
        let mut out = SheetIndicators::default();
        scan_pair(a, b, &[false], &[false], &mut out).unwrap();
        out.merge();
        assert_eq!(out.rows, vec![[0, 1], [99, 100], [199, 200]]);
        assert!(out.full_capture_reason.is_none());
    }

    #[test]
    fn cf_targets_include_blank_cells_and_x14_ranges() {
        let xml = br#"<worksheet><sheetData/><conditionalFormatting sqref="B2:B9 D20:F30"/><extLst><ext uri="{78C0D931-6437-407D-A8EE-F0AAD7539E65}"><x:conditionalFormattings><x:conditionalFormatting><m:sqref>$C$900:$D$990</m:sqref></x:conditionalFormatting></x:conditionalFormattings></ext></extLst></worksheet>"#;
        let mut out = SheetIndicators::default();
        scan_pair(xml, xml, &[false], &[false], &mut out).unwrap();
        out.merge();
        assert_eq!(out.rows, vec![[1, 9], [19, 30], [899, 990]]);
        assert!(out.full_capture_reason.is_none());
        assert_eq!(range_rows(b"B:D").unwrap(), vec![[0, MAX_ROWS]]);
        assert_eq!(range_rows(b"20:40").unwrap(), vec![[19, 40]]);
    }

    #[test]
    fn numeric_formats_are_flat_but_fills_and_fonts_are_indicators() {
        let xml = br#"<styleSheet><cellXfs><xf numFmtId="0"/><xf numFmtId="14"/><xf fillId="2"/><xf fontId="1"/><xf><alignment wrapText="1"/></xf></cellXfs></styleSheet>"#;
        let styles = decorated_styles(Some(xml)).unwrap();
        assert_eq!(styles, vec![false, false, true, true, true]);
        let sheet = br#"<worksheet><sheetData><row r="10"><c r="A10" s="1"/></row><row r="20"><c r="B20" s="2"/></row></sheetData></worksheet>"#;
        let mut out = SheetIndicators::default();
        scan_pair(sheet, sheet, &styles, &styles, &mut out).unwrap();
        out.merge();
        assert_eq!(out.rows, vec![[19, 20]]);
        let decorated_default = br#"<styleSheet><fonts><font><b/><name val="Arial"/></font></fonts><cellXfs><xf fontId="0"/></cellXfs></styleSheet>"#;
        assert_eq!(
            decorated_styles(Some(decorated_default)).unwrap(),
            vec![true]
        );
    }

    #[test]
    fn regular_font_columns_do_not_protect_a_million_empty_rows() {
        let xml = br#"<styleSheet><fonts>
            <font><sz val="11"/><name val="Calibri"/><family val="2"/></font>
            <font><sz val="12"/><name val="Calibri"/><family val="2"/></font>
            </fonts><cellXfs><xf/><xf fontId="1" fillId="0" borderId="0" xfId="0"/>
            </cellXfs></styleSheet>"#;
        let styles = decorated_styles(Some(xml)).unwrap();
        assert_eq!(styles, vec![false, false]);
        // Trial3 has ordinary data near the top and a SUM close to the last
        // worksheet row. Its regular column font is not a painted background.
        let sheet = br#"<worksheet><cols><col min="1" max="16384" style="1"/></cols>
            <sheetData><row r="3"><c r="N3" s="1"><v>12</v></c></row>
            <row r="1048574"><c r="N1048574" s="1"><f>SUM(N3:N1048573)</f><v>12</v></c></row>
            </sheetData></worksheet>"#;
        let mut out = SheetIndicators::default();
        scan_pair(sheet, sheet, &styles, &styles, &mut out).unwrap();
        assert!(out.rows.is_empty());
        assert!(out.full_capture_reason.is_none());

        let changed = String::from_utf8(sheet.to_vec())
            .unwrap()
            .replace("<v>12</v>", "<v>13</v>");
        let mut out = SheetIndicators::default();
        scan_pair(sheet, changed.as_bytes(), &styles, &styles, &mut out).unwrap();
        out.merge();
        assert_eq!(out.rows, vec![[2, 3], [1_048_573, 1_048_574]]);
    }

    #[test]
    fn decorated_and_unknown_font_resources_remain_capture_indicators() {
        let xml = br#"<styleSheet><fonts><font/>
            <font><name val="Arial"/><color rgb="FF000000"/></font>
            <font><b/></font><font><color rgb="FFFF0000"/></font>
            <font><condense/></font><font><i val="0"/><u val="none"/></font>
            </fonts><cellXfs><xf/><xf fontId="1"/><xf fontId="2"/>
            <xf fontId="3"/><xf fontId="4"/><xf fontId="5"/>
            <xf fontId="99"/><xf fillId="2"/><xf borderId="1"/>
            </cellXfs></styleSheet>"#;
        let styles = decorated_styles(Some(xml)).unwrap();
        assert_eq!(
            styles,
            vec![false, false, true, true, true, false, true, true, true]
        );
        let sheet = br#"<worksheet><sheetData><row r="500"><c r="A500" s="2"><v>1</v></c></row></sheetData></worksheet>"#;
        let mut out = SheetIndicators::default();
        scan_pair(sheet, sheet, &styles, &styles, &mut out).unwrap();
        assert_eq!(out.rows, vec![[499, 500]]);
        for style in [2, 3, 4, 6, 7, 8] {
            let sheet = format!("<worksheet><cols><col min=\"1\" max=\"1\" style=\"{style}\"/></cols><sheetData/></worksheet>");
            let mut out = SheetIndicators::default();
            scan_pair(
                sheet.as_bytes(),
                sheet.as_bytes(),
                &styles,
                &styles,
                &mut out,
            )
            .unwrap();
            out.merge();
            assert_eq!(out.rows, vec![[0, MAX_ROWS]], "style {style}");
        }
    }

    #[test]
    fn decorated_default_covers_implicit_cells_and_empty_rows() {
        let xml = br#"<worksheet><sheetData><row r="10"><c r="A10"><v>1</v></c></row></sheetData></worksheet>"#;
        let mut out = SheetIndicators::default();
        scan_pair(xml, xml, &[true], &[true], &mut out).unwrap();
        out.merge();
        assert_eq!(out.rows, vec![[0, MAX_ROWS]]);
    }

    #[test]
    fn columns_require_all_rows_and_unknown_extensions_cannot_be_skipped() {
        let a =
            br#"<worksheet><cols><col min="1" max="2" width="10"/></cols><sheetData/></worksheet>"#;
        let b =
            br#"<worksheet><cols><col min="1" max="2" width="12"/></cols><sheetData/></worksheet>"#;
        let mut out = SheetIndicators::default();
        scan_pair(a, b, &[false], &[false], &mut out).unwrap();
        assert!(out.full_capture_reason.is_some());
        let unknown = br#"<worksheet><sheetData/><extLst><ext uri="future"/></extLst></worksheet>"#;
        let mut out = SheetIndicators::default();
        scan_pair(unknown, unknown, &[false], &[false], &mut out).unwrap();
        assert!(out.full_capture_reason.is_some());
    }

    #[test]
    fn anchored_charts_keep_rows_but_unbounded_objects_force_full_capture() {
        let xml = br#"<x:wsDr><x:twoCellAnchor><x:from><x:row>100</x:row></x:from><x:to><x:row>120</x:row></x:to><x:graphicFrame/></x:twoCellAnchor></x:wsDr>"#;
        let mut out = SheetIndicators::default();
        drawing_rows(xml, &mut out).unwrap();
        assert_eq!(out.rows, vec![[99, 122]]);
        drawing_rows(br#"<x:wsDr><x:oneCellAnchor/></x:wsDr>"#, &mut out).unwrap();
        assert!(out.full_capture_reason.is_some());
    }

    #[test]
    fn malformed_ranges_and_unordered_rows_are_not_silently_ignored() {
        assert!(range_rows(b"A0:A5").is_err());
        assert!(range_rows(b"A9:A2").is_err());
        let xml = br#"<worksheet><sheetData><row r="20"/><row r="10"/></sheetData></worksheet>"#;
        assert!(scan_pair(
            xml,
            xml,
            &[false],
            &[false],
            &mut SheetIndicators::default()
        )
        .is_err());
    }

    #[test]
    fn package_analysis_combines_removed_values_cf_and_chart_anchors() {
        use std::io::Write;
        let directory =
            std::env::temp_dir().join(format!("excel-shrink-tall-analysis-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let write = |name: &str, value: &str| {
            let path = directory.join(name);
            let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
            let sheet = format!("<worksheet><sheetData><row r=\"500\">{value}</row></sheetData><conditionalFormatting sqref=\"A600:A610\"/><drawing r:id=\"d1\"/></worksheet>");
            let drawing_relationship = if name == "unresolved.xlsx" {
                "<Relationships><Relationship Id=\"missing\" Type=\"office/drawing\" Target=\"../drawings/drawing1.xml\"/></Relationships>"
            } else {
                "<Relationships><Relationship Id=\"d1\" Type=\"office/drawing\" Target=\"../drawings/drawing1.xml\"/></Relationships>"
            };
            for (name, bytes) in [
                ("xl/workbook.xml", "<workbook><sheets><sheet name=\"Data\" r:id=\"r1\"/></sheets></workbook>"),
                ("xl/_rels/workbook.xml.rels", "<Relationships><Relationship Id=\"r1\" Type=\"office/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>"),
                ("xl/worksheets/sheet1.xml", sheet.as_str()),
                ("xl/worksheets/_rels/sheet1.xml.rels", drawing_relationship),
                ("xl/drawings/drawing1.xml", "<wsDr><twoCellAnchor><from><row>700</row></from><to><row>710</row></to><graphicFrame/></twoCellAnchor></wsDr>"),
            ] { zip.start_file(name, zip::write::FileOptions::default()).unwrap(); zip.write_all(bytes.as_bytes()).unwrap(); }
            zip.finish().unwrap();
            path
        };
        let original = write("original.xlsx", "<c r=\"A500\"><v>42</v></c>");
        let variant = write("variant.xlsx", "");
        let result = analyze(&original, &[variant.clone()], &BTreeSet::from([1])).unwrap();
        assert_eq!(result[&1].rows, vec![[499, 500], [599, 610], [699, 712]]);
        assert!(result[&1].full_capture_reason.is_none());
        let unresolved = write("unresolved.xlsx", "");
        let result = analyze(&original, &[unresolved.clone()], &BTreeSet::from([1])).unwrap();
        assert!(result[&1]
            .full_capture_reason
            .as_ref()
            .unwrap()
            .contains("unresolved worksheet drawing"));
        std::fs::remove_file(original).unwrap();
        std::fs::remove_file(variant).unwrap();
        std::fs::remove_file(unresolved).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    #[ignore = "read-only analysis of an explicitly supplied existing workbook group"]
    fn existing_giant_workbook_indicators() {
        let root = PathBuf::from(
            std::env::var_os("EXCEL_SHRINK_ANALYSIS_FIXTURE").expect("set fixture directory"),
        );
        let wanted: BTreeSet<_> = std::env::var("EXCEL_SHRINK_ANALYSIS_SHEETS")
            .unwrap_or_else(|_| "3,5,10,12".into())
            .split(',')
            .map(|n| n.parse().unwrap())
            .collect();
        let variants: Vec<_> = ["default", "maximum-cleaning", "too-aggressive-cleaning"]
            .iter()
            .map(|name| root.join(format!("{name}.xlsx")))
            .collect();
        let start = std::time::Instant::now();
        let result = analyze(&root.join("original.xlsx"), &variants, &wanted).unwrap();
        for (index, indicators) in result {
            println!(
                "sheet={index} ranges={} selected_rows={} reasons={:?} fallback={:?}",
                indicators.rows.len(),
                indicators
                    .rows
                    .iter()
                    .map(|r| u64::from(r[1] - r[0]))
                    .sum::<u64>(),
                indicators.reasons,
                indicators.full_capture_reason
            );
        }
        println!("analysis_seconds={:.2}", start.elapsed().as_secs_f64());
    }
}
