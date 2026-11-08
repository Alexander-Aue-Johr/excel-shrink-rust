//! User-selected OOXML evidence for reusing an earlier visual observation.
//! This is an evaluation policy, never a new claim of visual equivalence.

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::Path,
};

/// Selected package contents whose identity is required to reuse an observation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PartSelection {
    pub worksheets: bool,
    pub workbook: bool,
    pub styles: bool,
    pub shared_strings: bool,
    pub relationships: bool,
    pub drawings: bool,
    pub charts: bool,
    pub other: bool,
    pub extra_parts: Vec<String>,
}

impl Default for PartSelection {
    fn default() -> Self {
        Self {
            worksheets: true,
            workbook: true,
            styles: true,
            shared_strings: true,
            relationships: true,
            drawings: true,
            charts: true,
            other: true,
            extra_parts: Vec::new(),
        }
    }
}

impl PartSelection {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.flags().iter().any(|flag| *flag) || !self.extra_parts.is_empty(),
            "Select at least one OOXML component for result reuse"
        );
        for name in &self.extra_parts {
            validate_part_name(name)?;
        }
        Ok(())
    }

    /// Categories partition package members. Explicit names additionally select one member.
    pub fn includes(&self, name: &str) -> bool {
        if self.extra_parts.iter().any(|extra| extra == name) {
            return true;
        }
        if name.ends_with(".rels") {
            self.relationships
        } else if name.starts_with("xl/worksheets/") && name.ends_with(".xml") {
            self.worksheets
        } else if name == "xl/workbook.xml" {
            self.workbook
        } else if name == "xl/styles.xml" {
            self.styles
        } else if name == "xl/sharedStrings.xml" {
            self.shared_strings
        } else if name.starts_with("xl/drawings/") || name.starts_with("xl/media/") {
            self.drawings
        } else if name.starts_with("xl/charts/") || name.starts_with("xl/chartsheets/") {
            self.charts
        } else {
            self.other
        }
    }

    /// Old evidence may cover more parts, but never fewer, than the new request.
    fn covers(&self, requested: &Self) -> bool {
        self.flags()
            .iter()
            .zip(requested.flags())
            .all(|(old, new)| !new || *old)
            && requested.extra_parts.iter().all(|name| self.includes(name))
    }

    fn flags(&self) -> [bool; 8] {
        [
            self.worksheets,
            self.workbook,
            self.styles,
            self.shared_strings,
            self.relationships,
            self.drawings,
            self.charts,
            self.other,
        ]
    }
}

/// Digest of one uncompressed package member, independent of ZIP compression.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PartDigest {
    pub bytes: u64,
    pub sha256: String,
}

/// Versioned content identities for both partners of an earlier comparison.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PairFingerprint {
    pub schema: u32,
    pub selection: PartSelection,
    pub original: BTreeMap<String, PartDigest>,
    pub candidate: BTreeMap<String, PartDigest>,
}

impl PairFingerprint {
    /// Read selected uncompressed member contents once; ZIP metadata and order are irrelevant.
    pub fn read(original: &Path, candidate: &Path, selection: &PartSelection) -> Result<Self> {
        selection.validate()?;
        Ok(Self {
            schema: 1,
            selection: selection.clone(),
            original: fingerprint(original, selection)?,
            candidate: fingerprint(candidate, selection)?,
        })
    }

    /// Both sides must still match the earlier observation under the current selection.
    pub fn matches_previous(&self, previous: &Self) -> bool {
        self.schema == previous.schema
            && previous.selection.covers(&self.selection)
            && selected_equal(&self.original, &previous.original, &self.selection)
            && selected_equal(&self.candidate, &previous.candidate, &self.selection)
    }
}

fn selected_equal(
    current: &BTreeMap<String, PartDigest>,
    previous: &BTreeMap<String, PartDigest>,
    selection: &PartSelection,
) -> bool {
    current
        .iter()
        .filter(|(name, _)| selection.includes(name))
        .eq(previous.iter().filter(|(name, _)| selection.includes(name)))
}

fn fingerprint(path: &Path, selection: &PartSelection) -> Result<BTreeMap<String, PartDigest>> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)
        .with_context(|| format!("Open OOXML package {}", path.display()))?;
    let mut names = BTreeSet::new();
    let mut parts = BTreeMap::new();
    let mut buffer = [0_u8; 128 * 1024];
    for index in 0..archive.len() {
        let mut member = archive.by_index(index)?;
        let name = member.name().to_owned();
        ensure!(names.insert(name.clone()), "Duplicate OOXML member: {name}");
        if member.is_dir() {
            continue;
        }
        validate_part_name(&name)?;
        if !selection.includes(&name) {
            continue;
        }
        let mut hash = Sha256::new();
        let mut bytes = 0_u64;
        loop {
            let count = member.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            hash.update(&buffer[..count]);
        }
        ensure!(bytes == member.size(), "Truncated OOXML member: {name}");
        parts.insert(
            name,
            PartDigest {
                bytes,
                sha256: format!("{:x}", hash.finalize()),
            },
        );
    }
    Ok(parts)
}

fn validate_part_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && !name.contains(['\\', ':'])
            && !name.chars().any(char::is_control)
            && name.split('/').all(|part| !matches!(part, "" | "." | "..")),
        "Invalid OOXML part name: {name:?}"
    );
    Ok(())
}

/// Streaming file digest used for process artifacts and provenance, not visual reuse decisions.
pub fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write, path::PathBuf};
    use zip::{write::FileOptions, CompressionMethod, ZipWriter};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("sheet-scope-reuse-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn workbook(&self, name: &str, parts: &[(&str, &str)], stored: bool) -> PathBuf {
            let path = self.0.join(name);
            let mut zip = ZipWriter::new(File::create(&path).unwrap());
            let method = if stored {
                CompressionMethod::Stored
            } else {
                CompressionMethod::Deflated
            };
            for (name, bytes) in parts {
                zip.start_file(*name, FileOptions::default().compression_method(method))
                    .unwrap();
                zip.write_all(bytes.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sheets_only() -> PartSelection {
        PartSelection {
            worksheets: true,
            workbook: false,
            styles: false,
            shared_strings: false,
            relationships: false,
            drawings: false,
            charts: false,
            other: false,
            extra_parts: Vec::new(),
        }
    }

    #[test]
    fn ignores_zip_order_compression_and_unselected_changes() -> Result<()> {
        let fixture = Fixture::new();
        let a = fixture.workbook(
            "a.xlsx",
            &[
                ("xl/worksheets/sheet2.xml", "second"),
                ("xl/worksheets/sheet1.xml", "first"),
                ("xl/styles.xml", "old style"),
            ],
            true,
        );
        let b = fixture.workbook(
            "b.xlsx",
            &[
                ("xl/styles.xml", "new style"),
                ("xl/worksheets/sheet1.xml", "first"),
                ("xl/worksheets/sheet2.xml", "second"),
            ],
            false,
        );
        assert_ne!(file_sha256(&a)?, file_sha256(&b)?);
        let old = PairFingerprint::read(&a, &a, &PartSelection::default())?;
        let current = PairFingerprint::read(&a, &b, &sheets_only())?;
        assert!(current.matches_previous(&old));
        assert!(!PairFingerprint::read(&a, &b, &PartSelection::default())?.matches_previous(&old));
        Ok(())
    }

    #[test]
    fn checks_both_sides_missing_parts_and_previous_coverage() -> Result<()> {
        let fixture = Fixture::new();
        let a = fixture.workbook(
            "a.xlsx",
            &[
                ("xl/worksheets/sheet1.xml", "a"),
                ("xl/worksheets/sheet2.xml", "b"),
                ("xl/styles.xml", "style"),
            ],
            true,
        );
        let b = fixture.workbook(
            "b.xlsx",
            &[
                ("xl/worksheets/sheet1.xml", "changed"),
                ("xl/styles.xml", "style"),
            ],
            true,
        );
        let old = PairFingerprint::read(&a, &a, &sheets_only())?;
        assert!(!PairFingerprint::read(&b, &a, &sheets_only())?.matches_previous(&old));
        assert!(!PairFingerprint::read(&a, &b, &sheets_only())?.matches_previous(&old));
        assert!(!PairFingerprint::read(&a, &a, &PartSelection::default())?.matches_previous(&old));
        Ok(())
    }

    #[test]
    fn rejects_duplicate_members_and_empty_selection() {
        let fixture = Fixture::new();
        let duplicate = fixture.workbook(
            "duplicate.xlsx",
            &[
                ("xl/worksheets/sheet1.xml", "a"),
                ("xl/worksheets/sheet1.xml", "b"),
            ],
            true,
        );
        assert!(fingerprint(&duplicate, &sheets_only()).is_err());
        let mut empty = sheets_only();
        empty.worksheets = false;
        assert!(empty.validate().is_err());
    }

    #[test]
    fn absent_optional_components_are_recorded_as_absent() -> Result<()> {
        let fixture = Fixture::new();
        let inline = fixture.workbook(
            "inline.xlsx",
            &[("xl/worksheets/sheet1.xml", "inline strings")],
            true,
        );
        let shared = fixture.workbook(
            "shared.xlsx",
            &[
                ("xl/worksheets/sheet1.xml", "inline strings"),
                ("xl/sharedStrings.xml", "new shared strings"),
            ],
            true,
        );
        let mut selection = sheets_only();
        selection.worksheets = false;
        selection.shared_strings = true;
        let old = PairFingerprint::read(&inline, &inline, &selection)?;
        assert!(PairFingerprint::read(&inline, &inline, &selection)?.matches_previous(&old));
        assert!(!PairFingerprint::read(&inline, &shared, &selection)?.matches_previous(&old));
        Ok(())
    }
}
