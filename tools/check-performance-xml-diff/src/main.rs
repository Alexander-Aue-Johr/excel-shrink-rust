//! Reuse the validation byte scanner for bounded, read-only worksheet evidence.
#[allow(dead_code)]
#[path = "../../../excel-shrink-cli/src/regression/streaming_xml.rs"]
mod streaming_xml;
#[allow(dead_code)]
#[path = "../../../excel-shrink-cli/src/regression/xml_views.rs"]
pub(crate) mod xml_views;
#[cfg(test)]
mod regression {
    pub(crate) use crate::xml_views;
}

use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::PathBuf,
};
use xml_views::{Attributes, Kind};

#[derive(Deserialize)]
struct Case {
    id: String,
    variants: BTreeMap<String, Variant>,
}

#[derive(Deserialize)]
struct Variant {
    path: PathBuf,
    sheets: BTreeMap<String, String>,
    styles: Vec<String>,
}

#[derive(Clone, Copy, Default)]
struct Cell {
    key: u64,
    style: u32,
    payload: bool,
    bytes: u64,
}

#[derive(Clone, Default)]
struct Row {
    index: u32,
    attrs: BTreeMap<String, String>,
    cells: u64,
    bytes: u64,
}

#[derive(Default)]
struct Snapshot {
    cells: Vec<Cell>,
    rows: Vec<Row>,
    layout: Value,
}

fn number(bytes: &[u8]) -> Result<u32> {
    let mut value = 0u32;
    ensure!(!bytes.is_empty(), "Missing numeric attribute");
    for &byte in bytes {
        ensure!(byte.is_ascii_digit(), "Invalid numeric attribute");
        value = value
            .checked_mul(10)
            .and_then(|v| v.checked_add(u32::from(byte - b'0')))
            .context("Numeric attribute overflow")?;
    }
    Ok(value)
}

fn address(bytes: &[u8]) -> Result<(u32, u32)> {
    let split = bytes
        .iter()
        .position(u8::is_ascii_digit)
        .context("Cell address has no row")?;
    let mut column = 0u32;
    for &byte in &bytes[..split] {
        ensure!(byte.is_ascii_uppercase(), "Invalid cell column");
        column = column
            .checked_mul(26)
            .and_then(|v| v.checked_add(u32::from(byte - b'A' + 1)))
            .context("Column overflow")?;
    }
    let row = number(&bytes[split..])?;
    ensure!(
        (1..=16_384).contains(&column) && (1..=1_048_576).contains(&row),
        "Cell outside supported grid"
    );
    Ok((row, column))
}

fn cell_address(key: u64) -> String {
    let row = key / 16_384;
    let mut column = key % 16_384 + 1;
    let mut letters = Vec::new();
    while column > 0 {
        column -= 1;
        letters.push(b'A' + (column % 26) as u8);
        column /= 26;
    }
    letters.reverse();
    format!("{}{row}", String::from_utf8(letters).unwrap())
}

fn attrs(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    Attributes::new(bytes)
        .map(|entry| {
            let (key, value) = entry?;
            Ok((
                String::from_utf8(key.to_vec())?,
                String::from_utf8(value.to_vec())?,
            ))
        })
        .collect()
}

/// Only cell/row records at worksheet/sheetData/row/c depth are counted.
fn scan(
    input: impl Read,
    mut on_cell: impl FnMut(Cell, &[u8]) -> Result<()>,
    mut on_row: impl FnMut(&Row) -> Result<()>,
) -> Result<Value> {
    let mut data = false;
    let mut prefix = Vec::new();
    let mut row: Option<Row> = None;
    let mut cell: Option<Cell> = None;
    let mut cell_attrs = Vec::new();
    let mut cell_start = 0;
    let mut row_start = 0;
    let mut previous_row = 0;
    let mut previous_column = 0;
    let mut layout = json!({"columns":[], "sheet_format":{}, "dimension":{}});
    streaming_xml::scan_with_offsets(input, |event| {
        let streaming_xml::Event::Tag(tag) = event else {
            return Ok(());
        };
        if tag.depth == 0 && tag.kind == Kind::Start {
            ensure!(tag.local() == b"worksheet", "Expected worksheet root");
            prefix.extend_from_slice(&tag.name[..tag.name.len() - b"worksheet".len()]);
        }
        let local = tag.local();
        let own_namespace =
            tag.name.len() == prefix.len() + local.len() && tag.name.starts_with(&prefix);
        if !own_namespace {
            return Ok(());
        }
        if data && matches!(tag.kind, Kind::Start | Kind::Empty) {
            for attribute in Attributes::new(tag.attrs) {
                let (name, _) = attribute?;
                ensure!(
                    name != b"xmlns" && !name.starts_with(b"xmlns:"),
                    "Namespace redeclaration inside sheetData"
                );
            }
        }
        if tag.depth == 1 && local == b"sheetData" {
            data = tag.kind == Kind::Start;
        }
        if !data {
            if matches!(tag.kind, Kind::Start | Kind::Empty) {
                if tag.depth == 1 && local == b"sheetFormatPr" {
                    layout["sheet_format"] = json!(attrs(tag.attrs)?);
                }
                if tag.depth == 1 && local == b"dimension" {
                    layout["dimension"] = json!(attrs(tag.attrs)?);
                }
                if tag.depth == 2 && local == b"col" {
                    layout["columns"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(attrs(tag.attrs)?));
                }
            }
            return Ok(());
        }
        if tag.depth == 2 && local == b"row" {
            if matches!(tag.kind, Kind::Start | Kind::Empty) {
                let index = tag
                    .attr(b"r")?
                    .map(number)
                    .transpose()?
                    .unwrap_or(previous_row + 1);
                ensure!(
                    index > previous_row && index <= 1_048_576,
                    "Unordered or invalid row"
                );
                previous_row = index;
                previous_column = 0;
                row_start = tag.start;
                row = Some(Row {
                    index,
                    attrs: attrs(tag.attrs)?,
                    cells: 0,
                    bytes: 0,
                });
            }
            if matches!(tag.kind, Kind::End | Kind::Empty) {
                let mut finished = row.take().context("Row end without start")?;
                finished.bytes = (tag.end - row_start) as u64;
                on_row(&finished)?;
            }
        }
        if tag.depth == 3 && local == b"c" {
            if matches!(tag.kind, Kind::Start | Kind::Empty) {
                let owner = row.as_mut().context("Cell outside row")?;
                let (r, column) = tag
                    .attr(b"r")?
                    .map(address)
                    .transpose()?
                    .unwrap_or((owner.index, previous_column + 1));
                ensure!(
                    r == owner.index && column > previous_column,
                    "Unordered or misplaced cell"
                );
                previous_column = column;
                owner.cells += 1;
                let style = tag.attr(b"s")?.map(number).transpose()?.unwrap_or(0);
                cell = Some(Cell {
                    key: u64::from(r) * 16_384 + u64::from(column - 1),
                    style,
                    payload: false,
                    bytes: 0,
                });
                cell_start = tag.start;
                cell_attrs.clear();
                cell_attrs.extend_from_slice(tag.attrs);
            }
            if matches!(tag.kind, Kind::End | Kind::Empty) {
                let mut finished = cell.take().context("Cell end without start")?;
                finished.bytes = (tag.end - cell_start) as u64;
                on_cell(finished, &cell_attrs)?;
            }
        } else if cell.is_some()
            && tag.depth > 3
            && matches!(tag.kind, Kind::Start | Kind::Empty)
            && matches!(local, b"v" | b"f" | b"is")
        {
            cell.as_mut().unwrap().payload = true;
        }
        Ok(())
    })?;
    Ok(layout)
}

fn snapshot(variant: &Variant, part: &str) -> Result<Snapshot> {
    let mut archive = zip::ZipArchive::new(File::open(&variant.path)?)?;
    let mut result = Snapshot::default();
    result.layout = scan(
        archive.by_name(part)?,
        |cell, _| {
            result.cells.push(cell);
            Ok(())
        },
        |row| {
            result.rows.push(row.clone());
            Ok(())
        },
    )?;
    Ok(result)
}

#[derive(Default)]
struct ExtraCell {
    cell: Cell,
    presence: [bool; 3],
}

#[derive(Default)]
struct ExtraRow {
    row: Row,
    presence: [bool; 3],
}

fn attribution(presence: [bool; 3]) -> &'static str {
    match presence {
        [_, _, true] => "removed_during_check_performance",
        [_, true, false] => "absent_after_cp_copy_open_save",
        [true, false, false] => "absent_after_original_open_save",
        [false, _, false] => "absent_from_raw_original",
    }
}

fn row_context(row: &Row, styles: &[String]) -> BTreeMap<String, String> {
    let mut result = row.attrs.clone();
    result.remove("r");
    result.remove("spans");
    let index = result
        .remove("s")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    result.insert(
        "resolved_style_signature".into(),
        styles
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("unresolved:{index}")),
    );
    result
}

fn analyze_sheet(case: &Case, name: &str) -> Result<Value> {
    let d = &case.variants["default"];
    let a = &case.variants["cp_after"];
    let default = snapshot(d, &d.sheets[name])?;
    let after = snapshot(a, &a.sheets[name])?;
    let mut extra_cells = Vec::new();
    let mut cp_only = 0;
    let mut cursor = 0;
    for &cell in &default.cells {
        while cursor < after.cells.len() && after.cells[cursor].key < cell.key {
            cp_only += 1;
            cursor += 1;
        }
        if cursor < after.cells.len() && after.cells[cursor].key == cell.key {
            cursor += 1;
        } else {
            extra_cells.push(ExtraCell {
                cell,
                ..Default::default()
            });
        }
    }
    cp_only += after.cells.len() - cursor;
    let mut extra_rows = Vec::new();
    let mut row_changes = Vec::new();
    let mut changed_attributes = BTreeMap::<String, u64>::new();
    for row in &default.rows {
        match after.rows.binary_search_by_key(&row.index, |r| r.index) {
            Ok(index) => {
                let other = &after.rows[index];
                let before_context = row_context(row, &d.styles);
                let after_context = row_context(other, &a.styles);
                if before_context != after_context {
                    for key in before_context
                        .keys()
                        .chain(after_context.keys())
                        .collect::<std::collections::BTreeSet<_>>()
                    {
                        if before_context.get(key) != after_context.get(key) {
                            *changed_attributes.entry(key.clone()).or_default() += 1;
                        }
                    }
                    if row_changes.len() < 5 {
                        row_changes.push(
                            json!({"row":row.index,"default":row.attrs,"cp_after":other.attrs}),
                        );
                    }
                }
            }
            Err(_) => extra_rows.push(ExtraRow {
                row: row.clone(),
                ..Default::default()
            }),
        }
    }
    for (index, variant_name) in ["original", "saved_original", "cp_before"]
        .iter()
        .enumerate()
    {
        let variant = &case.variants[*variant_name];
        let Some(part) = variant.sheets.get(name) else {
            continue;
        };
        let mut archive = zip::ZipArchive::new(File::open(&variant.path)?)?;
        scan(
            archive.by_name(part)?,
            |cell, _| {
                if let Ok(position) = extra_cells.binary_search_by_key(&cell.key, |x| x.cell.key) {
                    extra_cells[position].presence[index] = true;
                }
                Ok(())
            },
            |row| {
                if let Ok(position) = extra_rows.binary_search_by_key(&row.index, |x| x.row.index) {
                    extra_rows[position].presence[index] = true;
                }
                Ok(())
            },
        )?;
    }
    let last_payload_row = default
        .cells
        .iter()
        .filter(|c| c.payload)
        .map(|c| c.key / 16_384)
        .max()
        .unwrap_or(0);
    let mut cell_groups = BTreeMap::<String, u64>::new();
    let mut style_counts = BTreeMap::<u32, u64>::new();
    let mut cell_samples = Vec::new();
    for extra in &extra_cells {
        *cell_groups
            .entry(attribution(extra.presence).into())
            .or_default() += 1;
        *style_counts.entry(extra.cell.style).or_default() += 1;
        if cell_samples.len() < 6 {
            cell_samples.push(json!({"cell":cell_address(extra.cell.key),"style":extra.cell.style,
                "has_payload_element":extra.cell.payload,"attribution":attribution(extra.presence),"xml_bytes":extra.cell.bytes}));
        }
    }
    let sample_keys: Vec<_> = extra_cells
        .iter()
        .take(6)
        .map(|extra| extra.cell.key)
        .collect();
    let mut headers = BTreeMap::new();
    if !sample_keys.is_empty() {
        let mut archive = zip::ZipArchive::new(File::open(&d.path)?)?;
        scan(
            archive.by_name(&d.sheets[name])?,
            |cell, raw| {
                if sample_keys.binary_search(&cell.key).is_ok() {
                    headers.insert(cell.key, attrs(raw)?);
                }
                Ok(())
            },
            |_| Ok(()),
        )?;
    }
    for (sample, key) in cell_samples.iter_mut().zip(sample_keys) {
        sample["attrs"] = json!(headers.get(&key));
    }
    let mut empty_row_groups = BTreeMap::<String, u64>::new();
    let mut empty_row_samples = Vec::new();
    for extra in extra_rows.iter().filter(|x| x.row.cells == 0) {
        *empty_row_groups
            .entry(attribution(extra.presence).into())
            .or_default() += 1;
        if empty_row_samples.len() < 6 {
            empty_row_samples.push(json!({"row":extra.row.index,"attrs":extra.row.attrs,"attribution":attribution(extra.presence)}));
        }
    }
    Ok(
        json!({"sheet":name,"default_part":d.sheets[name],"cp_after_part":a.sheets[name],
        "default_cells":default.cells.len(),"cp_after_cells":after.cells.len(),
        "cells_only_in_default":extra_cells.len(),"cells_only_in_cp_after":cp_only,
        "default_only_cells_with_payload_elements":extra_cells.iter().filter(|x|x.cell.payload).count(),
        "default_only_cell_xml_bytes":extra_cells.iter().map(|x|x.cell.bytes).sum::<u64>(),
        "default_only_cells_below_last_payload_row":extra_cells.iter().filter(|x|x.cell.key / 16_384 > last_payload_row).count(),
        "last_default_payload_row":last_payload_row,"cell_attribution":cell_groups,
        "default_extra_cell_styles":style_counts,"cell_samples":cell_samples,
        "default_rows":default.rows.len(),"cp_after_rows":after.rows.len(),"rows_only_in_default":extra_rows.len(),
        "empty_rows_only_in_default":extra_rows.iter().filter(|x|x.row.cells==0).count(),
        "empty_row_attribution":empty_row_groups,"empty_row_samples":empty_row_samples,
        "changed_row_attributes":changed_attributes,"row_change_samples":row_changes,
        "default_layout":default.layout,"cp_after_layout":after.layout}),
    )
}

fn analyze_case(case: &Case) -> Value {
    let mut sheets = Vec::new();
    let mut unmatched = Vec::new();
    for name in case.variants["default"].sheets.keys() {
        if !case.variants["cp_after"].sheets.contains_key(name) {
            unmatched.push(name);
            continue;
        }
        match analyze_sheet(case, name) {
            Ok(value) => sheets.push(value),
            Err(error) => sheets.push(json!({"sheet":name,"error":format!("{error:#}")})),
        }
    }
    json!({"id":case.id,"sheets":sheets,"unmatched_default_sheets":unmatched})
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let manifest = PathBuf::from(args.next().context("Expected manifest JSON")?);
    let output = PathBuf::from(args.next().context("Expected output directory")?);
    fs::create_dir_all(&output)?;
    let cases: Vec<Case> = serde_json::from_slice(&fs::read(manifest)?)?;
    for (index, case) in cases.iter().enumerate() {
        let result = analyze_case(case);
        fs::write(
            output.join(format!("{}.json", case.id)),
            serde_json::to_vec_pretty(&result)?,
        )?;
        println!("{}/{} {}", index + 1, cases.len(), case.id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scans_hierarchy_and_handles_implicit_addresses() -> Result<()> {
        let xml = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="2" s="3"><c r="A2" s="4"/><c><f>1+1</f><v>2</v></c></row><row ht="14"/></sheetData><extLst><ext><c r="A99"/></ext></extLst></worksheet>"#;
        let mut cells = Vec::new();
        let mut rows = Vec::new();
        scan(
            &xml[..],
            |c, _| {
                cells.push(c);
                Ok(())
            },
            |r| {
                rows.push(r.clone());
                Ok(())
            },
        )?;
        assert_eq!(cells.len(), 2);
        assert_eq!(cell_address(cells[0].key), "A2");
        assert_eq!(cell_address(cells[1].key), "B2");
        assert!(!cells[0].payload);
        assert!(cells[1].payload);
        assert_eq!(rows[1].index, 3);
        assert_eq!(rows[1].cells, 0);
        assert!(cells[0].bytes > 0);
        Ok(())
    }
    #[test]
    fn attributes_removals_to_the_first_missing_control() {
        assert_eq!(
            attribution([true, true, true]),
            "removed_during_check_performance"
        );
        assert_eq!(
            attribution([true, true, false]),
            "absent_after_cp_copy_open_save"
        );
        assert_eq!(
            attribution([false, true, false]),
            "absent_after_cp_copy_open_save"
        );
        assert_eq!(
            attribution([true, false, false]),
            "absent_after_original_open_save"
        );
        assert_eq!(
            attribution([false, false, false]),
            "absent_from_raw_original"
        );
    }
}
