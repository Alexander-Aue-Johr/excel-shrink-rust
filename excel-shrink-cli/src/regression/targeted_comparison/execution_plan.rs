//! Proofs for omitting redundant input states and calculation-only controls.
use super::{telemetry::LaneId, xml_changes::Snapshot};
use crate::regression::{
    artifact_cache,
    streaming_xml::{self, Event},
    xml_views::{Kind, Tag},
};
use anyhow::{ensure, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs::File, io::Read, path::Path};
use zip::ZipArchive;

#[derive(Clone, Default, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum InputIdentity {
    #[default]
    NotChecked,
    Different,
    Identical {
        bytes: u64,
        sha256: String,
    },
}

impl InputIdentity {
    /// Compare owned raw input bytes before Excel opens either file. ZIP metadata
    /// equivalence alone never authorizes removal of the Default observation.
    pub fn check(original: &Path, default: &Path) -> Result<Self> {
        let mut left = File::open(original)?;
        let mut right = File::open(default)?;
        let bytes = left.metadata()?.len();
        if bytes != right.metadata()?.len() {
            return Ok(Self::Different);
        }
        let mut a = [0; 64 * 1024];
        let mut b = [0; 64 * 1024];
        let mut digest = Sha256::new();
        let mut remaining = bytes;
        while remaining > 0 {
            let count = remaining.min(a.len() as u64) as usize;
            left.read_exact(&mut a[..count])?;
            right.read_exact(&mut b[..count])?;
            if a[..count] != b[..count] {
                return Ok(Self::Different);
            }
            digest.update(&a[..count]);
            remaining -= count as u64;
        }
        ensure!(
            left.read(&mut a[..1])? == 0 && right.read(&mut b[..1])? == 0,
            "Inputs changed during identity check"
        );
        Ok(Self::Identical {
            bytes,
            sha256: format!("{:x}", digest.finalize()),
        })
    }

    pub fn identical(&self) -> bool {
        matches!(self, Self::Identical { .. })
    }
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum StateDecision {
    Observed { method: &'static str },
    SkippedIdenticalInput { source: LaneId },
    SkippedNoRebuild { source: LaneId },
    Disabled,
}

#[derive(Clone, Serialize)]
pub(super) struct RebuildCheck {
    pub source_sha256: String,
    pub blockers: BTreeSet<String>,
    pub checked_worksheets: usize,
    pub eligible_to_skip: bool,
    pub policy: &'static str,
}

/// Reuse worksheet facts from the existing byte stream. Unknown markup keeps
/// the rebuild path; neither cached cell values nor a lack of captures is proof.
pub(super) fn worksheet_blocker(tag: Tag<'_>) -> Option<&'static str> {
    if tag.kind == Kind::End {
        return None;
    }
    if matches!(tag.local(), b"f" | b"formula" | b"formula1" | b"formula2") {
        return Some("worksheet formula");
    }
    if matches!(tag.local(), b"extLst" | b"ext" | b"AlternateContent") {
        return Some("unsupported worksheet extension");
    }
    if tag.depth == 1
        && !matches!(
            tag.name,
            b"sheetPr"
                | b"dimension"
                | b"sheetViews"
                | b"sheetFormatPr"
                | b"cols"
                | b"sheetData"
                | b"sheetProtection"
                | b"protectedRanges"
                | b"mergeCells"
                | b"phoneticPr"
                | b"hyperlinks"
                | b"printOptions"
                | b"pageMargins"
                | b"pageSetup"
                | b"headerFooter"
                | b"rowBreaks"
                | b"colBreaks"
                | b"ignoredErrors"
        )
    {
        return Some(
            "conditional formatting, filtering, objects, tables or unsupported worksheet markup",
        );
    }
    None
}

/// Analyze each saved state independently, including Check Performance output.
/// Worksheet bodies are not inflated again: the snapshot already retained the
/// dependency facts. A strict package allowlist guards other calculation sources.
pub(super) fn check_rebuild(path: &Path, snapshot: &Snapshot) -> Result<RebuildCheck> {
    let mut blockers = BTreeSet::new();
    if snapshot.sheets.is_empty() {
        blockers.insert("no ordinary worksheet inventory".into());
    }
    for (name, sheet) in &snapshot.sheets {
        if !sheet.value_namespace_supported {
            blockers.insert(format!("{name}: unsupported worksheet namespace"));
        }
        for reason in &sheet.rebuild_blockers {
            blockers.insert(format!("{name}: {reason}"));
        }
    }
    let mut archive = ZipArchive::new(File::open(path)?)?;
    let mut seen = BTreeSet::new();
    let mut worksheet_parts = 0;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        if !seen.insert(name.clone()) {
            blockers.insert(format!("duplicate package part: {name}"));
        }
        if name == "xl/workbook.xml" {
            if let Err(error) = inspect_metadata(entry, b"workbook", &name, &mut blockers) {
                blockers.insert(format!("{name}: metadata could not be verified: {error}"));
            }
        } else if name == "_rels/.rels"
            || name == "xl/_rels/workbook.xml.rels"
            || name.starts_with("xl/worksheets/_rels/") && name.ends_with(".rels")
        {
            if let Err(error) = inspect_metadata(entry, b"Relationships", &name, &mut blockers) {
                blockers.insert(format!(
                    "{name}: relationships could not be verified: {error}"
                ));
            }
        } else if name.starts_with("xl/worksheets/")
            && !name["xl/worksheets/".len()..].contains('/')
            && name.ends_with(".xml")
        {
            worksheet_parts += 1;
        } else if name == "[Content_Types].xml"
            || matches!(
                name.as_str(),
                "docProps/app.xml"
                    | "docProps/core.xml"
                    | "docProps/custom.xml"
                    | "xl/styles.xml"
                    | "xl/sharedStrings.xml"
            )
            || name.starts_with("xl/theme/") && name.ends_with(".xml")
        {
            // Static properties, display tables, themes, and already-scanned worksheets.
        } else {
            blockers.insert(format!(
                "unsupported or calculation-dependent package part: {name}"
            ));
        }
    }
    if worksheet_parts != snapshot.sheets.len() {
        blockers.insert("worksheet inventory is incomplete".into());
    }
    Ok(RebuildCheck {
        source_sha256: artifact_cache::hash_file(path)?,
        checked_worksheets: snapshot.sheets.len(),
        eligible_to_skip: blockers.is_empty(),
        blockers,
        policy: "static-workbook-rebuild-v1",
    })
}

fn inspect_metadata(
    input: impl Read,
    root: &[u8],
    part: &str,
    blockers: &mut BTreeSet<String>,
) -> Result<()> {
    let mut static_metadata = super::static_workbook_metadata::WorkbookMetadata::default();
    streaming_xml::scan(input, |event| {
        let Event::Tag(tag) = event else {
            return Ok(());
        };
        if root == b"workbook" {
            if !static_metadata.accepts(tag)? {
                blockers.insert(format!(
                    "{part}: defined names, references or unsupported workbook metadata"
                ));
            }
        }
        if tag.kind == Kind::End {
            return Ok(());
        }
        if tag.depth == 0 {
            let namespace = if root == b"workbook" {
                b"http://schemas.openxmlformats.org/spreadsheetml/2006/main".as_slice()
            } else {
                b"http://schemas.openxmlformats.org/package/2006/relationships".as_slice()
            };
            if tag.name != root || tag.attr(b"xmlns")? != Some(namespace) {
                blockers.insert(format!("{part}: unsupported metadata namespace"));
            }
        } else if root == b"workbook" {
            // The metadata recognizer above already checked this subtree.
        } else if tag.name == b"Relationship" {
            let kind = tag.attr(b"Type")?.unwrap_or_default();
            let allowed = [
                b"/officeDocument".as_slice(),
                b"/worksheet",
                b"/styles",
                b"/sharedStrings",
                b"/theme",
                b"/hyperlink",
                b"/metadata/core-properties",
                b"/extended-properties",
                b"/custom-properties",
            ];
            if !allowed.iter().any(|suffix| kind.ends_with(suffix))
                || tag.attr(b"TargetMode")? == Some(b"External") && !kind.ends_with(b"/hyperlink")
            {
                blockers.insert(format!(
                    "{part}: calculation-dependent or unknown relationship"
                ));
            }
        } else {
            blockers.insert(format!("{part}: unknown relationship markup"));
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regression::xml_views::Reader;

    #[test]
    fn identity_requires_equal_bytes_even_when_lengths_match() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "input-identity-proof")?;
        std::fs::create_dir_all(&root)?;
        let a = root.join("a.xlsx");
        let b = root.join("b.xlsx");
        std::fs::write(&a, b"same bytes")?;
        std::fs::write(&b, b"same bytes")?;
        let proof = InputIdentity::check(&a, &b)?;
        assert!(proof.identical());
        assert_eq!(
            serde_json::to_value(proof)?["sha256"],
            artifact_cache::hash_file(&a)?
        );
        std::fs::write(&b, b"same byte!")?;
        assert!(!InputIdentity::check(&a, &b)?.identical());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    fn workbook(path: &Path, cell: &str, extra: Option<&str>, names: &str) -> Result<()> {
        use std::io::Write;
        let mut archive = zip::ZipWriter::new(File::create(path)?);
        let book = format!(
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" r:id="one"/></sheets>{names}</workbook>"#
        );
        let sheet = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1">{cell}</c></row></sheetData></worksheet>"#
        );
        for (name, xml) in [
            ("xl/workbook.xml", book.as_str()),
            ("xl/_rels/workbook.xml.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="one" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#),
            ("xl/worksheets/sheet1.xml", sheet.as_str()),
        ].into_iter().chain(extra.map(|part| (part, "<extra/>"))) {
            archive.start_file(name, zip::write::FileOptions::default())?;
            archive.write_all(xml.as_bytes())?;
        }
        archive.finish()?;
        Ok(())
    }

    #[test]
    fn static_package_proof_rejects_formulas_names_and_unknown_parts() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "static-rebuild-proof")?;
        std::fs::create_dir_all(&root)?;
        let path = root.join("input.xlsx");
        for (cell, part, names, skip) in [
            ("<v>1</v>", None, "<definedNames/>", true),
            ("<f>NOW()</f><v>1</v>", None, "", false),
            ("<v>1</v>", Some("xl/connections.xml"), "", false),
            ("<v>1</v>", Some("xl/charts/chart1.xml"), "", false),
            ("<v>1</v>", Some("xl/futureFeature.xml"), "", false),
            (
                "<v>1</v>",
                None,
                "<definedNames><definedName name=\"Input\">Data!A1</definedName></definedNames>",
                false,
            ),
        ] {
            workbook(&path, cell, part, names)?;
            let snapshot = Snapshot::read_with_scratch(&path, &root)?;
            let check = check_rebuild(&path, &snapshot)?;
            assert_eq!(
                check.eligible_to_skip, skip,
                "unexpected proof: {:?}",
                check.blockers
            );
            assert_eq!(check.source_sha256, artifact_cache::hash_file(&path)?);
        }
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn formulas_and_unknown_features_keep_rebuilds_but_plain_cells_do_not() -> Result<()> {
        for (xml, blocked) in [
            (
                "<worksheet><sheetData><row><c><v>1</v></c></row></sheetData></worksheet>",
                false,
            ),
            (
                "<worksheet><sheetData><row><c><f t=\"shared\"/></c></row></sheetData></worksheet>",
                true,
            ),
            ("<worksheet><conditionalFormatting/></worksheet>", true),
            ("<worksheet><extLst/></worksheet>", true),
            ("<worksheet><futureFeature/></worksheet>", true),
        ] {
            let mut reader = Reader::<128>::new(xml.as_bytes())?;
            let mut found = false;
            while let Some(tag) = reader.next()? {
                found |= worksheet_blocker(tag).is_some();
            }
            assert_eq!(found, blocked);
        }
        Ok(())
    }

    #[test]
    fn native_path_revision_and_recovery_metadata_are_static_but_other_choices_are_not(
    ) -> Result<()> {
        let xml = br#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:x15="http://schemas.microsoft.com/office/spreadsheetml/2010/11/main" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision" xmlns:xr6="http://schemas.microsoft.com/office/spreadsheetml/2016/revision6" xmlns:xr10="http://schemas.microsoft.com/office/spreadsheetml/2016/revision10"><mc:AlternateContent><mc:Choice Requires="x15"><x15ac:absPath url="C:\temp\" xmlns:x15ac="http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac"/></mc:Choice></mc:AlternateContent><xr:revisionPtr revIDLastSave="0" documentId="sample" xr6:coauthVersionLast="47" xr6:coauthVersionMax="47" xr10:uidLastSave="sample"/><fileRecoveryPr autoRecover="0"/></workbook>"#;
        let mut blockers = BTreeSet::new();
        inspect_metadata(xml.as_slice(), b"workbook", "workbook.xml", &mut blockers)?;
        assert!(blockers.is_empty(), "{blockers:?}");
        let unknown =
            String::from_utf8(xml.to_vec())?.replace("x15ac:absPath", "x15ac:futureFeature");
        inspect_metadata(
            unknown.as_bytes(),
            b"workbook",
            "workbook.xml",
            &mut blockers,
        )?;
        assert!(!blockers.is_empty());
        let wrong_namespace = String::from_utf8(xml.to_vec())?.replace("2010/11/ac", "2099/11/ac");
        blockers.clear();
        inspect_metadata(
            wrong_namespace.as_bytes(),
            b"workbook",
            "workbook.xml",
            &mut blockers,
        )?;
        assert!(!blockers.is_empty());
        Ok(())
    }
}
