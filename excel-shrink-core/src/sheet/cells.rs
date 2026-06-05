use crate::models::{CellAttributes, CellCoordinate, CleanSheetMetrics, CleanSheetParameters, Xf};
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use regex::bytes::{Captures, Regex};
use std::sync::OnceLock;

static CELL_REF_RE: OnceLock<Regex> = OnceLock::new();
static STYLE_RE: OnceLock<Regex> = OnceLock::new();

pub fn replace_cell(
    caps: &Captures,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let cell_raw = caps.get(0).unwrap().as_bytes();
    let Some(cell_attributes) = extract_cell_attributes(caps, params) else {
        return cell_raw.to_vec();
    };

    let coord = &cell_attributes.cell_coordinate;
    let style = cell_attributes.style.clone();

    let is_hidden = cell_is_hidden(coord, metrics);

    // --- ADVANCED CLEANUP LOGIC ---
    let col_num = letters_byte_string_to_integer(&coord.column);
    let row_num = std::str::from_utf8(&coord.row)
        .unwrap_or("0")
        .parse::<u32>()
        .unwrap_or(0);

    let mut force_delete = false;

    if let Some(adv) = &params.advanced_cleanup {
        if adv.safe_to_clean {
            let is_protected = adv.protected_cells.contains(col_num, row_num);
            // Feature 1: Whitespace
            if adv.remove_unreferenced_whitespace && is_whitespace_cell(cell_raw, params) {
                metrics.whitespace_candidates += 1;
                if !is_protected {
                    metrics.whitespace_removed += 1;
                    force_delete = true;
                } else {
                    metrics.whitespace_protected_by_reference += 1;
                }
            }

            // Feature 2: Hidden Content
            if adv.remove_unreferenced_hidden_content && is_hidden && has_content(cell_raw) {
                metrics.hidden_candidates += 1;
                if !is_protected {
                    metrics.hidden_removed += 1;
                    force_delete = true;
                } else {
                    metrics.hidden_protected_by_reference += 1;
                }
            }
        } else {
            // Unsafe to clean, but we count candidates for logging if we can
            if adv.remove_unreferenced_whitespace && is_whitespace_cell(cell_raw, params) {
                metrics.whitespace_candidates += 1;
            }
            if adv.remove_unreferenced_hidden_content && is_hidden && has_content(cell_raw) {
                metrics.hidden_candidates += 1;
            }
        }
    }

    if force_delete {
        metrics.deleted_cell_states.push(cell_attributes);
        return Vec::new();
    }
    // --- END ADVANCED CLEANUP LOGIC ---

    // Original hidden logic (deletes EMPTY hidden cells, but keeps them if they override styles)
    if is_hidden {
        // If it has content, and we didn't force delete it above, we MUST NOT delete it natively here!
        if !has_content(cell_raw) {
            metrics.deleted_cell_states.push(cell_attributes);
            return Vec::new();
        } else {
            // It's hidden but has content, and wasn't deleted by advanced cleanup. Keep it.
            metrics.kept_cell_states.push(cell_attributes);
            return cell_raw.to_vec();
        }
    }

    // Original style logic for visible cells
    let Some(style) = style else {
        // Safe to delete if it's empty. If it has content, don't delete!
        if !has_content(cell_raw) {
            metrics.deleted_cell_states.push(cell_attributes);
            return Vec::new();
        }
        metrics.kept_cell_states.push(cell_attributes);
        return cell_raw.to_vec();
    };

    if style.has_fill_or_border || style.has_apply_protection {
        // If it has content, keep the whole tag, else self-close
        if has_content(cell_raw) {
            metrics.kept_cell_states.push(cell_attributes);
            return cell_raw.to_vec();
        }
        let out = generate_self_closing_cell_xml(&cell_attributes.cell_attribute_string);
        metrics.kept_cell_states.push(cell_attributes);
        return out;
    }

    if !style.has_apply_fill_or_border {
        if !has_content(cell_raw) {
            metrics.deleted_cell_states.push(cell_attributes);
            return Vec::new();
        }
        metrics.kept_cell_states.push(cell_attributes);
        return cell_raw.to_vec();
    }

    if cell_overrides_fill_or_border(&style, coord, metrics) {
        if has_content(cell_raw) {
            metrics.kept_cell_states.push(cell_attributes);
            return cell_raw.to_vec();
        }
        let out = generate_self_closing_cell_xml(&cell_attributes.cell_attribute_string);
        metrics.kept_cell_states.push(cell_attributes);
        return out;
    }

    if !has_content(cell_raw) {
        metrics.deleted_cell_states.push(cell_attributes);
        return Vec::new();
    }

    metrics.kept_cell_states.push(cell_attributes);
    cell_raw.to_vec()
}

fn extract_cell_attributes(
    caps: &Captures,
    params: &CleanSheetParameters,
) -> Option<CellAttributes> {
    let cell_attr = caps.get(1)?.as_bytes().to_vec();

    let cell_ref_re = CELL_REF_RE.get_or_init(|| Regex::new(r#"r="([A-Z]+)(\d+)""#).unwrap());
    let style_re = STYLE_RE.get_or_init(|| Regex::new(r#"s="(\d+?)""#).unwrap());

    let ref_caps = cell_ref_re.captures(&cell_attr)?;
    let col_ref = ref_caps.get(1)?.as_bytes().to_vec();
    let row_ref = ref_caps.get(2)?.as_bytes().to_vec();

    let style = style_re
        .captures(&cell_attr)
        .and_then(|c| c.get(1))
        .map(|m| digits_byte_string_to_integer(m.as_bytes()))
        .and_then(|style_id| params.patterns.xfs_by_index.get(&style_id).cloned());

    Some(CellAttributes {
        cell_attribute_string: cell_attr,
        cell_coordinate: CellCoordinate {
            column: col_ref,
            row: row_ref,
        },
        style,
    })
}

fn cell_is_hidden(cell_coordinate: &CellCoordinate, metrics: &CleanSheetMetrics) -> bool {
    if metrics
        .row_attributes
        .get(&cell_coordinate.row)
        .is_some_and(|row| row.hidden)
    {
        return true;
    }

    let column_index = letters_byte_string_to_integer(&cell_coordinate.column);

    metrics
        .column_definitions
        .get(column_index)
        .is_some_and(|column| column.hidden)
}

fn cell_overrides_fill_or_border(
    cell_style: &Xf,
    cell_coordinate: &CellCoordinate,
    metrics: &CleanSheetMetrics,
) -> bool {
    let column_index = letters_byte_string_to_integer(&cell_coordinate.column);

    let column_style = metrics
        .column_definitions
        .get(column_index)
        .and_then(|column| column.cell_xf.as_ref());

    let row_style = metrics
        .row_attributes
        .get(&cell_coordinate.row)
        .and_then(|row| row.style.as_ref());

    let overrides_fill = row_style
        .is_some_and(|row| row.apply_fill && cell_style.fill_id != row.fill_id)
        || column_style
            .is_some_and(|column| column.apply_fill && cell_style.fill_id != column.fill_id);

    let overrides_border = row_style
        .is_some_and(|row| row.apply_border && cell_style.border_id != row.border_id)
        || column_style
            .is_some_and(|column| column.apply_border && cell_style.border_id != column.border_id);

    overrides_fill || overrides_border
}

fn generate_self_closing_cell_xml(cell_attr: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cell_attr.len() + 4);
    out.extend_from_slice(b"<c");
    out.extend_from_slice(cell_attr);
    out.extend_from_slice(b"/>");
    out
}

fn has_content(cell: &[u8]) -> bool {
    crate::xml::find_bytes(cell, b"<v>").is_some()
        || crate::xml::find_bytes(cell, b"<f>").is_some()
        || crate::xml::find_bytes(cell, b"<is>").is_some()
}

fn is_whitespace_cell(cell: &[u8], params: &CleanSheetParameters) -> bool {
    let Some(indices) = &params.patterns.whitespace_indices else {
        return false;
    };

    // Check if it's a shared string
    if !cell.windows(b"t=\"s\"".len()).any(|w| w == b"t=\"s\"") {
        return false;
    }

    let Some(v_start_rel) = crate::xml::find_bytes(cell, b"<v>") else {
        return false;
    };
    let value_start = v_start_rel + b"<v>".len();

    let Some(v_end_rel) = crate::xml::find_bytes(&cell[value_start..], b"</v>") else {
        return false;
    };

    let value = &cell[value_start..value_start + v_end_rel];

    if let Ok(value_str) = std::str::from_utf8(value) {
        if let Ok(index) = value_str.parse::<usize>() {
            return indices.contains(&index);
        }
    }

    false
}
