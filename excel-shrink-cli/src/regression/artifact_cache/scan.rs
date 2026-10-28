//! Stream each decompressed ZIP member once: exact digest plus conservative
//! cache metadata. A worksheet, cell value or comment is never retained in full.
use super::{hex, is_sheet, PartDigest};
use super::super::{
    streaming_xml::{self, Event, Scanner},
    xml_views::{truth, Attributes, Kind, Tag},
};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;

const MAX_FORMULA_BYTES: usize = 64 * 1024;
const CONTEXT_ELEMENTS: &[&[u8]] = &[
    b"conditionalFormatting", b"sparklineGroups", b"drawing", b"legacyDrawing",
    b"picture", b"oleObjects", b"controls", b"extLst", b"pivotTableParts", b"tableParts",
];
const VOLATILE: &[&[u8]] = &[
    b"NOW", b"TODAY", b"RAND", b"RANDBETWEEN", b"RANDARRAY", b"RTD", b"WEBSERVICE",
    b"STOCKHISTORY", b"CELL", b"INFO", b"INDIRECT", b"IMAGE",
];
// Keep the existing conservative certificate; unknown/UDF/external functions
// never make a persisted recalculation result reusable.
const PURE: &[&[u8]] = &[
    b"SUM", b"SUMIF", b"SUMIFS", b"SUMPRODUCT", b"PRODUCT", b"AVERAGE", b"AVERAGEIF", b"AVERAGEIFS",
    b"COUNT", b"COUNTA", b"COUNTBLANK", b"COUNTIF", b"COUNTIFS", b"MIN", b"MAX", b"ABS", b"ROUND", b"ROUNDUP", b"ROUNDDOWN",
    b"INT", b"MOD", b"POWER", b"SQRT", b"AND", b"OR", b"NOT", b"IF", b"IFERROR", b"IFNA", b"ISBLANK", b"ISNUMBER", b"ISTEXT", b"ISERROR", b"ISNA",
    b"INDEX", b"MATCH", b"VLOOKUP", b"HLOOKUP", b"XLOOKUP", b"CHOOSE", b"ROW", b"ROWS", b"COLUMN", b"COLUMNS", b"LEN", b"LEFT", b"RIGHT", b"MID",
    b"TRIM", b"CLEAN", b"SUBSTITUTE", b"REPLACE", b"CONCATENATE", b"CONCAT", b"TEXTJOIN", b"VALUE", b"TEXT", b"DATE", b"YEAR", b"MONTH", b"DAY",
    b"HOUR", b"MINUTE", b"SECOND", b"TIME", b"TRUE", b"FALSE", b"NA", b"SUBTOTAL", b"AGGREGATE", b"LN", b"LOG", b"LOG10", b"EXP", b"SIN", b"COS", b"TAN",
];

struct Metadata {
    worksheet: bool,
    refresh_sensitive: bool,
    relationships: bool,
    visual_context: bool,
    visual_runtime: bool,
    recalc_cacheable: bool,
    has_formula: bool,
    formula_depth: Option<usize>,
    formula: Vec<u8>,
    formula_uncertain: bool,
}

impl Metadata {
    fn new(name: &str) -> Self {
        let worksheet = is_sheet(name);
        let chart = name.starts_with("xl/chartsheets/") || name.starts_with("xl/charts/");
        let dependencies = name.starts_with("xl/externalLinks/")
            || name == "xl/connections.xml"
            || name.starts_with("xl/queryTables/")
            || name.starts_with("xl/model/")
            || name.ends_with("vbaProject.bin")
            || name.starts_with("xl/embeddings/");
        Self {
            worksheet,
            refresh_sensitive: worksheet || chart || name == "xl/workbook.xml"
                || name == "xl/connections.xml" || name.starts_with("xl/queryTables/")
                || name.starts_with("xl/externalLinks/"),
            relationships: name.ends_with(".rels"),
            visual_context: chart,
            visual_runtime: false,
            recalc_cacheable: !dependencies,
            has_formula: false,
            formula_depth: None,
            formula: Vec::new(),
            formula_uncertain: false,
        }
    }

    fn event(&mut self, event: Event<'_>) -> Result<()> {
        match event {
            Event::Text(bytes) => {
                if self.formula_depth.is_some() && !self.formula_uncertain {
                    if bytes.len() > MAX_FORMULA_BYTES - self.formula.len() {
                        self.uncertain_formula();
                    } else {
                        self.formula.extend_from_slice(bytes);
                    }
                }
            }
            Event::Cdata(_) => {
                if self.formula_depth.is_some() {
                    self.uncertain_formula();
                }
            }
            Event::Tag(tag) => self.tag(tag)?,
        }
        Ok(())
    }

    fn tag(&mut self, tag: Tag<'_>) -> Result<()> {
        if let Some(depth) = self.formula_depth {
            if tag.kind == Kind::End && tag.depth == depth {
                self.finish_formula();
                self.formula_depth = None;
            } else if tag.kind != Kind::End {
                // A formula is text, not an embedded XML subtree. Do not infer a
                // reusable result from a representation the certificate cannot model.
                self.uncertain_formula();
            }
            return Ok(());
        }
        if tag.kind == Kind::End {
            return Ok(());
        }
        let local = tag.local();
        self.visual_context |= self.worksheet
            && CONTEXT_ELEMENTS.iter().any(|name| name.eq_ignore_ascii_case(local));
        for attribute in Attributes::new(tag.attrs) {
            let (name, value) = attribute?;
            if (name.eq_ignore_ascii_case(b"refreshOnLoad")
                || name.eq_ignore_ascii_case(b"refreshOnFileOpen"))
                && (truth(Some(value)) || value.contains(&b'&'))
            {
                self.visual_runtime |= self.refresh_sensitive;
                self.recalc_cacheable = false;
            }
        }
        if local == b"calcPr" {
            let iterate = tag.attr(b"iterate")?;
            if truth(iterate) || iterate.is_some_and(|value| value.contains(&b'&')) {
                self.recalc_cacheable = false;
            }
        }
        if self.relationships && local == b"Relationship" {
            let mode = tag.attr(b"TargetMode")?.unwrap_or_default();
            let kind = tag.attr(b"Type")?.unwrap_or_default();
            if mode.contains(&b'&') || kind.contains(&b'&')
                || (mode.eq_ignore_ascii_case(b"External") && !kind.ends_with(b"/hyperlink"))
            {
                self.recalc_cacheable = false;
            }
        }
        if !self.relationships && matches!(local,
            b"f" | b"formula" | b"formula1" | b"formula2" | b"definedName"
                | b"calculatedColumnFormula" | b"totalsRowFormula")
        {
            self.has_formula = true;
            if tag.attr(b"t")? == Some(b"dataTable".as_slice()) {
                self.recalc_cacheable = false;
            }
            self.formula.clear();
            self.formula_uncertain = false;
            if tag.kind == Kind::Start {
                self.formula_depth = Some(tag.depth);
            }
        }
        Ok(())
    }

    fn uncertain_formula(&mut self) {
        self.formula_uncertain = true;
        self.formula.clear();
        self.recalc_cacheable = false;
        self.visual_runtime |= self.refresh_sensitive;
    }

    fn finish_formula(&mut self) {
        if self.formula_uncertain {
            return;
        }
        if !self.formula.is_ascii()
            || self.formula.iter().any(|&b| matches!(b, b'&' | b'[' | b'|'))
        {
            self.uncertain_formula();
            return;
        }
        static FUNCTIONS: std::sync::OnceLock<regex::bytes::Regex> = std::sync::OnceLock::new();
        let calls = FUNCTIONS.get_or_init(|| {
            regex::bytes::Regex::new(r"([A-Za-z_][A-Za-z_0-9.]*)\s*\(").unwrap()
        });
        for call in calls.captures_iter(&self.formula) {
            let function = call.get(1).unwrap().as_bytes();
            if !PURE.iter().any(|allowed| allowed.eq_ignore_ascii_case(function)) {
                self.recalc_cacheable = false;
            }
            let local = function.rsplit(|&byte| byte == b'.').next().unwrap_or(function);
            self.visual_runtime |= self.refresh_sensitive
                && VOLATILE.iter().any(|name| name.eq_ignore_ascii_case(local));
        }
        self.formula.clear();
    }

    fn uncertain_xml(&mut self) {
        self.recalc_cacheable = false;
        self.visual_context |= self.worksheet;
        self.visual_runtime |= self.refresh_sensitive;
        self.has_formula |= !self.relationships;
        self.formula.clear();
    }
}

pub(super) fn digest_part(
    name: &str,
    input: &mut impl Read,
    expected_bytes: u64,
) -> Result<PartDigest> {
    let mut metadata = Metadata::new(name);
    let mut scanner = (name.ends_with(".xml") || name.ends_with(".rels")).then(Scanner::new);
    let mut hash = Sha256::new();
    let mut buffer = [0u8; streaming_xml::READ_BYTES];
    let mut bytes = 0u64;
    loop {
        let count = streaming_xml::read_chunk(input, &mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes.checked_add(count as u64).context("ZIP member length overflow")?;
        ensure!(bytes <= expected_bytes, "ZIP member exceeds its declared length");
        let chunk = &buffer[..count];
        hash.update(chunk);
        if let Some(xml) = scanner.as_mut() {
            if let Err(error) = xml.feed(chunk, |event| metadata.event(event)) {
                // Uncertain XML disables reuse, but must not truncate the digest
                // or bypass the ZIP reader's end-of-stream CRC/I/O checks.
                tracing::warn!("[validation-cache] {name}: cannot certify streamed XML: {error:#}");
                metadata.uncertain_xml();
                scanner = None;
            }
        }
    }
    ensure!(bytes == expected_bytes, "ZIP member is shorter than its declared length");
    if let Some(xml) = scanner {
        if let Err(error) = xml.finish() {
            tracing::warn!("[validation-cache] {name}: cannot certify streamed XML: {error:#}");
            metadata.uncertain_xml();
        }
    }
    Ok(PartDigest {
        bytes,
        sha256: hex(&hash.finalize()),
        visual_context: metadata.visual_context,
        visual_runtime: metadata.visual_runtime,
        recalc_cacheable: metadata.recalc_cacheable,
        has_formula: metadata.has_formula,
    })
}

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
