//! Prove package equivalence without loading worksheet XML or starting Excel.
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};
use zip::ZipArchive;

use crate::regression::{artifact_cache, xml_views};

const BUFFER_BYTES: usize = 64 * 1024;
const MAX_PARTS: usize = 65_536;
const MAX_MEMBER_NAME_BYTES: usize = 8 * 1024 * 1024;
const MAX_WORKBOOK_BYTES: u64 = 8 * 1024 * 1024;
const WORKBOOK: &str = "xl/workbook.xml";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct PartDigest {
    bytes: u64,
    sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Header {
    index: usize,
    bytes: u64,
    crc32: u32,
}

struct Catalog {
    path: PathBuf,
    parts: BTreeMap<String, Header>,
}

#[derive(Clone)]
struct Package {
    archive_sha256: String,
    parts: Arc<BTreeMap<String, PartDigest>>,
}

/// A negative CRC/length comparison is cheap. Positive decisions always hash
/// every decompressed member; CRC equality is never an equivalence certificate.
pub(super) fn compare(original: &Path, variants: &[(&str, PathBuf)]) -> Result<Option<Value>> {
    let supplied: BTreeSet<_> = variants.iter().map(|(name, _)| *name).collect();
    ensure!(
        !variants.is_empty() && variants.len() == supplied.len()
            && supplied.iter().all(|name| super::PROFILES.contains(name)),
        "package audit requires distinct known cleaning profiles"
    );
    let original_catalog = catalog(original)?;
    let mut variant_catalogs = Vec::with_capacity(variants.len());
    for (name, path) in variants {
        let candidate = catalog(path)?;
        if !candidate.parts.keys().eq(original_catalog.parts.keys())
            || original_catalog.parts.iter().any(|(part, before)| {
                let after = &candidate.parts[part];
                part != WORKBOOK && (before.bytes != after.bytes || before.crc32 != after.crc32)
            })
        {
            return Ok(None);
        }
        let before = &original_catalog.parts[WORKBOOK];
        let after = &candidate.parts[WORKBOOK];
        if before.bytes > MAX_WORKBOOK_BYTES
            && (before.bytes != after.bytes || before.crc32 != after.crc32)
        {
            return Ok(None);
        }
        variant_catalogs.push((*name, candidate));
    }

    // Cache is local to this audit and keyed by the complete raw archive hash.
    // Equal output archives therefore need decompression only once.
    let mut cache = BTreeMap::new();
    let baseline = package(&original_catalog, &mut cache)?;
    let mut compared = Vec::with_capacity(variants.len());
    for (name, candidate) in &variant_catalogs {
        compared.push((*name, package(candidate, &mut cache)?));
    }
    let workbook_changed = compared
        .iter()
        .any(|(_, candidate)| candidate.parts[WORKBOOK] != baseline.parts[WORKBOOK]);
    let cleanup = if workbook_changed {
        if original_catalog.parts[WORKBOOK].bytes > MAX_WORKBOOK_BYTES
            || original_catalog
                .parts
                .keys()
                .any(|name| name.starts_with("_xmlsignatures/"))
        {
            return Ok(None);
        }
        redundant_defined_names(&read_workbook(&original_catalog)?)?
    } else {
        None
    };
    let mut profiles = Vec::with_capacity(variants.len());
    let mut any_cleanup = false;
    for (name, candidate) in &compared {
        let changed: Vec<_> = baseline
            .parts
            .iter()
            .filter(|(part, before)| candidate.parts.get(*part) != Some(*before))
            .map(|(part, _)| part.clone())
            .collect();
        let details = if changed.is_empty() {
            Value::Null
        } else if changed == [WORKBOOK] {
            let Some(cleanup) = &cleanup else {
                return Ok(None);
            };
            if candidate.parts[WORKBOOK] != cleanup.cleaned {
                return Ok(None);
            }
            any_cleanup = true;
            json!({
                "method":"remove only proven redundant built-in defined names; all other bytes unchanged",
                "removed_defined_names":cleanup.removed,
                "before":baseline.parts[WORKBOOK],"after":cleanup.cleaned,
            })
        } else {
            return Ok(None);
        };
        profiles.push(json!({
            "profile":name,"archive_sha256":candidate.archive_sha256,
            "parts_sha256":parts_hash(&candidate.parts)?,
            "equivalence":if changed.is_empty() {"identical_parts"} else {"redundant_defined_names_only"},
            "changed_parts":changed,"cleanup":details,
        }));
    }
    // A file changed after its scan (including an alias of a previously scanned
    // archive) cannot borrow that earlier certificate.
    unchanged(original, &baseline.archive_sha256)?;
    for ((_, path), (_, candidate)) in variants.iter().zip(&compared) {
        unchanged(path, &candidate.archive_sha256)?;
    }
    Ok(Some(json!({
        "version":1,"status":"equivalent",
        "method":"streamed SHA-256 and length of every uncompressed ZIP member",
        "equivalence":if any_cleanup {"redundant_defined_names_only"} else {"identical_parts"},
        "evidence_scope":"package contents; no Excel opening or visual measurement",
        "original":{"archive_sha256":baseline.archive_sha256,
            "parts_sha256":parts_hash(&baseline.parts)?,"parts":&*baseline.parts},
        "profiles":profiles,"distinct_archives_scanned":cache.len(),
    })))
}

fn catalog(path: &Path) -> Result<Catalog> {
    let mut archive = ZipArchive::new(File::open(path)?)
        .with_context(|| format!("read package catalog {}", path.display()))?;
    ensure!(
        archive.len() > 0 && archive.len() <= MAX_PARTS,
        "package must contain between 1 and {MAX_PARTS} members"
    );
    let mut parts = BTreeMap::new();
    let mut name_bytes = 0usize;
    for index in 0..archive.len() {
        let member = archive.by_index_raw(index)?;
        let name = std::str::from_utf8(member.name_raw())?;
        name_bytes = name_bytes
            .checked_add(name.len())
            .context("ZIP member names overflow")?;
        ensure!(
            name_bytes <= MAX_MEMBER_NAME_BYTES,
            "ZIP member names exceed audit memory limit"
        );
        ensure!(
            !name.is_empty()
                && name.len() <= 4096
                && !name.starts_with('/')
                && !name.contains(['\\', '\0', ':'])
                && name
                    .trim_end_matches('/')
                    .split('/')
                    .all(|part| { !part.is_empty() && part != "." && part != ".." }),
            "ambiguous ZIP member name {name:?}"
        );
        ensure!(
            parts
                .insert(
                    name.to_owned(),
                    Header {
                        index,
                        bytes: member.size(),
                        crc32: member.crc32(),
                    }
                )
                .is_none(),
            "duplicate ZIP member {name:?}"
        );
    }
    ensure!(
        parts.contains_key(WORKBOOK) && parts.contains_key("[Content_Types].xml"),
        "package is missing workbook or content types"
    );
    Ok(Catalog {
        path: path.into(),
        parts,
    })
}

fn package(
    catalog: &Catalog,
    cache: &mut BTreeMap<String, Arc<BTreeMap<String, PartDigest>>>,
) -> Result<Package> {
    let archive_sha256 = artifact_cache::hash_file(&catalog.path)?;
    let parts = if let Some(parts) = cache.get(&archive_sha256) {
        Arc::clone(parts)
    } else {
        let mut archive = ZipArchive::new(File::open(&catalog.path)?)?;
        ensure!(
            archive.len() == catalog.parts.len(),
            "package changed during audit"
        );
        let mut parts = BTreeMap::new();
        for (name, header) in &catalog.parts {
            let mut member = archive.by_index(header.index)?;
            ensure!(
                member.name_raw() == name.as_bytes()
                    && member.size() == header.bytes
                    && member.crc32() == header.crc32,
                "package member changed during audit: {name}"
            );
            let part = digest_reader(&mut member, header.bytes)
                .with_context(|| format!("hash ZIP member {name}"))?;
            parts.insert(name.clone(), part);
        }
        unchanged(&catalog.path, &archive_sha256)?;
        let parts = Arc::new(parts);
        cache.insert(archive_sha256.clone(), Arc::clone(&parts));
        parts
    };
    ensure!(
        parts.keys().eq(catalog.parts.keys()),
        "package catalog changed during audit"
    );
    Ok(Package {
        archive_sha256,
        parts,
    })
}

fn digest_reader(reader: &mut impl Read, expected: u64) -> Result<PartDigest> {
    let mut hash = Sha256::new();
    let mut buffer = [0u8; BUFFER_BYTES];
    let mut bytes = 0u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .context("ZIP member length overflow")?;
        ensure!(bytes <= expected, "ZIP member exceeds its declared length");
        hash.update(&buffer[..count]);
    }
    ensure!(
        bytes == expected,
        "ZIP member is shorter than its declared length"
    );
    Ok(PartDigest {
        bytes,
        sha256: artifact_cache::hex(&hash.finalize()),
    })
}

fn unchanged(path: &Path, expected: &str) -> Result<()> {
    ensure!(
        artifact_cache::hash_file(path)? == expected,
        "package changed during audit: {}",
        path.display()
    );
    Ok(())
}

fn parts_hash(parts: &BTreeMap<String, PartDigest>) -> Result<String> {
    Ok(artifact_cache::sha256(&serde_json::to_vec(parts)?))
}

fn read_workbook(catalog: &Catalog) -> Result<Vec<u8>> {
    let header = &catalog.parts[WORKBOOK];
    ensure!(
        header.bytes <= MAX_WORKBOOK_BYTES,
        "workbook XML exceeds exception limit"
    );
    let mut archive = ZipArchive::new(File::open(&catalog.path)?)?;
    let mut member = archive.by_index(header.index)?;
    ensure!(
        member.name_raw() == WORKBOOK.as_bytes(),
        "workbook entry changed"
    );
    let mut bytes = Vec::new();
    member
        .by_ref()
        .take(MAX_WORKBOOK_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == header.bytes,
        "workbook XML changed during audit"
    );
    Ok(bytes)
}

struct Cleanup {
    cleaned: PartDigest,
    removed: Vec<Value>,
}

struct DefinedName<'a> {
    name: &'a [u8],
    scope: Option<&'a [u8]>,
    attributes: BTreeMap<&'a [u8], &'a [u8]>,
    body: &'a [u8],
    range: Range<usize>,
}

/// This is intentionally narrower than core cleanup, which also removes aliases
/// whose formulas differ. Such a change cannot receive an identity certificate.
fn redundant_defined_names(xml: &[u8]) -> Result<Option<Cleanup>> {
    use xml_views::{Attributes, Kind, Reader, TextBytes};
    if xml.len() as u64 > MAX_WORKBOOK_BYTES || std::str::from_utf8(xml).is_err() {
        return Ok(None);
    }
    let mut reader = Reader::<128>::new(xml)?;
    let mut names = BTreeMap::new();
    let mut identities = BTreeSet::new();
    let mut root_seen = false;
    let mut group_seen = false;
    let mut in_group = false;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End {
            if tag.depth == 1 && tag.name == b"definedNames" {
                in_group = false;
            }
            continue;
        }
        let mut attributes = BTreeMap::new();
        for item in Attributes::new(tag.attrs) {
            let (name, value) = item?;
            ensure!(
                attributes.insert(name, value).is_none(),
                "duplicate XML attribute"
            );
        }
        if tag.depth == 0 {
            if root_seen || tag.name != b"workbook" {
                return Ok(None);
            }
            root_seen = true;
            if !matches!(
                attributes.get(b"xmlns".as_slice()).copied(),
                Some(
                    b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                        | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                )
            ) {
                return Ok(None);
            }
        }
        if tag.depth == 1 && tag.name == b"definedNames" {
            if group_seen || !attributes.is_empty() {
                return Ok(None);
            }
            group_seen = true;
            in_group = tag.kind == Kind::Start;
        } else if in_group && tag.depth == 2 {
            if tag.name != b"definedName"
                || attributes.keys().any(|name| name.starts_with(b"xmlns"))
            {
                return Ok(None);
            }
            let Some(name) = attributes.remove(b"name".as_slice()) else {
                return Ok(None);
            };
            // Built-in identifiers are ASCII. Encoded or case-ambiguous aliases
            // must not hide a second definition with the same effective name.
            if !name.is_ascii() || name.contains(&b'&') {
                return Ok(None);
            }
            let scope = attributes.get(b"localSheetId".as_slice()).copied();
            if scope.is_some_and(|scope| scope.is_empty() || !scope.iter().all(u8::is_ascii_digit))
            {
                return Ok(None);
            }
            let scope_number = scope.map(xml_views::decimal).transpose()?;
            if !identities.insert((name.to_ascii_lowercase(), scope_number)) {
                return Ok(None);
            }
            let element = reader.element(tag)?;
            if element.body.contains(&b'<') {
                return Ok(None);
            }
            for byte in TextBytes::new(element.body) {
                byte?;
            }
            let entry = DefinedName {
                name,
                scope,
                attributes,
                body: element.body,
                range: tag.start..tag.start + element.raw.len(),
            };
            if names.insert((name, scope), entry).is_some() {
                return Ok(None);
            }
        }
    }
    if !root_seen {
        return Ok(None);
    }
    let mut removals = Vec::new();
    for entry in names.values() {
        let retained_name: &[u8] = match entry.name {
            b"Print_Area" => b"_xlnm.Print_Area",
            b"Print_Titles" => b"_xlnm.Print_Titles",
            b"_FilterDatabase" => b"_xlnm._FilterDatabase",
            b"Database" => b"_xlnm.Database",
            _ => continue,
        };
        if let Some(retained) = names.get(&(retained_name, entry.scope)) {
            if entry.body == retained.body && entry.attributes == retained.attributes {
                removals.push((entry, retained_name));
            }
        }
    }
    if removals.is_empty() {
        return Ok(None);
    }
    removals.sort_by_key(|(entry, _)| entry.range.start);
    let mut hash = Sha256::new();
    let mut cursor = 0;
    let mut length = xml.len();
    let mut removed = Vec::new();
    for (entry, retained_name) in removals {
        hash.update(&xml[cursor..entry.range.start]);
        cursor = entry.range.end;
        length -= entry.range.len();
        removed.push(json!({
            "name":std::str::from_utf8(entry.name)?,
            "retained_name":std::str::from_utf8(retained_name)?,
            "local_sheet_id":entry.scope.map(std::str::from_utf8).transpose()?,
            "formula_sha256":artifact_cache::sha256(entry.body),
            "removed_bytes":entry.range.len(),
        }));
    }
    hash.update(&xml[cursor..]);
    Ok(Some(Cleanup {
        cleaned: PartDigest {
            bytes: length as u64,
            sha256: artifact_cache::hex(&hash.finalize()),
        },
        removed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};
    use zip::{write::FileOptions, CompressionMethod, DateTime, ZipWriter};

    const XML: &[u8] = br#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets/></workbook>"#;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Result<Self> {
            let path = artifact_cache::unique_path(&std::env::temp_dir(), "package-identity")?;
            fs::create_dir_all(&path)?;
            Ok(Self(path))
        }
        fn zip(
            &self,
            name: &str,
            entries: &[(String, Vec<u8>)],
            altered_container: bool,
        ) -> Result<PathBuf> {
            let path = self.0.join(name);
            let mut writer = ZipWriter::new(File::create(&path)?);
            let options = FileOptions::default()
                .compression_method(if altered_container {
                    CompressionMethod::Deflated
                } else {
                    CompressionMethod::Stored
                })
                .last_modified_time(
                    DateTime::from_date_and_time(
                        if altered_container { 2024 } else { 2020 },
                        1,
                        1,
                        0,
                        0,
                        0,
                    )
                    .unwrap(),
                );
            let mut entries = entries.iter().collect::<Vec<_>>();
            if altered_container {
                entries.reverse();
            }
            for (name, bytes) in entries {
                writer.start_file(name, options)?;
                writer.write_all(bytes)?;
            }
            writer.finish()?;
            Ok(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn entries(workbook: &[u8]) -> Vec<(String, Vec<u8>)> {
        [
            ("[Content_Types].xml", b"<Types/>".as_slice()),
            (WORKBOOK, workbook),
            (
                "xl/worksheets/sheet1.xml",
                b"<worksheet><v>1</v></worksheet>",
            ),
            ("xl/styles.xml", b"<styleSheet/>"),
            ("xl/sharedStrings.xml", b"<sst/>"),
            ("xl/calcChain.xml", b"<calcChain/>"),
            ("xl/_rels/workbook.xml.rels", b"<Relationships/>"),
            ("xl/media/image1.png", b"image bytes"),
        ]
        .into_iter()
        .map(|(name, data)| (name.into(), data.into()))
        .collect()
    }
    fn variants(path: &Path) -> Vec<(&'static str, PathBuf)> {
        super::super::PROFILES
            .iter()
            .map(|&name| (name, path.into()))
            .collect()
    }

    #[test]
    fn reordered_recompressed_packages_are_equal_and_identical_outputs_scan_once() -> Result<()> {
        let directory = Directory::new()?;
        let original = directory.zip("original.xlsx", &entries(XML), false)?;
        let output = directory.zip("output.xlsx", &entries(XML), true)?;
        let result = compare(&original, &variants(&output))?.unwrap();
        assert_eq!(result["status"], "equivalent");
        assert_eq!(result["equivalence"], "identical_parts");
        assert_eq!(result["distinct_archives_scanned"], 2);
        assert_eq!(result["original"]["parts"].as_object().unwrap().len(), 8);
        assert!(result["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["changed_parts"] == json!([])));
        Ok(())
    }

    #[test]
    fn every_member_and_selected_profiles_must_match() -> Result<()> {
        let directory = Directory::new()?;
        let original_entries = entries(XML);
        let original = directory.zip("original.xlsx", &original_entries, false)?;
        for index in 0..original_entries.len() {
            let mut changed = original_entries.clone();
            changed[index].1.push(b'!');
            let output = directory.zip("changed.xlsx", &changed, false)?;
            let mut candidates = variants(&original);
            candidates[2].1 = output;
            assert!(
                compare(&original, &candidates)?.is_none(),
                "accepted change to {}",
                changed[index].0
            );
        }
        for missing in [true, false] {
            let mut changed = original_entries.clone();
            if missing {
                changed.pop();
            } else {
                changed.push(("extra.xml".into(), b"extra".to_vec()));
            }
            let output = directory.zip("changed.xlsx", &changed, false)?;
            assert!(compare(&original, &variants(&output))?.is_none());
        }
        assert_eq!(compare(&original, &variants(&original)[..2])?.unwrap()["profiles"].as_array().unwrap().len(), 2);
        assert!(compare(&original, &[]).is_err());
        let mut duplicate = variants(&original);
        duplicate[2].0 = duplicate[1].0;
        assert!(compare(&original, &duplicate).is_err());
        Ok(())
    }

    #[test]
    fn equal_crc_and_size_are_not_a_positive_certificate() -> Result<()> {
        let directory = Directory::new()?;
        let mut before = entries(XML);
        let mut after = before.clone();
        before.last_mut().unwrap().1 = b"plumless".to_vec();
        after.last_mut().unwrap().1 = b"buckeroo".to_vec();
        assert_eq!(crc32fast::hash(b"plumless"), crc32fast::hash(b"buckeroo"));
        let original = directory.zip("original.xlsx", &before, false)?;
        let output = directory.zip("output.xlsx", &after, false)?;
        assert!(compare(&original, &variants(&output))?.is_none());
        Ok(())
    }

    #[test]
    fn duplicate_corrupt_and_mutated_archives_cannot_be_certified() -> Result<()> {
        let directory = Directory::new()?;
        let mut duplicated = entries(XML);
        duplicated.push(duplicated[2].clone());
        let path = directory.zip("duplicate.xlsx", &duplicated, false)?;
        assert!(compare(&path, &variants(&path)).is_err());
        let original = directory.zip("original.xlsx", &entries(XML), false)?;
        let expected = artifact_cache::hash_file(&original)?;
        let mut corrupt = fs::read(&original)?;
        let offset = corrupt
            .windows(b"image bytes".len())
            .position(|bytes| bytes == b"image bytes")
            .unwrap();
        corrupt[offset] ^= 1;
        fs::write(&original, corrupt)?;
        assert!(unchanged(&original, &expected).is_err());
        assert!(compare(&original, &variants(&original)).is_err());
        fs::write(&original, b"not a ZIP archive")?;
        assert!(compare(&original, &variants(&original)).is_err());
        Ok(())
    }

    #[test]
    fn streaming_digest_never_requests_a_large_member_buffer() -> Result<()> {
        struct LargeMember(u64);
        impl Read for LargeMember {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                assert!(buffer.len() <= BUFFER_BYTES);
                let count = buffer.len().min(self.0 as usize);
                buffer[..count].fill(42);
                self.0 -= count as u64;
                Ok(count)
            }
        }
        let bytes = 3 * BUFFER_BYTES as u64 + 7;
        assert_eq!(digest_reader(&mut LargeMember(bytes), bytes)?.bytes, bytes);
        assert!(digest_reader(&mut LargeMember(2), 1).is_err());
        assert!(digest_reader(&mut LargeMember(1), 2).is_err());
        Ok(())
    }

    #[test]
    fn only_equal_builtin_aliases_with_unchanged_surrounding_bytes_are_allowed() -> Result<()> {
        let directory = Directory::new()?;
        let duplicate =
            r#"<definedName name="Print_Area" localSheetId="0">Data!$A$1</definedName>"#;
        let retained =
            r#"<definedName name="_xlnm.Print_Area" localSheetId="0">Data!$A$1</definedName>"#;
        let make = |left: &str, right: &str| {
            format!(
                r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><definedNames>{left}{right}</definedNames><calcPr calcId="1"/></workbook>"#
            )
        };
        let original_xml = make(duplicate, retained);
        let original = directory.zip("original.xlsx", &entries(original_xml.as_bytes()), false)?;
        let cleaned_xml = make("", retained);
        let output = directory.zip("output.xlsx", &entries(cleaned_xml.as_bytes()), true)?;
        let result = compare(&original, &variants(&output))?.unwrap();
        assert_eq!(result["equivalence"], "redundant_defined_names_only");
        assert_eq!(result["profiles"][0]["changed_parts"], json!([WORKBOOK]));
        assert_eq!(
            result["profiles"][0]["cleanup"]["removed_defined_names"][0]["name"],
            "Print_Area"
        );
        for unsafe_duplicate in [
            duplicate.replace("$A$1", "$B$1"),
            duplicate.replace("localSheetId=\"0\"", "localSheetId=\"1\""),
            duplicate.replace("localSheetId", "hidden=\"1\" localSheetId"),
            duplicate.replace("Print_Area", "CustomName"),
        ] {
            let before = make(&unsafe_duplicate, retained);
            let original = directory.zip("unsafe.xlsx", &entries(before.as_bytes()), false)?;
            assert!(compare(&original, &variants(&output))?.is_none());
        }
        let altered = cleaned_xml.replace("calcId=\"1\"", "calcId=\"2\"");
        let output = directory.zip("altered.xlsx", &entries(altered.as_bytes()), false)?;
        assert!(compare(&original, &variants(&output))?.is_none());
        Ok(())
    }

    #[test]
    fn oversized_workbook_exception_is_a_conservative_miss() -> Result<()> {
        let directory = Directory::new()?;
        let xml = vec![b' '; MAX_WORKBOOK_BYTES as usize + 1];
        let original = directory.zip("original.xlsx", &entries(&xml), true)?;
        let output = directory.zip("output.xlsx", &entries(XML), true)?;
        assert!(compare(&original, &variants(&output))?.is_none());
        Ok(())
    }
}
