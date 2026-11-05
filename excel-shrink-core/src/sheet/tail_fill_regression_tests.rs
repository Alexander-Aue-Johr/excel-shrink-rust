use super::{build_clean_sheet_xml, tests};
use crate::models::{CleanSheetParameters, CleanedSheetData, WorkbookSpecificPatterns};
use crate::sheet::cols::process_cols_node;

// Explicit 10 pt / 18 pt fonts make the height-preservation case meaningful;
// different font IDs alone do not establish different font metrics.
fn params(maximum: bool) -> CleanSheetParameters {
    let styles = br#"<styleSheet><fonts count="2"><font><sz val="10"/><name val="Arial"/></font><font><sz val="18"/><name val="Times New Roman"/></font></fonts>
        <fills count="4"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFFFFF"/></patternFill></fill><fill><patternFill patternType="solid"><fgColor rgb="FF0070C0"/></patternFill></fill></fills>
        <borders count="2"><border/><border><bottom style="thin"/></border></borders>
        <cellXfs count="7"><xf fontId="0" fillId="0" borderId="0"/>
        <xf fontId="0" fillId="2" borderId="0" applyFill="1"/>
        <xf fontId="1" fillId="2" borderId="0" applyFont="1" applyFill="1" applyBorder="1"><alignment vertical="top" wrapText="1"/></xf>
        <xf fontId="1" fillId="3" borderId="0" applyFont="1" applyFill="1"/>
        <xf fontId="1" fillId="2" borderId="1" applyFont="1" applyFill="1" applyBorder="1"/>
        <xf fontId="0" fillId="0" borderId="0" applyFill="1"/>
        <xf fontId="1" fillId="0" borderId="0" applyFont="1"/></cellXfs></styleSheet>"#;
    let mut params = tests::default_params();
    params.patterns =
        WorkbookSpecificPatterns::new(crate::styles::parse_xfs(styles).unwrap(), None);
    if maximum {
        params.cleanup.keep_textless_default_font_representatives = false;
        params.cleanup.keep_textless_style_representatives = false;
        params.cleanup.keep_textless_column_style_representatives = false;
    }
    params
}

fn clean(rows: &[u8], columns: &[u8], maximum: bool, splitting_disabled: bool) -> CleanedSheetData {
    let params = params(maximum);
    let mut sheet = tests::preprocessed_with_sheet_data(rows, None);
    sheet.before_cols_node_output_chunks = b"<cols>".to_vec();
    sheet.in_cols_node_output_chunks = columns.to_vec();
    sheet.before_sheet_data_node_output_chunks = b"</cols><sheetData>".to_vec();
    sheet.clean_sheet_metrics.column_definitions = process_cols_node(columns, &params);
    build_clean_sheet_xml(sheet, &params, splitting_disabled).unwrap()
}

const WHITE_COLUMNS: &[u8] = br#"<col min="1" max="16384" style="1"/>"#;

/// A redundant row fill must not hide its separate role of masking a column border.
#[test]
fn borderless_tail_row_masks_a_visible_column_border_without_retaining_cells() {
    let rows = br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" s="2" customFormat="1"/>"#;
    let columns = br#"<col min="1" max="16384" style="1"/><col min="6" max="6" style="4"/>"#;
    for maximum in [false, true] {
        let result = clean(rows, columns, maximum, false);
        let xml = String::from_utf8_lossy(&result.xml);
        assert!(xml.contains(r#"<row r="2" s="2" customFormat="1"/>"#));
        assert!(!xml.contains("A2"));
    }
    // The exact override shadows the bordered interval, so no visible border remains.
    let shadowed = br#"<col min="1" max="16384" style="4"/><col min="1" max="16384" style="1"/>"#;
    let result = clean(rows, shadowed, true, false);
    assert!(!String::from_utf8_lossy(&result.xml).contains(r#"r="2""#));
}

/// Enron `3.395674.FFVA02E4NUID3ZH5TIILK5UPNDX4KVRQB.1.xls`
/// (`44ad86f1d9f249470fc6`), sheet `NOV`, retained 11,632 empty cells after
/// the no-fill fix. Its 518 styleless font representatives included A1424.
/// There are no values, formulas, visible fills/borders, merges or drawings.
/// The companion `3.395598.OAJKML4IPCG5UEVVOYVMXXLBP1NRKURWA.1.xls`
/// (`a8bd950dff33454721b5`) has the same tail. Font-only inherited row styles
/// cannot justify preserving an empty sheet through its last representative.
#[test]
fn enron_nov_styleless_representatives_do_not_anchor_a_content_free_sheet() {
    let rows = br#"<row r="1" ht="18" customHeight="1"/><row r="1424" s="6" customFormat="1"><c r="A1424"/><c r="B1424" s="5"/><c r="C1424" s="6"/></row>"#;
    for maximum in [false, true] {
        for disabled in [false, true] {
            let result = clean(rows, b"", maximum, disabled);
            let xml = String::from_utf8_lossy(&result.xml);
            assert!(!xml.contains("<row "), "{xml}");
            assert!(!xml.contains("<c "), "{xml}");
            assert_eq!(result.metrics.tail_anchor_max_row, None);
        }
    }
}

/// A font representative before content remains locally preserved. Pruning
/// must distinguish this row from the otherwise identical content-free tail.
#[test]
fn styleless_font_representative_before_content_is_preserved() {
    let rows = br#"<row r="2" s="6" customFormat="1"><c r="A2"/></row><row r="3"><c r="A3"><v>1</v></c></row>"#;
    for disabled in [false, true] {
        let result = clean(rows, b"", false, disabled);
        let xml = String::from_utf8_lossy(&result.xml);
        assert!(xml.contains("<c r=\"A2\"/>"), "{xml}");
        assert!(xml.contains("<c r=\"A3\"><v>1</v></c>"), "{xml}");
        assert_eq!(result.metrics.tail_anchor_max_row, Some(3));
    }
}

/// Styleless cells that clear inherited white/blue fills or borders are visual
/// anchors, regardless of the font-representative switches. Exercise both the
/// full pipeline and the fallback reverse scan without precomputed anchors.
#[test]
fn styleless_cells_masking_row_or_column_formats_remain_tail_anchors() {
    for style in [1, 3, 4] {
        for source in ["row", "column", "both"] {
            let row_style = if source == "column" {
                String::new()
            } else {
                format!(" s=\"{style}\" customFormat=\"1\"")
            };
            let columns = if source == "row" {
                String::new()
            } else {
                format!("<col min=\"1\" max=\"16384\" style=\"{style}\"/>")
            };
            let rows = format!("<row r=\"2\"{row_style}><c r=\"A2\"/></row>");
            for maximum in [false, true] {
                for disabled in [false, true] {
                    let result = clean(rows.as_bytes(), columns.as_bytes(), maximum, disabled);
                    let xml = String::from_utf8_lossy(&result.xml);
                    assert!(xml.contains("<c r=\"A2\"/>"), "{source} {style}: {xml}");
                    assert_eq!(result.metrics.tail_anchor_max_row, Some(2));
                }
                let params = params(maximum);
                let mut metrics = crate::models::CleanSheetMetrics::default();
                metrics.column_definitions = process_cols_node(columns.as_bytes(), &params);
                let trimmed =
                    super::trim_trailing_layout_only_rows(rows.as_bytes(), &params, &mut metrics);
                assert_eq!(trimmed, rows.as_bytes(), "{source} {style}");
            }
        }
    }
}

/// Reduced from Enron `3.395674.FFVA02E4NUID3ZH5TIILK5UPNDX4KVRQB.1.xls`
/// (prepared bundle `44ad86f1d9f249470fc6`), sheet `AUGUST`. It has no
/// values, formulas, fills, borders or drawing objects. Default retained 21,984
/// empty cells, including 21,947 with applyFill="1" and fillId="0". With no
/// inherited fill, these cells incorrectly anchored the tail through F2779.
#[test]
fn enron_august_no_fill_overrides_do_not_anchor_an_empty_sheet() {
    let rows = br#"<row r="1" ht="18" customHeight="1"/><row r="8"><c r="A8" s="5"/></row><row r="2779"><c r="F2779" s="5"/></row>"#;
    let columns = br#"<col min="1" max="8" width="10" customWidth="1"/>"#;
    for maximum in [false, true] {
        for disabled in [false, true] {
            let result = clean(rows, columns, maximum, disabled);
            let xml = String::from_utf8_lossy(&result.xml);
            assert!(!xml.contains("<c "), "{xml}");
            assert!(!xml.contains("<row "), "{xml}");
            assert_eq!(result.metrics.tail_anchor_max_row, None);
        }
    }
}

/// The Enron no-inheritance case must not weaken masking. Explicit no-fill
/// cells reveal gridlines through an inherited white or blue fill, whether
/// supplied by a row, a column, or both. They remain visual tail anchors even
/// when there is no content below them.
#[test]
fn no_fill_overrides_preserve_inherited_white_and_blue_fill_masks() {
    for fill_style in [1, 3] {
        for source in ["row", "column", "both"] {
            let row_style = if source == "column" {
                String::new()
            } else {
                format!(" s=\"{fill_style}\" customFormat=\"1\"")
            };
            let columns = if source == "row" {
                String::new()
            } else {
                format!("<col min=\"1\" max=\"16384\" style=\"{fill_style}\"/>")
            };
            let rows = format!("<row r=\"2\"{row_style}><c r=\"A2\" s=\"5\"/></row>");
            for maximum in [false, true] {
                for disabled in [false, true] {
                    let result = clean(rows.as_bytes(), columns.as_bytes(), maximum, disabled);
                    let xml = String::from_utf8_lossy(&result.xml);
                    assert!(
                        xml.contains("<c r=\"A2\" s=\"5\"/>"),
                        "{source} {fill_style}: {xml}"
                    );
                    assert_eq!(result.metrics.tail_anchor_max_row, Some(2));
                }
            }
        }
    }
}

/// With no content anchor at all, the reverse tail scan must use the same
/// full-width redundancy decision as row metrics. Blue still remains visible.
#[test]
fn fill_only_sheet_uses_the_same_tail_rule_without_a_content_boundary() {
    for (style, keep) in [(2, false), (3, true)] {
        let rows = format!("<row r=\"487\" s=\"{style}\" customFormat=\"1\" ht=\"13\"/>");
        let result = clean(rows.as_bytes(), WHITE_COLUMNS, true, false);
        assert_eq!(
            String::from_utf8_lossy(&result.xml).contains("r=\"487\""),
            keep
        );
    }
}

/// Reduced from Enron `native_000%2F3.1167415.GBNHITXDIFLTFXQ1QU5BFYSV452NK4U1B.1.xls`
/// (prepared bundle `ab7e13baa69a29ef50f4`), sheet `Feb`: content ends at K486,
/// followed by rows 487:49633 with white fills already supplied by A:XFD.
/// The companion `3.1167409.J5S5XW3WGA53AK01AAS5JSTDJCKRIDBTB.1.xls`
/// (`90e5499ce2287c6a9014`) ends at E485 and has the same redundant tail.
/// Check Performance removed these tails; COM captures of their boundaries and
/// interior matched the originals. Distinct fonts must not turn an empty tail
/// into a content anchor. Larger fonts here exercise the stronger height case.
#[test]
fn redundant_white_fill_and_font_override_do_not_anchor_an_empty_tail() {
    let rows = br#"<row r="486"><c r="K486"><v>1</v></c></row><row r="487" s="2" customFormat="1" ht="13" x14ac:dyDescent="0.3"><c r="A487" s="2"/></row>"#;
    for maximum in [false, true] {
        for disabled in [false, true] {
            let result = clean(rows, WHITE_COLUMNS, maximum, disabled);
            let xml = String::from_utf8_lossy(&result.xml);
            assert!(xml.contains("K486"));
            assert!(!xml.contains("A487"));
            assert!(!xml.contains("r=\"487\""));
        }
    }
}

/// Paired counterexample to the Enron trailing-row fixture: when content follows,
/// the 18 pt empty-cell font can affect row height and the position of that content.
/// Default must preserve the override even though its white fill is redundant.
/// This refines the June 2026 cell-only regression, which required an anchor but
/// did not include subsequent content or actual font sizes.
#[test]
fn font_override_is_preserved_when_content_follows() {
    let rows = br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" ht="13"><c r="A2" s="2"/></row><row r="3"><c r="A3"><v>2</v></c></row>"#;
    let result = clean(rows, WHITE_COLUMNS, false, false);
    let xml = String::from_utf8_lossy(&result.xml);
    assert!(xml.contains("A2\" s=\"2\""));
    assert!(xml.contains("A3"));
    assert_eq!(result.metrics.tail_anchor_max_row, Some(3));
}

/// The same empty-cell font without subsequent content may be pruned. This test
/// intentionally has no row style: a formatted column alone is not a cell anchor.
#[test]
fn font_override_without_content_below_does_not_anchor_the_tail() {
    let rows =
        br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" ht="13"><c r="A2" s="2"/></row>"#;
    let result = clean(rows, WHITE_COLUMNS, false, false);
    assert!(!String::from_utf8_lossy(&result.xml).contains("A2"));
    assert_eq!(result.metrics.tail_anchor_max_row, Some(1));
}

/// Controlled blue variants of both Enron workbooks: replacing the white tail
/// with #0070C0 made Check Performance visibly erase it, while Default and Maximum
/// preserved it. Empty does not mean invisible; only inherited-identical fills
/// qualify for the new rule.
#[test]
fn blue_tail_over_white_columns_remains_visible() {
    let rows = br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" s="3" customFormat="1"><c r="A2" s="3"/></row>"#;
    for maximum in [false, true] {
        let result = clean(rows, WHITE_COLUMNS, maximum, false);
        assert!(String::from_utf8_lossy(&result.xml).contains("r=\"2\""));
        assert_eq!(result.metrics.tail_anchor_max_row, Some(2));
    }
}

/// A single exception anywhere in A:XFD invalidates full-width fill redundancy,
/// even if the column is hidden or lies far beyond the populated cells.
#[test]
fn column_exceptions_and_uncovered_columns_preserve_the_white_tail() {
    let rows = br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" s="2" customFormat="1"/>"#;
    for columns in [
        &br#"<col min="1" max="16384" style="1"/><col min="8000" max="8000" style="3"/>"#[..],
        &br#"<col min="1" max="16384" style="1"/><col min="8000" max="8000" style="3" hidden="1"/>"#[..],
        &br#"<col min="1" max="16383" style="1"/>"#[..],
        &br#"<col min="2" max="16384" style="1"/>"#[..],
        &br#"<col min="1" max="8000" style="1"/><col min="8002" max="16384" style="1"/>"#[..],
    ] {
        let result = clean(rows, columns, true, false);
        assert!(String::from_utf8_lossy(&result.xml).contains("r=\"2\""));
    }
}

/// Matching fill IDs need not mean matching full styles; separate column ranges
/// can jointly cover A:XFD. Conversely, row borders and layout metadata must not
/// disappear merely because the fill is redundant.
#[test]
fn uniform_fill_across_ranges_does_not_hide_borders_or_row_metadata() {
    let columns = br#"<col min="1" max="10" style="1"/><col min="11" max="16384" style="2"/>"#;
    for attr in [
        "s=\"4\"",
        "s=\"2\" customHeight=\"1\" ht=\"45\"",
        "s=\"2\" outlineLevel=\"1\"",
        "s=\"2\" collapsed=\"1\"",
    ] {
        let rows = format!("<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"2\" {attr}/>");
        let result = clean(rows.as_bytes(), columns, true, false);
        assert!(
            String::from_utf8_lossy(&result.xml).contains("r=\"2\""),
            "{attr}"
        );
    }
    let rows = br#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" s="2" customFormat="1"/>"#;
    let result = clean(rows, columns, true, false);
    assert!(!String::from_utf8_lossy(&result.xml).contains("r=\"2\""));
}

/// A cell that clears a colored column is a real visual anchor. Unknown cell
/// style IDs cannot establish redundancy and must stay conservative.
#[test]
fn masking_cells_and_unknown_styles_remain_anchors() {
    for (row, columns) in [
        (
            &br#"<row r="2"><c r="A2" s="1"/></row>"#[..],
            &br#"<col min="1" max="16384" style="3"/>"#[..],
        ),
        (
            &br#"<row r="2"><c r="A2" s="999"/></row>"#[..],
            WHITE_COLUMNS,
        ),
    ] {
        let mut rows = br#"<row r="1"><c r="A1"><v>1</v></c></row>"#.to_vec();
        rows.extend_from_slice(row);
        let result = clean(&rows, columns, false, false);
        assert!(String::from_utf8_lossy(&result.xml).contains("r=\"2\""));
    }
}
