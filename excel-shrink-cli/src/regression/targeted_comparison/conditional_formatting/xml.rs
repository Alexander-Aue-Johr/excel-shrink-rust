//! Retain CF fingerprints and complete structured rules without building a worksheet DOM.
use crate::regression::{
    cf_inventory::scan::{self, Block},
    xml_views::{Kind, Reader},
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
    pub standard_namespaces: Vec<bool>,
}

pub(in crate::regression::targeted_comparison) struct Span {
    pub start: usize,
    pub end: usize,
    pub standard_namespace: bool,
}

struct HashingReader<R> {
    input: R,
    hash: Sha256,
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
        result.blocks.push(blocks.remove(0));
        result.standard_namespaces.push(span.standard_namespace);
        result.fingerprints.push(input.hash.finalize().into());
        position = end;
    }
    Ok(result)
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
