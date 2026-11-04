//! Reduced reproductions of the hidden-content losses inspected on 2026-11-04.
//!
//! Source workbooks and sheets (original coordinates, not generated examples):
//! - `native_001%2F3.384235.FG0FUFRY0QVIIBREKZKPSRNLVKM1QBSIB.1.xls`,
//!   `Top 100 Exposures`: hidden rows 110–114; e.g. G111=542509084, I111=-4343501.
//! - `native_000%2F3.473858.OFBWDDQGJUHSLEBS4UV14DQWTKGJFSHUA.1.xls`,
//!   `physical` and `financial`: mixed hidden detail tables, 346414 removed values.
//! - `native_000%2F3.477359.MZW1XUPI44V3IC3ZGMEN0KY4W2ACH51YB.1.xls`,
//!   `Sheet1`: hidden H:M, rows 10–39369; e.g. L10=-80042.
//! - `native_000%2F3.859146.DFFCA2MHTVZ2VGDOSFEN2ZEITLG2JL1JA.1.xls`,
//!   `TranscoZ6`, `Tetco_M3`, `Mich`, `Chi. Gate`, `CGT`: hidden mixed tables.
//! - `native_000%2F3.477914.BLYQDM5PVVFEKFWLK1TAH0EZETAMWCHQB.1.xls`,
//!   `FT-NY BASIS`: mixed hidden data, 182820 removed values in the old output.
//! - `native_000%2F3.473690.EU3J2UHPXKCZKUBZB20PDMWFJSJS45WVA.1.xls`,
//!   `Sheet1`: mixed hidden data, 163931 removed values in the old output.
//! - `wanderungen-2010120217005.xlsx`, original Destatis corpus: `Tabelle 3.4 `
//!   has long constant-zero tails to XFD; `Tabelle 2.1.1` has A65536=3;
//!   `Tabelle 3.3` has M44=-118, N44=78.971264999513238, O44=82.
//!
//! Related display-only investigations must not be mistaken for value losses:
//! `native_000%2F3.608401.CDOZVCACXNQKWR5NDN3SG3Y4LG1AO4FUA.1.xls`, `Deals`;
//! `native_002%2F3.392154.FJ0QPWHIHHNWVLN4RI45LYMEGMBMZV2SA.3.xls`, `Pwr west P&L`;
//! `native_000%2F3.774728.IJD2JAFTHS3TABW2JPAID41YCXHGJGFXA.1.xls`, `Sheet1`;
//! `native_000%2F3.1157820.NX0SZO2LQWWPUZXZA0TGW5B3LB1U0SYOA.1.xls`, `Description`.
//!
//! [Original Wanderungen workbook](https://github.com/Alexander-Aue-Johr/original_destatis_xlsx_files/blob/main/wanderungen-2010120217005.xlsx).
//!
//! These small synthetic packages reproduce placement, values and hidden flags.
//! They do not embed or claim to reproduce the complete source workbooks.
use super::regression_tests::cli_with_analysis_flags;
use super::*;
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::FileOptions;

fn shrink_hidden_fixture(sheet: &str) -> String {
    shrink_hidden_fixture_in_pool(sheet, 1)
}

fn shrink_hidden_fixture_in_pool(sheet: &str, threads: usize) -> String {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("hidden_tail_{}_{unique}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("source.xlsx");
    let mut writer = ZipWriter::new(File::create(&input).unwrap());
    for (name, xml) in [
        ("xl/workbook.xml", "<workbook><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"),
        ("xl/_rels/workbook.xml.rels", "<Relationships><Relationship Id=\"rId1\" Target=\"worksheets/sheet1.xml\"/></Relationships>"),
        ("xl/worksheets/sheet1.xml", sheet),
    ] {
        writer.start_file(name, FileOptions::default()).unwrap();
        writer.write_all(xml.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
    let mut cli = cli_with_analysis_flags(false, None);
    cli.path = input.clone();
    cli.output = root.join("out");
    cli.disable_multiprocessing = threads == 1;
    cli.parallel_range_chunk_size = 4096;
    cli.writer_workers = Some(1);
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(|| process_files(vec![input], &cli))
        .unwrap();
    let mut result = String::new();
    ZipArchive::new(File::open(cli.output.join("source.xlsx")).unwrap())
        .unwrap()
        .by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut result)
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
    result
}

/// Hidden detail columns between visible columns, reduced from Sheet1 H:M.
/// See the module's workbook/sheet references, especially `3.477359...1.xls`.
#[test]
fn hidden_tail_keeps_mixed_table_between_visible_columns() {
    let result = shrink_hidden_fixture(concat!(
        "<worksheet><cols><col min=\"8\" max=\"13\" hidden=\"1\"/></cols><sheetData>",
        "<row r=\"10\" ht=\"15\" customHeight=\"1\"><c r=\"G10\"><v>1</v></c>",
        "<c r=\"H10\" t=\"inlineStr\"><is><t>Contract</t></is></c>",
        "<c r=\"J10\"><v>37226</v></c><c r=\"L10\"><v>-80042</v></c>",
        "<c r=\"N10\"><v>2</v></c></row></sheetData></worksheet>"
    ));
    assert!(result.contains("Contract"));
    assert!(result.contains("-80042"));
}

/// Hidden mixed data to the right of visible content, matching the inspected
/// `physical`/`financial` and `TranscoZ6`/`Tetco_M3`/`Mich`/`Chi. Gate`/`CGT` tables.
#[test]
fn hidden_tail_keeps_mixed_table_right_of_visible_content() {
    let result = shrink_hidden_fixture(concat!(
        "<worksheet><cols><col min=\"8\" max=\"16384\" hidden=\"1\"/></cols><sheetData>",
        "<row r=\"2\" ht=\"15\" customHeight=\"1\"><c r=\"A2\"><v>1</v></c>",
        "<c r=\"H2\" t=\"inlineStr\"><is><t>Trade details</t></is></c>",
        "<c r=\"I2\"><v>961534</v></c><c r=\"J2\"><v>-4343501</v></c>",
        "</row></sheetData></worksheet>"
    ));
    assert!(result.contains("Trade details"));
    assert!(result.contains("961534"));
    assert!(result.contains("-4343501"));
}

/// Hidden table below the report: original `Top 100 Exposures`, rows 110–114.
#[test]
fn hidden_tail_keeps_mixed_table_below_visible_content() {
    let result = shrink_hidden_fixture(concat!(
        "<worksheet><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c></row>",
        "<row r=\"110\" hidden=\"1\"><c r=\"A110\" t=\"inlineStr\"><is><t>Details</t></is></c></row>",
        "<row r=\"111\" hidden=\"1\"><c r=\"G111\"><v>542509084</v></c>",
        "<c r=\"I111\"><v>-4343501</v></c></row></sheetData></worksheet>"
    ));
    assert!(result.contains("Details"));
    assert!(result.contains("542509084"));
    assert!(result.contains("-4343501"));
}

/// Destatis `wanderungen-2010120217005.xlsx`: preserve the isolated A65536=3
/// in `Tabelle 2.1.1` and differing constants in `Tabelle 3.3`, M44:IT44.
#[test]
fn hidden_tail_keeps_wanderungen_nonzero_details() {
    let result = shrink_hidden_fixture(concat!(
        "<worksheet><sheetData><row r=\"44\" hidden=\"1\">",
        "<c r=\"M44\"><v>-118</v></c><c r=\"N44\"><v>78.971264999513238</v></c>",
        "<c r=\"O44\"><v>82</v></c></row>",
        "<row r=\"65536\" hidden=\"1\"><c r=\"A65536\"><v>3</v></c></row>",
        "</sheetData></worksheet>"
    ));
    for value in ["-118", "78.971264999513238", "82", "3"] {
        assert!(result.contains(&format!("<v>{value}</v>")), "lost {value}");
    }
}

/// Destatis `wanderungen-2010120217005.xlsx`, `Tabelle 3.4 `: constant-zero
/// tails start at L11 and reach XFD. The real workbook has 343833 removed zeros.
#[test]
fn hidden_tail_still_removes_wanderungen_full_width_zero_tail() {
    let mut sheet = String::from("<worksheet><cols><col min=\"12\" max=\"16384\" hidden=\"1\"/></cols><sheetData><row r=\"11\" ht=\"15\" customHeight=\"1\"><c r=\"A11\"><v>7</v></c>");
    for col in 12..=16384 {
        let mut number = col;
        let mut letters = String::new();
        while number > 0 {
            number -= 1;
            letters.insert(0, (b'A' + (number % 26) as u8) as char);
            number /= 26;
        }
        sheet.push_str(&format!("<c r=\"{letters}11\"><v>0</v></c>"));
    }
    sheet.push_str("</row></sheetData></worksheet>");
    let result = shrink_hidden_fixture(&sheet);
    assert!(!result.contains("<v>0</v>"));
    assert!(result.contains("<v>7</v>"));
}

fn hidden_tail_with_anomaly(anomaly: &str) -> String {
    let mut sheet = String::from("<worksheet><cols><col min=\"12\" max=\"16384\" hidden=\"1\"/></cols><sheetData><row r=\"11\" ht=\"15\" customHeight=\"1\"><c r=\"A11\"><v>7</v></c>");
    for col in 12..=16384 {
        let mut number = col;
        let mut letters = String::new();
        while number > 0 {
            number -= 1;
            letters.insert(0, (b'A' + (number % 26) as u8) as char);
            number /= 26;
        }
        if col == 8000 {
            match anomaly {
                "gap" => continue,
                "formula" => {
                    sheet.push_str(&format!("<c r=\"{letters}11\"><f>1-1</f><v>0</v></c>"));
                    continue;
                }
                "text" => {
                    sheet.push_str(&format!(
                        "<c r=\"{letters}11\" t=\"inlineStr\"><is><t>Meaningful</t></is></c>"
                    ));
                    continue;
                }
                "different" => {
                    sheet.push_str(&format!("<c r=\"{letters}11\"><v>3</v></c>"));
                    continue;
                }
                "none" => {}
                _ => unreachable!(),
            }
        }
        sheet.push_str(&format!("<c r=\"{letters}11\"><v>0</v></c>"));
    }
    sheet.push_str("</row></sheetData></worksheet>");
    sheet
}

/// Do not carve homogeneous subruns out of a mixed or interrupted hidden area.
#[test]
fn hidden_tail_retains_holes_formulas_text_and_value_changes() {
    for anomaly in ["gap", "formula", "text", "different"] {
        let result = shrink_hidden_fixture(&hidden_tail_with_anomaly(anomaly));
        assert_eq!(
            result.matches("<v>0</v>").count(),
            16373 - usize::from(anomaly != "formula"),
            "{anomaly}"
        );
    }
}

/// A reference to even one member retains the entire admitted span.
#[test]
fn hidden_tail_reference_protects_the_whole_span() {
    let xml = b"<sheetData><row r=\"11\"><c r=\"L11\"><v>0</v></c><c r=\"XFD11\"><v>0</v></c></row></sheetData>".to_vec();
    let span = excel_shrink_core::models::HiddenContentSpan {
        row: 11,
        start_col: 12,
        end_col: 16384,
    };
    for global in [false, true] {
        let mut references = ProtectedWorkbookRefs::default();
        if global {
            references
                .global_or_unknown
                .insert_range(8000, 11, 8001, 11);
        } else {
            references
                .refs_for_sheet_mut("xl/worksheets/sheet1.xml")
                .insert_cell(8000, 11);
        }
        let (result, rows) = apply_deferred_candidate_cleanup_with_spans(
            xml.clone(),
            &[],
            &[span],
            &references,
            "xl/worksheets/sheet1.xml",
            &mut CleanupStats::enabled(),
        );
        assert_eq!(result, xml);
        assert!(rows.is_empty());
    }
}

/// Zero holes must remain holes under both sequential and row-chunked cleanup.
#[test]
fn hidden_tail_is_independent_of_parallel_row_chunks() {
    for anomaly in ["none", "gap"] {
        let mut sheet = hidden_tail_with_anomaly(anomaly);
        let begin = sheet.find("<row ").unwrap();
        let end = sheet.find("</row>").unwrap() + "</row>".len();
        let second = sheet[begin..end].replace("11\"", "12\"");
        sheet = sheet.replace("</sheetData>", &(second + "</sheetData>"));
        assert_eq!(
            shrink_hidden_fixture_in_pool(&sheet, 1),
            shrink_hidden_fixture_in_pool(&sheet, 2),
            "{anomaly}"
        );
    }
}

/// Uniform values alone are insufficient for small hidden suffixes.
#[test]
fn hidden_tail_keeps_short_constant_suffix() {
    let result = shrink_hidden_fixture("<worksheet><sheetData><row r=\"11\" hidden=\"1\"><c r=\"XFC11\"><v>0</v></c><c r=\"XFD11\"><v>0</v></c></row></sheetData></worksheet>");
    assert_eq!(result.matches("<v>0</v>").count(), 2);
}

/// Repeated negative constants use the same exact-integer admission as zeros.
#[test]
fn hidden_tail_still_removes_full_width_negative_constant() {
    let sheet = hidden_tail_with_anomaly("none").replace("<v>0</v>", "<v>-1</v>");
    let result = shrink_hidden_fixture(&sheet);
    assert!(!result.contains("<v>-1</v>"));
    assert!(result.contains("<v>7</v>"));
}

/// The interval guard must agree with existing point protection for both local
/// and unknown/global references, including the sparse and dense lookup paths.
#[test]
fn hidden_tail_interval_protection_matches_existing_point_queries() {
    let mut protected = ProtectedWorkbookRefs::default();
    let sheet = "xl/worksheets/sheet1.xml";
    for col in (1..=30).step_by(3) {
        protected.refs_for_sheet_mut(sheet).insert_cell(col, 2);
    }
    protected.refs_for_sheet_mut(sheet).insert_range(8, 3, 12, 4);
    protected.global_or_unknown.insert_cell(5, 1);
    protected.global_or_unknown.insert_range(17, 1, 19, 3);
    for row in 1..=5 {
        for start in 1..=20 {
            for end in start..=25 {
                let expected = (start..=end).any(|col| protected.contains(sheet, col, row));
                assert_eq!(protected.intersects_row(sheet, row, start, end), expected);
            }
        }
    }
}