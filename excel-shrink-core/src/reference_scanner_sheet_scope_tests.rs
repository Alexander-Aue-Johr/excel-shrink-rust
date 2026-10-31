use super::*;
use crate::models::{ProtectedWorkbookRefs, SheetScanBuffer};
use std::collections::HashMap;

/// Reduced regression from Destatis `verkehr-aktuell-xlsx-2080110.xlsx`,
/// worksheet `1.1.5` (`sheet13.xml`). `D166` is the shared-formula anchor
/// (`t="shared"`, `si="115"`, `ref="D166:J166"`) holding `D91/D64*100-100`;
/// follower `E166` has the same `si` but no formula text. Excel shifts the
/// relative references for `E166` to `E91/E64*100-100`. Default deleted `E64`
/// from hidden row 64, causing `#DIV/0!` after recalculation and a visual
/// difference.
#[test]
fn shared_formula_protects_shifted_hidden_inputs() {
    let xml = br#"<worksheet><sheetData>
        <row r="64" hidden="1"><c r="D64"><v>1869.695</v></c>
            <c r="E64"><v>4232.35</v></c><c r="F64"><v>296.63</v></c></row>
        <row r="166"><c r="D166"><f t="shared" ref="D166:J166" si="115">D91/D64*100-100</f></c>
            <c r="E166"><f t="shared" si="115"/></c>
            <c r="J166"><f t="shared" si="115"/></c></row>
    </sheetData></worksheet>"#;
    let mut protected = ProtectedWorkbookRefs::default();
    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet13.xml",
            sheet_name: "1.1.5",
            xml,
        }],
        &mut protected,
        None,
        None,
    );

    assert!(matches!(result, ScanResult::Safe));
    for col in 4..=10 {
        assert!(protected.contains("xl/worksheets/sheet13.xml", col, 64));
        assert!(protected.contains("xl/worksheets/sheet13.xml", col, 91));
    }
    assert!(!protected.contains("xl/worksheets/sheet13.xml", 3, 64));
    assert!(!protected.contains("xl/worksheets/sheet13.xml", 11, 64));
}

#[test]
fn shared_formula_preserves_absolute_axes_and_other_sheet_scope() {
    let xml = br#"<worksheet><sheetData><row r="10">
        <c r="B10"><f si="7" ref="B10:D12" t="shared">Inputs!$E5+Inputs!F$6+Inputs!$G$7</f></c>
        <c r="D12"><f t="shared" si="7"/></c>
    </row></sheetData></worksheet>"#;
    let mut names = HashMap::new();
    names.insert("Inputs".into(), "xl/worksheets/sheet2.xml".into());
    let mut protected = ProtectedWorkbookRefs::default();
    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet1.xml",
            sheet_name: "Output",
            xml,
        }],
        &mut protected,
        Some(&names),
        None,
    );

    assert!(matches!(result, ScanResult::Safe));
    for row in 5..=7 {
        assert!(protected.contains("xl/worksheets/sheet2.xml", 5, row));
    }
    for col in 6..=8 {
        assert!(protected.contains("xl/worksheets/sheet2.xml", col, 6));
    }
    assert!(protected.contains("xl/worksheets/sheet2.xml", 7, 7));
    assert!(!protected.contains("xl/worksheets/sheet1.xml", 5, 5));
    assert!(!protected.contains("xl/worksheets/sheet2.xml", 5, 8));
}

#[test]
fn shared_formula_resolves_xml_escaped_sheet_names() {
    let xml = br#"<worksheet><c r="B10"><f t="shared" ref="B10:C10" si="2">&apos;Input&apos;&apos;s data&apos;!A1</f></c>
        <c r="C10"><f t="shared" si="2"/></c></worksheet>"#;
    let mut names = HashMap::new();
    names.insert("Input's data".into(), "xl/worksheets/sheet2.xml".into());
    let mut protected = ProtectedWorkbookRefs::default();
    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet1.xml",
            sheet_name: "Output",
            xml,
        }],
        &mut protected,
        Some(&names),
        None,
    );

    assert!(matches!(result, ScanResult::Safe));
    assert!(protected.contains("xl/worksheets/sheet2.xml", 1, 1));
    assert!(protected.contains("xl/worksheets/sheet2.xml", 2, 1));
    assert!(!protected.contains("xl/worksheets/sheet1.xml", 2, 1));
}

#[test]
fn unresolved_shared_formula_prevents_reference_sensitive_cleanup() {
    for xml in [
        br#"<worksheet><c r="E166"><f t="shared" si="115"/></c></worksheet>"#.as_slice(),
        br#"<worksheet><c r="D166"><f t="shared" si="115">D64</f></c></worksheet>"#,
        br#"<worksheet><c r="XFD1"><f t="shared" si="115" ref="XFD1:XFD2">XFD1048576</f></c></worksheet>"#,
    ] {
        let mut protected = ProtectedWorkbookRefs::default();
        let result = scan_for_references_by_sheet(
            &[SheetScanBuffer { sheet_file: "xl/worksheets/sheet1.xml", sheet_name: "Sheet1", xml }],
            &mut protected, None, None,
        );
        assert!(matches!(result, ScanResult::Unsafe(_)));
    }
}

#[test]
fn shared_followers_do_not_swallow_the_next_formula_anchor() {
    let xml = br#"<worksheet><sheetData><row r="10">
        <c r="A10"><f t="shared" ref="A10:B10" si="1">A1</f></c>
        <c r="B10"><f t="shared" si="1"/></c>
        <c r="C10"><f t="shared" ref="C10:D10" si="2">C2</f></c>
        <c r="D10"><f t="shared" si="2"/></c>
    </row></sheetData></worksheet>"#;
    let mut protected = ProtectedWorkbookRefs::default();
    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet1.xml",
            sheet_name: "Sheet1",
            xml,
        }],
        &mut protected,
        None,
        None,
    );

    assert!(matches!(result, ScanResult::Safe));
    for (col, row) in [(1, 1), (2, 1), (3, 2), (4, 2)] {
        assert!(protected.contains("xl/worksheets/sheet1.xml", col, row));
    }
}

#[test]
fn prefixed_shared_formula_without_cell_context_is_unsafe() {
    let xml = br#"<x:worksheet><x:c r="B10"><x:f t="shared" ref="B10:C10" si="2">A1</x:f></x:c>
        <x:c r="C10"><x:f t="shared" si="2"/></x:c></x:worksheet>"#;
    let mut protected = ProtectedWorkbookRefs::default();
    let result = scan_for_references_by_sheet(
        &[SheetScanBuffer {
            sheet_file: "xl/worksheets/sheet1.xml",
            sheet_name: "Sheet1",
            xml,
        }],
        &mut protected,
        None,
        None,
    );

    assert!(matches!(result, ScanResult::Unsafe(_)));
}

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
fn named_chart_sources_protect_whole_columns_and_local_rows() {
    let names = HashMap::from([
        ("Data".to_owned(), "xl/worksheets/sheet1.xml".to_owned()),
        ("Results".to_owned(), "xl/worksheets/sheet2.xml".to_owned()),
    ]);
    let order = vec![
        "xl/worksheets/sheet1.xml".to_owned(),
        "xl/worksheets/sheet2.xml".to_owned(),
    ];
    let scanner = PreparedReferenceScanner::new(&[], Some(&names), Some(&order));
    let mut refs = ProtectedWorkbookRefs::default();
    let workbook = SheetScanBuffer {
        sheet_file: "xl/workbook.xml",
        sheet_name: "Workbook",
        xml: br#"<workbook><definedNames>
            <definedName name="SeriesValues">Data!$B:$D</definedName>
            <definedName name="SeriesLabels" localSheetId="1">$7:$9</definedName>
        </definedNames></workbook>"#,
    };
    assert!(matches!(
        scanner.scan(&[workbook], &mut refs),
        ScanResult::Safe
    ));
    assert!(matches!(
        scanner.scan_object_references(
            br#"<c:f>Book.xlsx!SeriesValues</c:f><c:f>Results!SeriesLabels</c:f>"#,
            &mut refs,
        ),
        ScanResult::Safe
    ));
    assert!(refs.contains("xl/worksheets/sheet1.xml", 3, 1_048_576));
    assert!(!refs.contains("xl/worksheets/sheet1.xml", 5, 20));
    assert!(refs.contains("xl/worksheets/sheet2.xml", 16_384, 8));
    assert!(!refs.contains("xl/worksheets/sheet2.xml", 3, 20));
}

#[test]
fn formulas_protect_whole_ranges_with_quoted_sheet_names() {
    let names = HashMap::from([("O'Brien".to_owned(), "xl/worksheets/sheet1.xml".to_owned())]);
    let buffer = SheetScanBuffer {
        sheet_file: "xl/worksheets/sheet2.xml",
        sheet_name: "Totals",
        xml: br#"<worksheet><sheetData><row r="1"><c r="A1">
            <f>SUM('O''Brien'!$A:$A)+SUM($12:$14)</f>
        </c></row></sheetData></worksheet>"#,
    };
    let mut refs = ProtectedWorkbookRefs::default();
    assert!(matches!(
        scan_for_references_by_sheet(&[buffer], &mut refs, Some(&names), None,),
        ScanResult::Safe
    ));
    assert!(refs.contains("xl/worksheets/sheet1.xml", 1, 500));
    assert!(!refs.contains("xl/worksheets/sheet1.xml", 2, 500));
    assert!(refs.contains("xl/worksheets/sheet2.xml", 500, 13));
    assert!(!refs.contains("xl/worksheets/sheet2.xml", 1, 500));
}

#[test]
fn print_metadata_does_not_protect_every_cell_in_the_print_area() {
    let names = HashMap::from([("Data".to_owned(), "xl/worksheets/sheet1.xml".to_owned())]);
    let buffer = SheetScanBuffer {
        sheet_file: "xl/workbook.xml",
        sheet_name: "Workbook",
        xml: br#"<workbook><definedNames>
            <definedName name="_xlnm.Print_Area">Data!$A$1:$XFD$1048576</definedName>
            <definedName name="_xlnm.Print_Titles">Data!$A:$Z,Data!$1:$100</definedName>
        </definedNames></workbook>"#,
    };
    let mut refs = ProtectedWorkbookRefs::default();
    assert!(matches!(
        scan_for_references_by_sheet(&[buffer], &mut refs, Some(&names), None,),
        ScanResult::Safe
    ));
    assert!(!refs.contains("xl/worksheets/sheet1.xml", 1, 1));
    assert!(!refs.contains("xl/worksheets/sheet1.xml", 26, 100));
    assert!(!refs.contains("xl/worksheets/sheet1.xml", 16_384, 1_048_576));
}

#[test]
fn named_chart_source_with_unresolved_table_columns_disables_candidate_removal() {
    let scanner = PreparedReferenceScanner::new(&[], None, None);
    let mut refs = ProtectedWorkbookRefs::default();
    let workbook = SheetScanBuffer {
        sheet_file: "xl/workbook.xml",
        sheet_name: "Workbook",
        xml: br#"<workbook><definedNames>
            <definedName name="SeriesData">Table1[Values]</definedName>
        </definedNames></workbook>"#,
    };
    // The chart itself names the series; the unsafe expression is in the name.
    assert!(matches!(
        scanner.scan_object_references(br#"<c:f>Book.xlsx!SeriesData</c:f>"#, &mut refs,),
        ScanResult::Safe
    ));
    assert!(
        matches!(scanner.scan(&[workbook], &mut refs), ScanResult::Unsafe(reason)
        if reason == "unresolved structured reference")
    );
}

#[test]
fn table_metadata_and_reference_lookalikes_do_not_block_cleanup() {
    let scanner = PreparedReferenceScanner::new(&[], None, None);
    let mut refs = ProtectedWorkbookRefs::default();
    assert!(matches!(
        scanner.scan_object_references(
            br#"<table name="Table1" ref="A1:XFD1048576"><tableColumns>
            <tableColumn name="Values"/></tableColumns></table>"#,
            &mut refs,
        ),
        ScanResult::Safe
    ));
    assert!(!refs.contains("sheet1", 1, 1));
    for formula in [
        "\"Table1[Values]\"",
        "&quot;Table1[Values]&quot;",
        "'[Book.xlsx]Sheet'!A1",
        "[1]Sheet!A1",
    ] {
        assert_eq!(
            reference_scan_blocker(formula.as_bytes(), unsafe_function_pattern()),
            None
        );
    }
}

#[test]
fn cell_and_object_formulas_detect_unresolved_structured_references() {
    for formula in [
        "SUM(Table1[Values])",
        "SUM([Values])",
        "[@Values]",
        "Table1[[#All],[Values]]",
    ] {
        assert_eq!(
            reference_scan_blocker(formula.as_bytes(), unsafe_function_pattern()),
            Some("unresolved structured reference".to_owned())
        );
    }
    let scanner = PreparedReferenceScanner::new(&[], None, None);
    assert!(matches!(
        scanner.scan_object_references(
            br#"<c:f>Table1[Values]</c:f>"#,
            &mut ProtectedWorkbookRefs::default(),
        ),
        ScanResult::Unsafe(_)
    ));
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
                <definedName name="LocalData" localSheetId="1">$S$41</definedName>
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
                <definedName name="LocalData" localSheetId="1">$S$41</definedName>
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

#[test]
fn byte_sheet_names_preserve_unicode_and_escaped_apostrophes() {
    let names: HashMap<Vec<u8>, String> = [
        ("Umsätze".as_bytes().to_vec(), "sheet1".to_owned()),
        (b"O'Brien".to_vec(), "sheet2".to_owned()),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        resolve_sheet_name("Umsätze".as_bytes(), &names),
        Some("sheet1")
    );
    assert_eq!(
        resolve_sheet_name("'Umsätze'".as_bytes(), &names),
        Some("sheet1")
    );
    assert_eq!(resolve_sheet_name(b"'O''Brien'", &names), Some("sheet2"));
    assert_eq!(resolve_sheet_name(b"Unknown", &names), None);
}
