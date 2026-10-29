//! Only coordinates of content demonstrably absent from the cleaned package.
//! Exact formulas and typed values are read from the original Excel client.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::regression) struct DiagnosticRemovedCell {
    pub sheet_index: usize,
    pub cell_ref: String,
    pub hidden: bool,
    pub whitespace: bool,
    pub has_formula: bool,
}

pub(in crate::regression) fn removed_diagnostic_cells(
    original: &Path,
    cleaned: &Path,
    sheets: &[(usize, String, bool)],
    max_cells: usize,
) -> Result<Vec<DiagnosticRemovedCell>> {
    const MAX_PART_BYTES: u64 = 256 * 1024 * 1024;
    const MAX_SCANNED_BYTES: u64 = 512 * 1024 * 1024;
    const MAX_PRESENT_CELLS: usize = 2_000_000;
    let mut before = Workbook::open(original)?;
    let mut after = Workbook::open(cleaned)?;
    for book in [&before, &after] {
        if let Some(part) = book.manifest.parts.get("xl/sharedStrings.xml") {
            ensure!(
                part.bytes <= MAX_PART_BYTES,
                "diagnosis shared-string budget exceeded"
            );
        }
    }
    before.ensure_sst()?;
    after.ensure_sst()?;
    let mut result = Vec::new();
    let mut buffer = Vec::new();
    let mut scanned = 0u64;
    for (sheet_index, path, sheet_hidden) in sheets {
        for book in [&before, &after] {
            let part = book
                .manifest
                .parts
                .get(path)
                .context("diagnosis worksheet missing")?;
            ensure!(
                part.bytes <= MAX_PART_BYTES,
                "diagnosis worksheet byte budget exceeded: {path}"
            );
            scanned = scanned
                .checked_add(part.bytes)
                .context("diagnosis XML byte overflow")?;
        }
        ensure!(
            scanned <= MAX_SCANNED_BYTES,
            "diagnosis total XML byte budget exceeded"
        );
        after.read_sheet(path, &mut buffer)?;
        let mut cells = Cells::new(&buffer, after.sst.as_ref().unwrap())?;
        let mut present = BTreeSet::new();
        while let Some(cell) = cells.next()? {
            if cell.value.has_content()? || cell.formula.is_some() {
                ensure!(
                    present.insert(cell.coordinate),
                    "duplicate diagnosis cell address"
                );
                ensure!(
                    present.len() <= MAX_PRESENT_CELLS,
                    "diagnosis present-cell budget exceeded"
                );
            }
        }
        before.read_sheet(path, &mut buffer)?;
        let mut cells = Cells::new(&buffer, before.sst.as_ref().unwrap())?;
        while let Some(cell) = cells.next()? {
            if present.contains(&cell.coordinate) {
                continue;
            }
            let content = cell.value.has_content()? || cell.formula.is_some();
            if !content {
                continue;
            }
            let hidden = *sheet_hidden || cell.hidden;
            let whitespace = cell.formula.is_none()
                && cell.value.kind == ValueKind::Text
                && cell.value.is_whitespace()?;
            if hidden || whitespace {
                ensure!(
                    result.len() < max_cells,
                    "diagnosis removed-cell budget exceeded"
                );
                result.push(DiagnosticRemovedCell {
                    sheet_index: *sheet_index,
                    cell_ref: cell.coordinate.name(),
                    hidden,
                    whitespace,
                    has_formula: cell.formula.is_some(),
                });
            }
        }
    }
    before.check_unchanged()?;
    after.check_unchanged()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn workbook(path: &Path, worksheet: &[u8]) -> Result<()> {
        let mut zip = zip::ZipWriter::new(File::create(path)?);
        let options = zip::write::FileOptions::default();
        for (name, bytes) in [
            ("xl/workbook.xml", br#"<workbook><sheets><sheet name="Data" r:id="r1"/></sheets></workbook>"#.as_slice()),
            ("xl/_rels/workbook.xml.rels", br#"<Relationships><Relationship Id="r1" Target="worksheets/sheet1.xml"/></Relationships>"#.as_slice()),
            ("xl/worksheets/sheet1.xml", worksheet),
        ] {
            zip.start_file(name, options)?;
            zip.write_all(bytes)?;
        }
        zip.finish()?;
        Ok(())
    }

    #[test]
    fn only_deleted_content_is_restored_and_formula_presence_is_preserved() -> Result<()> {
        let root = cache::unique_path(&std::env::temp_dir(), "diagnostic-cells")?;
        std::fs::create_dir_all(&root)?;
        let original = root.join("original.xlsx");
        let cleaned = root.join("cleaned.xlsx");
        workbook(
            &original,
            br#"<worksheet><cols><col min="2" max="2" hidden="1"/></cols><sheetData><row r="1">
            <c r="A1" t="inlineStr"><is><t> &#9; </t></is></c>
            <c r="B1"><f>SUM(1,2)</f><v>3</v></c>
            <c r="C1"><v>9</v></c>
            <c r="D1" t="inlineStr"><is><t>keep</t></is></c>
            </row></sheetData></worksheet>"#,
        )?;
        workbook(
            &cleaned,
            br#"<worksheet><sheetData><row r="1">
            <c r="A1" s="7"/><c r="C1"><v>0</v></c>
            <c r="D1" t="inlineStr"><is><t>keep</t></is></c>
            </row></sheetData></worksheet>"#,
        )?;
        let sheets = [(1, "xl/worksheets/sheet1.xml".into(), false)];
        let cells = removed_diagnostic_cells(&original, &cleaned, &sheets, 20)?;
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].cell_ref, "A1");
        assert!(cells[0].whitespace);
        assert!(!cells[0].has_formula);
        assert_eq!(cells[1].cell_ref, "B1");
        assert!(cells[1].hidden);
        assert!(cells[1].has_formula);
        assert!(!cells[1].whitespace);
        assert!(removed_diagnostic_cells(&original, &cleaned, &sheets, 1).is_err());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
