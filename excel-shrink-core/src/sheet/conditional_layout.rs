//! Conditional formatting can make an otherwise empty tail visibly bounded.
//! Preserve row layout through its application area without protecting cell payloads.
use crate::sheet::streaming::PreprocessedSheetXml;
use crate::xml::attributes::Attributes;
use memchr::{memchr, memmem};

const LAST_ROW: u32 = 1_048_576;

/// Inspect metadata only: worksheet cell bytes are not scanned a second time.
pub(super) fn boundary(sheet: &PreprocessedSheetXml) -> Option<u32> {
    [
        &sheet.until_worksheet_start_output_chunks,
        &sheet.before_cols_node_output_chunks,
        &sheet.in_cols_node_output_chunks,
        &sheet.before_sheet_data_node_output_chunks,
        &sheet.before_merge_cells_node_output_chunks,
        &sheet.in_merge_cells_node_output_chunks,
        &sheet.until_worksheet_end_output_chunks,
    ]
    .into_iter()
    .filter_map(|chunk| metadata_boundary(chunk))
    .max()
}

/// Follow ordinary sqref attributes and x14's nested sqref text. Unknown scopes
/// retain existing layout to the sheet end; they never create new rows or cells.
fn metadata_boundary(bytes: &[u8]) -> Option<u32> {
    let mut offset = 0;
    let mut maximum = None;
    let mut conditional = false;
    let mut has_area = false;
    while let Some(relative) = memchr(b'<', &bytes[offset..]) {
        let start = offset + relative;
        let special_end = if bytes[start..].starts_with(b"<!--") {
            Some(b"-->".as_slice())
        } else if bytes[start..].starts_with(b"<![CDATA[") {
            Some(b"]]>".as_slice())
        } else {
            None
        };
        if let Some(marker) = special_end {
            let Some(end) = memmem::find(&bytes[start..], marker) else {
                return Some(LAST_ROW);
            };
            offset = start + end + marker.len();
            continue;
        }
        let Some(end) = tag_end(bytes, start + 1) else {
            return conditional.then_some(LAST_ROW).or(maximum);
        };
        let closing = bytes.get(start + 1) == Some(&b'/');
        let name_start = start + if closing { 2 } else { 1 };
        let name_end = (name_start..end)
            .find(|&index| bytes[index].is_ascii_whitespace() || bytes[index] == b'/')
            .unwrap_or(end);
        let name = bytes[name_start..name_end]
            .rsplit(|&byte| byte == b':')
            .next()?;
        if name == b"conditionalFormatting" {
            if closing {
                if !has_area {
                    maximum = Some(LAST_ROW);
                }
                conditional = false;
            } else {
                if conditional {
                    return Some(LAST_ROW);
                }
                conditional = true;
                has_area = false;
                let attributes = bytes[name_end..end].trim_ascii_end();
                let attributes = attributes.strip_suffix(b"/").unwrap_or(attributes);
                for attribute in Attributes::new(attributes) {
                    let Ok(attribute) = attribute else {
                        return Some(LAST_ROW);
                    };
                    if attribute.name == b"sqref" {
                        maximum = Some(maximum.unwrap_or(0).max(area_last_row(attribute.value)));
                        has_area = true;
                    }
                }
                if bytes[end - 1] == b'/' {
                    conditional = false;
                    if !has_area {
                        maximum = Some(LAST_ROW);
                    }
                }
            }
        } else if conditional && name == b"sqref" && !closing {
            let Some(text_length) = memchr(b'<', &bytes[end + 1..]) else {
                return Some(LAST_ROW);
            };
            let text_end = end + 1 + text_length;
            maximum = Some(
                maximum
                    .unwrap_or(0)
                    .max(area_last_row(&bytes[end + 1..text_end])),
            );
            has_area = true;
        }
        offset = end + 1;
    }
    if conditional && !has_area {
        Some(LAST_ROW)
    } else {
        maximum
    }
}

fn tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut quote = None;
    for (index, &byte) in bytes.iter().enumerate().skip(start) {
        match (quote, byte) {
            (Some(expected), found) if expected == found => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'>') => return Some(index),
            _ => {}
        }
    }
    None
}

fn area_last_row(area: &[u8]) -> u32 {
    let mut maximum = 0;
    for coordinate in area
        .split(|byte| byte.is_ascii_whitespace() || *byte == b':')
        .filter(|part| !part.is_empty())
    {
        let digits = coordinate.iter().position(u8::is_ascii_digit);
        let Some(start) = digits else { return LAST_ROW };
        if !coordinate[..start]
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || *byte == b'$')
        {
            return LAST_ROW;
        }
        let Some(row) = crate::utils::parse_unsigned_bytes::<u32>(&coordinate[start..]) else {
            return LAST_ROW;
        };
        if row == 0 || row > LAST_ROW {
            return LAST_ROW;
        }
        maximum = maximum.max(row);
    }
    if maximum == 0 {
        LAST_ROW
    } else {
        maximum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_scopes_cover_absolute_ranges_extensions_and_unknowns() {
        assert_eq!(metadata_boundary(br#"<conditionalFormatting sqref='$A$5:$C$44 J2:J20'><cfRule/></conditionalFormatting>"#), Some(44));
        assert_eq!(metadata_boundary(br#"<x14:conditionalFormatting><x14:cfRule/><xm:sqref>A2:B90</xm:sqref></x14:conditionalFormatting>"#), Some(90));
        assert_eq!(metadata_boundary(br#"<!-- <conditionalFormatting sqref='A1:A999'/> --><dataValidation sqref='A1:A500'/>"#), None);
        assert_eq!(
            metadata_boundary(br#"<conditionalFormatting sqref='A:A'/>"#),
            Some(LAST_ROW)
        );
        assert_eq!(
            metadata_boundary(
                br#"<conditionalFormatting sqref='unknown'><cfRule/></conditionalFormatting>"#
            ),
            Some(LAST_ROW)
        );
    }
}
