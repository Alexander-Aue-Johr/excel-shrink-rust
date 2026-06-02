use crate::styles::parse_xfs;
use crate::workbook::clean_workbook;

#[test]
fn test_parse_xfs() {
    let xml = br#"
        <cellXfs count="2">
            <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
            <xf numFmtId="0" fontId="1" fillId="2" borderId="1" xfId="0" applyFill="1" applyBorder="1"/>
        </cellXfs>
    "#;

    let xfs = parse_xfs(xml).expect("Should parse valid XML successfully");
    
    assert_eq!(xfs.len(), 2);
    
    // Prüfe erstes XF Element (kein Fill, kein Border)
    assert_eq!(xfs[0].fill_id, Some(0));
    assert!(!xfs[0].apply_fill);
    assert!(!xfs[0].has_fill_or_border);

    // Prüfe zweites XF Element (enthält Fill und Border)
    assert_eq!(xfs[1].fill_id, Some(2));
    assert!(xfs[1].apply_fill);
    assert!(xfs[1].has_fill_or_border);
}

#[test]
fn test_clean_workbook_duplicate_defined_names() {
    let xml = br#"
        <workbook>
            <definedNames>
                <definedName name="Print_Area" localSheetId="0">Sheet1!$A$1:$B$2</definedName>
                <definedName name="_xlnm.Print_Area" localSheetId="0">Sheet1!$A$1:$B$2</definedName>
                <definedName name="ValidName" localSheetId="1">Sheet2!$A$1</definedName>
            </definedNames>
        </workbook>
    "#;

    let cleaned = clean_workbook(xml).expect("Failed to clean workbook");
    let cleaned_str = String::from_utf8_lossy(&cleaned);

    // "Print_Area" (ohne _xlnm) sollte gelöscht werden, weil "_xlnm.Print_Area" mit identischer ID existiert
    assert!(!cleaned_str.contains(r#"name="Print_Area""#));
    
    // Die berechtigten Werte müssen erhalten bleiben
    assert!(cleaned_str.contains(r#"name="_xlnm.Print_Area""#));
    assert!(cleaned_str.contains(r#"name="ValidName""#));
}

#[test]
fn test_clean_workbook_no_duplicates_leaves_unchanged() {
    let xml = br#"
        <workbook>
            <definedNames>
                <definedName name="Print_Area" localSheetId="0">Sheet1!$A$1:$B$2</definedName>
                <definedName name="ValidName" localSheetId="1">Sheet2!$A$1</definedName>
            </definedNames>
        </workbook>
    "#;

    let cleaned = clean_workbook(xml).expect("Failed to clean workbook");
    let cleaned_str = String::from_utf8_lossy(&cleaned);

    // Da es keine "_xlnm"-Alternative gibt, bleibt "Print_Area" unberührt.
    assert!(cleaned_str.contains(r#"name="Print_Area""#));
    assert!(cleaned_str.contains(r#"name="ValidName""#));
}