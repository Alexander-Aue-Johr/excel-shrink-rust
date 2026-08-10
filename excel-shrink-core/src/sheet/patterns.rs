use crate::models::{WorkbookSpecificPatterns, Xf};

pub fn generate_workbook_specific_patterns(
    xfs: Vec<Xf>,
    whitespace_indices: Option<Vec<usize>>,
) -> WorkbookSpecificPatterns {
    WorkbookSpecificPatterns::new(xfs, whitespace_indices)
}

#[inline]
pub fn style_id_has_no_empty_cell_visible_style(
    patterns: &WorkbookSpecificPatterns,
    style_id: u32,
) -> bool {
    patterns
        .xfs_by_index
        .get(&style_id)
        .is_some_and(style_has_no_empty_cell_visible_style)
}

#[inline]
pub fn style_id_has_fill_border_or_protection(
    patterns: &WorkbookSpecificPatterns,
    style_id: u32,
) -> bool {
    patterns
        .xfs_by_index
        .get(&style_id)
        .is_some_and(style_has_fill_border_or_protection)
}

#[inline]
fn style_has_no_empty_cell_visible_style(xf: &Xf) -> bool {
    has_default_fill(xf)
        && has_default_border(xf)
        && !xf.apply_fill
        && !xf.apply_border
        && !xf.has_apply_protection
}

#[inline]
fn has_default_fill(xf: &Xf) -> bool {
    xf.fill_id.is_none_or(|fill_id| fill_id == 0)
}

#[inline]
fn has_default_border(xf: &Xf) -> bool {
    xf.border_id.is_none_or(|border_id| border_id == 0)
}

#[inline]
fn style_has_fill_border_or_protection(xf: &Xf) -> bool {
    xf.has_fill_or_border || xf.applies_default_fill_or_border_override() || xf.has_apply_protection
}
