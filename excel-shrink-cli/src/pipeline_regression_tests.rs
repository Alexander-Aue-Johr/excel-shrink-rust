use super::*;
use crate::LogLevel;
use excel_shrink_core::models::{
    CleanSheetParameters, SheetPreprocessConfig, WorkbookSpecificPatterns,
};
use excel_shrink_core::sheet::output::trim_trailing_layout_only_rows;
use excel_shrink_core::sheet::sheet_data::{
    clean_empty_rows_from_after_cell_range, clean_selected_rows_and_cells_in_cell_range,
};
use std::collections::{HashMap, HashSet};
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
        parallel_range_chunk_size: 134_217_728,
        force_overwrite: false,
        sheets: Vec::new(),
        only_clean_workbook: false,
        disable_sheetdata_splitting: false,
        analysis_mode,
        analysis_csv,
        working_directory: None,
        disable_tail_pruning: false,
        disable_trailing_layout_only_rows: false,
        disable_extraneous_column_merge: false,
        disable_column_tail_fill_merge: false,
        disable_default_font_representatives: false,
        disable_empty_font_style_representatives: false,
        disable_column_font_style_representatives: false,
        preprocess_empty_cells: false,
        writer_workers: None,
        max_active_workbooks: None,
        compression_level: 1,
        store_output: false,
        keep_unreferenced_whitespace_cells: false,
        remove_unreferenced_hidden_content_cells: false,
        remove_apply_protection_without_sheet_protection: false,
        console_log_level: LogLevel::Info,
        file_log_level: None,
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
fn empty_sheet_selection_cleans_all_sheets() {
    let selected = selected_sheet_targets(&[]);

    assert!(sheet_selected_for_cleanup(
        "xl/worksheets/sheet84.xml",
        &selected
    ));
}

#[test]
fn sheet_selection_accepts_basename_or_zip_path() {
    let selected = selected_sheet_targets(&[
        "sheet84.xml".to_string(),
        "worksheets/sheet2.xml".to_string(),
        "xl/worksheets/sheet7.xml".to_string(),
    ]);

    assert!(sheet_selected_for_cleanup(
        "xl/worksheets/sheet84.xml",
        &selected
    ));
    assert!(sheet_selected_for_cleanup(
        "xl/worksheets/sheet2.xml",
        &selected
    ));
    assert!(sheet_selected_for_cleanup(
        "xl/worksheets/sheet7.xml",
        &selected
    ));
    assert!(!sheet_selected_for_cleanup(
        "xl/worksheets/sheet1.xml",
        &selected
    ));
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
            no_empty_cell_visible_style_ids: Vec::new(),
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

    let (cleaned, touched_rows) = apply_deferred_candidate_cleanup(
        xml,
        &candidates,
        &protected,
        "xl/worksheets/sheet1.xml",
        &mut stats,
    );

    assert_eq!(touched_rows, [41].into_iter().collect());
    assert!(
        !String::from_utf8_lossy(&cleaned).contains("r=\"S41\""),
        "sheet1!S41 should be removed when unprotected"
    );
}

#[test]
fn phase3_strips_unprotected_x4_whitespace_candidate_cell_content() {
    let xml = br#"<sheetData><row r="4"><c r="W4" s="91"><v>118</v></c><c r="X4" s="91" t="s"><v>15801</v></c><c r="Y4" s="91"><v>7</v></c></row></sheetData>"#
        .to_vec();
    let candidates = vec![candidate_x4_whitespace()];
    let protected = ProtectedWorkbookRefs::default();
    let mut stats = CleanupStats::enabled();

    let (cleaned, touched_rows) = apply_deferred_candidate_cleanup(
        xml,
        &candidates,
        &protected,
        "xl/worksheets/sheet1.xml",
        &mut stats,
    );

    assert_eq!(touched_rows, [4].into_iter().collect());
    let cleaned = String::from_utf8_lossy(&cleaned);
    assert!(cleaned.contains(r#"<c r="X4" s="91" t="s"/>"#));
    assert!(!cleaned.contains(r#"<c r="X4" s="91" t="s"><v>15801</v></c>"#));
    assert!(cleaned.contains(r#"r="W4""#));
    assert!(cleaned.contains(r#"r="Y4""#));
}

#[test]
fn phase3_row_cleanup_preserves_stripped_whitespace_cell_with_visible_fill() {
    let xml = br#"<sheetData><row r="4"><c r="W4" s="91"><v>118</v></c><c r="X4" s="37" t="s"><v>15801</v></c><c r="Y4" s="91"><v>7</v></c></row></sheetData>"#
        .to_vec();
    let candidates = vec![candidate_x4_whitespace()];
    let protected = ProtectedWorkbookRefs::default();
    let mut params = default_sheet_params();
    params.patterns.xfs_by_index.insert(
        37,
        excel_shrink_core::models::Xf::builder()
            .font_id(Some(0))
            .fill_id(Some(2))
            .border_id(Some(0))
            .apply_fill(true)
            .has_visible_fill(true)
            .has_fill_or_border(true)
            .has_apply_fill_or_border(true)
            .build_xf(),
    );
    let mut stats = CleanupStats::enabled();
    let mut metrics = CleanSheetMetrics {
        cleanup_stats: CleanupStats::enabled(),
        ..CleanSheetMetrics::default()
    };

    let (cleaned, touched_rows) = apply_deferred_candidate_cleanup(
        xml,
        &candidates,
        &protected,
        "xl/worksheets/sheet1.xml",
        &mut stats,
    );
    let cleaned =
        clean_selected_rows_and_cells_in_cell_range(&cleaned, &params, &mut metrics, &touched_rows)
            .expect("row cleanup should succeed");
    let cleaned = String::from_utf8_lossy(&cleaned);

    assert!(cleaned.contains(r#"<c r="X4" s="37" t="s"/>"#));
    assert!(!cleaned.contains(r#"<v>15801</v>"#));
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

    let (cleaned, touched_rows) = apply_deferred_candidate_cleanup(
        xml,
        &candidates,
        &protected,
        "xl/worksheets/sheet1.xml",
        &mut stats,
    );

    assert!(touched_rows.is_empty());
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
        tail_anchor_max_row: Some(41),
        ..CleanSheetMetrics::default()
    };

    let (cleaned, touched_rows) = apply_deferred_candidate_cleanup(
        xml,
        &candidates,
        &protected,
        "xl/worksheets/sheet1.xml",
        &mut stats,
    );
    assert_eq!(touched_rows, [41].into_iter().collect());
    invalidate_tail_anchor_for_touched_rows(&mut metrics, &touched_rows);
    let cleaned = trim_trailing_layout_only_rows(&cleaned, &params, &mut metrics);
    let cleaned = String::from_utf8_lossy(&cleaned);

    assert!(cleaned.contains(r#"r="1""#));
    assert!(!cleaned.contains(r#"r="41""#));
}

#[test]
fn phase3_sheet_data_block_cleanup_preserves_worksheet_tail() {
    let prefix = b"<worksheet><sheetData>";
    let sheet_data =
        br#"<row r="1"><c r="A1"><v>keep</v></c></row><row r="41"><c r="S41"><v>secret</v></c></row>"#;
    let suffix =
        br#"</sheetData><mergeCells count="1"><mergeCell ref="A1:A1"/></mergeCells></worksheet>"#;
    let mut xml = Vec::new();
    xml.extend_from_slice(prefix);
    let sheet_data_range = xml.len()..xml.len() + sheet_data.len();
    xml.extend_from_slice(sheet_data);
    xml.extend_from_slice(suffix);
    let candidates = vec![candidate_s41()];
    let protected = ProtectedWorkbookRefs::default();
    let params = default_sheet_params();
    let mut stats = CleanupStats::enabled();
    let mut metrics = CleanSheetMetrics {
        cleanup_stats: CleanupStats::enabled(),
        ..CleanSheetMetrics::default()
    };

    let mut touched_rows = HashSet::new();
    let (cleaned, sheet_data_range) =
        transform_sheet_data_range(&xml, sheet_data_range, |sheet_data| {
            let (cleaned, rows) = apply_deferred_candidate_cleanup(
                sheet_data.to_vec(),
                &candidates,
                &protected,
                "xl/worksheets/sheet1.xml",
                &mut stats,
            );
            touched_rows = rows;
            Ok(cleaned)
        })
        .expect("candidate cleanup should succeed");
    assert_eq!(touched_rows, [41].into_iter().collect());

    let (cleaned, sheet_data_range) =
        transform_sheet_data_range(&cleaned, sheet_data_range, |sheet_data| {
            clean_selected_rows_and_cells_in_cell_range(
                sheet_data,
                &params,
                &mut metrics,
                &touched_rows,
            )
        })
        .expect("row cleanup should succeed");
    let (cleaned, sheet_data_range) =
        transform_sheet_data_range(&cleaned, sheet_data_range, |sheet_data| {
            clean_empty_rows_from_after_cell_range(sheet_data, &params, &mut metrics)
        })
        .expect("empty row cleanup should succeed");
    let (cleaned, _sheet_data_range) =
        transform_sheet_data_range(&cleaned, sheet_data_range, |sheet_data| {
            Ok(
                excel_shrink_core::sheet::output::trim_trailing_layout_only_rows_vec(
                    sheet_data.to_vec(),
                    &params,
                    &mut metrics,
                ),
            )
        })
        .expect("tail cleanup should succeed");
    let cleaned = String::from_utf8_lossy(&cleaned);

    assert!(cleaned.contains(r#"r="1""#));
    assert!(!cleaned.contains(r#"r="41""#));
    assert!(cleaned.contains("</sheetData>"));
    assert!(cleaned.contains("<mergeCells"));
    assert!(cleaned.ends_with("</worksheet>"));
}

#[test]
fn phase3_preserves_hidden_rows_emptied_by_deferred_candidate_cleanup() {
    let xml = br#"<row r="44504"><c r="A44504"><v>keep</v></c></row><row r="44505" spans="1:117" hidden="1" x14ac:dyDescent="0.25"><c r="S44505"><v>secret</v></c></row><row r="44506" spans="1:117" hidden="1" x14ac:dyDescent="0.25"><c r="S44506"><v>secret</v></c></row><row r="44596"><c r="A44596"><v>keep</v></c></row><row r="44597" spans="1:117" hidden="1" x14ac:dyDescent="0.25"><c r="S44597"><v>secret</v></c></row><row r="44598" spans="1:117" hidden="1" x14ac:dyDescent="0.25"><c r="S44598"><v>secret</v></c></row><row r="44650"><c r="A44650"><v>keep</v></c></row>"#.to_vec();
    let candidates = vec![
        CleanupCandidate {
            col: 19,
            row: 44505,
            cause: CleanupCause::UnreferencedHiddenContentCells,
        },
        CleanupCandidate {
            col: 19,
            row: 44506,
            cause: CleanupCause::UnreferencedHiddenContentCells,
        },
        CleanupCandidate {
            col: 19,
            row: 44597,
            cause: CleanupCause::UnreferencedHiddenContentCells,
        },
        CleanupCandidate {
            col: 19,
            row: 44598,
            cause: CleanupCause::UnreferencedHiddenContentCells,
        },
    ];
    let protected = ProtectedWorkbookRefs::default();
    let params = default_sheet_params();
    let mut stats = CleanupStats::enabled();
    let mut metrics = CleanSheetMetrics {
        cleanup_stats: CleanupStats::enabled(),
        ..CleanSheetMetrics::default()
    };

    let (cleaned, touched_rows) = apply_deferred_candidate_cleanup(
        xml,
        &candidates,
        &protected,
        "xl/worksheets/sheet1.xml",
        &mut stats,
    );
    assert_eq!(
        touched_rows,
        [44505, 44506, 44597, 44598].into_iter().collect()
    );
    let cleaned =
        clean_selected_rows_and_cells_in_cell_range(&cleaned, &params, &mut metrics, &touched_rows)
            .expect("row cleanup should succeed");
    let cleaned = clean_empty_rows_from_after_cell_range(&cleaned, &params, &mut metrics).unwrap();
    let cleaned = String::from_utf8_lossy(&cleaned);

    assert!(cleaned.contains(r#"r="44504""#));
    assert!(cleaned.contains(r#"<row r="44505""#));
    assert!(cleaned.contains(r#"<row r="44506""#));
    assert!(cleaned.contains(r#"hidden="1""#));
    assert!(cleaned.contains(r#"r="44596""#));
    assert!(cleaned.contains(r#"<row r="44597""#));
    assert!(cleaned.contains(r#"<row r="44598""#));
    assert!(cleaned.contains(r#"r="44650""#));
    assert!(!cleaned.contains("secret"));
}

#[test]
fn pipeline_preserves_hidden_empty_rows_from_streamed_sheet_entry() {
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
    assert!(sheet.contains(r#"r="307906""#));
    assert!(sheet.contains(r#"r="307907""#));
    assert!(sheet.contains(r#"r="307908""#));

    let _ = std::fs::remove_dir_all(temp_root);
}

#[test]
fn calc_chain_removes_missing_cells_for_processed_sheet_only() {
    let workbook_xml =
        br#"<workbook><sheets><sheet name="Sheet1" sheetId="7" r:id="rId1"/></sheets></workbook>"#;
    let workbook_rels_xml = br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let sheet_id_to_file = build_sheet_id_to_file_map(workbook_xml, workbook_rels_xml);
    let mut known_cells_by_sheet = HashMap::new();
    known_cells_by_sheet.insert(
        "xl/worksheets/sheet1.xml".to_string(),
        ["A1".to_string(), "C3".to_string()].into_iter().collect(),
    );

    let cleaned = clean_calc_chain(
        br#"<calcChain><c r="A1" i="7"/><c r="B2"/><c r="C3"/></calcChain>"#.to_vec(),
        &sheet_id_to_file,
        &known_cells_by_sheet,
    );
    let cleaned = String::from_utf8(cleaned).unwrap();

    assert!(cleaned.contains(r#"r="A1""#));
    assert!(!cleaned.contains(r#"r="B2""#));
    assert!(cleaned.contains(r#"r="C3""#));
}

#[test]
fn pipeline_cleans_calc_chain_entries_for_cells_removed_from_sheet() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_root = std::env::temp_dir().join(format!(
        "excel_shrink_calc_chain_{}_{}",
        std::process::id(),
        unique
    ));
    let input_dir = temp_root.join("in");
    let output_dir = temp_root.join("out");
    std::fs::create_dir_all(&input_dir).unwrap();

    let input = input_dir.join("calc_chain.xlsx");
    let file = File::create(&input).unwrap();
    let mut writer = ZipWriter::new(file);
    let options = FileOptions::default().compression_method(CompressionMethod::Deflated);

    writer.start_file("xl/workbook.xml", options).unwrap();
    writer
        .write_all(br#"<workbook><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#)
        .unwrap();
    writer
        .start_file("xl/_rels/workbook.xml.rels", options)
        .unwrap();
    writer
        .write_all(br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#)
        .unwrap();
    writer
        .start_file("xl/worksheets/sheet1.xml", options)
        .unwrap();
    writer
        .write_all(
            br#"<worksheet><sheetData><row r="1"><c r="A1"><f>1+1</f><v>2</v></c><c r="B1"/><c r="C1"/></row></sheetData></worksheet>"#,
        )
        .unwrap();
    writer.start_file("xl/calcChain.xml", options).unwrap();
    writer
        .write_all(br#"<calcChain><c r="A1" i="1"/><c r="B1"/><c r="C1"/></calcChain>"#)
        .unwrap();
    writer.finish().unwrap();

    let mut cli = cli_with_analysis_flags(false, None);
    cli.path = input.clone();
    cli.output = output_dir.clone();
    cli.force_overwrite = true;
    cli.disable_multiprocessing = true;
    cli.chunk_size = 64;
    cli.writer_workers = Some(1);

    process_files(vec![input.clone()], &cli).unwrap();

    let output = output_dir.join("calc_chain.xlsx");
    let file = File::open(output).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    let mut sheet = String::new();
    archive
        .by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut sheet)
        .unwrap();
    let mut calc_chain = String::new();
    archive
        .by_name("xl/calcChain.xml")
        .unwrap()
        .read_to_string(&mut calc_chain)
        .unwrap();

    assert!(sheet.contains(r#"r="B1""#));
    assert!(!sheet.contains(r#"r="C1""#));
    assert!(calc_chain.contains(r#"r="A1""#));
    assert!(calc_chain.contains(r#"r="B1""#));
    assert!(!calc_chain.contains(r#"r="C1""#));

    let _ = std::fs::remove_dir_all(temp_root);
}
