//! Experimental, streamed prefix normalization with expanded-name evidence. Not a shrink mode.
use super::{Detector, MAIN};
use crate::regression::{
    streaming_xml::{self, Event},
    xml_views::{Attributes, Kind},
};
use anyhow::{ensure, Context, Result};
use clap::Parser;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::PathBuf,
};
use zip::{write::FileOptions, ZipArchive, ZipWriter};

#[derive(Debug, Parser)]
pub(crate) struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    report: PathBuf,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
struct Evidence {
    semantic_sha256: String,
    rows: u64,
    cells: u64,
    max_row: u64,
}

fn expanded(scope: &Detector, name: &[u8], attribute: bool) -> Result<Vec<u8>> {
    let (prefix, local) = name
        .iter()
        .position(|b| *b == b':')
        .map(|p| (&name[..p], &name[p + 1..]))
        .unwrap_or((b"", name));
    let uri = if prefix == b"xml" {
        b"http://www.w3.org/XML/1998/namespace".as_slice()
    } else if attribute && prefix.is_empty() {
        b""
    } else {
        scope.bindings.get(prefix).map(Vec::as_slice).unwrap_or(b"")
    };
    ensure!(prefix.is_empty() || !uri.is_empty(), "Unbound XML prefix");
    let mut result = uri.to_vec();
    result.push(0);
    result.extend_from_slice(local);
    Ok(result)
}

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn attribute(output: &mut impl Write, name: &[u8], value: &[u8]) -> Result<()> {
    let quote = if value.contains(&b'"') { b'\'' } else { b'"' };
    ensure!(!value.contains(&quote), "Cannot preserve attribute quoting");
    output.write_all(b" ")?;
    output.write_all(name)?;
    output.write_all(&[b'=', quote])?;
    output.write_all(value)?;
    output.write_all(&[quote])?;
    Ok(())
}

/// Preserve raw text/attributes while normalizing only elements bound to SpreadsheetML.
fn transform(input: impl io::Read, mut output: impl Write) -> Result<Evidence> {
    let mut scope = Detector::default();
    let mut stack: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut hash = Sha256::new();
    let (mut rows, mut cells, mut max_row) = (0, 0, 0);
    let mut payload = None;
    streaming_xml::scan(input, |event| {
        match event {
            Event::Tag(tag) => {
                payload = None;
                if tag.kind == Kind::End {
                    let (name, _) = stack.pop().context("Missing normalized opening tag")?;
                    output.write_all(b"</")?;
                    output.write_all(&name)?;
                    output.write_all(b">")?;
                    hash.update([3]);
                    scope.tag(tag)?;
                    return Ok(());
                }
                let mut opening = tag;
                opening.kind = Kind::Start;
                scope.tag(opening)?;
                let qualified = expanded(&scope, tag.name, false)?;
                let split = qualified.iter().position(|b| *b == 0).unwrap();
                let uri = &qualified[..split];
                let name = if uri == MAIN { tag.local() } else { tag.name };
                let inherited_default =
                    stack.last().map(|(_, uri)| uri.clone()).unwrap_or_default();
                let mut default = inherited_default.clone();
                hash.update([1]);
                field(&mut hash, &qualified);
                output.write_all(b"<")?;
                output.write_all(name)?;
                for attr in Attributes::new(tag.attrs) {
                    let (key, value) = attr?;
                    if key == b"xmlns" || key.starts_with(b"xmlns:") {
                        ensure!(
                            !value.contains(&b'&'),
                            "Encoded namespace URI is not normalized experimentally"
                        );
                        if key == b"xmlns" {
                            default = value.to_vec();
                            continue;
                        }
                        if value == MAIN {
                            continue;
                        }
                        attribute(&mut output, key, value)?;
                    } else {
                        let expanded = expanded(&scope, key, true)?;
                        ensure!(
                            !expanded.starts_with(MAIN),
                            "SpreadsheetML-qualified attributes require separate validation"
                        );
                        field(&mut hash, &expanded);
                        field(&mut hash, value);
                        attribute(&mut output, key, value)?;
                    }
                }
                if uri == MAIN || !name.contains(&b':') {
                    default = uri.to_vec();
                }
                if default != inherited_default {
                    attribute(&mut output, b"xmlns", &default)?;
                }
                if uri == MAIN && tag.local() == b"row" {
                    rows += 1;
                    if let Some(value) = tag.attr(b"r")? {
                        max_row = max_row.max(std::str::from_utf8(value)?.parse::<u64>()?);
                    }
                }
                if uri == MAIN && tag.local() == b"c" {
                    cells += 1;
                }
                // Explicit end tags give the same digest event sequence as nonempty elements.
                output.write_all(b">")?;
                if tag.kind == Kind::Empty {
                    output.write_all(b"</")?;
                    output.write_all(name)?;
                    output.write_all(b">")?;
                    hash.update([3]);
                    let mut closing = tag;
                    closing.kind = Kind::End;
                    scope.tag(closing)?;
                } else {
                    stack.push((name.to_vec(), default));
                }
            }
            Event::Text(bytes) | Event::Cdata(bytes) => {
                if bytes.is_empty() {
                    return Ok(());
                }
                let kind = if matches!(event, Event::Cdata(_)) {
                    4
                } else {
                    2
                };
                if payload != Some(kind) {
                    hash.update([kind]);
                    payload = Some(kind);
                }
                hash.update(bytes);
                if kind == 4 {
                    output.write_all(b"<![CDATA[")?;
                }
                output.write_all(bytes)?;
                if kind == 4 {
                    output.write_all(b"]]>")?;
                }
            }
        }
        Ok(())
    })?;
    output.flush()?;
    Ok(Evidence {
        semantic_sha256: format!("{:x}", hash.finalize()),
        rows,
        cells,
        max_row,
    })
}

/// Verify every rewritten XML entry by reading the actual output back through expanded-name hashing.
pub(crate) fn run(args: Args) -> Result<()> {
    ensure!(
        !args.output.exists() && !args.report.exists(),
        "Use new experiment outputs"
    );
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut input = ZipArchive::new(File::open(&args.input)?)?;
    let mut output = ZipWriter::new(File::create(&args.output)?);
    let options = FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));
    let mut evidence = BTreeMap::new();
    for index in 0..input.len() {
        let mut entry = input.by_index(index)?;
        let name = entry.name().to_owned();
        output.start_file(&name, options)?;
        if name.starts_with("xl/") && name.ends_with(".xml") {
            let result =
                transform(&mut entry, BufWriter::new(&mut output)).with_context(|| name.clone())?;
            evidence.insert(name, result);
        } else {
            io::copy(&mut entry, &mut output)?;
        }
    }
    output.finish()?;
    let mut verify = ZipArchive::new(File::open(&args.output)?)?;
    for (name, expected) in &evidence {
        let actual = transform(verify.by_name(name)?, io::sink())?;
        ensure!(
            &actual == expected,
            "Normalization changed expanded XML evidence: {name}"
        );
    }
    fs::write(args.report, serde_json::to_vec_pretty(&evidence)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_values_cdata_and_foreign_prefixes_across_namespace_scopes() {
        let source=format!("<p:worksheet xmlns:p='{}' xmlns:r='relationship'><p:sheetData><p:row r='1'><p:c r='A1'><p:v><![CDATA[<&text]]></p:v></p:c></p:row></p:sheetData><r:other r:id='x'/></p:worksheet>",String::from_utf8_lossy(MAIN));
        let mut out = Vec::new();
        let before = transform(source.as_bytes(), &mut out).unwrap();
        let after = transform(out.as_slice(), io::sink()).unwrap();
        assert_eq!(before, after);
        assert_eq!(before.cells, 1);
        assert!(String::from_utf8(out).unwrap().contains("<sheetData>"));
    }

    #[test]
    fn restores_local_empty_element_bindings_and_foreign_default_namespaces() {
        let source=format!("<p:worksheet xmlns:p='{}' xmlns='foreign'><p:sheetData><p:row r='1'><q:c xmlns:q='{}' r='A1'/></p:row></p:sheetData><other xml:space='preserve'>text</other></p:worksheet>",String::from_utf8_lossy(MAIN),String::from_utf8_lossy(MAIN));
        let mut out = Vec::new();
        let before = transform(source.as_bytes(), &mut out).unwrap();
        let after = transform(out.as_slice(), io::sink()).unwrap();
        assert_eq!(before, after);
        assert_eq!(before.cells, 1);
    }
}
