use crate::error::Result;
use bstr::ByteSlice;
use regex::bytes::{Captures, Regex};
use std::collections::HashSet;
use tracing::debug;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct DefinedName {
    name: Vec<u8>,
    local_sheet_id: Vec<u8>,
}

pub fn clean_workbook(workbook_xml_bytes: &[u8]) -> Result<Vec<u8>> {
    // 1. Suche den Start- und Endbereich des Blocks <definedNames>
    let start_tag = b"<definedNames";
    let end_tag = b"</definedNames>";

    let start_idx = match workbook_xml_bytes.find(start_tag) {
        Some(idx) => idx,
        None => return Ok(workbook_xml_bytes.to_vec()), // Kein <definedNames> Block gefunden
    };

    let tag_close_idx = match workbook_xml_bytes[start_idx..].find(b">") {
        Some(idx) => start_idx + idx + 1,
        None => return Ok(workbook_xml_bytes.to_vec()),
    };

    let end_idx = match workbook_xml_bytes[tag_close_idx..].find(end_tag) {
        Some(idx) => tag_close_idx + idx,
        None => return Ok(workbook_xml_bytes.to_vec()),
    };

    let defined_names_content = &workbook_xml_bytes[tag_close_idx..end_idx];

    // Regex zum Extrahieren aller Namen und IDs
    let search_pattern = Regex::new(
        r#"<definedName\b[^>]*?name="([^"]*?)"(?:[^>]*?localSheetId="([^"]*?)")?[^>]*?>"#
    )?;

    let mut print_area = HashSet::new();
    let mut xlnm_print_area = HashSet::new();
    let mut print_titles = HashSet::new();
    let mut xlnm_print_titles = HashSet::new();
    let mut filter_database = HashSet::new();
    let mut xlnm_filter_database = HashSet::new();
    let mut database = HashSet::new();
    let mut xlnm_database = HashSet::new();

    for caps in search_pattern.captures_iter(defined_names_content) {
        let name = caps.get(1).map(|m| m.as_bytes().to_vec()).unwrap_or_default();
        let sheet_id = caps.get(2).map(|m| m.as_bytes().to_vec()).unwrap_or_default();

        match name.as_slice() {
            b"Print_Area" => { print_area.insert(sheet_id); }
            b"_xlnm.Print_Area" => { xlnm_print_area.insert(sheet_id); }
            b"Print_Titles" => { print_titles.insert(sheet_id); }
            b"_xlnm.Print_Titles" => { xlnm_print_titles.insert(sheet_id); }
            b"_FilterDatabase" => { filter_database.insert(sheet_id); }
            b"_xlnm._FilterDatabase" => { xlnm_filter_database.insert(sheet_id); }
            b"Database" => { database.insert(sheet_id); }
            b"_xlnm.Database" => { xlnm_database.insert(sheet_id); }
            _ => {}
        }
    }

    let mut to_delete = HashSet::new();
    
    for id in print_area {
        if xlnm_print_area.contains(&id) { to_delete.insert(DefinedName { name: b"Print_Area".to_vec(), local_sheet_id: id }); }
    }
    for id in print_titles {
        if xlnm_print_titles.contains(&id) { to_delete.insert(DefinedName { name: b"Print_Titles".to_vec(), local_sheet_id: id }); }
    }
    for id in filter_database {
        if xlnm_filter_database.contains(&id) { to_delete.insert(DefinedName { name: b"_FilterDatabase".to_vec(), local_sheet_id: id }); }
    }
    for id in database {
        if xlnm_database.contains(&id) { to_delete.insert(DefinedName { name: b"Database".to_vec(), local_sheet_id: id }); }
    }

    if to_delete.is_empty() {
        return Ok(workbook_xml_bytes.to_vec());
    }

    // Ersetze nun die fehlerhaften Duplikate. (?s) lässt den Regex auch Zeilenumbrüche matchen.
    let replace_pattern = Regex::new(
        r#"(?s)<definedName\b[^>]*?name="([^"]*?)"(?:[^>]*?localSheetId="([^"]*?)")?[^>]*?>.*?</definedName>"#
    )?;

    let replaced_content = replace_pattern.replace_all(defined_names_content, |caps: &Captures| {
        let name = caps.get(1).map(|m| m.as_bytes().to_vec()).unwrap_or_default();
        let sheet_id = caps.get(2).map(|m| m.as_bytes().to_vec()).unwrap_or_default();

        let def_name = DefinedName { name, local_sheet_id: sheet_id };
        if to_delete.contains(&def_name) {
            debug!("Removing duplicate definedName: {:?}", String::from_utf8_lossy(&def_name.name));
            Vec::new() // Gib leeren Vektor zurück -> Löscht das Element
        } else {
            caps.get(0).unwrap().as_bytes().to_vec() // Unverändert lassen
        }
    });

    let mut output = Vec::with_capacity(workbook_xml_bytes.len());
    output.extend_from_slice(&workbook_xml_bytes[..tag_close_idx]);
    output.extend_from_slice(&replaced_content);
    output.extend_from_slice(&workbook_xml_bytes[end_idx..]);

    Ok(output)
}