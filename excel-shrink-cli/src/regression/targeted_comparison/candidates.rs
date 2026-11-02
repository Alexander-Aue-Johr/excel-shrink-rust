//! Candidate discovery; unrelated workbook changes remain explicit limitations.
use super::xml_changes::{cell_candidates, Snapshot};
use super::{Candidate, WorkingPaths};
use anyhow::Result;
use std::{collections::BTreeSet, path::Path};

/// Ordinary save effects are distinct controls for Original and Default.
pub(super) struct SaveChanges {
    pub(super) original: BTreeSet<Candidate>,
    pub(super) default: BTreeSet<Candidate>,
}

/// Compare each working copy with its own pre-save snapshot, not with its partner.
pub(super) fn find_changes_caused_by_save(
    before: &WorkingPaths,
    after: &WorkingPaths,
) -> Result<SaveChanges> {
    let original = discover_candidates(&before.original, &after.original)?;
    let default = discover_candidates(&before.default, &after.default)?;
    Ok(SaveChanges { original, default })
}

/// Maps changed cell XML to visible cell or row-geometry capture candidates.
pub(super) fn discover_candidates(left: &Path, right: &Path) -> Result<BTreeSet<Candidate>> {
    let before = Snapshot::read(left)?;
    let after = Snapshot::read(right)?;
    Ok(cell_candidates(&before.content_changes(&after)?.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regression::artifact_cache;
    use std::{
        fs::{self, File},
        io::Write,
    };

    #[test]
    fn save_changes_compare_each_workbook_with_its_own_snapshot() -> Result<()> {
        let directory = artifact_cache::unique_path(&std::env::temp_dir(), "save-effects")?;
        fs::create_dir_all(&directory)?;
        let before = WorkingPaths {
            original: directory.join("original-before.xlsx"),
            default: directory.join("default-before.xlsx"),
        };
        let after = WorkingPaths {
            original: directory.join("original-after.xlsx"),
            default: directory.join("default-after.xlsx"),
        };
        write_cells(&before.original, [1, 2])?;
        write_cells(&before.default, [100, 200])?;
        write_cells(&after.original, [3, 2])?;
        write_cells(&after.default, [100, 300])?;

        let changes = find_changes_caused_by_save(&before, &after)?;
        assert_eq!(
            changes.original,
            BTreeSet::from([Candidate {
                sheet: "Data".into(),
                cell: "A1".into(),
            }])
        );
        assert_eq!(
            changes.default,
            BTreeSet::from([Candidate {
                sheet: "Data".into(),
                cell: "B1".into(),
            }])
        );
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    fn write_cells(path: &Path, values: [u32; 2]) -> Result<()> {
        let sheet = format!(
            r#"<worksheet><sheetData><row r="1"><c r="A1"><v>{}</v></c><c r="B1"><v>{}</v></c></row></sheetData></worksheet>"#,
            values[0], values[1],
        );
        let mut archive = zip::ZipWriter::new(File::create(path)?);
        for (name, xml) in [
            (
                "xl/workbook.xml",
                r#"<workbook><sheets><sheet name="Data" r:id="one"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="one" Type="office/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            ("xl/worksheets/sheet1.xml", sheet.as_str()),
        ] {
            archive.start_file(name, zip::write::FileOptions::default())?;
            archive.write_all(xml.as_bytes())?;
        }
        archive.finish()?;
        Ok(())
    }

    #[test]
    fn more_than_64_candidates_are_retained() -> Result<()> {
        let directory = artifact_cache::unique_path(&std::env::temp_dir(), "many-candidates")?;
        fs::create_dir_all(&directory)?;
        let left = directory.join("original.xlsx");
        let right = directory.join("changed.xlsx");
        for (path, value) in [(&left, 1), (&right, 2)] {
            let mut archive = zip::ZipWriter::new(File::create(path)?);
            for (name, xml) in [
                (
                    "xl/workbook.xml",
                    r#"<workbook><sheets><sheet name="Data" r:id="one"/></sheets></workbook>"#,
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    r#"<Relationships><Relationship Id="one" Type="office/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
                ),
            ] {
                archive.start_file(name, zip::write::FileOptions::default())?;
                archive.write_all(xml.as_bytes())?;
            }
            archive.start_file(
                "xl/worksheets/sheet1.xml",
                zip::write::FileOptions::default(),
            )?;
            archive.write_all(b"<worksheet><sheetData>")?;
            for row in 1..=1000 {
                write!(
                    archive,
                    "<row r=\"{row}\"><c r=\"A{row}\"><v>{value}</v></c></row>"
                )?;
            }
            archive.write_all(b"</sheetData></worksheet>")?;
            archive.finish()?;
        }
        let found = discover_candidates(&left, &right)?;
        assert_eq!(found.len(), 1000);
        assert!(found.contains(&Candidate {
            sheet: "Data".into(),
            cell: "A1000".into()
        }));
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    #[ignore = "large streaming integration fixture; run explicitly"]
    fn million_cells_over_old_xml_limit_select_only_the_last_change() -> Result<()> {
        let directory = artifact_cache::unique_path(&std::env::temp_dir(), "million-cell-stream")?;
        fs::create_dir_all(&directory)?;
        let started = std::time::Instant::now();
        let left = directory.join("original.xlsx");
        let right = directory.join("changed.xlsx");
        write_large_sheet(&left, false, false)?;
        write_large_sheet(&right, true, false)?;
        let mut archive = zip::ZipArchive::new(File::open(&left)?)?;
        let size = archive.by_name("xl/worksheets/sheet1.xml")?.size();
        assert!(size > 64 * 1024 * 1024);
        drop(archive);
        let found = discover_candidates(&left, &right)?;
        assert_eq!(
            found,
            BTreeSet::from([Candidate {
                sheet: "Data".into(),
                cell: "A1048576".into()
            }])
        );
        println!("1,048,576 cells, {size} XML bytes; one final-cell change; {:.2}s including fixture creation",started.elapsed().as_secs_f64());
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    #[ignore = "wide-row streaming integration fixture; run explicitly"]
    fn a_row_larger_than_the_old_fragment_buffer_is_streamed() -> Result<()> {
        let directory = artifact_cache::unique_path(&std::env::temp_dir(), "wide-row-stream")?;
        fs::create_dir_all(&directory)?;
        let left = directory.join("original.xlsx");
        let right = directory.join("same.xlsx");
        write_large_sheet(&left, false, true)?;
        fs::copy(&left, &right)?;
        assert!(discover_candidates(&left, &right)?.is_empty());
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// Stream valid Excel-sized rows/cells directly into ZIP members without a giant fixture string.
    fn write_large_sheet(path: &Path, changed: bool, wide: bool) -> Result<()> {
        let mut archive = zip::ZipWriter::new(File::create(path)?);
        let options = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(1));
        for (name, xml) in [
            (
                "xl/workbook.xml",
                r#"<workbook><sheets><sheet name="Data" r:id="one"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="one" Type="office/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
        ] {
            archive.start_file(name, options)?;
            archive.write_all(xml.as_bytes())?;
        }
        archive.start_file("xl/worksheets/sheet1.xml", options)?;
        archive.write_all(b"<worksheet><sheetData>")?;
        if wide {
            archive.write_all(b"<row r=\"1\">")?;
            for column in 1..=8000 {
                let cell = super::super::cell_stream::Cell {
                    row: 1,
                    column,
                    digest: [0; 32],
                    visible: true,
                    has_formula: false,
                };
                write!(
                    archive,
                    "<c r=\"{}\" t=\"inlineStr\"><is><t>",
                    cell.address()
                )?;
                archive.write_all(&[b'x'; 2048])?;
                archive.write_all(b"</t></is></c>")?;
            }
            archive.write_all(b"</row>")?;
        } else {
            for row in 1..=1_048_576u32 {
                let value = if changed && row == 1_048_576 { 0 } else { row };
                write!(
                    archive,
                    "<row r=\"{row}\" spans=\"1:1\"><c r=\"A{row}\" s=\"0\"><v>{value}</v></c></row>"
                )?;
            }
        }
        archive.write_all(b"</sheetData></worksheet>")?;
        archive.finish()?;
        Ok(())
    }
}
