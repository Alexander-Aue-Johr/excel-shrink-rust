use super::*;
use excel_shrink_core::models::CellCoordinate;

fn cli_with_analysis_flags(analysis_mode: bool, analysis_csv: Option<PathBuf>) -> Cli {
    Cli {
        path: PathBuf::from("input.xlsx"),
        output: PathBuf::from("out"),
        recursive: false,
        disable_multiprocessing: false,
        chunk_size: 16_777_216,
        force_overwrite: false,
        sheets: None,
        only_clean_workbook: false,
        disable_sheetdata_splitting: false,
        analysis_mode,
        analysis_csv,
        working_directory: None,
        disable_tail_pruning: false,
        disable_trailing_layout_only_rows: false,
        disable_extraneous_column_merge: false,
        preprocess_hidden_rows: false,
        preprocess_empty_cells: false,
        writer_workers: None,
        compression_level: 1,
        store_output: false,
        remove_unreferenced_whitespace_cells: false,
        remove_unreferenced_hidden_content_cells: false,
    }
}

#[test]
fn analysis_csv_implies_attribution_safe_analysis() {
    let cli = cli_with_analysis_flags(false, Some(PathBuf::from("analysis.csv")));

    assert!(attribution_safe_analysis(&cli));
}

#[test]
fn no_analysis_flags_uses_production_tail_pruning_policy() {
    let cli = cli_with_analysis_flags(false, None);

    assert!(!attribution_safe_analysis(&cli));
}

#[test]
fn reduction_ratio_uses_original_bytes_as_denominator() {
    assert_eq!(reduction_ratio(25, 100), 0.25);
    assert_eq!(reduction_ratio(-10, 100), -0.1);
    assert_eq!(reduction_ratio(25, 0), 0.0);
}

#[test]
fn analysis_csv_row_includes_net_bytes_and_ratios() {
    let row = AnalysisCsvRow {
        input_file: "input,quoted.xlsx".to_string(),
        sheet_file: "xl/worksheets/sheet1.xml".to_string(),
        cause: CleanupCause::EmptyCellsWithoutContent,
        removed_items: 3,
        removed_bytes_uncompressed_xml: 50,
        added_bytes_uncompressed_xml: 5,
        reduction_bytes_uncompressed_xml: 45,
        reduction_ratio: 0.225,
        original_sheet_xml_bytes: 200,
        cleaned_sheet_xml_bytes: 160,
        sheet_reduction_bytes_uncompressed_xml: 40,
        sheet_reduction_ratio: 0.2,
        analysis_mode: true,
    };
    let mut out = Vec::new();

    write_analysis_csv_row(&mut out, &row).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "\"input,quoted.xlsx\",xl/worksheets/sheet1.xml,EmptyCellsWithoutContent,3,50,5,45,0.225000,200,160,40,0.200000,true\n"
    );
}

#[test]
fn analysis_csv_header_uses_long_format_columns() {
    let mut out = Vec::new();

    write_analysis_csv_header(&mut out).unwrap();
    let header = String::from_utf8(out).unwrap();

    assert_eq!(
        header,
        "input_file,sheet_file,cause,removed_items,removed_bytes_uncompressed_xml,added_bytes_uncompressed_xml,reduction_bytes_uncompressed_xml,reduction_ratio,original_sheet_xml_bytes,cleaned_sheet_xml_bytes,sheet_reduction_bytes_uncompressed_xml,sheet_reduction_ratio,analysis_mode\n"
    );
}

fn candidate_s41() -> CleanupCandidate {
    CleanupCandidate {
        coordinate: CellCoordinate {
            column: b"S".to_vec(),
            row: b"41".to_vec(),
        },
        cause: CleanupCause::UnreferencedHiddenContentCells,
    }
}

fn candidate_x4_whitespace() -> CleanupCandidate {
    CleanupCandidate {
        coordinate: CellCoordinate {
            column: b"X".to_vec(),
            row: b"4".to_vec(),
        },
        cause: CleanupCause::UnreferencedWhitespaceCells,
    }
}

#[test]
fn phase3_removes_unprotected_hidden_candidate_cell() {
    let xml = br#"<sheetData><row r="41"><c r="S41"><v>secret</v></c><c r="A1"><v>x</v></c></row></sheetData>"#
        .to_vec();
    let candidates = vec![candidate_s41()];
    let protected = ProtectedWorkbookRefs::default();
    let mut stats = CleanupStats::enabled();

    let cleaned = apply_deferred_candidate_cleanup(
        xml,
        &candidates,
        &protected,
        "xl/worksheets/sheet1.xml",
        &mut stats,
    );

    assert!(
        !String::from_utf8_lossy(&cleaned).contains("r=\"S41\""),
        "sheet1!S41 should be removed when unprotected"
    );
}

#[test]
fn phase3_removes_unprotected_x4_whitespace_candidate_cell() {
    let xml = br#"<sheetData><row r="4"><c r="W4" s="91"><v>118</v></c><c r="X4" s="91" t="s"><v>15801</v></c><c r="Y4" s="91"><v>7</v></c></row></sheetData>"#
        .to_vec();
    let candidates = vec![candidate_x4_whitespace()];
    let protected = ProtectedWorkbookRefs::default();
    let mut stats = CleanupStats::enabled();

    let cleaned = apply_deferred_candidate_cleanup(
        xml,
        &candidates,
        &protected,
        "xl/worksheets/sheet1.xml",
        &mut stats,
    );

    let cleaned = String::from_utf8_lossy(&cleaned);
    assert!(!cleaned.contains(r#"r="X4""#));
    assert!(cleaned.contains(r#"r="W4""#));
    assert!(cleaned.contains(r#"r="Y4""#));
}

#[test]
fn phase3_keeps_sheet1_candidate_when_protected_by_sheet1_ref() {
    let xml = br#"<sheetData><row r="41"><c r="S41"><v>secret</v></c></row></sheetData>"#.to_vec();
    let candidates = vec![candidate_s41()];
    let mut protected = ProtectedWorkbookRefs::default();
    protected
        .refs_for_sheet_mut("xl/worksheets/sheet1.xml")
        .insert_cell(19, 41);
    let mut stats = CleanupStats::enabled();

    let cleaned = apply_deferred_candidate_cleanup(
        xml,
        &candidates,
        &protected,
        "xl/worksheets/sheet1.xml",
        &mut stats,
    );

    assert!(
        String::from_utf8_lossy(&cleaned).contains("r=\"S41\""),
        "sheet1!S41 should be kept when protected on target sheet"
    );
}
