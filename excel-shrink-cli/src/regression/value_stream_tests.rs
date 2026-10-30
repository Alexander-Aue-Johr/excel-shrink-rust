use super::*;

#[test]
fn streamed_selection_matches_buffered_typed_values_and_shared_strings() -> Result<()> {
    let left_strings = SharedStrings::new(b"<sst><si><t>before</t></si></sst>".to_vec())?;
    let right_strings = SharedStrings::new(b"<sst><si><t>after</t></si></sst>".to_vec())?;
    let left = br#"<worksheet><cols><col min="1" max="2" hidden="1"/></cols><sheetData>
        <row r="1"><c r="A1" t="s"><v>0</v></c></row>
        <row r="2"><c r="A2"><v>1.0</v></c><c r="B2" t="inlineStr"><is><t>a&amp;b</t></is></c></row>
        <row r="3"><c r="A3" t="b"><v>1</v></c></row><row r="5"/>
        </sheetData></worksheet>"#;
    let right = br#"<worksheet><sheetData>
        <row r="1"><c r="A1" t="s"><v>0</v></c></row>
        <row r="2"><c r="A2"><v>1</v></c><c r="B2" t="inlineStr"><is><t>a&#38;b</t></is></c></row>
        <row r="4"><c r="A4" t="e"><v>#REF!</v></c></row><row r="5"/>
        </sheetData></worksheet>"#;
    let expected = changed_row_ranges(left, right, &left_strings, &right_strings)?;
    let actual = streamed_changed_row_ranges(
        left.as_slice(),
        right.as_slice(),
        &left_strings,
        &right_strings,
    )?;
    assert_eq!(actual, expected);
    assert!(actual.iter().any(|span| span[0] == 0));
    assert!(actual.iter().any(|span| span[0] <= 2 && span[1] >= 4));
    Ok(())
}

#[test]
fn streamed_selection_rejects_unordered_rows_and_cells() -> Result<()> {
    let strings = SharedStrings::new(Vec::new())?;
    for xml in [
        b"<worksheet><sheetData><row r='2'/><row r='1'/></sheetData></worksheet>".as_slice(),
        b"<worksheet><sheetData><row r='1'><c r='B1'/><c r='A1'/></row></sheetData></worksheet>",
        b"<worksheet><sheetData><row r='1'><c r='A2'/></row></sheetData></worksheet>",
    ] {
        assert!(streamed_changed_row_ranges(xml, xml, &strings, &strings).is_err());
    }
    Ok(())
}

#[test]
fn empty_streamed_sheets_have_no_changed_values() -> Result<()> {
    let strings = SharedStrings::new(Vec::new())?;
    for xml in [
        b"<worksheet/>".as_slice(),
        b"<worksheet><sheetData/></worksheet>",
    ] {
        assert!(streamed_changed_row_ranges(xml, xml, &strings, &strings)?.is_empty());
    }
    Ok(())
}

#[test]
#[ignore = "read-only value selection on an explicitly supplied large workbook pair"]
fn existing_giant_value_selection() -> Result<()> {
    let root = PathBuf::from(std::env::var_os("EXCEL_SHRINK_ANALYSIS_FIXTURE")
        .context("set EXCEL_SHRINK_ANALYSIS_FIXTURE")?);
    let wanted = HashSet::from([
        "xl/worksheets/sheet2.xml".to_owned(),
        "xl/worksheets/sheet3.xml".to_owned(),
    ]);
    let start = std::time::Instant::now();
    let changes = compare_workbook_changed_rows(&root.join("original.xlsx"),
        &root.join("default.xlsx"), &wanted)?;
    println!("value_selection_seconds={:.2}; changed_rows={changes:?}", start.elapsed().as_secs_f64());
    ensure!(changes.is_empty(), "the supplied Default fixture changed cell values");
    Ok(())
}
