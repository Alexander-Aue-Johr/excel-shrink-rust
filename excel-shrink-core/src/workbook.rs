use crate::error::Result;
use crate::models::{CleanupCause, CleanupStats};
use bstr::ByteSlice;
use regex::bytes::{Captures, Regex};
use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Range;
use tracing::debug;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct DefinedName {
    name: Vec<u8>,
    local_sheet_id: Vec<u8>,
}

#[derive(Default)]
struct KnownDefinedNames {
    print_area: HashSet<Vec<u8>>,
    xlnm_print_area: HashSet<Vec<u8>>,
    print_titles: HashSet<Vec<u8>>,
    xlnm_print_titles: HashSet<Vec<u8>>,
    filter_database: HashSet<Vec<u8>>,
    xlnm_filter_database: HashSet<Vec<u8>>,
    database: HashSet<Vec<u8>>,
    xlnm_database: HashSet<Vec<u8>>,
}

impl KnownDefinedNames {
    fn record(&mut self, name: &[u8], sheet_id: Vec<u8>) {
        match name {
            b"Print_Area" => self.print_area.insert(sheet_id),
            b"_xlnm.Print_Area" => self.xlnm_print_area.insert(sheet_id),
            b"Print_Titles" => self.print_titles.insert(sheet_id),
            b"_xlnm.Print_Titles" => self.xlnm_print_titles.insert(sheet_id),
            b"_FilterDatabase" => self.filter_database.insert(sheet_id),
            b"_xlnm._FilterDatabase" => self.xlnm_filter_database.insert(sheet_id),
            b"Database" => self.database.insert(sheet_id),
            b"_xlnm.Database" => self.xlnm_database.insert(sheet_id),
            _ => false,
        };
    }

    fn duplicates(self) -> HashSet<DefinedName> {
        let mut duplicates = HashSet::new();
        mark_duplicates(
            self.print_area,
            &self.xlnm_print_area,
            b"Print_Area",
            &mut duplicates,
        );
        mark_duplicates(
            self.print_titles,
            &self.xlnm_print_titles,
            b"Print_Titles",
            &mut duplicates,
        );
        mark_duplicates(
            self.filter_database,
            &self.xlnm_filter_database,
            b"_FilterDatabase",
            &mut duplicates,
        );
        mark_duplicates(
            self.database,
            &self.xlnm_database,
            b"Database",
            &mut duplicates,
        );
        duplicates
    }
}

pub fn clean_workbook(workbook_xml_bytes: &[u8]) -> Result<Vec<u8>> {
    let mut stats = CleanupStats::default();
    clean_workbook_with_stats(workbook_xml_bytes, &mut stats)
}

pub fn clean_workbook_with_stats(
    workbook_xml_bytes: &[u8],
    stats: &mut CleanupStats,
) -> Result<Vec<u8>> {
    let Some(content_range) = find_defined_names_content(workbook_xml_bytes) else {
        return Ok(workbook_xml_bytes.to_vec());
    };
    let content = &workbook_xml_bytes[content_range.clone()];
    let duplicates = collect_defined_names(content)?.duplicates();

    if duplicates.is_empty() {
        return Ok(workbook_xml_bytes.to_vec());
    }

    let cleaned_content = remove_duplicate_defined_names(content, &duplicates, stats)?;
    Ok(replace_defined_names_content(
        workbook_xml_bytes,
        content_range,
        &cleaned_content,
    ))
}

fn find_defined_names_content(workbook_xml: &[u8]) -> Option<Range<usize>> {
    let block_start = workbook_xml.find(b"<definedNames")?;
    let content_start = block_start + workbook_xml[block_start..].find(b">")? + 1;
    let content_end = content_start + workbook_xml[content_start..].find(b"</definedNames>")?;
    Some(content_start..content_end)
}

fn collect_defined_names(content: &[u8]) -> Result<KnownDefinedNames> {
    let pattern = Regex::new(r"(?s)<definedName\b([^>]*[^>/])>")?;
    let mut known = KnownDefinedNames::default();

    for captures in pattern.captures_iter(content) {
        if let Some(defined_name) = defined_name_from_captures(&captures) {
            known.record(&defined_name.name, defined_name.local_sheet_id);
        }
    }

    Ok(known)
}

fn mark_duplicates(
    unprefixed_ids: HashSet<Vec<u8>>,
    prefixed_ids: &HashSet<Vec<u8>>,
    unprefixed_name: &[u8],
    duplicates: &mut HashSet<DefinedName>,
) {
    duplicates.extend(
        unprefixed_ids
            .into_iter()
            .filter(|sheet_id| prefixed_ids.contains(sheet_id))
            .map(|local_sheet_id| DefinedName {
                name: unprefixed_name.to_vec(),
                local_sheet_id,
            }),
    );
}

fn remove_duplicate_defined_names<'a>(
    content: &'a [u8],
    duplicates: &HashSet<DefinedName>,
    stats: &mut CleanupStats,
) -> Result<Cow<'a, [u8]>> {
    let pattern = Regex::new(r"(?s)<definedName\b([^>]*[^>/])>.*?</definedName>")?;
    let cleaned = pattern.replace_all(content, |captures: &Captures| {
        replace_defined_name_match(captures, duplicates, stats)
    });
    Ok(cleaned)
}

fn replace_defined_name_match(
    captures: &Captures<'_>,
    duplicates: &HashSet<DefinedName>,
    stats: &mut CleanupStats,
) -> Vec<u8> {
    let complete_match = captures.get(0).unwrap().as_bytes();
    let Some(defined_name) = defined_name_from_captures(captures) else {
        return complete_match.to_vec();
    };

    if !duplicates.contains(&defined_name) {
        return complete_match.to_vec();
    }

    debug!(
        "Removing duplicate definedName: {:?}",
        String::from_utf8_lossy(&defined_name.name)
    );
    stats.record_removed(
        CleanupCause::WorkbookDefinedNameCleanup,
        complete_match.len(),
    );
    Vec::new()
}

/// XML attribute order and quote choice must not change a defined name's scope.
fn defined_name_from_captures(captures: &Captures<'_>) -> Option<DefinedName> {
    let mut name = None;
    let mut local_sheet_id = Vec::new();
    let mut scope_seen = false;
    for attribute in crate::xml::attributes::Attributes::new(captures.get(1)?.as_bytes()) {
        let attribute = attribute.ok()?;
        match attribute.name {
            b"name" if name.is_none() => name = Some(attribute.value.to_vec()),
            b"localSheetId" if !scope_seen => {
                local_sheet_id = attribute.value.to_vec();
                scope_seen = true;
            }
            b"name" | b"localSheetId" => return None,
            _ => {}
        }
    }
    Some(DefinedName {
        name: name?,
        local_sheet_id,
    })
}

fn replace_defined_names_content(
    workbook_xml: &[u8],
    content_range: Range<usize>,
    cleaned_content: &[u8],
) -> Vec<u8> {
    let mut output = Vec::with_capacity(workbook_xml.len());
    output.extend_from_slice(&workbook_xml[..content_range.start]);
    output.extend_from_slice(cleaned_content);
    output.extend_from_slice(&workbook_xml[content_range.end..]);
    output
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn aliases_on_different_sheets_are_not_duplicates_when_scope_precedes_name() -> Result<()> {
        let xml = br#"<workbook><definedNames><definedName localSheetId="1" name="Print_Area">Second!$A$1</definedName><definedName localSheetId="0" name="_xlnm.Print_Area">First!$A$1</definedName></definedNames></workbook>"#;
        assert_eq!(clean_workbook(xml)?, xml);
        Ok(())
    }

    #[test]
    fn equivalent_alias_scope_is_independent_of_attribute_order_and_quotes() -> Result<()> {
        let xml = br#"<workbook><definedNames><definedName localSheetId='0' name='Print_Area'>First!$A$1</definedName><definedName name="_xlnm.Print_Area" localSheetId="0">First!$A$1</definedName></definedNames></workbook>"#;
        let expected = br#"<workbook><definedNames><definedName name="_xlnm.Print_Area" localSheetId="0">First!$A$1</definedName></definedNames></workbook>"#;
        assert_eq!(clean_workbook(xml)?, expected);
        Ok(())
    }

    #[test]
    fn a_self_closing_name_does_not_consume_a_following_element() -> Result<()> {
        let xml = br#"<workbook><definedNames><definedName name="Print_Area" localSheetId="0"/><definedName name="Other">A1</definedName><definedName name="_xlnm.Print_Area" localSheetId="0">First!$A$1</definedName></definedNames></workbook>"#;
        assert_eq!(clean_workbook(xml)?, xml);
        Ok(())
    }
}
