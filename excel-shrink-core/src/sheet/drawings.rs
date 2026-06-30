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

    let Some(rels_path) = worksheet_relationships_path(&params.zip_info_filename) else {
        return VisibleContentBounds::default();
    };

    let Ok(file) = File::open(&params.xlsx_file_path) else {
        return VisibleContentBounds::default();
    };
    let Ok(mut archive) = ZipArchive::new(file) else {
        return VisibleContentBounds::default();
    };

    let Some(rels_xml) = read_zip_entry(&mut archive, &rels_path) else {
        return VisibleContentBounds::default();
    };

    let drawing_targets =
        collect_drawing_targets(&rels_xml, &drawing_rids, &params.zip_info_filename);
    let mut bounds = VisibleContentBounds::default();

    for target in drawing_targets {
        let Some(drawing_xml) = read_zip_entry(&mut archive, &target) else {
            continue;
        };

        let drawing_bounds = drawing_anchor_bounds_from_xml(&drawing_xml);
        if let Some(row) = drawing_bounds.max_row {
            bounds.include_row(row);
        }
        if let Some(col) = drawing_bounds.max_col {
            bounds.include_col(col);
        }
    }

    bounds
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

    for rel_match in relationship_re.find_iter(rels_xml) {
        let rel = rel_match.as_bytes();
        let Some(id) = extract_attr_value(rel, b"Id") else {
            continue;
        };
        if !drawing_rids.contains(id) {
            continue;
        }

        let Some(rel_type) = extract_attr_value(rel, b"Type") else {
            continue;
        };
        if !rel_type.ends_with(b"/drawing") {
            continue;
        }

        let Some(target) = extract_attr_value(rel, b"Target") else {
            continue;
        };
        let Ok(target) = std::str::from_utf8(target) else {
            continue;
        };
        let Some(resolved) = resolve_part_target(source_part, target) else {
            continue;
        };

        targets.push(resolved);
    }

    targets
}

fn drawing_anchor_bounds_from_xml(drawing_xml: &[u8]) -> VisibleContentBounds {
    let row_re = DRAWING_ROW_RE.get_or_init(|| {
        Regex::new(r#"<(?:[A-Za-z_][\w.-]*:)?row>(\d+)</(?:[A-Za-z_][\w.-]*:)?row>"#).unwrap()
    });
    let col_re = DRAWING_COL_RE.get_or_init(|| {
        Regex::new(r#"<(?:[A-Za-z_][\w.-]*:)?col>(\d+)</(?:[A-Za-z_][\w.-]*:)?col>"#).unwrap()
    });
    let mut bounds = VisibleContentBounds::default();

    for caps in row_re.captures_iter(drawing_xml) {
        if let Some(row) = caps.get(1).and_then(|m| parse_u32(m.as_bytes())) {
            bounds.include_row(row.saturating_add(1));
        }
    }

    for caps in col_re.captures_iter(drawing_xml) {
        if let Some(col) = caps.get(1).and_then(|m| parse_u32(m.as_bytes())) {
            bounds.include_col(col.saturating_add(1));
        }
    }

    bounds
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

    let slash = source_part.rfind('/')?;
    let source_dir = &source_part[..slash];
    let mut parts: Vec<&str> = source_dir
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();

    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }

    Some(parts.join("/"))
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
    let mut pos = 0;
    while pos + attr_name.len() + 2 <= element.len() {
        let rel = crate::xml::find_bytes(&element[pos..], attr_name)?;
        let start = pos + rel;
        let before_ok =
            start == 0 || element[start - 1].is_ascii_whitespace() || element[start - 1] == b'<';
        let after_name = start + attr_name.len();
        if before_ok && after_name < element.len() && element[after_name] == b'=' {
            let quote_pos = after_name + 1;
            if quote_pos < element.len() && element[quote_pos] == b'"' {
                let value_start = quote_pos + 1;
                let value_end =
                    value_start + element[value_start..].iter().position(|b| *b == b'"')?;
                return Some(&element[value_start..value_end]);
            }
        }
        pos = after_name;
    }

    None
}

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
