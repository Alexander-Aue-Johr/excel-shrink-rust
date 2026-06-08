use crate::models::{WorkbookSpecificPatterns, Xf};

pub fn generate_workbook_specific_patterns(
    xfs: Vec<Xf>,
    whitespace_indices: Option<Vec<usize>>,
) -> WorkbookSpecificPatterns {
    WorkbookSpecificPatterns::new(xfs, whitespace_indices)
}

pub fn style_id_has_only_font_style(patterns: &WorkbookSpecificPatterns, style_id: u32) -> bool {
    patterns.xfs_by_index.get(&style_id).is_some_and(|xf| {
        (xf.fill_id.is_none() || xf.fill_id == Some(0))
            && (xf.border_id.is_none() || xf.border_id == Some(0))
            && !xf.apply_fill
            && !xf.apply_border
            && !xf.has_apply_protection
    })
}

pub fn style_id_has_fill_border_or_protection(
    patterns: &WorkbookSpecificPatterns,
    style_id: u32,
) -> bool {
    patterns
        .xfs_by_index
        .get(&style_id)
        .is_some_and(|xf| xf.has_fill_or_border || xf.has_apply_protection)
}
