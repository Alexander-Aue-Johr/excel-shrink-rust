//! Bounded candidate discovery; unrelated workbook changes remain explicit limitations.
use super::{Candidate, MAX_CANDIDATES};
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
fn read_cells(path: &Path, sheet_file: &str) -> Result<BTreeMap<String, Vec<u8>>> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let sheet = archive.by_name(sheet_file)?;
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
            }
        }
    }
    Ok(cells)
}
