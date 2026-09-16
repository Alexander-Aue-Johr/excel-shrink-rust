use super::*;
use crate::models::{ProtectedWorkbookRefs, SheetScanBuffer};
use std::collections::HashMap;

#[test]
fn local_reference_on_same_sheet_keeps_candidate() {
    let sheet1_xml = br#"
        <worksheet>
            <sheetData>
                <row r="41">
                    <c r="S41"><f>S41</f></c>
                </row>
            </sheetData>
        </worksheet>
    "#;

    let mut protected = ProtectedWorkbookRefs::default();

    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet1.xml",
            sheet_name: "Sheet1",
            xml: sheet1_xml,
        }],
        &mut protected,
        None,
        None,
    );

    assert!(matches!(result, ScanResult::Safe));
    assert!(protected.contains("xl/worksheets/sheet1.xml", 19, 41));
}

#[test]
fn local_reference_on_other_sheet_does_not_keep_candidate() {
    let sheet1_xml = br#"
        <worksheet>
            <sheetData>
                <row r="41">
                    <c r="S41"><v>secret</v></c>
                </row>
            </sheetData>
        </worksheet>
    "#;

    let sheet2_xml = br#"
        <worksheet>
            <sheetData>
                <row r="41">
                    <c r="S41"><f>S41</f></c>
                </row>
            </sheetData>
        </worksheet>
    "#;

    let mut protected = ProtectedWorkbookRefs::default();

    let result = scan_for_references_by_sheet(
        &[
            SheetScanBuffer {
                sheet_file: "xl/worksheets/sheet1.xml",
                sheet_name: "Sheet1",
                xml: sheet1_xml,
            },
            SheetScanBuffer {
                sheet_file: "xl/worksheets/sheet2.xml",
                sheet_name: "Sheet2",
                xml: sheet2_xml,
            },
        ],
        &mut protected,
        None,
        None,
    );

    assert!(matches!(result, ScanResult::Safe));
    assert!(
        !protected.contains("xl/worksheets/sheet1.xml", 19, 41),
        "sheet2 local S41 must not protect sheet1 S41"
    );
    assert!(
        protected.contains("xl/worksheets/sheet2.xml", 19, 41),
        "sheet2 local S41 should protect sheet2 S41"
    );
}

#[test]
fn cross_sheet_reference_from_other_sheet_keeps_target_candidate() {
    let sheet2_xml = br#"
        <worksheet>
            <sheetData>
                <row r="41">
                    <c r="S41"><f>Sheet1!S41</f></c>
                </row>
            </sheetData>
        </worksheet>
    "#;

    let mut seed = HashMap::new();
    seed.insert("Sheet1".to_string(), "xl/worksheets/sheet1.xml".to_string());
    seed.insert("Sheet2".to_string(), "xl/worksheets/sheet2.xml".to_string());

    let mut protected = ProtectedWorkbookRefs::default();

    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet2.xml",
            sheet_name: "Sheet2",
            xml: sheet2_xml,
        }],
        &mut protected,
        Some(&seed),
        None,
    );

    assert!(matches!(result, ScanResult::Safe));

    assert!(
        protected.contains("xl/worksheets/sheet1.xml", 19, 41),
        "Sheet1!S41 from sheet2 must protect the target sheet"
    );
    assert!(
        !protected.contains("xl/worksheets/sheet2.xml", 19, 41),
        "Sheet1!S41 must not accidentally protect sheet2 S41"
    );
}

#[test]
fn cross_sheet_reference_to_other_sheet_does_not_keep_current_candidate() {
    let sheet2_xml = br#"
        <worksheet>
            <sheetData>
                <row r="41">
                    <c r="S41"><f>Sheet2!S41</f></c>
                </row>
            </sheetData>
        </worksheet>
    "#;

    let mut protected = ProtectedWorkbookRefs::default();

    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet2.xml",
            sheet_name: "Sheet2",
            xml: sheet2_xml,
        }],
        &mut protected,
        None,
        None,
    );

    assert!(matches!(result, ScanResult::Safe));
    assert!(
        !protected.contains("xl/worksheets/sheet1.xml", 19, 41),
        "Sheet2!S41 must not protect sheet1 S41"
    );
}

#[test]
fn selection_sqref_does_not_keep_candidate() {
    let sheet1_xml = br#"
        <worksheet>
            <sheetView>
                <selection sqref="S41"/>
            </sheetView>
        </worksheet>
    "#;

    let mut protected = ProtectedWorkbookRefs::default();

    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet1.xml",
            sheet_name: "Sheet1",
            xml: sheet1_xml,
        }],
        &mut protected,
        None,
        None,
    );

    assert!(matches!(result, ScanResult::Safe));
    assert!(!protected.contains("xl/worksheets/sheet1.xml", 19, 41));
}

#[test]
fn non_selection_sqref_keeps_candidate() {
    let sheet1_xml = br#"
        <worksheet>
            <dataValidations>
                <dataValidation sqref="S41"/>
            </dataValidations>
        </worksheet>
    "#;

    let mut protected = ProtectedWorkbookRefs::default();

    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet1.xml",
            sheet_name: "Sheet1",
            xml: sheet1_xml,
        }],
        &mut protected,
        None,
        None,
    );

    assert!(matches!(result, ScanResult::Safe));
    assert!(protected.contains("xl/worksheets/sheet1.xml", 19, 41));
}

#[test]
fn autofilter_ref_does_not_keep_whitespace_candidate_cells() {
    let sheet1_xml = br#"
        <worksheet>
            <autoFilter ref="A3:FP2792">
                <sortState ref="A4:FP2792"/>
            </autoFilter>
            <sortState ref="A4:FP2791"/>
        </worksheet>
    "#;

    let mut protected = ProtectedWorkbookRefs::default();

    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet1.xml",
            sheet_name: "Sheet1",
            xml: sheet1_xml,
        }],
        &mut protected,
        None,
        None,
    );

    assert!(matches!(result, ScanResult::Safe));
    assert!(
        !protected.contains("xl/worksheets/sheet1.xml", 24, 4),
        "filter and sort ranges should not protect X4 whitespace content from cleanup"
    );
}

#[test]
fn filter_defined_names_do_not_keep_whitespace_candidate_cells() {
    let workbook_xml = br#"
        <workbook>
            <definedNames>
                <definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">Sheet1!$A$3:$FP$2792</definedName>
                <definedName name="Z_08BA4DC8_1B9B_43C9_BE6A_1B0C51DB5350_.wvu.FilterData" localSheetId="0" hidden="1">Sheet1!$A$3:$BP$3</definedName>
            </definedNames>
        </workbook>
    "#;

    let sheet_order = vec!["xl/worksheets/sheet1.xml".to_string()];
    let mut seed = HashMap::new();
    seed.insert("Sheet1".to_string(), "xl/worksheets/sheet1.xml".to_string());
    let mut protected = ProtectedWorkbookRefs::default();

    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/workbook.xml",
            sheet_name: "Workbook",
            xml: workbook_xml,
        }],
        &mut protected,
        Some(&seed),
        Some(&sheet_order),
    );

    assert!(matches!(result, ScanResult::Safe));
    assert!(
        !protected.contains("xl/worksheets/sheet1.xml", 24, 4),
        "filter metadata defined names should not protect X4 whitespace content from cleanup"
    );
}

#[test]
fn cross_sheet_reference_with_custom_sheet_name_seed_map_keeps_target_candidate() {
    let sheet2_xml = br#"
        <worksheet>
            <sheetData>
                <row r="41">
                    <c r="S41"><f>'Hidden Data'!S41</f></c>
                </row>
            </sheetData>
        </worksheet>
    "#;

    let mut seed = HashMap::new();
    seed.insert(
        "Hidden Data".to_string(),
        "xl/worksheets/sheet7.xml".to_string(),
    );
    seed.insert(
        "Visible Calculations".to_string(),
        "xl/worksheets/sheet2.xml".to_string(),
    );

    let mut protected = ProtectedWorkbookRefs::default();

    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet2.xml",
            sheet_name: "Visible Calculations",
            xml: sheet2_xml,
        }],
        &mut protected,
        Some(&seed),
        None,
    );

    assert!(matches!(result, ScanResult::Safe));
    assert!(protected.contains("xl/worksheets/sheet7.xml", 19, 41));
    assert!(!protected.contains("xl/worksheets/sheet2.xml", 19, 41));
}

#[test]
fn workbook_defined_name_with_local_sheet_id_is_sheet_scoped() {
    let workbook_xml = br#"
        <workbook>
            <definedNames>
                <definedName name="_xlnm.Print_Area" localSheetId="1">$S$41</definedName>
            </definedNames>
        </workbook>
    "#;

    let sheet_order = vec![
        "xl/worksheets/sheet1.xml".to_string(),
        "xl/worksheets/sheet7.xml".to_string(),
    ];

    let mut protected = ProtectedWorkbookRefs::default();

    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/workbook.xml",
            sheet_name: "Workbook",
            xml: workbook_xml,
        }],
        &mut protected,
        None,
        Some(&sheet_order),
    );

    assert!(matches!(result, ScanResult::Safe));
    assert!(protected.contains("xl/worksheets/sheet7.xml", 19, 41));
    assert!(!protected.contains("xl/worksheets/sheet1.xml", 19, 41));
}

#[test]
fn defined_names_parent_tag_is_not_scanned_as_defined_name() {
    let workbook_xml = br#"
        <workbook>
            <definedNames>
                <definedName name="_xlnm.Print_Area" localSheetId="1">$S$41</definedName>
            </definedNames>
        </workbook>
    "#;

    let sheet_order = vec![
        "xl/worksheets/sheet1.xml".to_string(),
        "xl/worksheets/sheet7.xml".to_string(),
    ];

    let mut protected = ProtectedWorkbookRefs::default();

    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/workbook.xml",
            sheet_name: "Workbook",
            xml: workbook_xml,
        }],
        &mut protected,
        None,
        Some(&sheet_order),
    );

    assert!(matches!(result, ScanResult::Safe));

    assert!(
        protected.contains("xl/worksheets/sheet7.xml", 19, 41),
        "localSheetId=1 should protect sheet7 S41"
    );

    assert!(
        !protected.contains("xl/worksheets/sheet1.xml", 19, 41),
        "localSheetId=1 must not globally protect sheet1 S41"
    );

    assert!(
        !protected.global_or_unknown.contains(19, 41),
        "S41 must not be inserted into global_or_unknown"
    );
}

#[test]
fn prepared_scanner_shares_names_across_parallel_subset_scans() {
    use super::PreparedReferenceScanner;
    let buffers = [
        SheetScanBuffer { sheet_file: "xl/worksheets/sheet1.xml", sheet_name: "Source", xml: br#"<worksheet><sheetData><row><c><f>Target!B7</f></c></row></sheetData></worksheet>"# },
        SheetScanBuffer { sheet_file: "xl/worksheets/sheet2.xml", sheet_name: "Target", xml: b"<worksheet/>" },
    ];
    let scanner = PreparedReferenceScanner::new(&buffers, None, None);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let scanner = &scanner;
                let buffers = &buffers;
                scope.spawn(move || {
                    let mut refs = ProtectedWorkbookRefs::default();
                    assert!(matches!(
                        scanner.scan(&buffers[..1], &mut refs),
                        ScanResult::Safe
                    ));
                    assert!(refs.contains("xl/worksheets/sheet2.xml", 2, 7));
                    assert!(!refs.contains("xl/worksheets/sheet1.xml", 2, 7));
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    let unsafe_buffer = SheetScanBuffer {
        sheet_file: "xl/worksheets/sheet1.xml",
        sheet_name: "Source",
        xml: br#"<worksheet><c><f>INDIRECT("Target!A1")</f></c></worksheet>"#,
    };
    assert!(matches!(
        scanner.scan(&[unsafe_buffer], &mut ProtectedWorkbookRefs::default()),
        ScanResult::Unsafe(_)
    ));
}
