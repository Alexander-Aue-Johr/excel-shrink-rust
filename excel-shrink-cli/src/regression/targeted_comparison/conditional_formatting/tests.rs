use super::*;
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

fn write_book(
    path: &Path,
    values: &str,
    rule: &str,
    extra_names: &str,
    layout: &str,
    style: &str,
) -> Result<()> {
    let mut zip = zip::ZipWriter::new(File::create(path)?);
    let workbook = format!(
        "<workbook><sheets><sheet name=\"Data\" r:id=\"one\"/>{extra_names}</sheets></workbook>"
    );
    let relationships = concat!(
        "<Relationships><Relationship Id=\"one\" ",
        "Type=\"office/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>"
    );
    let styles = format!("<styleSheet xmlns=\"{NS}\">{style}</styleSheet>");
    let worksheet = format!(
        "<worksheet xmlns=\"{NS}\">{layout}<sheetData>{values}</sheetData>{rule}</worksheet>"
    );
    for (name, body) in [
        ("xl/workbook.xml", workbook.as_str()),
        ("xl/_rels/workbook.xml.rels", relationships),
        ("xl/styles.xml", styles.as_str()),
        ("xl/worksheets/sheet1.xml", worksheet.as_str()),
    ] {
        zip.start_file(name, zip::write::FileOptions::default())?;
        zip.write_all(body.as_bytes())?;
    }
    zip.finish()?;
    Ok(())
}

fn literal_rule() -> String {
    "<conditionalFormatting sqref=\"A1:B1048576\"><cfRule type=\"cellIs\" operator=\"greaterThan\" priority=\"1\"><formula>0</formula></cfRule></conditionalFormatting>".into()
}

fn check(left: &Path, right: &Path) -> Result<(Vec<Decision>, BTreeSet<Candidate>)> {
    let a = Snapshot::read(left)?;
    let b = Snapshot::read(right)?;
    let changes = a.changes(&b)?;
    let mut candidates = BTreeSet::new();
    let decisions = select(&a, &b, &changes, &mut candidates)?;
    Ok((decisions, candidates))
}

/// Exercise retained local corpus evidence without making it a repository fixture.
#[test]
#[ignore = "requires CF_ORIGINAL and CF_SHRUNK paths to local corpus copies"]
fn retained_dummy_workbook_keeps_its_missing_target_proof_after_shrinking() -> Result<()> {
    let original = std::env::var("CF_ORIGINAL")?;
    let shrunk = std::env::var("CF_SHRUNK")?;
    let (decisions, _) = check(Path::new(&original), Path::new(&shrunk))?;
    assert!(!decisions.is_empty());
    for decision in &decisions {
        assert_eq!(decision.outcome, "excluded");
        assert_eq!(decision.reason, "fixed-dummy-targets-absent-in-both-states");
    }
    println!(
        "{} retained real-workbook CF blocks excluded",
        decisions.len()
    );
    Ok(())
}

#[test]
fn unchanged_inputs_exclude_a_million_row_rule_but_changed_cells_remain() -> Result<()> {
    let root = crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-local")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let before = "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>";
    write_book(&a, before, &literal_rule(), "", "", "")?;
    fs::copy(&a, &b)?;
    let (decisions, candidates) = check(&a, &b)?;
    assert_eq!(decisions[0].outcome, "excluded");
    assert!(candidates.is_empty());
    write_book(
        &b,
        "<row r=\"1\"><c r=\"A1\"><v>2</v></c></row>",
        &literal_rule(),
        "",
        "",
        "",
    )?;
    let (decisions, candidates) = check(&a, &b)?;
    assert_eq!(decisions[0].outcome, "partial");
    assert_eq!(
        candidates,
        BTreeSet::from([Candidate {
            sheet: "Data".into(),
            cell: "A1".into()
        }])
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn formula_inputs_and_changed_row_formats_are_not_proven_by_equal_cached_values() -> Result<()> {
    let root =
        crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-formulas")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let values = "<row r=\"1\"><c r=\"A1\"><f>NOW()</f><v>1</v></c></row>";
    write_book(&a, values, &literal_rule(), "", "", "")?;
    fs::copy(&a, &b)?;
    let (decisions, candidates) = check(&a, &b)?;
    assert_eq!(decisions[0].outcome, "partial");
    assert!(candidates.iter().any(|c| c.cell == "A1"));
    write_book(
        &b,
        "<row r=\"1\" s=\"1\" customFormat=\"1\"><c r=\"A1\"><f>NOW()</f><v>1</v></c></row>",
        &literal_rule(),
        "",
        "",
        "",
    )?;
    let (decisions, candidates) = check(&a, &b)?;
    assert_eq!(decisions[0].layout_rows, 1);
    assert!(candidates.iter().any(|c| c.cell.contains(':')));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn context_rule_or_range_changes_force_capture_of_the_old_and_new_scope() -> Result<()> {
    let root = crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-context")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let values = "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>1</v></c></row>";
    write_book(&a, values, &literal_rule(), "", "", "")?;
    for (rule, layout, style) in [
        (literal_rule(), "", "<dxfs/>"),
        (
            literal_rule(),
            "<cols><col min=\"1\" max=\"2\" width=\"30\"/></cols>",
            "",
        ),
        (literal_rule().replace("<formula>0", "<formula>1"), "", ""),
        (String::new(), "", ""),
    ] {
        write_book(&b, values, &rule, "", layout, style)?;
        let (decisions, candidates) = check(&a, &b)?;
        assert!(decisions.iter().any(|d| d.outcome == "capture"));
        assert!(!candidates.is_empty());
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn missing_dummy_targets_are_proven_only_when_absent_on_both_sides() -> Result<()> {
    let root = crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-dummy")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let values = "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>";
    let rule=literal_rule().replace("greaterThan","notEqual").replace("<formula>0</formula>","<formula>INDIRECT(&quot;Dummy_for_Comparison1!&quot;&amp;ADDRESS(ROW(),COLUMN()))</formula>");
    write_book(&a, values, &rule, "", "", "")?;
    fs::copy(&a, &b)?;
    assert_eq!(check(&a, &b)?.0[0].outcome, "excluded");
    for name in [
        "Dummy_for_Comparison1",
        "DUMMY_FOR_COMPARISON1",
        "Dummy&#95;for_Comparison1",
    ] {
        let names = format!("<sheet name=\"{name}\" state=\"hidden\" r:id=\"unused\"/>");
        write_book(&b, values, &rule, &names, "", "")?;
        let (decisions, candidates) = check(&a, &b)?;
        assert_eq!(decisions[0].outcome, "capture");
        assert!(!candidates.is_empty());
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn custom_theme_parts_invalidate_the_literal_input_proof() -> Result<()> {
    let root = crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-theme")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    for (path, color) in [(&a, "red"), (&b, "blue")] {
        write_book(
            path,
            "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>",
            &literal_rule(),
            "",
            "",
            "",
        )?;
        let file = fs::OpenOptions::new().read(true).write(true).open(path)?;
        let mut archive = zip::ZipWriter::new_append(file)?;
        archive.start_file("[Content_Types].xml", zip::write::FileOptions::default())?;
        archive.write_all(b"<Types><Override PartName=\"/custom/colors.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/></Types>")?;
        archive.start_file("custom/colors.xml", zip::write::FileOptions::default())?;
        write!(archive, "<theme color=\"{color}\"/>")?;
        archive.finish()?;
    }
    let (decisions, candidates) = check(&a, &b)?;
    assert_eq!(decisions[0].outcome, "capture");
    assert!(!candidates.is_empty());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn missing_ranges_and_extension_namespaces_remain_capture_fallbacks() -> Result<()> {
    let root = crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-unknown")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let values = "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>";
    let missing = literal_rule().replace(" sqref=\"A1:B1048576\"", " pivot=\"1\"");
    let foreign = format!(
        "<extLst><extension xmlns=\"other\">{}</extension></extLst>",
        literal_rule()
    );
    for rule in [missing, foreign] {
        write_book(&a, values, &rule, "", "", "")?;
        fs::copy(&a, &b)?;
        let (decisions, candidates) = check(&a, &b)?;
        assert_eq!(decisions[0].outcome, "capture");
        assert!(!candidates.is_empty());
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn blank_zero_empty_string_and_deleted_values_are_never_collapsed() -> Result<()> {
    let root = crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-types")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    write_book(
        &a,
        "<row r=\"1\"><c r=\"A1\"><v>0</v></c></row>",
        &literal_rule(),
        "",
        "",
        "",
    )?;
    for value in [
        "<c r=\"A1\"/>",
        "<c r=\"A1\" t=\"inlineStr\"><is><t/></is></c>",
        "",
    ] {
        write_book(
            &b,
            &format!("<row r=\"1\">{value}</row>"),
            &literal_rule(),
            "",
            "",
            "",
        )?;
        let (decisions, candidates) = check(&a, &b)?;
        assert_eq!(decisions[0].outcome, "partial");
        assert!(candidates.iter().any(|c| c.cell == "A1"));
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn entity_encoded_constants_are_decoded_but_cdata_is_never_double_decoded() -> Result<()> {
    let root =
        crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-entities")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let values = "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>";
    let encoded =
        literal_rule().replace("<formula>0</formula>", "<formula>&quot;.&quot;</formula>");
    write_book(&a, values, &encoded, "", "", "")?;
    fs::copy(&a, &b)?;
    assert_eq!(check(&a, &b)?.0[0].outcome, "excluded");
    let hidden_expression = literal_rule().replace(
        "<formula>0</formula>",
        "<formula>&quot;a&quot;&amp;A1&amp;&quot;b&quot;</formula>",
    );
    write_book(&a, values, &hidden_expression, "", "", "")?;
    fs::copy(&a, &b)?;
    assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
    let cdata = literal_rule().replace("greaterThan", "notEqual").replace(
        "<formula>0</formula>",
        "<formula><![CDATA[INDIRECT(&quot;Dummy_for_Comparison1!&quot;&amp;ADDRESS(ROW(),COLUMN()))]]></formula>",
    );
    write_book(&a, values, &cdata, "", "", "")?;
    fs::copy(&a, &b)?;
    assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
    fs::remove_dir_all(root)?;
    Ok(())
}
