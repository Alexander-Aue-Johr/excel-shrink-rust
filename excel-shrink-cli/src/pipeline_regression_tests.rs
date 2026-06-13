use super::*;
use excel_shrink_core::models::{
    CleanSheetParameters, SheetPreprocessConfig, WorkbookSpecificPatterns,
};
use excel_shrink_core::sheet::output::trim_trailing_layout_only_rows;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

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
        disable_unreferenced_whitespace_cells: false,
        disable_unreferenced_hidden_content_cells: false,
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
        col: 19,
        row: 41,
        cause: CleanupCause::UnreferencedHiddenContentCells,
    }
}

fn candidate_x4_whitespace() -> CleanupCandidate {
    CleanupCandidate {
        col: 24,
        row: 4,
        cause: CleanupCause::UnreferencedWhitespaceCells,
    }
}

fn default_sheet_params() -> CleanSheetParameters {
    CleanSheetParameters {
        xlsx_file_path: "test.xlsx".to_string(),
        zip_info_filename: "xl/worksheets/sheet1.xml".to_string(),
        compress_method: CompressionMethod::Deflated,
        patterns: WorkbookSpecificPatterns {
            xfs_by_index: HashMap::new(),
            only_font_style_ids: Vec::new(),
            fill_or_border_or_protection_ids: Vec::new(),
            whitespace_indices: None,
        },
        advanced_cleanup: None,
        preprocess: SheetPreprocessConfig::default(),
        cleanup: Default::default(),
        collect_cleanup_stats: false,
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

#[test]
fn phase3_trims_trailing_row_after_deferred_candidate_cleanup() {
    let xml =
        br#"<row r="1"><c r="A1"><v>keep</v></c></row><row r="41"><c r="S41"><v>secret</v></c></row>"#
            .to_vec();
    let candidates = vec![candidate_s41()];
    let protected = ProtectedWorkbookRefs::default();
    let params = default_sheet_params();
    let mut stats = CleanupStats::enabled();
    let mut metrics = CleanSheetMetrics {
        cleanup_stats: CleanupStats::enabled(),
        ..CleanSheetMetrics::default()
    };

    let cleaned = apply_deferred_candidate_cleanup(
        xml,
        &candidates,
        &protected,
        "xl/worksheets/sheet1.xml",
        &mut stats,
    );
    let cleaned = trim_trailing_layout_only_rows(&cleaned, &params, &mut metrics);
    let cleaned = String::from_utf8_lossy(&cleaned);

    assert!(cleaned.contains(r#"r="1""#));
    assert!(!cleaned.contains(r#"r="41""#));
}

#[test]
fn pipeline_collapses_consecutive_hidden_empty_rows_from_streamed_sheet_entry() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_root = std::env::temp_dir().join(format!(
        "excel_shrink_streamed_hidden_rows_{}_{}",
        std::process::id(),
        unique
    ));
    let input_dir = temp_root.join("in");
    let output_dir = temp_root.join("out");
    std::fs::create_dir_all(&input_dir).unwrap();

    let input = input_dir.join("hidden_rows.xlsx");
    let file = File::create(&input).unwrap();
    let mut writer = ZipWriter::new(file);
    let options = FileOptions::default().compression_method(CompressionMethod::Deflated);

    writer.start_file("xl/workbook.xml", options).unwrap();
    writer
        .write_all(br#"<workbook><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#)
        .unwrap();
    writer
        .start_file("xl/worksheets/sheet1.xml", options)
        .unwrap();
    writer
        .write_all(
            br#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="307905" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307906" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307907" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="307908" spans="1:117" hidden="1" x14ac:dyDescent="0.25"></row><row r="400000"><c r="A400000"><v>2</v></c></row></sheetData></worksheet>"#,
        )
        .unwrap();
    writer.finish().unwrap();

    let mut cli = cli_with_analysis_flags(false, None);
    cli.path = input.clone();
    cli.output = output_dir.clone();
    cli.force_overwrite = true;
    cli.disable_multiprocessing = true;
    cli.chunk_size = 64;
    cli.preprocess_hidden_rows = true;
    cli.writer_workers = Some(1);

    process_files(vec![input.clone()], &cli).unwrap();

    let output = output_dir.join("hidden_rows.xlsx");
    let file = File::open(output).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    let mut sheet = String::new();
    archive
        .by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut sheet)
        .unwrap();

    assert!(sheet.contains(r#"r="307905""#));
    assert!(!sheet.contains(r#"r="307906""#));
    assert!(!sheet.contains(r#"r="307907""#));
    assert!(!sheet.contains(r#"r="307908""#));

    let _ = std::fs::remove_dir_all(temp_root);
}
