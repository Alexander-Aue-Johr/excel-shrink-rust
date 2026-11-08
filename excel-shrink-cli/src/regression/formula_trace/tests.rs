use super::*;
use std::io::{Cursor, Write};

fn cell(formula: &str, value: &str) -> workbook::Cell {
    workbook::Cell {
        formula: Some(formula.into()),
        value: Some(value.into()),
        kind: "n".into(),
        shared_anchor: None,
    }
}

#[test]
fn highlights_the_complete_volatile_call_without_matching_string_literals() {
    let formula = "IF(A1>RANDBETWEEN(1,SUM(2,3)),\"RAND()\",0)";
    assert_eq!(
        &formula[syntax::volatile_span(formula).unwrap()],
        "RANDBETWEEN(1,SUM(2,3))"
    );
    assert!(syntax::volatile_span("IF(A1=1,\"RAND()\",0)").is_none());
    assert!(syntax::volatile_span("MYRAND()").is_none());
    assert!(syntax::volatile_span("ÅRAND()").is_none());
    assert!(syntax::volatile_span("'RAND()'!A1").is_none());
}

#[test]
fn filename_calls_require_a_literal_info_type_and_preserve_balanced_arguments() {
    let formula = r#"IF(A1=1,CELL("filename",INDEX(B1:B3,2)),"")"#;
    assert_eq!(
        &formula[syntax::environment_span(formula).unwrap()],
        r#"CELL("filename",INDEX(B1:B3,2))"#
    );
    assert!(syntax::environment_span(r#"cell ( "FILENAME" )"#).is_some());
    for unsupported in [
        r#"CELL(A1)"#,
        r#"CELL("address",A1)"#,
        r#"MYCELL("filename")"#,
        r#"CELL("filename"&A1)"#,
        r#""CELL(""filename"")""#,
        r#"'CELL("filename")'!A1"#,
        r#"CELL("filename",A1"#,
    ] {
        assert!(syntax::environment_span(unsupported).is_none(), "{unsupported}");
    }
}

#[test]
fn filename_dependency_paths_are_distinguished_from_random_time_and_keep_raw_changes() -> Result<()> {
    let start = Address::parse("S", "A1")?;
    let precedent = Address::parse("S", "B1")?;
    let left = Workbook(BTreeMap::from([
        (start.clone(), cell("B1", "left path")),
        (precedent.clone(), cell(r#"CELL("filename")"#, "left path")),
    ]));
    let right = Workbook(BTreeMap::from([
        (start, cell("B1", "right path")),
        (precedent, cell(r#"CELL("filename")"#, "right path")),
    ]));
    let diagnosis = compare(&left, &right, "S", "A1")?;
    assert_eq!(diagnosis["changed_value_cells"], 1);
    assert_eq!(diagnosis["volatile_paths"], 0);
    assert_eq!(diagnosis["environment_paths"], 1);
    assert_eq!(diagnosis["all_changed_values_have_volatile_dependency"], false);
    assert_eq!(diagnosis["exclusive_cause_proven"], false);
    let example = &diagnosis["examples"][0];
    assert_eq!(example["dependency_kind"], "environment");
    assert_eq!(example["path"][0]["cell"], "A1");
    assert_eq!(example["path"][1]["cell"], "B1");
    assert_eq!(example["path"][1]["environment_span"], json!([0, 16]));
    assert!(example["path"][1]["volatile_span"].is_null());
    assert_eq!(example["path"][1]["left_value"], "left path");
    assert_eq!(example["path"][1]["right_value"], "right path");
    let source = source_examples(&right, "S", "A1:B1")?;
    assert_eq!(source["evidence"], "prepared_source_cached_values");
    assert_eq!(source["environment_paths"], 2);
    assert_eq!(source["volatile_paths"], 0);
    assert_eq!(source["coverage_complete"], false);
    Ok(())
}

#[test]
#[ignore = "Requires FORMULA_TRACE_FILENAME_SOURCE/OUTPUT for the retained Enron filename case"]
fn retained_enron_filename_source_has_an_environment_path() -> Result<()> {
    let source = read(Path::new(&std::env::var("FORMULA_TRACE_FILENAME_SOURCE")?))?;
    let diagnosis = source_examples(&source, "Tab 1", "A13")?;
    assert_eq!(diagnosis["environment_paths"], 1);
    assert_eq!(diagnosis["volatile_paths"], 0);
    assert_eq!(diagnosis["evidence"], "prepared_source_cached_values");
    assert_eq!(diagnosis["exclusive_cause_proven"], false);
    assert_eq!(diagnosis["examples"][0]["path"][0]["cell"], "A13");
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(std::env::var("FORMULA_TRACE_FILENAME_OUTPUT")?)?;
    serde_json::to_writer_pretty(&mut output, &diagnosis)?;
    Ok(())
}

#[test]
fn shared_formula_translation_preserves_absolute_references_and_strings() -> Result<()> {
    assert_eq!(
        syntax::shifted_formula(
            "A1+$B2+C$3+$D$4+IF(A1=\"A1\",1,0)",
            &Address::parse("S", "E5")?,
            &Address::parse("S", "F7")?
        )?,
        "B3+$B4+D$3+$D$4+IF(B3=\"A1\",1,0)"
    );
    Ok(())
}

#[test]
fn quoted_sheet_names_and_literal_indirect_resolve() -> Result<()> {
    let refs = syntax::references("'O''Brien'!$A$1+INDIRECT(\"'Other Sheet'!B2\",TRUE)", "S")?;
    let names: Vec<_> = refs
        .iter()
        .map(|d| match d {
            Dependency::Cell(a) => a.sheet.as_str(),
            _ => "range",
        })
        .collect();
    assert_eq!(names, vec!["Other Sheet", "O'Brien"]);
    assert!(syntax::references("INDIRECT(\"A\"&1)", "S").is_err());
    assert!(syntax::references("OFFSET(A1,1,0)", "S").is_err());
    Ok(())
}

#[test]
fn cycles_and_large_ranges_find_one_short_path_without_expanding_the_grid() -> Result<()> {
    let a = Address::parse("S", "A1")?;
    let b = Address::parse("S", "B999999")?;
    let cells = BTreeMap::from([
        (a.clone(), cell("A1+SUM(B1:B1048576)", "1")),
        (b.clone(), cell("RAND()", ".5")),
    ]);
    let mut budget = 100;
    assert_eq!(find_path(&cells, &a, &mut budget)?, Some(vec![a, b]));
    assert!(budget > 0);
    Ok(())
}

#[test]
fn saved_value_changes_and_unknown_dependencies_are_not_certified_as_exclusive_cause() -> Result<()>
{
    let a = Address::parse("S", "A1")?;
    let left = Workbook(BTreeMap::from([(a.clone(), cell("RAND()", ".2"))]));
    let right = Workbook(BTreeMap::from([(a, cell("RAND()", ".8"))]));
    let explanation = compare(&left, &right, "S", "A1:B1")?;
    assert_eq!(explanation["changed_value_cells"], 1);
    assert_eq!(explanation["volatile_paths"], 1);
    assert_eq!(explanation["exclusive_cause_proven"], false);
    assert_eq!(explanation["examples"][0]["path"][0]["left_value"], ".2");
    assert_eq!(explanation["examples"][0]["path"][0]["right_value"], ".8");
    Ok(())
}

#[test]
fn highlight_offsets_use_browser_utf16_coordinates() -> Result<()> {
    let key = Address::parse("S", "A1")?;
    let cells = BTreeMap::from([(key.clone(), cell("IF(\"🙂\"=\"x\",RAND(),0)", "1"))]);
    let node = nodes(&cells, &cells, &[key])?.remove(0);
    let start = node["volatile_span"][0].as_u64().unwrap() as usize;
    let end = node["volatile_span"][1].as_u64().unwrap() as usize;
    let units: Vec<_> = node["formula"].as_str().unwrap().encode_utf16().collect();
    assert_eq!(String::from_utf16(&units[start..end])?, "RAND()");
    Ok(())
}

#[test]
fn streams_hidden_dependencies_shared_formulas_and_decoded_values() -> Result<()> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (part, xml) in [
        (
            "xl/workbook.xml",
            r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Hidden" state="hidden" r:id="one"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships><Relationship Id="one" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "xl/sharedStrings.xml",
            r#"<sst><si><t>A&amp;B</t></si></sst>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<worksheet><sheetData><row r="1"><c r="A1"><f t="shared" si="0" ref="A1:A2">RAND()</f><v>0.2</v></c><c r="B1" t="s"><v>0</v></c></row><row r="2"><c r="A2"><f t="shared" si="0"/><v>0.3</v></c></row></sheetData></worksheet>"#,
        ),
    ] {
        zip.start_file(part, zip::write::FileOptions::default())?;
        zip.write_all(xml.as_bytes())?;
    }
    let bytes = zip.finish()?.into_inner();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let cells = workbook::read_archive(&mut archive)?;
    let key = Address::parse("Hidden", "A2")?;
    assert_eq!(cells[&key].formula.as_deref(), Some("RAND()"));
    assert_eq!(cells[&key].shared_anchor.as_deref(), Some("A1"));
    assert_eq!(
        cells[&Address::parse("Hidden", "B1")?].value.as_deref(),
        Some("A&B")
    );
    assert!(find_path(&cells, &key, &mut 100)?.is_some());
    Ok(())
}

#[test]
fn retained_example_limit_does_not_limit_dependency_counts() -> Result<()> {
    let mut left=BTreeMap::new();let mut right=BTreeMap::new();
    for row in 1..=25 {
        let key=Address::parse("S",&format!("A{row}"))?;
        left.insert(key.clone(),cell("RAND()",".1"));
        right.insert(key,cell("RAND()",".9"));
    }
    let diagnosis=compare(&Workbook(left),&Workbook(right),"S","A1:A25")?;
    assert_eq!(diagnosis["volatile_paths"],25);
    assert_eq!(diagnosis["examples"].as_array().unwrap().len(),20);
    assert_eq!(diagnosis["coverage_complete"],true);
    assert_eq!(diagnosis["all_changed_values_have_volatile_dependency"],true);
    assert_eq!(diagnosis["exclusive_cause_proven"],false);
    Ok(())
}

#[test]
#[ignore = "Requires FORMULA_TRACE_LEFT/RIGHT/OUTPUT pointing to retained native Cluster2 states"]
fn retained_native_cluster_values_have_supported_random_dependencies() -> Result<()> {
    let left=read(Path::new(&std::env::var("FORMULA_TRACE_LEFT")?))?;
    let right=read(Path::new(&std::env::var("FORMULA_TRACE_RIGHT")?))?;
    let diagnosis=compare(&left,&right,"Fig. 5a Aux.ARWH","A1:AK55")?;
    assert_eq!(diagnosis["changed_value_cells"],722);
    assert_eq!(diagnosis["volatile_paths"],722);
    assert_eq!(diagnosis["all_changed_values_have_volatile_dependency"],true);
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(std::env::var("FORMULA_TRACE_OUTPUT")?)?;
    serde_json::to_writer_pretty(&mut file,&diagnosis)?;
    Ok(())
}
