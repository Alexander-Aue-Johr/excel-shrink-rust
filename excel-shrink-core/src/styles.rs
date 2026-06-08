use crate::error::Result;
use crate::models::Xf;
use regex::bytes::Regex;
use std::sync::OnceLock;

static XF_TAG_PATTERN: OnceLock<Regex> = OnceLock::new();
static FILL_ID_PATTERN: OnceLock<Regex> = OnceLock::new();
static BORDER_ID_PATTERN: OnceLock<Regex> = OnceLock::new();
static APPLY_BORDER_PATTERN: OnceLock<Regex> = OnceLock::new();
static APPLY_FILL_PATTERN: OnceLock<Regex> = OnceLock::new();
static APPLY_PROTECTION_PATTERN: OnceLock<Regex> = OnceLock::new();

pub fn parse_xfs(cell_styles: &[u8]) -> Result<Vec<Xf>> {
    let xf_pattern = XF_TAG_PATTERN.get_or_init(|| Regex::new(r"<xf[^>]*/>|<xf[^>]*>").unwrap());
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

    let mut xfs = Vec::new();

    for mat in xf_pattern.find_iter(cell_styles) {
        let chunk = mat.as_bytes();

        let fill_id = extract_u32(chunk, fill_id_pattern);
        let border_id = extract_u32(chunk, border_id_pattern);
        let apply_border = extract_bool(chunk, apply_border_pattern);
        let apply_fill = extract_bool(chunk, apply_fill_pattern);
        let apply_protection = extract_bool(chunk, apply_prot_pattern);

        let has_fill_or_border = fill_id.unwrap_or(0) != 0 || border_id.unwrap_or(0) != 0;
        let has_apply_fill_or_border = apply_fill || apply_border;
        let has_only_font_style =
            !has_fill_or_border && !has_apply_fill_or_border && !apply_protection;

        xfs.push(Xf::new(
            fill_id,
            border_id,
            apply_border,
            apply_fill,
            has_fill_or_border,
            has_apply_fill_or_border,
            has_only_font_style,
            apply_protection,
        ));
    }

    Ok(xfs)
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
