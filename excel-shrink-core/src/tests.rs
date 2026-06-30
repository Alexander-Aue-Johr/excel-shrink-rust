use crate::models::Xf;
use crate::styles::parse_xfs;
use crate::workbook::clean_workbook;

#[test]
fn test_parse_xfs() {
    let xml = br#"
        <cellXfs count="3">
            <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
            <xf numFmtId="0" fontId="1" fillId="2" borderId="1" xfId="0" applyFill="1" applyBorder="1"/>
            <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyProtection="1"/>
        </cellXfs>
    "#;

    let xfs = parse_xfs(xml).expect("Should parse valid XML successfully");

    assert_eq!(xfs.len(), 3);

    assert_eq!(xfs[0].fill_id, Some(0));
    assert_eq!(xfs[0].font_id, Some(0));
    assert!(!xfs[0].apply_fill);
    assert!(!xfs[0].apply_font);
    assert!(!xfs[0].has_fill_or_border);
    assert!(xfs[0].has_no_empty_cell_visible_style);

    assert_eq!(xfs[1].font_id, Some(1));
    assert_eq!(xfs[1].fill_id, Some(2));
    assert!(xfs[1].apply_fill);
    assert!(xfs[1].has_fill_or_border);

    assert!(xfs[2].has_apply_protection);
    assert!(!xfs[2].has_no_empty_cell_visible_style);
}

#[test]
fn test_parse_xfs_uses_fill_and_border_tables_when_styles_xml_is_available() {
    let xml = br#"
        <styleSheet>
            <fills count="3">
                <fill><patternFill patternType="none"/></fill>
                <fill><patternFill patternType="none"/></fill>
                <fill><patternFill patternType="solid"><fgColor rgb="FFFF0000"/></patternFill></fill>
            </fills>
            <borders count="3">
                <border><left/><right/><top/><bottom/><diagonal/></border>
                <border><left><color indexed="64"/></left></border>
                <border><left style="thin"><color indexed="64"/></left></border>
            </borders>
            <cellXfs count="4">
                <xf numFmtId="0" fontId="0" fillId="1" borderId="1" xfId="0"/>
                <xf numFmtId="0" fontId="0" fillId="2" borderId="1" xfId="0"/>
                <xf numFmtId="0" fontId="0" fillId="1" borderId="2" xfId="0"/>
                <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
            </cellXfs>
        </styleSheet>
    "#;

    let xfs = parse_xfs(xml).expect("Should parse styles XML successfully");

    assert_eq!(xfs.len(), 4);
    assert!(!xfs[0].has_fill_or_border);
    assert!(xfs[1].has_fill_or_border);
    assert!(xfs[2].has_fill_or_border);
    assert!(!xfs[3].has_fill_or_border);
}

#[test]
fn test_applies_visible_fill_or_border_pairs_visibility_with_matching_apply_flag() {
    let mismatched_style = Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(4))
        .border_id(Some(0))
        .apply_border(true)
        .has_visible_fill(true)
        .has_fill_or_border(true)
        .has_apply_fill_or_border(true)
        .build_xf();

    assert!(!mismatched_style.applies_visible_fill_or_border());

    let applied_fill_style = Xf::builder()
        .font_id(Some(0))
        .fill_id(Some(4))
        .border_id(Some(0))
        .apply_fill(true)
        .has_visible_fill(true)
        .has_fill_or_border(true)
        .has_apply_fill_or_border(true)
        .build_xf();

    assert!(applied_fill_style.applies_visible_fill_or_border());
}

#[test]
fn test_apply_border_default_style_has_empty_cell_visible_effect() {
    let xml = br#"
        <cellXfs count="1">
            <xf numFmtId="0" fontId="10" fillId="0" borderId="0" xfId="0" applyFont="1" applyBorder="1"/>
        </cellXfs>
    "#;

    let xfs = parse_xfs(xml).expect("Should parse valid XML successfully");

    assert_eq!(xfs.len(), 1);
    assert!(xfs[0].apply_border);
    assert!(xfs[0].has_apply_fill_or_border);
    assert!(!xfs[0].has_no_empty_cell_visible_style);
}

#[test]
fn test_apply_fill_default_style_has_empty_cell_visible_effect() {
    let xml = br#"
        <cellXfs count="1">
            <xf numFmtId="0" fontId="10" fillId="0" borderId="0" xfId="0" applyFont="1" applyFill="1"/>
        </cellXfs>
    "#;

    let xfs = parse_xfs(xml).expect("Should parse valid XML successfully");

    assert_eq!(xfs.len(), 1);
    assert!(xfs[0].apply_fill);
    assert!(xfs[0].has_apply_fill_or_border);
    assert!(!xfs[0].has_no_empty_cell_visible_style);
}

#[test]
fn test_apply_font_only_style_is_empty_cell_font_style() {
    let xml = br#"
        <cellXfs count="1">
            <xf numFmtId="0" fontId="10" fillId="0" borderId="0" xfId="0" applyFont="1"/>
        </cellXfs>
    "#;

    let xfs = parse_xfs(xml).expect("Should parse valid XML successfully");

    assert_eq!(xfs.len(), 1);
    assert_eq!(xfs[0].font_id, Some(10));
    assert!(xfs[0].apply_font);
    assert!(xfs[0].has_no_empty_cell_visible_style);
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

    assert!(!cleaned_str.contains(r#"name="Print_Area""#));

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

    assert!(cleaned_str.contains(r#"name="Print_Area""#));
    assert!(cleaned_str.contains(r#"name="ValidName""#));
}
