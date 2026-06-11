use crate::error::Result;
use crate::models::Xf;
use regex::bytes::Regex;
use std::sync::OnceLock;

static XF_TAG_PATTERN: OnceLock<Regex> = OnceLock::new();
static CELL_XFS_BLOCK_PATTERN: OnceLock<Regex> = OnceLock::new();
static FILLS_BLOCK_PATTERN: OnceLock<Regex> = OnceLock::new();
static FILL_TAG_PATTERN: OnceLock<Regex> = OnceLock::new();
static BORDERS_BLOCK_PATTERN: OnceLock<Regex> = OnceLock::new();
static BORDER_TAG_PATTERN: OnceLock<Regex> = OnceLock::new();
static FILL_ID_PATTERN: OnceLock<Regex> = OnceLock::new();
static BORDER_ID_PATTERN: OnceLock<Regex> = OnceLock::new();
static APPLY_BORDER_PATTERN: OnceLock<Regex> = OnceLock::new();
static APPLY_FILL_PATTERN: OnceLock<Regex> = OnceLock::new();
static APPLY_PROTECTION_PATTERN: OnceLock<Regex> = OnceLock::new();

pub fn parse_xfs(styles_xml_or_cell_xfs: &[u8]) -> Result<Vec<Xf>> {
    let xf_pattern = XF_TAG_PATTERN.get_or_init(|| Regex::new(r"<xf[^>]*/>|<xf[^>]*>").unwrap());
    let cell_xfs_block_pattern = CELL_XFS_BLOCK_PATTERN
        .get_or_init(|| Regex::new(r"(?s)<cellXfs\b[^>]*>.*?</cellXfs>").unwrap());
    let fill_id_pattern =
        FILL_ID_PATTERN.get_or_init(|| Regex::new(r#"fillId="([^"]+)""#).unwrap());
    let border_id_pattern =
        BORDER_ID_PATTERN.get_or_init(|| Regex::new(r#"borderId="([^"]+)""#).unwrap());
    let apply_border_pattern =
        APPLY_BORDER_PATTERN.get_or_init(|| Regex::new(r#"applyBorder="([^"]+)""#).unwrap());
    let apply_fill_pattern =
        APPLY_FILL_PATTERN.get_or_init(|| Regex::new(r#"applyFill="([^"]+)""#).unwrap());
    let apply_prot_pattern = APPLY_PROTECTION_PATTERN
        .get_or_init(|| Regex::new(r#"applyProtection="([^"]+)""#).unwrap());

    let visible_fills = parse_visible_fills(styles_xml_or_cell_xfs);
    let visible_borders = parse_visible_borders(styles_xml_or_cell_xfs);
    let has_style_tables = !visible_fills.is_empty() || !visible_borders.is_empty();
    let cell_styles = cell_xfs_block_pattern
        .find(styles_xml_or_cell_xfs)
        .map_or(styles_xml_or_cell_xfs, |m| m.as_bytes());

    let mut xfs = Vec::new();

    for mat in xf_pattern.find_iter(cell_styles) {
        let chunk = mat.as_bytes();

        let fill_id = extract_u32(chunk, fill_id_pattern);
        let border_id = extract_u32(chunk, border_id_pattern);
        let apply_border = extract_bool(chunk, apply_border_pattern);
        let apply_fill = extract_bool(chunk, apply_fill_pattern);
        let apply_protection = extract_bool(chunk, apply_prot_pattern);

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
        let has_only_font_style =
            !has_fill_or_border && !has_apply_fill_or_border && !apply_protection;

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
    }

    Ok(xfs)
}

fn parse_visible_fills(styles_xml: &[u8]) -> Vec<bool> {
    let fills_block_pattern = FILLS_BLOCK_PATTERN
        .get_or_init(|| Regex::new(r"(?s)<fills\b[^>]*>.*?</fills>").unwrap());
    let fill_tag_pattern =
        FILL_TAG_PATTERN.get_or_init(|| Regex::new(r"(?s)<fill\b[^>]*>.*?</fill>|<fill\b[^>]*/>").unwrap());

    let Some(fills_block) = fills_block_pattern.find(styles_xml) else {
        return Vec::new();
    };

    fill_tag_pattern
        .find_iter(fills_block.as_bytes())
        .map(|m| fill_has_visible_pattern(m.as_bytes()))
        .collect()
}

fn fill_has_visible_pattern(fill: &[u8]) -> bool {
    let Some(pattern_pos) = crate::xml::find_bytes(fill, b"<patternFill") else {
        return false;
    };

    let Some(tag_end_rel) = fill[pattern_pos..].iter().position(|b| *b == b'>') else {
        return false;
    };

    let pattern_tag = &fill[pattern_pos..pattern_pos + tag_end_rel + 1];
    extract_attr_value(pattern_tag, b"patternType").map_or(true, |pattern_type| {
        pattern_type != b"none"
    })
}

fn parse_visible_borders(styles_xml: &[u8]) -> Vec<bool> {
    let borders_block_pattern = BORDERS_BLOCK_PATTERN
        .get_or_init(|| Regex::new(r"(?s)<borders\b[^>]*>.*?</borders>").unwrap());
    let border_tag_pattern = BORDER_TAG_PATTERN
        .get_or_init(|| Regex::new(r"(?s)<border\b[^>]*>.*?</border>|<border\b[^>]*/>").unwrap());

    let Some(borders_block) = borders_block_pattern.find(styles_xml) else {
        return Vec::new();
    };

    border_tag_pattern
        .find_iter(borders_block.as_bytes())
        .map(|m| border_has_visible_side(m.as_bytes()))
        .collect()
}

fn border_has_visible_side(border: &[u8]) -> bool {
    [b"left".as_slice(), b"right", b"top", b"bottom", b"diagonal"]
        .iter()
        .any(|side| border_side_has_style(border, side))
}

fn border_side_has_style(border: &[u8], side: &[u8]) -> bool {
    let mut needle = Vec::with_capacity(side.len() + 1);
    needle.push(b'<');
    needle.extend_from_slice(side);

    let mut search_start = 0;
    while search_start < border.len() {
        let Some(pos_rel) = crate::xml::find_bytes(&border[search_start..], &needle) else {
            return false;
        };
        let pos = search_start + pos_rel;
        let after_name = pos + needle.len();

        if border
            .get(after_name)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
        {
            let tag_end = border[after_name..]
                .iter()
                .position(|b| *b == b'>')
                .map(|end_rel| after_name + end_rel + 1)
                .unwrap_or(border.len());

            return extract_attr_value(&border[pos..tag_end], b"style").is_some();
        }

        search_start = after_name;
    }

    false
}

fn extract_attr_value<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let mut needle = Vec::with_capacity(attr_name.len() + 2);
    needle.extend_from_slice(attr_name);
    needle.extend_from_slice(b"=\"");

    let pos = crate::xml::find_bytes(data, &needle)?;
    let value_start = pos + needle.len();
    let value_end_rel = data[value_start..].iter().position(|b| *b == b'"')?;

    Some(&data[value_start..value_start + value_end_rel])
}

fn extract_u32(data: &[u8], pattern: &Regex) -> Option<u32> {
    pattern.captures(data).and_then(|c| {
        std::str::from_utf8(c.get(1)?.as_bytes())
            .ok()
            .and_then(|s| s.parse().ok())
    })
}

fn extract_bool(data: &[u8], pattern: &Regex) -> bool {
    pattern
        .captures(data)
        .and_then(|c| c.get(1))
        .map_or(false, |m| m.as_bytes() == b"1")
}
