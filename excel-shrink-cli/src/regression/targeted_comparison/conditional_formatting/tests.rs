use super::*;
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

#[test]
fn x14_expression_adapter_requires_known_namespaces_and_static_format_content() -> Result<()> {
    let root =
        crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-x14-expression")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let rule = concat!(
        "<extLst><ext xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\">",
        "<x14:conditionalFormattings><x14:conditionalFormatting xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\">",
        "<x14:cfRule type=\"expression\" priority=\"1\" id=\"stable-id\"><xm:f>A1=1</xm:f>",
        "<x14:dxf><fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFFF0000\"/></patternFill></fill></x14:dxf>",
        "</x14:cfRule><xm:sqref>B2:D5</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"
    );
    let values =
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"5\"><c r=\"D5\"><v>1</v></c></row>";
    write_book(&a, values, rule, "", "", "")?;
    fs::copy(&a, &b)?;
    assert_eq!(check(&a, &b)?.0[0].outcome, "excluded");
    write_book(&b, &values.replacen("<v>1", "<v>2", 1), rule, "", "", "")?;
    assert!(check(&a, &b)?.1.iter().any(|c| c.cell == "B2:B2"));
    for unknown in [
        rule.replace("2009/9/main", "unknown"),
        rule.replace("excel/2006/main", "excel/unknown"),
        rule.replace("<fill>", "<fill><x14:unknown/>"),
        rule.replace("<fill>", "<fill unexpected=\"1\">"),
        rule.replace("<xm:f>", "<xm:f unexpected=\"1\">"),
        rule.replace("type=\"expression\"", "type=\"dataBar\""),
    ] {
        write_book(&a, values, &unknown, "", "", "")?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
    }
    write_book(&a, values, rule, "", "", "")?;
    write_book(
        &b,
        values,
        &rule.replace("FFFF0000", "FF0000FF"),
        "",
        "",
        "",
    )?;
    assert!(check(&a, &b)?.0.iter().all(|d| d.outcome == "capture"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn cross_sheet_changes_and_uncertain_formulas_select_the_target_sheet() -> Result<()> {
    let root =
        crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-cross-sheet")?;
    fs::create_dir_all(&root)?;
    let target = root.join("target.xlsx");
    let source = root.join("source.xlsx");
    for (formula, expected) in [
        ("other!A1=1", "B2:B2"),
        ("'O''Brien data'!$A$1=1", "B2:D5"),
        ("SUM(Other!$A$1:$A$2)&gt;0", "B2:D5"),
    ] {
        let block = format!("<conditionalFormatting sqref=\"B2:D5\"><cfRule type=\"expression\" priority=\"1\"><formula>{formula}</formula></cfRule></conditionalFormatting>");
        write_book(
            &target,
            "<row r=\"5\"><c r=\"D5\"><v>1</v></c></row>",
            &block,
            "",
            "",
            "",
        )?;
        let values = "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>";
        write_book(&source, values, "", "", "", "")?;
        let mut a = Snapshot::read(&target)?;
        let name = if formula.contains("Brien") {
            "O'Brien data"
        } else {
            "Other"
        };
        let source_sheet = Snapshot::read(&source)?.sheets.remove("Data").unwrap();
        a.sheets.insert(name.into(), source_sheet);
        let mut b = a.clone();
        let mut selected = BTreeSet::new();
        assert_eq!(
            select(&a, &b, &a.changes(&b)?, &mut selected)?[0].outcome,
            "excluded"
        );
        for changed in [
            values.replace("<v>1", "<v>2"),
            values.replace("<v>1", "<f>NOW()</f><v>1"),
        ] {
            write_book(&source, &changed, "", "", "", "")?;
            b.sheets.insert(
                name.into(),
                Snapshot::read(&source)?.sheets.remove("Data").unwrap(),
            );
            selected.clear();
            let decisions = select(&a, &b, &a.changes(&b)?, &mut selected)?;
            assert_eq!(decisions[0].outcome, "partial");
            assert!(selected
                .iter()
                .any(|c| c.sheet == "Data" && c.cell == expected));
        }
        b.sheets.get_mut(name).unwrap().value_namespace_supported = false;
        selected.clear();
        assert_eq!(
            select(&a, &b, &a.changes(&b)?, &mut selected)?[0].outcome,
            "capture"
        );
        b.sheets.remove(name);
        selected.clear();
        assert_eq!(
            select(&a, &b, &a.changes(&b)?, &mut selected)?[0].outcome,
            "capture"
        );
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn namespace_uncertainty_is_local_to_its_cf_block_and_scopes_are_restored() -> Result<()> {
    let root =
        crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-namespaces")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let extension = "<extLst><ext xmlns=\"foreign\"><x14:conditionalFormatting xmlns:x14=\"foreign\" sqref=\"A1\"><x14:cfRule type=\"expression\"><x14:formula>A1</x14:formula></x14:cfRule></x14:conditionalFormatting></ext></extLst>";
    let values = "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>";
    for rules in [
        format!("{}{extension}", literal_rule()),
        format!("{extension}{}", literal_rule()),
    ] {
        write_book(&a, values, &rules, "", "", "")?;
        fs::copy(&a, &b)?;
        let (decisions, _) = check(&a, &b)?;
        assert_eq!(
            decisions.iter().filter(|d| d.outcome == "excluded").count(),
            1
        );
        assert_eq!(
            decisions.iter().filter(|d| d.outcome == "capture").count(),
            1
        );
    }
    let foreign_child = literal_rule().replace("<cfRule ", "<cfRule xmlns=\"foreign\" ");
    write_book(&a, values, &foreign_child, "", "", "")?;
    fs::copy(&a, &b)?;
    assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
    // Changing only the inherited namespace must invalidate identical raw CF bytes.
    write_book(&a, values, &literal_rule(), "", "", "")?;
    write_book(&b, values, &literal_rule(), "", "", "")?;
    let file = fs::OpenOptions::new().read(true).write(true).open(&b)?;
    let mut zip = zip::ZipWriter::new_append(file)?;
    zip.start_file(
        "xl/worksheets/sheet1.xml",
        zip::write::FileOptions::default(),
    )?;
    write!(
        zip,
        "<worksheet xmlns=\"foreign\"><sheetData>{values}</sheetData>{}</worksheet>",
        literal_rule()
    )?;
    zip.finish()?;
    assert!(check(&a, &b)?.0.iter().all(|d| d.outcome == "capture"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn mixed_blocks_union_known_dependencies_and_keep_unknown_members_conservative() -> Result<()> {
    let root = crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-mixed")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let values = concat!(
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>",
        "<row r=\"3\"><c r=\"B3\"><v>1</v></c></row>",
        "<row r=\"5\"><c r=\"D5\"><v>2</v></c></row>"
    );
    let expression =
        "<cfRule type=\"expression\" priority=\"1\"><formula>LEN(TRIM(A1))=0</formula></cfRule>";
    let duplicate = "<cfRule type=\"duplicateValues\" priority=\"2\"/>";
    let scale = "<cfRule type=\"colorScale\" priority=\"2\"><colorScale><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FFFF0000\"/><color rgb=\"FF00FF00\"/></colorScale></cfRule>";
    let dummy = "<cfRule type=\"cellIs\" operator=\"notEqual\" priority=\"3\"><formula>INDIRECT(&quot;Dummy_for_Comparison1!&quot;&amp;ADDRESS(ROW(),COLUMN()))</formula></cfRule>";
    let bar = "<cfRule type=\"dataBar\" priority=\"2\"><dataBar><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FF638EC6\"/></dataBar></cfRule>";
    for group in [duplicate, scale, bar] {
        let block = format!("<conditionalFormatting sqref=\"B2:D5\">{expression}{group}{dummy}<cfRule type=\"expression\" priority=\"4\"><formula>#REF!</formula></cfRule></conditionalFormatting>");
        write_book(&a, values, &block, "", "", "")?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "excluded");
        write_book(&b, &values.replacen("<v>1", "<v>3", 1), &block, "", "", "")?;
        let (decisions, candidates) = check(&a, &b)?;
        assert_eq!(decisions[0].outcome, "partial");
        assert!(candidates.iter().any(|candidate| candidate.cell == "B2:B2"));
        assert!(!candidates.iter().any(|candidate| candidate.cell == "B2:D5"));
        write_book(
            &b,
            &values.replace("r=\"B3\"><v>1", "r=\"B3\"><v>3"),
            &block,
            "",
            "",
            "",
        )?;
        let (decisions, candidates) = check(&a, &b)?;
        assert_eq!(decisions[0].outcome, "capture");
        assert!(candidates.iter().any(|candidate| candidate.cell == "B2:D5"));
        let names = "<sheet name=\"Dummy_for_Comparison1\" state=\"hidden\" r:id=\"unused\"/>";
        write_book(&a, values, &block, names, "", "")?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
        let unknown = block.replace("LEN(TRIM(A1))=0", "INDIRECT(&quot;A1&quot;)=0");
        write_book(&a, values, &unknown, "", "", "")?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn simple_formulas_select_every_reference_and_reject_unknown_dependencies() -> Result<()> {
    let root =
        crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-expression")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let values = concat!(
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"F1\"><v>0</v></c></row>",
        "<row r=\"5\"><c r=\"D5\"><v>1</v></c></row>"
    );
    let formula = "IF(A1&gt;0,$F$1=1,FALSE)";
    let block = format!("<conditionalFormatting sqref=\"B2:D5\"><cfRule type=\"expression\" priority=\"1\"><formula>{formula}</formula></cfRule></conditionalFormatting>");
    write_book(&a, values, &block, "", "", "")?;
    fs::copy(&a, &b)?;
    assert_eq!(check(&a, &b)?.0[0].outcome, "excluded");
    for (changed, target) in [
        (values.replacen("<v>1", "<v>2", 1), "B2:B2"),
        (values.replace("<v>0", "<v>1"), "B2:D5"),
    ] {
        write_book(&b, &changed, &block, "", "", "")?;
        assert!(check(&a, &b)?
            .1
            .iter()
            .any(|candidate| candidate.cell == target));
    }
    for unsupported in [
        "Other!A1=1",
        "INDIRECT(&quot;A1&quot;)=1",
        "NamedRange=1",
        "SUBTOTAL(109,A1:B2)&gt;0",
    ] {
        let unknown = block.replace(formula, unsupported);
        write_book(&a, values, &unknown, "", "", "")?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn len_trim_dependencies_capture_targets_outside_the_changed_input() -> Result<()> {
    let root =
        crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-references")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    for (reference, target) in [
        ("A1", "B2:B2"),
        ("$A1", "B2:D2"),
        ("A$1", "B2:B5"),
        ("$A$1", "B2:D5"),
    ] {
        let block = format!("<conditionalFormatting sqref=\"B2:D5\"><cfRule type=\"expression\" priority=\"1\"><formula>LEN(TRIM({reference}))=0</formula></cfRule></conditionalFormatting>");
        let before = "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"5\"><c r=\"D5\"><v>1</v></c></row>";
        write_book(&a, before, &block, "", "", "")?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "excluded");
        write_book(&b, &before.replacen("<v>1", "<v>2", 1), &block, "", "", "")?;
        let (decisions, candidates) = check(&a, &b)?;
        assert_eq!(decisions[0].outcome, "partial");
        assert_eq!(decisions[0].referenced_input_changes, 1);
        assert!(
            candidates.iter().any(|c| c.cell == target),
            "{reference}: {candidates:?}"
        );
        write_book(
            &b,
            &before.replace("<c r=\"A1\"><v>1</v></c>", ""),
            &block,
            "",
            "",
            "",
        )?;
        assert!(check(&a, &b)?.1.iter().any(|c| c.cell == target));
        let formula = before.replacen("<v>1", "<f>NOW()</f><v>1", 1);
        write_book(&a, &formula, &block, "", "", "")?;
        fs::copy(&a, &b)?;
        let (decisions, candidates) = check(&a, &b)?;
        assert_eq!(decisions[0].referenced_formula_observations, 2);
        assert!(candidates.iter().any(|c| c.cell == target));
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn whole_area_rules_exclude_equal_inputs_but_capture_every_dependent_cell() -> Result<()> {
    let root = crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-group")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let duplicate = "<cfRule type=\"duplicateValues\" priority=\"1\"/>";
    let scale = "<cfRule type=\"colorScale\" priority=\"1\"><colorScale><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FFFF0000\"/><color rgb=\"FF00FF00\"/></colorScale></cfRule>";
    let bar = "<cfRule type=\"dataBar\" priority=\"1\"><dataBar><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FF638EC6\"/></dataBar></cfRule>";
    let fixed_bar = bar
        .replace("type=\"min\"", "type=\"num\" val=\"0\"")
        .replace("type=\"max\"", "type=\"percentile\" val=\"90\"");
    for rule in [duplicate, scale, bar, fixed_bar.as_str()] {
        let block =
            format!("<conditionalFormatting sqref=\"A1:B5\">{rule}</conditionalFormatting>");
        let values = concat!(
            "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>",
            "<row r=\"5\"><c r=\"B5\"><v>3</v></c></row>"
        );
        write_book(&a, values, &block, "", "", "")?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "excluded");
        // Even a change that preserves the rule's result must trigger the full area.
        write_book(&b, &values.replace("<v>1", "<v>2"), &block, "", "", "")?;
        let (decisions, candidates) = check(&a, &b)?;
        assert_eq!(decisions[0].outcome, "capture");
        assert!(candidates.iter().any(|candidate| candidate.cell == "A1:B5"));
        let outside = format!("{values}<row r=\"6\"><c r=\"C6\"><v>9</v></c></row>");
        write_book(&b, &outside, &block, "", "", "")?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "excluded");
        let deleted = values.replace("<c r=\"A1\"><v>1</v></c>", "");
        write_book(&b, &deleted, &block, "", "", "")?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
        // Unchanged formula caches do not establish unchanged live inputs.
        let formula = values.replace("<v>1", "<f>NOW()</f><v>1");
        write_book(&a, &formula, &block, "", "", "")?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
        // A formula threshold cannot be treated as a local value-set dependency.
        if rule == scale || rule == bar {
            let dependent = block.replace("type=\"min\"", "type=\"formula\" val=\"Other!A1\"");
            write_book(&a, values, &dependent, "", "", "")?;
            fs::copy(&a, &b)?;
            assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
        }
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn linked_and_x14_data_bars_keep_whole_area_dependencies() -> Result<()> {
    let root = crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-bars")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let standard = concat!(
        "<conditionalFormatting sqref=\"A1:B5\"><cfRule type=\"dataBar\" priority=\"1\">",
        "<dataBar><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FF638EC6\"/></dataBar>",
        "<extLst><ext uri=\"{B025F937-C7B1-47D3-B67F-A62EFF666E3E}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\">",
        "<x14:id>stable-id</x14:id></ext></extLst></cfRule></conditionalFormatting>"
    );
    let extension = concat!(
        "<extLst><ext xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\">",
        "<x14:conditionalFormattings><x14:conditionalFormatting xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\">",
        "<x14:cfRule type=\"dataBar\" id=\"stable-id\"><x14:dataBar minLength=\"0\" maxLength=\"100\" gradient=\"0\">",
        "<x14:cfvo type=\"autoMin\"/><x14:cfvo type=\"autoMax\"/>",
        "<x14:negativeFillColor rgb=\"FFFF0000\"/><x14:axisColor rgb=\"FF000000\"/>",
        "</x14:dataBar></x14:cfRule><xm:sqref>A1:B5</xm:sqref>",
        "</x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"
    );
    let values =
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"5\"><c r=\"B5\"><v>3</v></c></row>";
    let paired = format!("{standard}{extension}");
    let mixed = paired.replace("</cfRule></conditionalFormatting>", "</cfRule><cfRule type=\"colorScale\" priority=\"2\"><colorScale><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FFFF0000\"/><color rgb=\"FF00FF00\"/></colorScale></cfRule></conditionalFormatting>");
    for block in [paired.clone(), mixed] {
        write_book(&a, values, &block, "", "", "")?;
        fs::copy(&a, &b)?;
        assert!(check(&a, &b)?.0.iter().all(|d| d.outcome == "excluded"));
        write_book(&b, &values.replace("<v>3", "<v>9"), &block, "", "", "")?;
        let (decisions, candidates) = check(&a, &b)?;
        assert!(decisions.iter().all(|d| d.outcome == "capture"));
        assert!(candidates.iter().any(|c| c.cell == "A1:B5"));
        write_book(
            &b,
            values,
            &block
                .replace("FF638EC6", "FF123456")
                .replace("FFFF0000", "FF00FF00"),
            "",
            "",
            "",
        )?;
        assert!(check(&a, &b)?.0.iter().all(|d| d.outcome == "capture"));
        write_book(&b, values, &block, "", "", "<dxfs/>")?;
        assert!(check(&a, &b)?.0.iter().all(|d| d.outcome == "capture"));
    }
    for unknown in [
        standard.to_owned(),
        extension.to_owned(),
        paired.replace("<xm:sqref>A1:B5", "<xm:sqref>A1:C5"),
        paired.replace("<x14:id>stable-id", "<x14:id>missing-id"),
        format!("{standard}{paired}"),
        standard.replace(
            "{B025F937-C7B1-47D3-B67F-A62EFF666E3E}",
            "unknown-extension",
        ),
        standard.replace("<dataBar>", "<dataBar unknown=\"1\">"),
        extension.replace("gradient=\"0\"", "unknown=\"1\""),
        extension.replace(
            "type=\"autoMin\"/>",
            "type=\"formula\"><xm:f>Other!A1</xm:f></x14:cfvo>",
        ),
        extension.replace(
            "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main",
            "foreign",
        ),
    ] {
        write_book(&a, values, &unknown, "", "", "")?;
        fs::copy(&a, &b)?;
        assert!(check(&a, &b)?.0.iter().all(|d| d.outcome == "capture"));
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

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
        ("[Content_Types].xml", "<Types/>"),
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

#[test]
fn bare_isblank_requires_proven_name_absence_in_both_states() -> Result<()> {
    let root =
        crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-missing-name")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let values =
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"5\"><c r=\"B5\"><v>3</v></c></row>";
    let block = "<conditionalFormatting sqref=\"A1:B5\"><cfRule type=\"expression\" dxfId=\"0\" priority=\"1\" stopIfTrue=\"1\"><formula>isblank</formula></cfRule></conditionalFormatting>";
    write_book(&a, values, block, "", "", "")?;
    write_book(&b, &values.replace("<v>1", "<v>2"), block, "", "", "")?;
    assert_eq!(check(&a, &b)?.0[0].outcome, "excluded");
    let before = Snapshot::read(&a)?;
    let after = Snapshot::read(&b)?;
    for (left, right) in [
        (Some(["isblank".into()].into()), Some(BTreeSet::new())),
        (Some(BTreeSet::new()), Some(["isblank".into()].into())),
        (None, Some(BTreeSet::new())),
        (Some(BTreeSet::new()), None),
    ] {
        let mut before = before.clone();
        let mut after = after.clone();
        before.defined_names = left;
        after.defined_names = right;
        let decisions = select(
            &before,
            &after,
            &before.changes(&after)?,
            &mut BTreeSet::new(),
        )?;
        assert_eq!(decisions[0].outcome, "capture");
    }
    let function = block.replace("<formula>isblank", "<formula>ISBLANK(A1)");
    write_book(&a, values, &function, "", "", "")?;
    write_book(&b, &values.replace("<v>1", "<v>2"), &function, "", "", "")?;
    assert_ne!(check(&a, &b)?.0[0].outcome, "excluded");
    for unknown in [
        "UnknownName",
        "Other!isblank",
        "INDIRECT(&quot;isblank&quot;)",
    ] {
        write_book(
            &a,
            values,
            &block.replace("<formula>isblank", &format!("<formula>{unknown}")),
            "",
            "",
            "",
        )?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
    }
    for changed in [
        block.replace("dxfId=\"0\"", "dxfId=\"1\""),
        block.replace("A1:B5", "A1:B4"),
    ] {
        write_book(&a, values, block, "", "", "")?;
        write_book(&b, values, &changed, "", "", "")?;
        assert!(check(&a, &b)?.0.iter().all(|d| d.outcome == "capture"));
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn defined_name_catalogue_includes_local_hidden_names_and_rejects_uncertain_encodings() -> Result<()>
{
    assert!(xml::defined_names(br#"<workbook><definedNames><definedName name="_xlnm.Print_Area">Data!A1:B5</definedName></definedNames></workbook>"#)?.is_some());
    let names = xml::defined_names(br#"<workbook><definedNames><definedName name="ISBLANK" localSheetId="2" hidden="1">TRUE</definedName></definedNames></workbook>"#)?.unwrap();
    assert!(names.contains("isblank"));
    for name in ["i&#115;blank", "_x0069_sblank", "näme"] {
        assert!(xml::defined_names(format!("<workbook><definedNames><definedName name='{name}'>TRUE</definedName></definedNames></workbook>").as_bytes())?.is_none());
    }
    Ok(())
}

#[test]
fn constant_ref_errors_have_no_value_dependency_but_keep_rule_and_context_guards() -> Result<()> {
    let root =
        crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-ref-error")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let values =
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"5\"><c r=\"B5\"><v>3</v></c></row>";
    let template = "<conditionalFormatting sqref=\"A1:B5\"><cfRule type=\"expression\" priority=\"1\" stopIfTrue=\"1\"><formula>FORMULA</formula></cfRule></conditionalFormatting>";
    for formula in ["#REF!", "#REF!=1", "= #REF! = 1", "#REF!&#61;1"] {
        let block = template.replace("FORMULA", formula);
        write_book(&a, values, &block, "", "", "")?;
        write_book(&b, &values.replace("<v>1", "<v>9"), &block, "", "", "")?;
        let mut before = Snapshot::read(&a)?;
        let mut after = Snapshot::read(&b)?;
        before.defined_names = None;
        after.defined_names = None;
        let decisions = select(
            &before,
            &after,
            &before.changes(&after)?,
            &mut BTreeSet::new(),
        )?;
        assert_eq!(decisions[0].outcome, "excluded");
        write_book(&b, values, &block, "", "", "<dxfs/>")?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
        write_book(&b, values, &template.replace("FORMULA", "1"), "", "", "")?;
        assert!(check(&a, &b)?.0.iter().all(|d| d.outcome == "capture"));
    }
    for formula in [
        "#REF!+A1",
        "#REF!=A1",
        "Other!#REF!",
        "#REF!A1",
        "ISERROR(#REF!)",
        "#REF!=1+NOW()",
    ] {
        write_book(
            &a,
            values,
            &template.replace("FORMULA", formula),
            "",
            "",
            "",
        )?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture", "{formula}");
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn cell_is_ref_error_operands_preserve_operator_and_context_guards() -> Result<()> {
    let root =
        crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "cf-cell-is-error")?;
    fs::create_dir_all(&root)?;
    let a = root.join("a.xlsx");
    let b = root.join("b.xlsx");
    let values =
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"5\"><c r=\"B5\"><v>3</v></c></row>";
    let template = "<conditionalFormatting sqref=\"A1:B5\"><cfRule type=\"cellIs\" operator=\"OPERATOR\" priority=\"1\" stopIfTrue=\"1\"><formula>#REF!</formula></cfRule></conditionalFormatting>";
    for operator in [
        "equal",
        "notEqual",
        "lessThan",
        "lessThanOrEqual",
        "greaterThan",
        "greaterThanOrEqual",
    ] {
        let block = template.replace("OPERATOR", operator);
        write_book(&a, values, &block, "", "", "")?;
        write_book(&b, &values.replace("<v>1", "<v>9"), &block, "", "", "")?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "excluded");
        write_book(&b, values, &block, "", "", "<dxfs/>")?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
        write_book(&b, values, &block.replace("#REF!", "0"), "", "", "")?;
        assert!(check(&a, &b)?.0.iter().all(|d| d.outcome == "capture"));
    }
    for operand in [
        "Other!#REF!",
        "'[other.xlsx]Sheet'!#REF!",
        "#REF!+A1",
        "#REF!=1",
    ] {
        let block = template
            .replace("OPERATOR", "equal")
            .replace("#REF!", operand);
        write_book(&a, values, &block, "", "", "")?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
    }
    for operator in ["between", "notBetween", "unknown"] {
        let block = template
            .replace("OPERATOR", operator)
            .replace("</formula>", "</formula><formula>0</formula>");
        write_book(&a, values, &block, "", "", "")?;
        fs::copy(&a, &b)?;
        assert_eq!(check(&a, &b)?.0[0].outcome, "capture");
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[ignore = "Requires CF_INVENTORY with retained Data.gov rule metadata; no Excel"]
fn retained_cell_is_errors_are_supported_but_the_external_reference_is_not() -> Result<()> {
    use std::io::{BufRead, BufReader};
    let root = std::path::PathBuf::from(std::env::var("CF_INVENTORY")?);
    let snapshot = Snapshot {
        sheets: BTreeMap::new(),
        context: String::new(),
        cf_context: String::new(),
        sheet_names: None,
        defined_names: None,
    };
    let (mut errors, mut external) = (0, 0);
    for line in BufReader::new(File::open(root.join("sheets.jsonl"))?).lines() {
        let sheet: serde_json::Value = serde_json::from_str(&line?)?;
        let id = sheet["source_id"].as_u64().unwrap();
        if ![1024, 2090, 4712, 4851, 1030].contains(&id) {
            continue;
        }
        for raw in sheet["blocks"].as_array().unwrap() {
            let mut blocks = Vec::new();
            crate::regression::cf_inventory::scan::scan(
                raw["raw_xml"].as_str().unwrap().as_bytes(),
                &mut blocks,
                256,
                32,
            )?;
            for block in blocks {
                if block.rules.iter().all(|rule| {
                    rule.attributes.get("type").map(String::as_str) == Some("cellIs")
                        && rule.formulas == ["#REF!"]
                }) {
                    assert!(matches!(
                        proof(&block, &snapshot, &snapshot),
                        Proof::InputIndependent(_)
                    ));
                    errors += 1;
                }
                if id == 1030 {
                    for rule in &block.rules {
                        assert!(input_independent::proof(rule, &snapshot, &snapshot).is_none());
                        external += 1;
                    }
                }
            }
        }
    }
    assert_eq!(errors, 16);
    assert_eq!(external, 1);
    Ok(())
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
    // This is now a supported expression dependency, never a literal constant.
    assert_eq!(check(&a, &b)?.0[0].outcome, "excluded");
    write_book(
        &b,
        &values.replace("<v>1", "<v>2"),
        &hidden_expression,
        "",
        "",
        "",
    )?;
    let (decisions, _) = check(&a, &b)?;
    assert_eq!(decisions[0].outcome, "partial");
    assert_eq!(decisions[0].referenced_input_changes, 1);
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
