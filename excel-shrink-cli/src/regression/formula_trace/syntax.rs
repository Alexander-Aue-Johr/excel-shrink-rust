//! Deliberately small A1 dependency grammar, not an Excel formula evaluator.
use anyhow::{ensure, Result};
use regex::Regex;
use std::{ops::Range, sync::OnceLock};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct Address {
    pub sheet: String,
    pub column: u32,
    pub row: u32,
}

impl Address {
    pub fn cell(&self) -> String {
        let mut column = self.column;
        let mut letters = String::new();
        while column > 0 {
            column -= 1;
            letters.insert(0, char::from(b'A' + (column % 26) as u8));
            column /= 26;
        }
        format!("{letters}{}", self.row)
    }

    pub fn parse(sheet: &str, cell: &str) -> Result<Self> {
        let mut references = tokens(cell, sheet)?;
        ensure!(
            references.len() == 1 && references[0].span == (0..cell.len()),
            "Expected one cell coordinate"
        );
        Ok(references.remove(0).address)
    }
}

#[derive(Clone, Debug)]
pub(super) enum Dependency {
    Cell(Address),
    Range(Address, Address),
}

struct Reference {
    address: Address,
    absolute_column: bool,
    absolute_row: bool,
    span: Range<usize>,
    coordinate_span: Range<usize>,
}

fn reference_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(
        r"(?:(?:'(?P<quoted>(?:[^']|'')+)'|(?P<sheet>[A-Za-z_][A-Za-z0-9_.]*))!)?(?P<ac>\$?)(?P<col>[A-Za-z]{1,3})(?P<ar>\$?)(?P<row>[1-9][0-9]{0,6})"
    ).expect("Static A1 pattern"))
}

fn word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$')
}

fn word_before(text: &str, position: usize) -> bool {
    text[..position]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '$'))
}

/// Replace string contents with spaces while preserving UTF-8 byte offsets.
fn masked_strings(formula: &str) -> String {
    let mut bytes = formula.as_bytes().to_vec();
    let mut quoted = false;
    for byte in &mut bytes {
        if *byte == b'"' {
            quoted = !quoted;
            *byte = b' ';
        } else if quoted {
            *byte = b' ';
        }
    }
    // Every non-ASCII byte inside a quoted string was replaced with ASCII too.
    String::from_utf8(bytes).expect("Mask preserves remaining UTF-8")
}

/// Function-like text inside quoted worksheet names is an identifier, not a call.
fn masked_call_context(formula: &str) -> String {
    let mut masked = masked_strings(formula).into_bytes();
    let mut quoted = false;
    for (index, byte) in formula.bytes().enumerate() {
        if byte == b'\'' && masked[index] != b' ' {
            quoted = !quoted;
            masked[index] = b' ';
        } else if quoted {
            masked[index] = b' ';
        }
    }
    String::from_utf8(masked).expect("Mask preserves remaining UTF-8")
}

fn tokens(formula: &str, sheet: &str) -> Result<Vec<Reference>> {
    let masked = masked_strings(formula);
    let mut result = Vec::new();
    for capture in reference_pattern().captures_iter(&masked) {
        let matched = capture.get(0).unwrap();
        if word_before(&masked, matched.start()) {
            continue;
        }
        if masked
            .as_bytes()
            .get(matched.end())
            .is_some_and(|b| word(*b) || *b == b'(')
        {
            continue;
        }
        let mut column = 0;
        for byte in capture["col"].bytes() {
            column = column * 26 + u32::from(byte.to_ascii_uppercase() - b'A' + 1);
        }
        let row = capture["row"].parse()?;
        ensure!(
            column <= 16384 && row <= 1048576,
            "Reference outside worksheet bounds"
        );
        let target = capture
            .name("quoted")
            .map(|m| m.as_str().replace("''", "'"))
            .or_else(|| capture.name("sheet").map(|m| m.as_str().into()))
            .unwrap_or_else(|| sheet.into());
        result.push(Reference {
            address: Address {
                sheet: target,
                column,
                row,
            },
            absolute_column: !capture["ac"].is_empty(),
            absolute_row: !capture["ar"].is_empty(),
            span: matched.range(),
            coordinate_span: capture.name("ac").unwrap().start()
                ..capture.name("row").unwrap().end(),
        });
    }
    Ok(result)
}

/// Return the complete first random/time call, including its balanced arguments.
pub(super) fn volatile_span(formula: &str) -> Option<Range<usize>> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r"(?i)(?:_xlfn\.)?(?:RAND|RANDBETWEEN|RANDARRAY|NOW|TODAY)\s*\(").unwrap()
    });
    let masked = masked_call_context(formula);
    for matched in pattern.find_iter(&masked) {
        if word_before(&masked, matched.start()) {
            continue;
        }
        let mut depth = 1;
        for (offset, byte) in masked.as_bytes()[matched.end()..].iter().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                return Some(matched.start()..matched.end() + offset + 1);
            }
        }
    }
    None
}

/// Recognize only a literal filename information request, without evaluating CELL arguments.
pub(super) fn environment_span(formula: &str) -> Option<Range<usize>> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r#"(?i)(?:_xlfn\.)?CELL\s*\(\s*"filename"\s*(?:,|\))"#).unwrap()
    });
    let masked = masked_call_context(formula);
    for matched in pattern.find_iter(formula) {
        if word_before(&masked, matched.start()) || masked.as_bytes()[matched.start()] == b' ' {
            continue;
        }
        let opening = matched.start() + matched.as_str().find('(')?;
        let mut depth = 0;
        for (offset, byte) in masked.as_bytes()[opening..].iter().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                return Some(matched.start()..opening + offset + 1);
            }
        }
    }
    None
}

pub(super) fn references(formula: &str, sheet: &str) -> Result<Vec<Dependency>> {
    static INDIRECT: OnceLock<Regex> = OnceLock::new();
    static DYNAMIC: OnceLock<Regex> = OnceLock::new();
    let literal = INDIRECT.get_or_init(|| {
        Regex::new(r#"(?i)\bINDIRECT\(\s*"((?:[^"]|"")*)"\s*(?:,\s*(?:TRUE|1)\s*)?\)"#).unwrap()
    });
    let mut cleaned = formula.to_owned();
    let original_mask = masked_strings(formula);
    let mut result = Vec::new();
    for capture in literal.captures_iter(formula) {
        let matched = capture.get(0).unwrap();
        if original_mask.as_bytes()[matched.start()] == b' ' {
            continue;
        }
        let target = capture[1].replace("\"\"", "\"");
        let address = Address::parse(sheet, &target)?;
        result.push(Dependency::Cell(address));
        cleaned.replace_range(matched.range(), &" ".repeat(matched.len()));
    }
    let masked = masked_strings(&cleaned);
    ensure!(
        !masked.contains(['[', ']']),
        "External or structured reference is unsupported"
    );
    let dynamic = DYNAMIC.get_or_init(|| Regex::new(r"(?i)\b(?:INDIRECT|OFFSET)\s*\(").unwrap());
    ensure!(
        !dynamic.is_match(&masked),
        "Computed dynamic reference is unsupported"
    );
    let mut refs = tokens(&cleaned, sheet)?.into_iter().peekable();
    while let Some(reference) = refs.next() {
        if masked.as_bytes().get(reference.span.end) == Some(&b':') {
            let end = refs
                .next()
                .ok_or_else(|| anyhow::anyhow!("Unresolved range endpoint"))?;
            ensure!(
                end.span.start == reference.span.end + 1,
                "Unsupported range syntax"
            );
            let mut end_address = end.address;
            // An unqualified second endpoint inherits the first endpoint's sheet.
            if !cleaned[end.span.clone()].contains('!') {
                end_address.sheet = reference.address.sheet.clone();
            }
            ensure!(
                end_address.sheet == reference.address.sheet,
                "3D range is unsupported"
            );
            result.push(Dependency::Range(reference.address, end_address));
        } else {
            result.push(Dependency::Cell(reference.address));
        }
    }
    Ok(result)
}

/// Translate only relative A1 coordinates in an Excel shared formula.
pub(super) fn shifted_formula(formula: &str, source: &Address, target: &Address) -> Result<String> {
    let mut result = formula.to_owned();
    for reference in tokens(formula, &source.sheet)?.into_iter().rev() {
        let mut address = reference.address;
        let x = i64::from(address.column)
            + if reference.absolute_column {
                0
            } else {
                i64::from(target.column) - i64::from(source.column)
            };
        let y = i64::from(address.row)
            + if reference.absolute_row {
                0
            } else {
                i64::from(target.row) - i64::from(source.row)
            };
        ensure!(
            (1..=16384).contains(&x) && (1..=1048576).contains(&y),
            "Shared formula leaves worksheet bounds"
        );
        address.column = x as u32;
        address.row = y as u32;
        let cell = address.cell();
        let split = cell.find(char::is_numeric).unwrap();
        let replacement = format!(
            "{}{}{}{}",
            if reference.absolute_column { "$" } else { "" },
            &cell[..split],
            if reference.absolute_row { "$" } else { "" },
            &cell[split..]
        );
        result.replace_range(reference.coordinate_span, &replacement);
    }
    Ok(result)
}
