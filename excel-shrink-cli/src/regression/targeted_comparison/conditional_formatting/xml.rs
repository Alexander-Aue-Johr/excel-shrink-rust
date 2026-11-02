//! Retain CF fingerprints and complete structured rules without building a worksheet DOM.
use crate::regression::{
    cf_inventory::scan::{self, Block},
    xml_views::{Attributes, Kind, Reader, Tag},
};
use anyhow::{ensure, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, Read},
};
use zip::ZipArchive;

#[derive(Default)]
pub(in crate::regression::targeted_comparison) struct Catalogue {
    pub blocks: Vec<Block>,
    pub fingerprints: Vec<[u8; 32]>,
    pub namespace_supported: Vec<bool>,
}

pub(in crate::regression::targeted_comparison) struct Span {
    pub start: usize,
    pub end: usize,
    pub namespace_supported: bool,
    pub x14_extension: bool,
}

struct HashingReader<R> {
    input: R,
    hash: Sha256,
}

/// Accept known static x14 bar settings and static inline differential formatting.
pub(in crate::regression::targeted_comparison) fn static_format_attributes(
    tag: Tag<'_>,
) -> Result<bool> {
    if matches!(tag.name, b"x14:cfRule" | b"x14:conditionalFormatting") {
        return Ok(true);
    }
    if tag.name.starts_with(b"x14:") && super::data_bars::static_attributes(tag)? {
        return Ok(true);
    }
    for attribute in Attributes::new(tag.attrs) {
        let (name, _) = attribute?;
        if tag.name.contains(&b':') {
            if ![b"xmlns".as_slice(), b"xmlns:x14", b"xmlns:xm"].contains(&name) {
                return Ok(false);
            }
            continue;
        }
        if ![
            b"val".as_slice(),
            b"rgb",
            b"theme",
            b"indexed",
            b"auto",
            b"tint",
            b"patternType",
            b"numFmtId",
            b"formatCode",
            b"horizontal",
            b"vertical",
            b"textRotation",
            b"wrapText",
            b"shrinkToFit",
            b"indent",
            b"relativeIndent",
            b"justifyLastLine",
            b"readingOrder",
            b"style",
            b"diagonalUp",
            b"diagonalDown",
            b"outline",
            b"locked",
            b"hidden",
        ]
        .contains(&name)
        {
            return Ok(false);
        }
    }
    Ok(true)
}
impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.input.read(bytes)?;
        self.hash.update(&bytes[..count]);
        Ok(count)
    }
}

/// Reopen only CF-bearing sheets; selected subtree bytes are hashed while being parsed.
pub(in crate::regression::targeted_comparison) fn catalogue(
    archive: &mut ZipArchive<File>,
    part: &str,
    spans: &[Span],
) -> Result<Catalogue> {
    let mut result = Catalogue::default();
    if spans.is_empty() {
        return Ok(result);
    }
    let mut entry = archive.by_name(part)?;
    let mut position = 0;
    let mut data_bar_links = super::data_bars::Links::default();
    for span in spans {
        let (start, end) = (span.start, span.end);
        ensure!(
            start >= position && end >= start,
            "Invalid CF subtree offsets"
        );
        let skipped = io::copy(
            &mut (&mut entry).take((start - position) as u64),
            &mut io::sink(),
        )?;
        ensure!(skipped == (start - position) as u64, "Truncated CF prefix");
        let mut input = HashingReader {
            input: (&mut entry).take((end - start) as u64),
            hash: Sha256::new(),
        };
        let mut blocks = Vec::new();
        scan::scan(&mut input, &mut blocks, 256, 32)?;
        ensure!(
            blocks.len() == 1,
            "CF subtree did not yield exactly one block"
        );
        let mut block = blocks.remove(0);
        let links_complete = data_bar_links.record(&block, result.blocks.len());
        let supported = links_complete
            && span.namespace_supported
            && (!span.x14_extension
                || normalize_x14_expression(&mut block)
                || super::data_bars::normalize_x14(&mut block));
        result.blocks.push(block);
        result.namespace_supported.push(supported);
        result.fingerprints.push(input.hash.finalize().into());
        position = end;
    }
    data_bar_links.constrain(&mut result.namespace_supported);
    Ok(result)
}

/// Adapt only x14 expression rules with static inline formatting to the existing dependency model.
/// Original bytes, including formatting and IDs, remain covered by the catalogue fingerprint.
fn normalize_x14_expression(block: &mut Block) -> bool {
    const FORMAT_TAGS: &[&str] = &[
        "fill",
        "patternFill",
        "fgColor",
        "bgColor",
        "font",
        "name",
        "charset",
        "family",
        "b",
        "i",
        "strike",
        "outline",
        "shadow",
        "condense",
        "extend",
        "color",
        "sz",
        "u",
        "vertAlign",
        "scheme",
        "numFmt",
        "alignment",
        "border",
        "left",
        "right",
        "top",
        "bottom",
        "diagonal",
        "vertical",
        "horizontal",
        "start",
        "end",
        "protection",
    ];
    if block.qualified_name != "x14:conditionalFormatting"
        || block.sqref.is_empty()
        || block
            .attributes
            .keys()
            .any(|key| !["xmlns:xm", "xmlns:x14"].contains(&key.as_str()))
        || block.rules.is_empty()
        || block.child_elements.iter().any(|name| {
            !["x14:cfRule", "xm:f", "xm:sqref", "x14:dxf"].contains(&name.as_str())
                && !FORMAT_TAGS.contains(&name.as_str())
        })
        || block.rules.iter().any(|rule| {
            rule.attributes.get("type").map(String::as_str) != Some("expression")
                || rule.attributes.keys().any(|key| {
                    !["type", "priority", "id", "stopIfTrue", "dxfId"].contains(&key.as_str())
                })
                || rule.formulas.len() != 1
                || rule.child_elements.iter().any(|name| {
                    !["xm:f", "x14:dxf"].contains(&name.as_str())
                        && !FORMAT_TAGS.contains(&name.as_str())
                })
        })
    {
        return false;
    }
    block.qualified_name = "conditionalFormatting".into();
    block.attributes.clear();
    block.attributes.insert("sqref".into(), block.sqref.clone());
    block.child_elements = BTreeSet::from(["cfRule".into(), "formula".into()]);
    for rule in &mut block.rules {
        rule.attributes.remove("id");
        rule.child_elements = BTreeSet::from(["formula".into()]);
    }
    true
}

/// Absence is provable only when all sheet names are unambiguous ASCII literals.
/// Encoded/escaped names deliberately keep INDIRECT on the fallback path.
pub(in crate::regression::targeted_comparison) fn sheet_names(
    bytes: &[u8],
) -> Result<Option<BTreeSet<String>>> {
    let mut names = BTreeSet::new();
    let mut reader = Reader::<128>::new(bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"sheet" {
            continue;
        }
        let Some(name) = tag.attr(b"name")? else {
            return Ok(None);
        };
        let name = std::str::from_utf8(name)?.to_ascii_lowercase();
        if !name.is_ascii() || name.contains('&') || name.contains("_x") {
            return Ok(None);
        }
        names.insert(name);
    }
    Ok(Some(names))
}

/// Include workbook- and sheet-scoped names; uncertain encodings cannot prove a name absent.
pub(in crate::regression::targeted_comparison) fn defined_names(
    bytes: &[u8],
) -> Result<Option<BTreeSet<String>>> {
    let mut names = BTreeSet::new();
    let mut reader = Reader::<128>::new(bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.local() != b"definedName" || tag.kind == Kind::End {
            continue;
        }
        let Some(name) = tag.attr(b"name")? else {
            return Ok(None);
        };
        let name = std::str::from_utf8(name)?.to_ascii_lowercase();
        let encoded = name.as_bytes().windows(7).any(|window| {
            window.starts_with(b"_x")
                && window[2..6].iter().all(u8::is_ascii_hexdigit)
                && window[6] == b'_'
        });
        if !name.is_ascii() || name.contains('&') || encoded {
            return Ok(None);
        }
        names.insert(name);
    }
    Ok(Some(names))
}

/// Tables provide another bare-name source. Keep them conservative rather than resolving table names.
pub(in crate::regression::targeted_comparison) fn defined_names_in_package(
    archive: &mut ZipArchive<File>,
    workbook: &[u8],
) -> Result<Option<BTreeSet<String>>> {
    let Some(names) = defined_names(workbook)? else {
        return Ok(None);
    };
    if archive
        .file_names()
        .any(|name| name.starts_with("xl/tables/"))
    {
        return Ok(None);
    }
    let mut types = match archive.by_name("[Content_Types].xml") {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    types.read_to_end(&mut bytes)?;
    let mut reader = Reader::<128>::new(&bytes)?;
    while let Some(tag) = reader.next()? {
        if let Some(kind) = tag.attr(b"ContentType")? {
            if kind.contains(&b'&') || kind.to_ascii_lowercase().ends_with(b".table+xml") {
                return Ok(None);
            }
        }
    }
    Ok(Some(names))
}

#[cfg(test)]
mod name_package_tests {
    use super::*;
    use std::io::Write;

    #[test]
    #[ignore = "Requires CF_INVENTORY with retained PCC sources 8036/8037; reads metadata only"]
    fn retained_pcc_sources_prove_isblank_absent_without_table_ambiguity() -> Result<()> {
        let root = std::path::PathBuf::from(std::env::var("CF_INVENTORY")?);
        for id in [8036, 8037] {
            let mut archive =
                ZipArchive::new(File::open(root.join(format!("sources/{id:05}.xlsx")))?)?;
            let mut workbook = Vec::new();
            archive
                .by_name("xl/workbook.xml")?
                .read_to_end(&mut workbook)?;
            let names = defined_names_in_package(&mut archive, &workbook)?
                .expect("Unambiguous name catalogue");
            assert!(!names.contains("isblank"));
        }
        Ok(())
    }

    #[test]
    fn tables_and_missing_package_metadata_cannot_prove_a_name_absent() -> Result<()> {
        for mode in [
            "ordinary",
            "standard-table",
            "custom-table",
            "missing-types",
        ] {
            let path = std::env::temp_dir().join(format!("cf-names-{}.xlsx", uuid::Uuid::new_v4()));
            let mut output = zip::ZipWriter::new(File::create(&path)?);
            if mode != "missing-types" {
                output.start_file("[Content_Types].xml", zip::write::FileOptions::default())?;
                let types = if mode == "custom-table" {
                    "<Types><Override PartName='/custom/named.xml' ContentType='application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml'/></Types>"
                } else {
                    "<Types/>"
                };
                output.write_all(types.as_bytes())?;
            }
            if mode == "standard-table" {
                output.start_file("xl/tables/table1.xml", zip::write::FileOptions::default())?;
                output.write_all(b"<table name='isblank'/>")?;
            }
            output.finish()?;
            let mut archive = ZipArchive::new(File::open(&path)?)?;
            assert_eq!(
                defined_names_in_package(&mut archive, b"<workbook/>")?.is_some(),
                mode == "ordinary"
            );
            drop(archive);
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

/// Include nonstandard part locations and theme overrides declared by the package.
pub(in crate::regression::targeted_comparison) fn visual_parts(
    archive: &mut ZipArchive<File>,
) -> Result<BTreeSet<String>> {
    let mut parts: BTreeSet<String> = archive
        .file_names()
        .filter(|name| name.starts_with("xl/theme/") && name.ends_with(".xml"))
        .map(str::to_owned)
        .collect();
    let mut types = match archive.by_name("[Content_Types].xml") {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(parts),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    types.read_to_end(&mut bytes)?;
    let mut reader = Reader::<128>::new(&bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.local() != b"Override" || tag.kind == Kind::End {
            continue;
        }
        let Some(kind) = tag.attr(b"ContentType")? else {
            continue;
        };
        if [
            b".theme+xml".as_slice(),
            b".themeOverride+xml",
            b".styles+xml",
            b".sharedStrings+xml",
        ]
        .iter()
        .any(|suffix| kind.ends_with(suffix))
        {
            let part = std::str::from_utf8(tag.attr(b"PartName")?.unwrap_or_default())?
                .trim_start_matches('/');
            ensure!(!part.is_empty(), "Visual part declaration has no name");
            if !["xl/styles.xml", "xl/sharedStrings.xml"].contains(&part) {
                parts.insert(part.into());
            }
        }
    }
    Ok(parts)
}
