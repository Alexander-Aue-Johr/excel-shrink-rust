//! Conservative eligibility for replaying a historical full-rebuild result.
use crate::regression::{
    streaming_xml::{self, Event},
    xml_views::Kind,
};
use anyhow::Result;
use std::{fs::File, path::Path};

/// Unknown dependencies require a fresh rebuild. Optimized pre-rebuild artifacts
/// remain reusable independently; this gate never removes capture candidates.
pub(super) fn reusable_rebuild(path: &Path) -> Result<bool> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name();
        let known = name == "[Content_Types].xml"
            || name.ends_with(".rels")
            || name.starts_with("docProps/")
            || name == "xl/workbook.xml"
            || name == "xl/styles.xml"
            || name == "xl/sharedStrings.xml"
            || name == "xl/calcChain.xml"
            || name.starts_with("xl/theme/")
            || name.starts_with("xl/worksheets/")
            || name.starts_with("xl/charts/")
            || name.starts_with("xl/drawings/")
            || name.starts_with("xl/media/")
            || name.starts_with("xl/printerSettings/");
        if !known {
            return Ok(false);
        }
        if !name.ends_with(".xml") && !name.ends_with(".rels") {
            continue;
        }
        let entry_name_is_worksheet = name.starts_with("xl/worksheets/");
        let mut eligible = true;
        let mut formula_depth = None;
        let mut formula = Vec::new();
        streaming_xml::scan(entry, |event| {
            match event {
                Event::Tag(tag) => {
                    if entry_name_is_worksheet
                        && !matches!(tag.local(), b"f" | b"formula" | b"formula1" | b"formula2")
                        && super::execution_plan::worksheet_blocker(tag).is_some()
                    {
                        eligible = false;
                    }
                    if tag.kind != Kind::End
                        && tag.local() == b"cfvo"
                        && tag.attr(b"type")? == Some(b"formula")
                    {
                        eligible &= tag.attr(b"val")?.is_some_and(deterministic_formula);
                    }
                    if tag.kind != Kind::End
                        && (tag.local() == b"definedName"
                            || tag.local() == b"extLst" && entry_name_is_worksheet)
                    {
                        eligible = false;
                    }
                    if tag.kind != Kind::End
                        && tag.local() == b"calcPr"
                        && matches!(tag.attr(b"iterate")?, Some(b"1" | b"true"))
                    {
                        eligible = false;
                    }
                    if tag.kind != Kind::End && tag.local() == b"Relationship" {
                        if tag
                            .attr(b"Type")?
                            .is_some_and(|t| t.ends_with(b"/externalLink"))
                        {
                            eligible = false;
                        }
                    }
                    if matches!(tag.local(), b"f" | b"formula" | b"formula1" | b"formula2") {
                        if tag.kind == Kind::Start {
                            formula.clear();
                            formula_depth = Some(tag.depth);
                            if tag.local() == b"f"
                                && tag
                                    .attr(b"t")?
                                    .is_some_and(|t| t != b"normal" && t != b"array")
                            {
                                eligible = false;
                            }
                        } else if tag.kind == Kind::End && formula_depth == Some(tag.depth) {
                            eligible &= deterministic_formula(&formula);
                            formula_depth = None;
                        } else if tag.kind == Kind::Empty {
                            eligible = false;
                        }
                    }
                }
                Event::Text(text) if formula_depth.is_some() => formula.extend_from_slice(text),
                _ => {}
            }
            Ok(())
        })?;
        if !eligible {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Recognize ordinary references, literals, arithmetic and a bounded set of pure
/// built-ins. Names, UDFs, external/structured references and volatile calls fail
/// closed; this is dependency classification, not formula evaluation.
fn deterministic_formula(formula: &[u8]) -> bool {
    let mut index = 0;
    while index < formula.len() {
        let byte = formula[index];
        if byte == b'[' || !byte.is_ascii() {
            return false;
        }
        if byte == b'"' || byte == b'\'' {
            let quote = byte;
            index += 1;
            loop {
                if index == formula.len() {
                    return false;
                }
                if formula[index] == b'[' {
                    return false;
                }
                if formula[index] == quote {
                    index += 1;
                    if formula.get(index) == Some(&quote) {
                        index += 1;
                        continue;
                    }
                    break;
                }
                index += 1;
            }
            if quote == b'\'' && formula.get(index) != Some(&b'!') {
                return false;
            }
        } else if byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' {
            let start = index;
            while index < formula.len()
                && (formula[index].is_ascii_alphanumeric()
                    || matches!(formula[index], b'_' | b'$' | b'.'))
            {
                index += 1;
            }
            let token = String::from_utf8_lossy(&formula[start..index]).to_ascii_uppercase();
            let next = formula[index..]
                .iter()
                .copied()
                .find(|b| !b.is_ascii_whitespace());
            if next == Some(b'!') {
                continue;
            }
            if next == Some(b'(') {
                if !matches!(
                    token.as_str(),
                    "SUM"
                        | "AVERAGE"
                        | "MIN"
                        | "MAX"
                        | "IF"
                        | "IFERROR"
                        | "AND"
                        | "OR"
                        | "NOT"
                        | "ROUND"
                        | "ROUNDUP"
                        | "ROUNDDOWN"
                        | "ABS"
                        | "COUNT"
                        | "COUNTA"
                        | "COUNTBLANK"
                        | "SUMIF"
                        | "SUMIFS"
                        | "COUNTIF"
                        | "COUNTIFS"
                        | "MOD"
                        | "INT"
                        | "ROW"
                        | "ROWS"
                        | "COLUMN"
                        | "COLUMNS"
                        | "LEN"
                        | "TRIM"
                        | "LEFT"
                        | "RIGHT"
                        | "MID"
                        | "LOWER"
                        | "UPPER"
                        | "CONCAT"
                        | "CONCATENATE"
                        | "INDEX"
                        | "MATCH"
                        | "VLOOKUP"
                        | "HLOOKUP"
                        | "TRUE"
                        | "FALSE"
                        | "ISBLANK"
                        | "ISNUMBER"
                        | "ISTEXT"
                        | "ISERROR"
                ) {
                    return false;
                }
            } else if !matches!(token.as_str(), "TRUE" | "FALSE") {
                let reference = token.replace('$', "");
                let letters = reference
                    .bytes()
                    .take_while(u8::is_ascii_alphabetic)
                    .count();
                if letters == 0 || letters > 3 {
                    return false;
                }
                let rest = &reference[letters..];
                if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
                    return false;
                }
            }
        } else {
            index += 1;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_rebuilds_require_recognized_deterministic_dependencies() {
        for formula in [
            "B1*2",
            "SUM($A$1:B4)",
            "IF(A1=1,\"NOW()\",0)",
            "'Other sheet'!A1+Data!B2",
        ] {
            assert!(deterministic_formula(formula.as_bytes()), "{formula}");
        }
        for formula in [
            "NOW()",
            "TODAY()",
            "RAND()",
            "INDIRECT(\"A1\")",
            "CELL(\"filename\")",
            "MyUdf(A1)",
            "NamedInput*2",
            "[external.xlsx]Data!A1",
            "Table1[Input]",
        ] {
            assert!(!deterministic_formula(formula.as_bytes()), "{formula}");
        }
    }

    #[test]
    fn package_gate_checks_formula_nodes_names_and_external_parts() -> Result<()> {
        use crate::regression::artifact_cache;
        use std::io::Write;
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "performance-dependencies")?;
        std::fs::create_dir_all(&root)?;
        let path = root.join("input.xlsx");
        for (body, extra, reusable) in [
            ("<f>A1*2</f>", None, true),
            ("<f>NOW()</f>", None, false),
            ("<f t=\"shared\"/>", None, false),
            ("<definedName>Data!A1</definedName>", None, false),
            ("<cfvo type=\"formula\" val=\"NOW()\"/>", None, false),
            ("<calcPr iterate=\"1\"/>", None, false),
            ("<extLst/>", None, false),
            ("<f>A1*2</f>", Some("xl/connections.xml"), false),
        ] {
            let mut archive = zip::ZipWriter::new(File::create(&path)?);
            archive.start_file(
                "xl/worksheets/sheet1.xml",
                zip::write::FileOptions::default(),
            )?;
            archive.write_all(format!("<worksheet>{body}</worksheet>").as_bytes())?;
            if let Some(name) = extra {
                archive.start_file(name, zip::write::FileOptions::default())?;
                archive.write_all(b"<connections/>")?;
            }
            archive.finish()?;
            assert_eq!(reusable_rebuild(&path)?, reusable, "{body}");
        }
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[ignore = "read-only native artifact review; requires CP_REBUILD_REVIEW_FILE"]
    fn native_rebuild_parent_is_reusable() -> Result<()> {
        assert!(reusable_rebuild(Path::new(&std::env::var(
            "CP_REBUILD_REVIEW_FILE"
        )?))?);
        Ok(())
    }
}
