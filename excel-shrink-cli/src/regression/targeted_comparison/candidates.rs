//! Bounded candidate discovery; unrelated workbook changes remain explicit limitations.
use super::{Candidate, WorkingPaths, MAX_CANDIDATES};
use crate::regression::{
    office_plan::build_workbook_visual_plan,
    worksheet_stream::WorksheetStream,
    xml_views::{Kind, Reader},
};
use anyhow::{bail, ensure, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    path::Path,
};
use zip::ZipArchive;

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
/// The fixed candidate cap prevents accidental whole-sheet screenshot work.
pub(super) fn discover_candidates(left: &Path, right: &Path) -> Result<BTreeSet<Candidate>> {
    let left_plan = build_workbook_visual_plan(left)?;
    let right_plan = build_workbook_visual_plan(right)?;
    let mut found = BTreeSet::new();
    for left_sheet in &left_plan.sheets {
        let Some(right_sheet) = right_plan
            .sheets
            .iter()
            .find(|sheet| sheet.sheet == left_sheet.sheet)
        else {
            bail!("sheet removed: {}", left_sheet.sheet);
        };
        let left_cells = read_cells(left, &left_sheet.sheet_file)?;
        let right_cells = read_cells(right, &right_sheet.sheet_file)?;
        for cell in left_cells.keys().chain(right_cells.keys()) {
            if left_cells.get(cell) != right_cells.get(cell) {
                let changed_cell = if has_visible_content(left_cells.get(cell))
                    || has_visible_content(right_cells.get(cell))
                {
                    cell.clone()
                } else {
                    // Empty styled cells can change row height. One geometry
                    // probe per row is enough even when Excel removes 16k cells.
                    let row =
                        cell.trim_start_matches(|character: char| character.is_ascii_alphabetic());
                    format!("A{row}")
                };
                found.insert(Candidate {
                    sheet: left_sheet.sheet.clone(),
                    cell: changed_cell,
                });
                ensure!(
                    found.len() <= MAX_CANDIDATES,
                    "more than {MAX_CANDIDATES} changed cells; aborting bounded experiment"
                );
            }
        }
    }
    Ok(found)
}

fn has_visible_content(cell: Option<&Vec<u8>>) -> bool {
    cell.is_some_and(|bytes| {
        [b"<v".as_slice(), b"<f", b"<is", b"<t"]
            .iter()
            .any(|marker| bytes.windows(marker.len()).any(|window| window == *marker))
    })
}

/// Reads bounded worksheet fragments, then indexes cells for this WIP comparison.
/// The map still scales with the cell count; a streaming merge is needed for
/// large production sheets.
pub(super) fn read_cells(path: &Path, sheet_file: &str) -> Result<BTreeMap<String, Vec<u8>>> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let sheet = archive.by_name(sheet_file)?;
    ensure!(sheet.size() <= 64 * 1024 * 1024,
        "Targeted comparison cannot index worksheet XML larger than 64 MiB; use the corpus comparison for this workbook");
    let mut stream = WorksheetStream::new(sheet);
    let mut fragment = Vec::new();
    let mut cells = BTreeMap::new();
    while stream.next(&mut fragment)? {
        let mut reader = Reader::<128>::new(&fragment)?;
        while let Some(tag) = reader.next()? {
            if tag.local() == b"c" && matches!(tag.kind, Kind::Start | Kind::Empty) {
                let Some(reference) = tag.attr(b"r")? else {
                    continue;
                };
                let address = String::from_utf8(reference.to_vec())?;
                let end = if tag.kind == Kind::Empty {
                    tag.end
                } else {
                    let mut end = tag.end;
                    while let Some(child) = reader.next()? {
                        end = child.end;
                        if child.local() == b"c" && child.kind == Kind::End {
                            break;
                        }
                    }
                    end
                };
                cells.insert(address, fragment[tag.start..end].to_vec());
                ensure!(
                    cells.len() <= 250_000,
                    "Targeted comparison cell-index limit exceeded; no visual equality established"
                );
            }
        }
    }
    Ok(cells)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regression::artifact_cache;
    use std::{fs, io::Write};

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
}
