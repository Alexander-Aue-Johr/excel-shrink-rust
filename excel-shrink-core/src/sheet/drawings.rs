use crate::models::{CleanSheetParameters, VisibleContentBounds};
use crate::sheet::streaming::PreprocessedSheetXml;
use regex::bytes::Regex;
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::sync::OnceLock;
use zip::ZipArchive;

static DRAWING_TAG_RE: OnceLock<Regex> = OnceLock::new();
static RELATIONSHIP_RE: OnceLock<Regex> = OnceLock::new();
static DRAWING_ROW_RE: OnceLock<Regex> = OnceLock::new();
static DRAWING_COL_RE: OnceLock<Regex> = OnceLock::new();

pub fn find_worksheet_drawing_anchor_bounds(
    preprocessed: &PreprocessedSheetXml,
    params: &CleanSheetParameters,
) -> VisibleContentBounds {
    let drawing_rids = collect_drawing_rids(preprocessed);
    if drawing_rids.is_empty() {
        return VisibleContentBounds::default();
    }

    let Some(mut archive) = open_workbook_archive(&params.xlsx_file_path) else {
        return VisibleContentBounds::default();
    };
    let Some(drawing_targets) =
        drawing_targets_for_worksheet(&mut archive, &drawing_rids, &params.zip_info_filename)
    else {
        return VisibleContentBounds::default();
    };

    drawing_anchor_bounds_from_targets(&mut archive, drawing_targets)
}

fn open_workbook_archive(path: &str) -> Option<ZipArchive<File>> {
    let file = File::open(path).ok()?;
    ZipArchive::new(file).ok()
}

fn drawing_targets_for_worksheet(
    archive: &mut ZipArchive<File>,
    drawing_rids: &HashSet<Vec<u8>>,
    sheet_part: &str,
) -> Option<Vec<String>> {
    let relationships_path = worksheet_relationships_path(sheet_part)?;
    let relationships_xml = read_zip_entry(archive, &relationships_path)?;

    Some(collect_drawing_targets(
        &relationships_xml,
        drawing_rids,
        sheet_part,
    ))
}

fn drawing_anchor_bounds_from_targets(
    archive: &mut ZipArchive<File>,
    drawing_targets: Vec<String>,
) -> VisibleContentBounds {
    let mut bounds = VisibleContentBounds::default();

    for target in drawing_targets {
        let Some(drawing_xml) = read_zip_entry(archive, &target) else {
            continue;
        };

        include_bounds(&mut bounds, drawing_anchor_bounds_from_xml(&drawing_xml));
    }

    bounds
}

#[inline]
fn include_bounds(bounds: &mut VisibleContentBounds, additional: VisibleContentBounds) {
    if let Some(row) = additional.max_row {
        bounds.include_row(row);
    }
    if let Some(column) = additional.max_col {
        bounds.include_col(column);
    }
}

fn collect_drawing_rids(preprocessed: &PreprocessedSheetXml) -> HashSet<Vec<u8>> {
    let drawing_tag_re = DRAWING_TAG_RE.get_or_init(|| {
        Regex::new(r#"(?s)<(?:[A-Za-z_][\w.-]*:)?drawing\b[^>]*\br:id="([^"]+)""#).unwrap()
    });
    let chunks = [
        preprocessed.until_worksheet_start_output_chunks.as_slice(),
        preprocessed.before_cols_node_output_chunks.as_slice(),
        preprocessed.in_cols_node_output_chunks.as_slice(),
        preprocessed.before_sheet_data_node_output_chunks.as_slice(),
        preprocessed
            .before_merge_cells_node_output_chunks
            .as_slice(),
        preprocessed.in_merge_cells_node_output_chunks.as_slice(),
        preprocessed.until_worksheet_end_output_chunks.as_slice(),
    ];
    let mut rids = HashSet::new();

    for chunk in chunks {
        for caps in drawing_tag_re.captures_iter(chunk) {
            if let Some(rid) = caps.get(1) {
                rids.insert(rid.as_bytes().to_vec());
            }
        }
    }

    rids
}

fn collect_drawing_targets(
    rels_xml: &[u8],
    drawing_rids: &HashSet<Vec<u8>>,
    source_part: &str,
) -> Vec<String> {
    let relationship_re = RELATIONSHIP_RE
        .get_or_init(|| Regex::new(r#"(?s)<(?:[A-Za-z_][\w.-]*:)?Relationship\b[^>]*>"#).unwrap());
    let mut targets = Vec::new();

    for relationship in relationship_re.find_iter(rels_xml) {
        if let Some(target) =
            drawing_target_from_relationship(relationship.as_bytes(), drawing_rids, source_part)
        {
            targets.push(target);
        }
    }

    targets
}

fn drawing_target_from_relationship(
    relationship: &[u8],
    drawing_rids: &HashSet<Vec<u8>>,
    source_part: &str,
) -> Option<String> {
    let id = extract_attr_value(relationship, b"Id")?;
    if !drawing_rids.contains(id) {
        return None;
    }

    let relationship_type = extract_attr_value(relationship, b"Type")?;
    if !relationship_type.ends_with(b"/drawing") {
        return None;
    }

    let target = std::str::from_utf8(extract_attr_value(relationship, b"Target")?).ok()?;
    resolve_part_target(source_part, target)
}

fn drawing_anchor_bounds_from_xml(drawing_xml: &[u8]) -> VisibleContentBounds {
    let mut bounds = VisibleContentBounds::default();
    include_drawing_row_anchors(&mut bounds, drawing_xml);
    include_drawing_column_anchors(&mut bounds, drawing_xml);
    bounds
}

fn include_drawing_row_anchors(bounds: &mut VisibleContentBounds, drawing_xml: &[u8]) {
    let row_re = DRAWING_ROW_RE.get_or_init(|| {
        Regex::new(r#"<(?:[A-Za-z_][\w.-]*:)?row>(\d+)</(?:[A-Za-z_][\w.-]*:)?row>"#).unwrap()
    });

    for captures in row_re.captures_iter(drawing_xml) {
        if let Some(row) = captures
            .get(1)
            .and_then(|capture| parse_u32(capture.as_bytes()))
        {
            bounds.include_row(row.saturating_add(1));
        }
    }
}

fn include_drawing_column_anchors(bounds: &mut VisibleContentBounds, drawing_xml: &[u8]) {
    let column_re = DRAWING_COL_RE.get_or_init(|| {
        Regex::new(r#"<(?:[A-Za-z_][\w.-]*:)?col>(\d+)</(?:[A-Za-z_][\w.-]*:)?col>"#).unwrap()
    });

    for captures in column_re.captures_iter(drawing_xml) {
        if let Some(column) = captures
            .get(1)
            .and_then(|capture| parse_u32(capture.as_bytes()))
        {
            bounds.include_col(column.saturating_add(1));
        }
    }
}

fn worksheet_relationships_path(sheet_part: &str) -> Option<String> {
    let slash = sheet_part.rfind('/')?;
    let dir = &sheet_part[..=slash];
    let file_name = &sheet_part[slash + 1..];
    if file_name.is_empty() {
        return None;
    }
    Some(format!("{dir}_rels/{file_name}.rels"))
}

fn resolve_part_target(source_part: &str, target: &str) -> Option<String> {
    if target.is_empty() {
        return None;
    }
    if target.starts_with('/') {
        return Some(target.trim_start_matches('/').to_string());
    }

    resolve_relative_part_target(source_part, target)
}

fn resolve_relative_part_target(source_part: &str, target: &str) -> Option<String> {
    let source_directory_end = source_part.rfind('/')?;
    let mut parts: Vec<&str> = source_part[..source_directory_end]
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();

    for part in target.split('/') {
        apply_target_path_part(&mut parts, part)?;
    }

    Some(parts.join("/"))
}

#[inline]
fn apply_target_path_part<'a>(parts: &mut Vec<&'a str>, part: &'a str) -> Option<()> {
    match part {
        "" | "." => Some(()),
        ".." => parts.pop().map(|_| ()),
        part => {
            parts.push(part);
            Some(())
        }
    }
}

fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Option<Vec<u8>> {
    let Ok(mut file) = archive.by_name(name) else {
        return None;
    };
    let mut data = Vec::new();
    file.read_to_end(&mut data).ok()?;
    Some(data)
}

fn extract_attr_value<'a>(element: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let mut search_start = 0;

    while search_start + attr_name.len() + 2 <= element.len() {
        let relative = crate::xml::find_bytes(&element[search_start..], attr_name)?;
        let attribute_start = search_start + relative;
        let after_name = attribute_start + attr_name.len();

        if quoted_attribute_starts_at(element, attribute_start, after_name) {
            return quoted_attribute_value(element, after_name);
        }

        search_start = after_name;
    }

    None
}

#[inline]
fn quoted_attribute_starts_at(element: &[u8], start: usize, after_name: usize) -> bool {
    attribute_has_left_boundary(element, start)
        && element.get(after_name) == Some(&b'=')
        && element.get(after_name + 1) == Some(&b'"')
}

#[inline]
fn attribute_has_left_boundary(element: &[u8], start: usize) -> bool {
    start == 0 || element[start - 1].is_ascii_whitespace() || element[start - 1] == b'<'
}

fn quoted_attribute_value(element: &[u8], after_name: usize) -> Option<&[u8]> {
    let value_start = after_name + 2;
    let value_end = value_start
        + element[value_start..]
            .iter()
            .position(|byte| *byte == b'"')?;

    Some(&element[value_start..value_end])
}

#[inline]
fn parse_u32(bytes: &[u8]) -> Option<u32> {
    let mut value = 0u32;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_rows_and_columns_are_one_based_for_sheet_bounds() {
        let xml = br#"<xdr:wsDr><xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:row>109</xdr:row></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:row>110</xdr:row></xdr:to></xdr:twoCellAnchor></xdr:wsDr>"#;

        let bounds = drawing_anchor_bounds_from_xml(xml);

        assert_eq!(bounds.max_row, Some(111));
        assert_eq!(bounds.max_col, Some(4));
    }

    #[test]
    fn resolves_drawing_target_relative_to_sheet_part() {
        assert_eq!(
            resolve_part_target("xl/worksheets/sheet18.xml", "../drawings/drawing18.xml"),
            Some("xl/drawings/drawing18.xml".to_string())
        );
    }
}
