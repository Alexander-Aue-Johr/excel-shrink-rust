use crate::error::Result;
use crate::models::Xf;
use crate::xml::{find_byte_from, find_bytes};

pub fn parse_xfs(styles_xml_or_cell_xfs: &[u8]) -> Result<Vec<Xf>> {
    let visible_fills = parse_visible_fills(styles_xml_or_cell_xfs);
    let visible_borders = parse_visible_borders(styles_xml_or_cell_xfs);
    let has_style_tables = !visible_fills.is_empty() || !visible_borders.is_empty();
    let cell_styles = find_xml_block(styles_xml_or_cell_xfs, b"cellXfs")
        .map_or(styles_xml_or_cell_xfs, |block| block);

    let mut xfs = Vec::new();
    let mut pos = 0;

    while let Some(tag) = find_next_tag(cell_styles, b"xf", pos) {
        let chunk = tag.raw;

        let fill_id = extract_u32(chunk, b"fillId");
        let border_id = extract_u32(chunk, b"borderId");
        let font_id = extract_u32(chunk, b"fontId");
        let apply_border = extract_bool(chunk, b"applyBorder");
        let apply_fill = extract_bool(chunk, b"applyFill");
        let apply_font = extract_bool(chunk, b"applyFont");
        let apply_protection = extract_bool(chunk, b"applyProtection");

        let has_visible_fill = if has_style_tables {
            fill_id.is_some_and(|id| visible_fills.get(id as usize).copied().unwrap_or(false))
        } else {
            fill_id.unwrap_or(0) != 0
        };

        let has_visible_border = if has_style_tables {
            border_id.is_some_and(|id| visible_borders.get(id as usize).copied().unwrap_or(false))
        } else {
            border_id.unwrap_or(0) != 0
        };

        let has_apply_fill_or_border = apply_fill || apply_border;
        let has_fill_or_border = has_visible_fill || has_visible_border;
        let has_font = font_id.unwrap_or(0) != 0 || apply_font;
        let has_only_font_style = has_font && !has_fill_or_border && !apply_protection;

        xfs.push(Xf::new_with_visible_parts(
            fill_id,
            border_id,
            apply_border,
            apply_fill,
            has_visible_fill,
            has_visible_border,
            has_apply_fill_or_border,
            has_only_font_style,
            apply_protection,
        ));

        pos = tag.end;
    }

    Ok(xfs)
}

fn parse_visible_fills(styles_xml: &[u8]) -> Vec<bool> {
    let Some(fills_block) = find_xml_block(styles_xml, b"fills") else {
        return Vec::new();
    };

    let mut fills = Vec::new();
    let mut pos = 0;

    while let Some(tag) = find_next_element(fills_block, b"fill", pos) {
        fills.push(fill_has_visible_pattern(tag.raw));
        pos = tag.end;
    }

    fills
}

fn fill_has_visible_pattern(fill: &[u8]) -> bool {
    let Some(pattern_tag) = find_next_tag(fill, b"patternFill", 0) else {
        return false;
    };

    extract_attr_value(pattern_tag.raw, b"patternType")
        .map_or(true, |pattern_type| pattern_type != b"none")
}

fn parse_visible_borders(styles_xml: &[u8]) -> Vec<bool> {
    let Some(borders_block) = find_xml_block(styles_xml, b"borders") else {
        return Vec::new();
    };

    let mut borders = Vec::new();
    let mut pos = 0;

    while let Some(tag) = find_next_element(borders_block, b"border", pos) {
        borders.push(border_has_visible_side(tag.raw));
        pos = tag.end;
    }

    borders
}

fn border_has_visible_side(border: &[u8]) -> bool {
    [b"left".as_slice(), b"right", b"top", b"bottom", b"diagonal"]
        .iter()
        .any(|side| border_side_has_style(border, side))
}

fn border_side_has_style(border: &[u8], side: &[u8]) -> bool {
    let mut pos = 0;

    while let Some(tag) = find_next_tag(border, side, pos) {
        if extract_attr_value(tag.raw, b"style").is_some() {
            return true;
        }

        pos = tag.end;
    }

    false
}

#[derive(Debug, Clone, Copy)]
struct XmlTag<'a> {
    raw: &'a [u8],
    start: usize,
    open_end: usize,
    end: usize,
    self_closing: bool,
}

fn find_xml_block<'a>(data: &'a [u8], tag_name: &[u8]) -> Option<&'a [u8]> {
    let tag = find_next_tag(data, tag_name, 0)?;

    if tag.self_closing {
        return Some(tag.raw);
    }

    let closing = closing_tag(tag_name);
    let close_start_rel = find_bytes(&data[tag.open_end..], &closing)?;
    let close_start = tag.open_end + close_start_rel;
    let close_end = close_start + closing.len();

    Some(&data[tag.start..close_end])
}

fn find_next_element<'a>(data: &'a [u8], tag_name: &[u8], start: usize) -> Option<XmlTag<'a>> {
    let tag = find_next_tag(data, tag_name, start)?;

    if tag.self_closing {
        return Some(tag);
    }

    let closing = closing_tag(tag_name);
    let close_start_rel = find_bytes(&data[tag.open_end..], &closing)?;
    let end = tag.open_end + close_start_rel + closing.len();

    Some(XmlTag {
        raw: &data[tag.start..end],
        start: tag.start,
        open_end: tag.open_end,
        end,
        self_closing: false,
    })
}

fn find_next_tag<'a>(data: &'a [u8], tag_name: &[u8], start: usize) -> Option<XmlTag<'a>> {
    let mut needle = Vec::with_capacity(tag_name.len() + 1);
    needle.push(b'<');
    needle.extend_from_slice(tag_name);

    let mut pos = start;

    while pos < data.len() {
        let tag_start_rel = find_bytes(&data[pos..], &needle)?;
        let tag_start = pos + tag_start_rel;
        let after_name = tag_start + needle.len();

        if !data
            .get(after_name)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
        {
            pos = after_name;
            continue;
        }

        let tag_end = find_byte_from(data, b'>', tag_start)? + 1;
        let self_closing = tag_end >= 2 && data[tag_end - 2] == b'/';

        return Some(XmlTag {
            raw: &data[tag_start..tag_end],
            start: tag_start,
            open_end: tag_end,
            end: tag_end,
            self_closing,
        });
    }

    None
}

fn closing_tag(tag_name: &[u8]) -> Vec<u8> {
    let mut closing = Vec::with_capacity(tag_name.len() + 3);
    closing.extend_from_slice(b"</");
    closing.extend_from_slice(tag_name);
    closing.push(b'>');
    closing
}

fn extract_attr_value<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let mut needle = Vec::with_capacity(attr_name.len() + 2);
    needle.extend_from_slice(attr_name);
    needle.extend_from_slice(b"=\"");

    let mut search_start = 0;

    while search_start < data.len() {
        let pos_rel = find_bytes(&data[search_start..], &needle)?;
        let pos = search_start + pos_rel;

        if pos > 0 && !data[pos - 1].is_ascii_whitespace() {
            search_start = pos + 1;
            continue;
        }

        let value_start = pos + needle.len();
        let value_end_rel = data[value_start..].iter().position(|b| *b == b'"')?;

        return Some(&data[value_start..value_start + value_end_rel]);
    }

    None
}

fn extract_u32(data: &[u8], attr_name: &[u8]) -> Option<u32> {
    extract_attr_value(data, attr_name).and_then(|value| {
        std::str::from_utf8(value)
            .ok()
            .and_then(|value| value.parse().ok())
    })
}

fn extract_bool(data: &[u8], attr_name: &[u8]) -> bool {
    extract_attr_value(data, attr_name).is_some_and(|value| value == b"1")
}
