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

pub(super) fn cli_with_analysis_flags(analysis_mode: bool, analysis_csv: Option<PathBuf>) -> Cli {
    Cli {
        trace_file: None,
        path: PathBuf::from("input.xlsx"),
        output: PathBuf::from("out"),
        recursive: false,
        batch_report: None,
        disable_multiprocessing: false,
        rayon_threads: None,
        chunk_size: 16_777_216,
        parallel_range_chunk_size: crate::DEFAULT_PARALLEL_RANGE_CHUNK_SIZE,
        force_overwrite: false,
        sheets: Vec::new(),
        only_clean_workbook: false,
        disable_sheetdata_splitting: false,
        analysis_mode,
        analysis_csv,
        shrink_evidence_json: None,
        working_directory: None,
        disable_tail_pruning: false,
        disable_trailing_layout_only_rows: false,
        disable_extraneous_column_merge: false,
        disable_column_tail_fill_merge: false,
        remove_textless_default_font_representatives: false,
        remove_textless_style_representatives: false,
        remove_textless_column_style_representatives: false,
        preprocess_empty_cells: false,
        writer_workers: None,
        max_active_workbooks: None,
        compression_level: 1,
        verify_unchanged_xml: false,
        write_workbook_only_changes: false,
        store_output: false,
        remove_unreferenced_whitespace_cells: false,
        keep_unreferenced_hidden_content_cells: false,
        cell_formula_references_only: false,
        inactive_protection_cell_policy: crate::InactiveProtectionCellPolicyArg::Remove,
        console_log_level: LogLevel::Info,
        file_log_level: None,
    }
}

fn copy_test_entry(original: Vec<u8>, cleaned: Vec<u8>) -> ZipArchive<Cursor<Vec<u8>>> {
    let mut source = ZipArchive::new(Cursor::new(original)).unwrap();
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    copy_precompressed_entry(&mut source, 0, &mut output, cleaned).unwrap();
    ZipArchive::new(output.finish().unwrap()).unwrap()
}

fn compressed_test_entry(xml: &[u8], method: CompressionMethod, level: i32) -> Vec<u8> {
    precompress_entry("xl/worksheets/sheet1.xml", xml.to_vec(), method, level).unwrap()
}

fn entry_compressed_size(bytes: &[u8]) -> u64 {
    ZipArchive::new(Cursor::new(bytes))
        .unwrap()
        .by_index(0)
        .unwrap()
        .compressed_size()
}

#[test]
fn unchanged_xml_keeps_the_smaller_compressed_stream() {
    let rows: String = (1..=1000)
        .map(|row| format!(r#"<row r="{row}"><c r="A{row}"><v>7</v></c></row>"#))
        .collect();
    let xml = format!("<worksheet><sheetData>{rows}</sheetData></worksheet>");
    for (original_level, cleaned_level) in [(9, 0), (0, 9)] {
        let original =
            compressed_test_entry(xml.as_bytes(), CompressionMethod::Deflated, original_level);
        let cleaned =
            compressed_test_entry(xml.as_bytes(), CompressionMethod::Deflated, cleaned_level);
        let original_size = entry_compressed_size(&original);
        let cleaned_size = entry_compressed_size(&cleaned);
        assert_ne!(original_size, cleaned_size);

        let mut output = copy_test_entry(original, cleaned);
        let mut entry = output.by_index(0).unwrap();
        assert_eq!(entry.compressed_size(), original_size.min(cleaned_size));
        let mut actual = String::new();
        entry.read_to_string(&mut actual).unwrap();
        assert_eq!(actual, xml);
    }
}

#[test]
fn equal_size_and_crc_do_not_hide_changed_xml() {
    // Equal-length strings with the same CRC32, inside otherwise identical XML.
    let original_xml = b"<worksheet><!--a02552f76dbdf5fd--></worksheet>";
    let cleaned_xml = b"<worksheet><!--670009b22c087272--></worksheet>";
    assert_eq!(original_xml.len(), cleaned_xml.len());
    assert_eq!(crc32fast::hash(original_xml), crc32fast::hash(cleaned_xml));
    let original = compressed_test_entry(original_xml, CompressionMethod::Stored, 0);
    let cleaned = compressed_test_entry(cleaned_xml, CompressionMethod::Stored, 0);

    let mut output = copy_test_entry(original, cleaned);
    let mut actual = Vec::new();
    output
        .by_index(0)
        .unwrap()
        .read_to_end(&mut actual)
        .unwrap();
    assert_eq!(actual, cleaned_xml);
}

#[test]
fn unchanged_check_filters_length_and_crc_before_optional_exact_comparison() {
    let original_xml = b"<worksheet><!--a02552f76dbdf5fd--></worksheet>";
    let collision_xml = b"<worksheet><!--670009b22c087272--></worksheet>";
    let original = compressed_test_entry(original_xml, CompressionMethod::Stored, 0);

    for trust_crc in [false, true] {
        assert!(unchanged_original_entry(
            &original,
            0,
            original_xml,
            CompressionMethod::Stored,
            trust_crc,
        )
        .unwrap());
        assert!(!unchanged_original_entry(
            &original,
            0,
            &original_xml[..original_xml.len() - 1],
            CompressionMethod::Stored,
            trust_crc,
        )
        .unwrap());
        assert!(!unchanged_original_entry(
            &original,
            0,
            original_xml,
            CompressionMethod::Deflated,
            trust_crc,
        )
        .unwrap());
    }

    assert!(!unchanged_original_entry(
        &original,
        0,
        collision_xml,
        CompressionMethod::Stored,
        false,
    )
    .unwrap());
    assert!(unchanged_original_entry(
        &original,
        0,
        collision_xml,
        CompressionMethod::Stored,
        true,
    )
    .unwrap());
}

#[test]
fn unchanged_workbook_is_copied_byte_for_byte_in_both_modes() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("excel_shrink_noop_{unique}"));
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("unchanged.xlsx");
    let mut writer = ZipWriter::new(File::create(&input).unwrap());
    for (name, xml) in [
        ("xl/workbook.xml", b"<workbook><sheets/></workbook>".as_slice()),
        ("xl/worksheets/sheet1.xml", b"<worksheet><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c></row></sheetData></worksheet>"),
    ] {
        writer
            .start_file(
                name,
                FileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(xml).unwrap();
    }
    writer.finish().unwrap();
    let source = std::fs::read(&input).unwrap();

    for trust_crc in [false, true] {
        let mut cli = cli_with_analysis_flags(false, None);
        cli.output = root.join(format!("out-{trust_crc}"));
        cli.disable_multiprocessing = true;
        cli.writer_workers = Some(1);
        cli.verify_unchanged_xml = !trust_crc;
        cli.shrink_evidence_json = Some(root.join(format!("evidence-{trust_crc}.json")));
        process_files(vec![input.clone()], &cli).unwrap();
        assert_eq!(std::fs::read(cli.output.join("unchanged.xlsx")).unwrap(), source);
        let evidence: serde_json::Value = serde_json::from_slice(
            &std::fs::read(cli.shrink_evidence_json.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(evidence["version"], 1);
        assert_eq!(evidence["workbooks"][0]["workbook_original_archive_reused"], true);
        assert_eq!(evidence["workbooks"][0]["workbook_crc_length_equal"], true);
        assert_eq!(
            evidence["workbooks"][0]["original_package_crc_length_sha256"],
            evidence["workbooks"][0]["cleaned_package_crc_length_sha256"]
        );
        assert_eq!(
            evidence["workbooks"][0]["workbook_xml_original_entries"],
            evidence["workbooks"][0]["workbook_xml_cleaned_entries"]
        );
        assert_eq!(
            evidence["workbooks"][0]["sheets"]["xl/worksheets/sheet1.xml"]
                ["crc_length_equal"],
            true
        );
        let sheet = &evidence["workbooks"][0]["sheets"]["xl/worksheets/sheet1.xml"];
        assert_eq!(sheet["source_content_cell_count"], 1);
        assert_eq!(sheet["tail_pruning_removed_bytes_uncompressed_xml"], 0);
        assert_eq!(sheet["tail_content_guard_triggered"], false);
    }

    let mut cli = cli_with_analysis_flags(false, None);
    cli.output = root.join("stored-output");
    cli.disable_multiprocessing = true;
    cli.writer_workers = Some(1);
    cli.store_output = true;
    cli.shrink_evidence_json = Some(root.join("stored-evidence.json"));
    assert!(!attribution_safe_analysis(&cli));
    process_files(vec![input.clone()], &cli).unwrap();
    let evidence: serde_json::Value = serde_json::from_slice(
        &std::fs::read(cli.shrink_evidence_json.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(evidence["workbooks"][0]["workbook_original_archive_reused"], false);
    assert_eq!(evidence["workbooks"][0]["workbook_crc_length_equal"], true);
    assert_eq!(
        evidence["workbooks"][0]["sheets"]["xl/worksheets/sheet1.xml"]
            ["crc_length_equal"],
        true
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn workbook_only_cleanup_is_reported_and_published_only_when_requested() {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("excel_shrink_workbook_only_{unique}"));
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("workbook-only.xlsx");
    let mut writer = ZipWriter::new(File::create(&input).unwrap());
    writer.start_file("xl/workbook.xml", FileOptions::default()).unwrap();
    writer.write_all(br#"<workbook><sheets/><definedNames><definedName name="Print_Area" localSheetId="0">A</definedName><definedName name="_xlnm.Print_Area" localSheetId="0">A</definedName></definedNames></workbook>"#).unwrap();
    writer.start_file("xl/worksheets/sheet1.xml", FileOptions::default()).unwrap();
    writer.write_all(b"<worksheet><sheetData/></worksheet>").unwrap();
    writer.finish().unwrap();
    let source = std::fs::read(&input).unwrap();

    for write_workbook_only_changes in [false, true] {
        let mut cli = cli_with_analysis_flags(false, None);
        cli.output = root.join(format!("out-{write_workbook_only_changes}"));
        cli.shrink_evidence_json = Some(root.join(format!("evidence-{write_workbook_only_changes}.json")));
        cli.write_workbook_only_changes = write_workbook_only_changes;
        cli.disable_multiprocessing = true;
        cli.writer_workers = Some(1);
        process_files(vec![input.clone()], &cli).unwrap();

        let output = std::fs::read(cli.output.join("workbook-only.xlsx")).unwrap();
        assert_eq!(output == source, !write_workbook_only_changes);
        let evidence: serde_json::Value = serde_json::from_slice(
            &std::fs::read(cli.shrink_evidence_json.unwrap()).unwrap(),
        ).unwrap();
        let workbook = &evidence["workbooks"][0];
        assert_eq!(workbook["workbook_defined_names_cleanup_available"], true);
        assert_eq!(workbook["workbook_only_cleanup_available"], true);
        assert_eq!(workbook["worksheet_xml_changed"], false);
        assert_eq!(workbook["workbook_original_archive_reused"], !write_workbook_only_changes);
        assert_eq!(workbook["workbook_xml_crc_length_equal"], !write_workbook_only_changes);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn store_output_does_not_reuse_deflated_original_xml() {
    let xml = b"<worksheet><sheetData/></worksheet>";
    let original = compressed_test_entry(xml, CompressionMethod::Deflated, 9);
    let cleaned = compressed_test_entry(
        xml,
        output_compression_method(CompressionMethod::Deflated, true),
        9,
    );

    let mut output = copy_test_entry(original, cleaned);
    let mut entry = output.by_index(0).unwrap();
    assert_eq!(entry.compression(), CompressionMethod::Stored);
    let mut actual = Vec::new();
    entry.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, xml);
}

#[test]
fn analysis_csv_collects_statistics_without_changing_tail_pruning() {
    let cli = cli_with_analysis_flags(false, Some(PathBuf::from("analysis.csv")));

    assert!(cleanup_stats_enabled(&cli));
    assert!(!attribution_safe_analysis(&cli));
    assert!(cleanup_switches(&cli, advanced_cleanup_selection(&cli)).sheetdata_range_tail_pruning);
}

#[test]
fn analysis_mode_keeps_the_normal_tail_pruning_policy() {
    let cli = cli_with_analysis_flags(true, None);

    assert!(cleanup_stats_enabled(&cli));
    assert!(!attribution_safe_analysis(&cli));
    assert!(cleanup_switches(&cli, advanced_cleanup_selection(&cli)).sheetdata_range_tail_pruning);
}

#[test]
fn no_analysis_flags_uses_production_tail_pruning_policy() {
    let cli = cli_with_analysis_flags(false, None);

    assert!(!cleanup_stats_enabled(&cli));
    assert!(!attribution_safe_analysis(&cli));
    assert!(cleanup_switches(&cli, advanced_cleanup_selection(&cli)).sheetdata_range_tail_pruning);
}

#[test]
fn explicit_tail_pruning_switch_selects_the_old_attribution_policy() {
    let mut cli = cli_with_analysis_flags(false, Some(PathBuf::from("analysis.csv")));
    cli.disable_tail_pruning = true;

    assert!(cleanup_stats_enabled(&cli));
    assert!(attribution_safe_analysis(&cli));
    assert!(!cleanup_switches(&cli, advanced_cleanup_selection(&cli)).sheetdata_range_tail_pruning);
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
        tail_pruning_disabled: true,
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
        "input_file,sheet_file,cause,removed_items,removed_bytes_uncompressed_xml,added_bytes_uncompressed_xml,reduction_bytes_uncompressed_xml,reduction_ratio,original_sheet_xml_bytes,cleaned_sheet_xml_bytes,sheet_reduction_bytes_uncompressed_xml,sheet_reduction_ratio,tail_pruning_disabled\n"
    );
}

#[test]
fn analysis_writer_failure_is_reported_after_batch_cleanup() {
    let writer = std::thread::spawn(|| Err(std::io::Error::other("analysis disk full")));
    let writer_pool = ThreadPool::new(1);
    let (stop_watchdog_tx, stop_watchdog_rx) = mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        stop_watchdog_rx.recv().unwrap();
    });

    let error = finish_batch(None, Some(writer), &writer_pool, stop_watchdog_tx, watchdog)
        .unwrap_err();

    assert!(error.to_string().contains("analysis disk full"));
}

#[test]
fn package_fingerprint_includes_non_sheet_members() {
    let mut original = BTreeMap::from([
        (
            "xl/workbook.xml".to_string(),
            vec![ZipEntryIdentity {
                uncompressed_bytes: 30,
                crc32: 11,
            }],
        ),
        (
            "xl/styles.xml".to_string(),
            vec![ZipEntryIdentity {
                uncompressed_bytes: 40,
                crc32: 22,
            }],
        ),
    ]);
    let before = package_crc_length_fingerprint(&original);
    original.get_mut("xl/styles.xml").unwrap()[0].crc32 = 23;

    assert_ne!(before, package_crc_length_fingerprint(&original));
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
        chart_format_anchors: Default::default(),
        preprocess: SheetPreprocessConfig::default(),
        cleanup: Default::default(),
        collect_cleanup_stats: false,
    }
}

#[test]
fn prefixed_spreadsheetml_is_skipped_without_publishing_output() {
    // Each spelling is valid SpreadsheetML, but the cleanup parsers currently
    // rely on literal unprefixed names. Include other cleanable parts so this
    // checks that the decision precedes all workbook/global-reference edits.
    let cases: &[(&str, &[u8])] = &[
        ("xl/worksheets/sheet2.xml", br#"<p:worksheet xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><p:sheetData><p:row r="1"><p:c r="A1"><p:v>42</p:v></p:c></p:row></p:sheetData></p:worksheet>"#),
        ("xl/worksheets/sheet2.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><p:row xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/main" r="1"><p:c r="A1"><p:v>42</p:v></p:c></p:row></sheetData></worksheet>"#),
        ("xl/worksheets/sheet2.xml", br#"<worksheet xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><p:c r="A1"><v>42</v></p:c></row></sheetData></worksheet>"#),
        ("xl/sharedStrings.xml", br#"<sst xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><p:t>Nonempty text must survive</p:t></si></sst>"#),
        ("xl/styles.xml", br#"<p:styleSheet xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><p:cellXfs count="1"><p:xf fontId="0"/></p:cellXfs></p:styleSheet>"#),
        ("xl/pivotTables/pivotTable1.xml", br#"<p:pivotTableDefinition xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><p:location ref="A1:B4"/></p:pivotTableDefinition>"#),
    ];
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "excel_shrink_namespace_{}_{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    for (index, &(name, bytes)) in cases.iter().enumerate() {
        let input = root.join(format!("case-{index}.xlsx"));
        let mut writer = ZipWriter::new(File::create(&input).unwrap());
        let mut entries = vec![
            ("xl/workbook.xml", br#"<workbook><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/><sheet name="Sheet2" sheetId="2" r:id="rId2"/></sheets></workbook>"#.as_slice()),
            ("xl/_rels/workbook.xml.rels", br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Target="worksheets/sheet2.xml"/></Relationships>"#),
            ("xl/worksheets/sheet1.xml", br#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"/><c r="C1"/></row></sheetData></worksheet>"#),
            ("xl/calcChain.xml", br#"<calcChain><c r="A1" i="1"/><c r="B1"/><c r="C1"/></calcChain>"#),
        ];
        entries.push((name, bytes));
        for (entry, xml) in entries {
            writer.start_file(entry, FileOptions::default()).unwrap();
            writer.write_all(xml).unwrap();
        }
        writer.finish().unwrap();
        let original = std::fs::read(&input).unwrap();
        for aggressive in [false, true] {
            let mut cli = cli_with_analysis_flags(false, None);
            cli.output = root.join(format!("out-{aggressive}"));
            cli.disable_multiprocessing = true;
            cli.writer_workers = Some(1);
            cli.chunk_size = 7;
            cli.remove_unreferenced_whitespace_cells = aggressive;
            cli.preprocess_empty_cells = aggressive;
            process_files(vec![input.clone()], &cli).unwrap();
            let output = cli.output.join(input.file_name().unwrap());
            assert!(!output.exists(), "case {index}, aggressive={aggressive}");
            assert!(!partial_output_path(&output).exists());
            assert_eq!(std::fs::read(&input).unwrap(), original);
            // A forced retry must not replace an existing output with a partial
            // cleaned workbook or disguise exclusion as a byte-identical pass.
            cli.force_overwrite = true;
            std::fs::write(&output, b"previous output").unwrap();
            process_files(vec![input.clone()], &cli).unwrap();
            assert_eq!(std::fs::read(output).unwrap(), b"previous output");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn streaming_sheet_parser_rejects_unsupported_namespaces_and_missing_end() {
    use excel_shrink_core::error::ShrinkError;
    use excel_shrink_core::sheet::streaming::preprocess_sheet_xml_streamwise;
    let params = default_sheet_params();
    for xml in [
        br#"<p:worksheet xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><p:sheetData/></p:worksheet>"#.as_slice(),
        br#"<worksheet><sheetData><row xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/main" r="1"><p:c r="A1"><v>42</v></p:c></row></sheetData></worksheet>"#,
    ] {
        for chunk_size in [1, 7, 64, 4096] {
            assert!(matches!(preprocess_sheet_xml_streamwise(xml, &params, chunk_size), Err(ShrinkError::UnsupportedNamespace)));
        }
    }
    for xml in [
        b"<worksheet><sheetData/>".as_slice(),
        b"<worksheet><sheetData></sheetData><ignored/>",
    ] {
        assert!(matches!(
            preprocess_sheet_xml_streamwise(xml, &params, 7),
            Err(ShrinkError::UnexpectedEof)
        ));
    }
}

#[test]
fn incomplete_sheet_aborts_the_workbook_without_replacing_existing_output() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("excel_shrink_eof_{}_{unique}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("incomplete.xlsx");
    let mut writer = ZipWriter::new(File::create(&input).unwrap());
    for (name, bytes) in [
        ("xl/workbook.xml", br#"<workbook><sheets><sheet name="Complete" sheetId="1" r:id="rId1"/><sheet name="Incomplete" sheetId="2" r:id="rId2"/></sheets></workbook>"#.as_slice()),
        ("xl/worksheets/sheet1.xml", br#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"/><c r="C1"/></row></sheetData></worksheet>"#),
        ("xl/worksheets/sheet2.xml", br#"<worksheet><sheetData><row r="1"><c r="A1"><v>42</v></c></row></sheetData>"#),
    ] {
        writer.start_file(name, FileOptions::default()).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
    let mut cli = cli_with_analysis_flags(false, None);
    cli.output = root.join("out");
    cli.force_overwrite = true;
    cli.disable_multiprocessing = true;
    cli.writer_workers = Some(1);
    cli.chunk_size = 7;
    let target = cli.output.join(input.file_name().unwrap());
    assert!(process_files(vec![input.clone()], &cli).is_err());
    assert!(
        !target.exists(),
        "an incomplete workbook must not be published"
    );
    assert!(
        !partial_output_path(&target).exists(),
        "aborted partial output must be removed"
    );
    std::fs::write(&target, b"previous output must survive").unwrap();
    assert!(process_files(vec![input], &cli).is_err());
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"previous output must survive"
    );
    assert!(!partial_output_path(&target).exists());
    std::fs::remove_dir_all(root).unwrap();
}

fn batch_test_directory(label: &str) -> PathBuf {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("excel_shrink_{label}_{}_{unique}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn write_batch_test_workbook(path: &Path, sheet: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    for (name, bytes) in [
        ("xl/workbook.xml", br#"<workbook><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#.as_slice()),
        ("xl/worksheets/sheet1.xml", sheet),
    ] {
        writer.start_file(name, FileOptions::default()).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn analysis_csv_keeps_the_measured_shrink_output() {
    let root = batch_test_directory("analysis_same_output");
    let input = root.join("source.xlsx");
    let sheet = br#"<worksheet><sheetData><row r="1"><c r="A1"><v>7</v></c><c r="B1"/><c r="C1"/><c r="D1"/></row><row r="2"></row></sheetData></worksheet>"#;
    write_batch_test_workbook(&input, sheet);
    let original_bytes = std::fs::read(&input).unwrap();

    let mut measured = cli_with_analysis_flags(false, None);
    measured.output = root.join("measured");
    measured.disable_multiprocessing = true;
    measured.writer_workers = Some(1);
    process_files(vec![input.clone()], &measured).unwrap();

    let mut analyzed = cli_with_analysis_flags(false, Some(root.join("analysis.csv")));
    analyzed.output = root.join("analyzed");
    analyzed.disable_multiprocessing = true;
    analyzed.writer_workers = Some(1);
    process_files(vec![input], &analyzed).unwrap();

    assert_eq!(
        std::fs::read(measured.output.join("source.xlsx")).unwrap(),
        std::fs::read(analyzed.output.join("source.xlsx")).unwrap()
    );
    assert_ne!(std::fs::read(measured.output.join("source.xlsx")).unwrap(), original_bytes);
    assert!(root.join("analysis.csv").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cdata_sheet_is_published_unchanged_across_decompression_boundaries() {
    let root = batch_test_directory("cdata_passthrough");
    let input = root.join("cdata.xlsx");
    let sheet = br#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t><![CDATA[<row r="999"></sheetData>&text]]></t></is></c></row></sheetData></worksheet>"#;
    write_batch_test_workbook(&input, sheet);
    let mut original_archive = ZipArchive::new(File::open(&input).unwrap()).unwrap();
    let original_sheet = original_archive.by_name("xl/worksheets/sheet1.xml").unwrap();
    let original_crc = original_sheet.crc32();
    let original_compressed_size = original_sheet.compressed_size();

    for chunk_size in [1, 7, 64] {
        let mut cli = cli_with_analysis_flags(false, None);
        cli.output = root.join(format!("out-{chunk_size}"));
        cli.chunk_size = chunk_size;
        cli.disable_multiprocessing = true;
        cli.writer_workers = Some(1);
        process_files(vec![input.clone()], &cli).unwrap();
        let mut output = ZipArchive::new(File::open(cli.output.join("cdata.xlsx")).unwrap()).unwrap();
        let mut copied_sheet = output.by_name("xl/worksheets/sheet1.xml").unwrap();
        let mut bytes = Vec::new();
        copied_sheet.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, sheet);
        assert_eq!(copied_sheet.crc32(), original_crc);
        assert_eq!(copied_sheet.compressed_size(), original_compressed_size);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cdata_in_one_sheet_does_not_prevent_cleaning_another_sheet() {
    let root = batch_test_directory("cdata_with_cleanable_sheet");
    let input = root.join("mixed.xlsx");
    let cdata_sheet = br#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t><![CDATA[<&text]]></t></is></c></row></sheetData></worksheet>"#;
    let cleanable_sheet = br#"<worksheet><sheetData><row r="1"><c r="A1"><v>7</v></c><c r="B1"/><c r="C1"/><c r="D1"/></row></sheetData></worksheet>"#;
    let mut writer = ZipWriter::new(File::create(&input).unwrap());
    for (name, bytes) in [
        ("xl/workbook.xml", br#"<workbook><sheets><sheet name="First" sheetId="1" r:id="rId1"/><sheet name="Second" sheetId="2" r:id="rId2"/></sheets></workbook>"#.as_slice()),
        ("xl/_rels/workbook.xml.rels", br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Target="worksheets/sheet2.xml"/></Relationships>"#),
        ("xl/worksheets/sheet1.xml", cdata_sheet),
        ("xl/worksheets/sheet2.xml", cleanable_sheet),
    ] {
        writer.start_file(name, FileOptions::default()).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();

    let mut cli = cli_with_analysis_flags(false, None);
    cli.output = root.join("output");
    cli.chunk_size = 7;
    cli.preprocess_empty_cells = true;
    cli.disable_multiprocessing = true;
    cli.writer_workers = Some(1);
    cli.shrink_evidence_json = Some(root.join("evidence.json"));
    process_files(vec![input], &cli).unwrap();
    let mut output = ZipArchive::new(File::open(cli.output.join("mixed.xlsx")).unwrap()).unwrap();
    let mut preserved = Vec::new();
    output.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_end(&mut preserved).unwrap();
    assert_eq!(preserved, cdata_sheet);
    let mut cleaned = Vec::new();
    output.by_name("xl/worksheets/sheet2.xml").unwrap().read_to_end(&mut cleaned).unwrap();
    assert_ne!(cleaned, cleanable_sheet);
    assert!(cleaned.windows(b"<v>7</v>".len()).any(|part| part == b"<v>7</v>"));
    let evidence: serde_json::Value =
        serde_json::from_slice(&std::fs::read(cli.shrink_evidence_json.unwrap()).unwrap())
            .unwrap();
    let sheets = &evidence["workbooks"][0]["sheets"];
    assert!(sheets["xl/worksheets/sheet1.xml"]["source_content_cell_count"].is_null());
    assert_eq!(sheets["xl/worksheets/sheet2.xml"]["source_content_cell_count"], 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_batch_preserves_paths_with_duplicate_file_names() {
    let root = batch_test_directory("batch_paths");
    let mut cli = cli_with_analysis_flags(false, None);
    cli.path = root.join("input");
    cli.output = root.join("output");
    cli.recursive = true;
    cli.writer_workers = Some(2);
    for (directory, value) in [("sales", 10), ("finance", 20)] {
        let sheet = format!("<worksheet><sheetData><row r=\"1\"><c r=\"A1\"><v>{value}</v></c></row></sheetData></worksheet>");
        write_batch_test_workbook(&cli.path.join(directory).join("book.xlsx"), sheet.as_bytes());
    }
    let inputs = crate::gather_files(&cli.path, cli.recursive).unwrap();
    assert_eq!(inputs.len(), 2);
    process_files(inputs, &cli).unwrap();
    for (directory, value) in [("sales", 10), ("finance", 20)] {
        let mut archive = ZipArchive::new(File::open(cli.output.join(directory).join("book.xlsx")).unwrap()).unwrap();
        let mut sheet = String::new();
        archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
        assert!(sheet.contains(&format!("<v>{value}</v>")));
    }
    assert!(!cli.output.join("book.xlsx").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn batch_report_accounts_for_success_namespace_skips_and_failures() {
    let root = batch_test_directory("batch_report");
    let mut cli = cli_with_analysis_flags(false, None);
    cli.path = root.join("input");
    cli.output = root.join("output");
    cli.batch_report = Some(root.join("reports/batch.json"));
    cli.writer_workers = Some(2);
    let ordinary = br#"<worksheet><sheetData><row r="1"><c r="A1"><v>42</v></c></row></sheetData></worksheet>"#;
    write_batch_test_workbook(&cli.path.join("good.xlsx"), ordinary);
    write_batch_test_workbook(&cli.path.join("skipped.xlsx"), ordinary);
    write_batch_test_workbook(&cli.path.join("unsupported.xlsx"), br#"<p:worksheet xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><p:sheetData/></p:worksheet>"#);
    write_batch_test_workbook(&cli.path.join("incomplete.xlsx"), b"<worksheet><sheetData/>");
    std::fs::write(cli.path.join("invalid.xlsx"), b"not a ZIP file").unwrap();
    std::fs::create_dir_all(&cli.output).unwrap();
    std::fs::write(cli.output.join("skipped.xlsx"), b"existing file").unwrap();
    let inputs = crate::gather_files(&cli.path, false).unwrap();
    let error = process_files(inputs, &cli).unwrap_err();
    assert!(error.to_string().contains("2 of 5 XLSX files failed"));
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(cli.batch_report.as_ref().unwrap()).unwrap()).unwrap();
    assert_eq!(report["summary"], serde_json::json!({
        "total": 5, "processed": 1, "preserved": 0, "skipped": 2, "failed": 2,
    }));
    let files = report["files"].as_array().unwrap();
    assert_eq!(files.len(), 5);
    let invalid = files.iter().find(|file| file["input"].as_str().unwrap().ends_with("invalid.xlsx")).unwrap();
    assert_eq!(invalid["status"], "failed");
    assert!(invalid["reason"].as_str().unwrap().contains("Invalid ZIP archive"));
    assert!(invalid["output_bytes"].is_null());
    let unsupported = files.iter().find(|file| file["input"].as_str().unwrap().ends_with("unsupported.xlsx")).unwrap();
    assert_eq!(unsupported["status"], "skipped");
    assert!(unsupported["reason"].as_str().unwrap().contains("unsupported XML namespaces"));
    assert!(unsupported["output_bytes"].is_null());
    assert!(cli.output.join("good.xlsx").exists());
    assert!(!cli.output.join("unsupported.xlsx").exists());
    assert_eq!(std::fs::read(cli.output.join("skipped.xlsx")).unwrap(), b"existing file");
    assert!(!cli.output.join("incomplete.xlsx").exists());
    std::fs::remove_dir_all(root).unwrap();
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
        [b"A1".to_vec(), b"C3".to_vec()].into_iter().collect(),
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

#[test]
fn pipeline_retains_hidden_chart_and_drawing_inputs() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("excel_shrink_object_refs_{unique}"));
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("objects.xlsx");
    let mut writer = ZipWriter::new(File::create(&input).unwrap());
    // A fully homogeneous hidden tail is otherwise eligible. Object sources
    // must protect the whole run, even when most of its cells are unreferenced.
    let mut source_sheet = String::from("<worksheet><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"2\" hidden=\"1\">");
    for col in 1..=16384 {
        let mut number = col;
        let mut letters = String::new();
        while number > 0 {
            number -= 1;
            letters.insert(0, (b'A' + (number % 26) as u8) as char);
            number /= 26;
        }
        source_sheet.push_str(&format!("<c r=\"{letters}2\"><v>0</v></c>"));
    }
    source_sheet.push_str("</row><row r=\"3\"><c r=\"A3\"><v>2</v></c></row></sheetData><pivotTableParts><pivotTablePart r:id=\"rId2\"/></pivotTableParts></worksheet>");
    let entries: [(&str, &[u8]); 8] = [
        ("xl/workbook.xml", br#"<workbook><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#),
        ("xl/_rels/workbook.xml.rels", br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#),
        ("xl/worksheets/sheet1.xml", source_sheet.as_bytes()),
        ("xl/charts/chart1.xml", br#"<c:chart><c:numRef><c:f>Data!$A$2:$B$2</c:f></c:numRef></c:chart>"#),
        ("xl/drawings/drawing1.xml", br#"<xdr:wsDr><xdr:sp textlink="Data!$C$2"/></xdr:wsDr>"#),
        ("xl/pivotCache/pivotCacheDefinition1.xml", br#"<pivotCacheDefinition><cacheSource><worksheetSource ref="D2:D2" sheet="Data"/></cacheSource></pivotCacheDefinition>"#),
        ("xl/pivotTables/pivotTable1.xml", br#"<pivotTableDefinition><location ref="F2:F2"/></pivotTableDefinition>"#),
        ("xl/worksheets/_rels/sheet1.xml.rels", br#"<Relationships><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable" Target="../pivotTables/pivotTable1.xml"/></Relationships>"#),
    ];
    for (name, bytes) in entries {
        writer.start_file(name, FileOptions::default()).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();

    let mut cli = cli_with_analysis_flags(false, None);
    cli.output = root.join("out");
    cli.disable_multiprocessing = true;
    cli.writer_workers = Some(1);
    process_files(vec![input.clone()], &cli).unwrap();
    let mut archive =
        ZipArchive::new(File::open(cli.output.join("objects.xlsx")).unwrap()).unwrap();
    let mut sheet = String::new();
    archive
        .by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut sheet)
        .unwrap();
    for address in ["A2", "B2", "C2", "D2", "F2"] {
        assert!(
            sheet.contains(&format!(r#"r="{address}""#)),
            "missing {address}: {sheet}"
        );
    }
    assert!(
        sheet.contains(r#"r="E2""#),
        "a protected homogeneous run must also retain its unreferenced members"
    );

    cli.cell_formula_references_only = true;
    cli.output = root.join("cell-only");
    process_files(vec![input], &cli).unwrap();
    let mut archive =
        ZipArchive::new(File::open(cli.output.join("objects.xlsx")).unwrap()).unwrap();
    let mut sheet = String::new();
    archive
        .by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut sheet)
        .unwrap();
    for address in ["A2", "B2", "C2", "D2", "E2", "F2"] {
        assert!(!sheet.contains(&format!(r#"r="{address}""#)));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn chart_format_anchor_survives_streaming_without_value_reference_cleanup() {
    let root = batch_test_directory("chart_format_anchor");
    let input = root.join("chart.xlsx");
    let mut writer = ZipWriter::new(File::create(&input).unwrap());
    let entries: &[(&str, &[u8])] = &[
        ("xl/workbook.xml", br#"<workbook><sheets><sheet name="Pre-Retrofit" sheetId="1" r:id="rId1"/></sheets></workbook>"#),
        ("xl/_rels/workbook.xml.rels", br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#),
        ("xl/styles.xml", br#"<styleSheet><fonts count="1"><font><sz val="11"/></font></fonts><fills count="1"><fill><patternFill patternType="none"/></fill></fills><borders count="1"><border/></borders><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/><xf numFmtId="2" fontId="0" fillId="0" borderId="0" applyFont="1"/></cellXfs></styleSheet>"#),
        ("xl/worksheets/sheet1.xml", br#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="7" ht="15" customHeight="1"><c r="C7" s="1"/></row><row r="8" ht="15" customHeight="1"><c r="C8" s="1"/></row><row r="9"><c r="A9"><v>2</v></c></row></sheetData></worksheet>"#),
        ("xl/charts/chart1.xml", br#"<c:chart><c:numRef><c:f>'Pre-Retrofit'!$C$7:$C$18</c:f></c:numRef></c:chart>"#),
    ];
    for &(name, bytes) in entries {
        writer.start_file(name, FileOptions::default()).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
    let mut cli = cli_with_analysis_flags(false, None);
    cli.output = root.join("out");
    cli.force_overwrite = true;
    cli.disable_multiprocessing = true;
    cli.keep_unreferenced_hidden_content_cells = true;
    cli.cell_formula_references_only = true;
    cli.chunk_size = 64;
    process_files(vec![input], &cli).unwrap();
    let mut archive = ZipArchive::new(File::open(cli.output.join("chart.xlsx")).unwrap()).unwrap();
    let mut sheet = String::new();
    archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
    assert!(sheet.contains(r#"<c r="C7" s="1"/>"#), "{sheet}");
    assert!(!sheet.contains(r#"r="C8""#), "{sheet}");
    drop(archive);
    std::fs::remove_dir_all(root).unwrap();
}
