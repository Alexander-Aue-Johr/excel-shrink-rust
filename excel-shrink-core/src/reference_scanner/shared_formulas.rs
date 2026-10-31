//! Conservative reference protection for Excel shared formulas.
use super::*;

static SHARED_FORMULA_TAG: OnceLock<Regex> = OnceLock::new();

#[derive(Clone, Copy)]
struct CellAddress {
    col: u32,
    row: u32,
}

#[derive(Clone, Copy)]
struct FormulaArea {
    first: CellAddress,
    last: CellAddress,
}

impl FormulaArea {
    fn contains(self, cell: CellAddress) -> bool {
        (self.first.col..=self.last.col).contains(&cell.col)
            && (self.first.row..=self.last.row).contains(&cell.row)
    }
}

#[derive(Clone, Copy)]
struct AxisReference {
    value: u32,
    absolute: bool,
}

#[derive(Clone, Copy)]
struct FormulaReference {
    col: Option<AxisReference>,
    row: Option<AxisReference>,
}

/// Protects every reference reachable by copying a shared anchor formula.
///
/// # Correctness
///
/// A shared formula stores its expression once; follower cells have no text to
/// scan. Relative A1 references move with each follower, while `$` axes stay
/// fixed. We insert the enclosing reference rectangle rather than materialize
/// every copied formula. Invalid or incomplete groups return an unsafe reason;
/// the caller then retains reference-sensitive cells for the workbook.
pub(super) fn scan_shared_formula_references(
    buffer: &SheetScanBuffer<'_>,
    protected: &mut ProtectedWorkbookRefs,
    sheet_name_to_file: &HashMap<Vec<u8>, String>,
    reference_tokens: &Regex,
) -> Option<String> {
    if memchr::memmem::find(buffer.xml, b"shared").is_none() {
        return None;
    }
    let pattern = SHARED_FORMULA_TAG.get_or_init(|| {
        Regex::new(
            r"(?s)<(?:[\w.-]+:)?f\b([^>]*)/>|<(?:[\w.-]+:)?f\b([^>]*)>(.*?)</(?:[\w.-]+:)?f>",
        )
        .unwrap()
    });
    let mut anchors = HashMap::new();
    let mut followers = Vec::new();

    for captures in pattern.captures_iter(buffer.xml) {
        let attributes = captures
            .get(1)
            .or_else(|| captures.get(2))
            .unwrap()
            .as_bytes();
        if extract_quoted_attr_value(attributes, b"t") != Some(b"shared".as_slice()) {
            continue;
        }
        let Some(group) = extract_quoted_attr_value(attributes, b"si")
            .and_then(crate::utils::parse_unsigned_bytes::<u32>)
        else {
            return Some("shared formula has no valid group index".into());
        };
        let Some(cell) = formula_cell_address(buffer.xml, captures.get(0).unwrap().start()) else {
            return Some(format!(
                "shared formula group {group} in {} has no valid cell address",
                buffer.sheet_file
            ));
        };
        let formula = captures.get(3).map(|body| trim_ascii(body.as_bytes()));
        if let Some(formula) = formula.filter(|formula| !formula.is_empty()) {
            let range = extract_quoted_attr_value(attributes, b"ref");
            let Some(area) = range.and_then(parse_formula_area) else {
                return Some(format!(
                    "shared formula group {group} in {} has no valid anchor range: {}",
                    buffer.sheet_file,
                    String::from_utf8_lossy(range.unwrap_or_default())
                ));
            };
            if !area.contains(cell) || anchors.insert(group, area).is_some() {
                return Some(format!(
                    "shared formula group {group} in {} has a conflicting anchor",
                    buffer.sheet_file
                ));
            }
            if let Some(reason) = protect_anchor_references(
                formula,
                cell,
                area,
                buffer.sheet_file,
                sheet_name_to_file,
                protected,
                reference_tokens,
            ) {
                return Some(reason);
            }
        } else {
            followers.push((group, cell));
        }
    }

    for (group, cell) in followers {
        if !anchors.get(&group).is_some_and(|area| area.contains(cell)) {
            return Some(format!(
                "shared formula group {group} in {} has a follower outside its anchor range",
                buffer.sheet_file
            ));
        }
    }
    None
}

fn formula_cell_address(xml: &[u8], formula_start: usize) -> Option<CellAddress> {
    let cell_start = crate::xml::rfind_bytes(&xml[..formula_start], b"<c")?;
    let tag = &xml[cell_start + 2..];
    if !tag.first()?.is_ascii_whitespace() {
        return None;
    }
    if crate::xml::rfind_bytes(&xml[..formula_start], b"</c>")
        .is_some_and(|closed| closed > cell_start)
    {
        return None;
    }
    let tag_end = tag.iter().position(|byte| *byte == b'>')?;
    extract_quoted_attr_value(&tag[..tag_end], b"r").and_then(parse_cell_address)
}

fn parse_cell_address(text: &[u8]) -> Option<CellAddress> {
    let reference = parse_formula_reference(text)?;
    Some(CellAddress {
        col: reference.col?.value,
        row: reference.row?.value,
    })
}

fn parse_formula_area(text: &[u8]) -> Option<FormulaArea> {
    let (first, last) = match memchr::memchr(b':', text) {
        Some(index) => (&text[..index], &text[index + 1..]),
        None => (text, text),
    };
    let first = parse_cell_address(first)?;
    let last = parse_cell_address(last)?;
    (first.col <= last.col && first.row <= last.row).then_some(FormulaArea { first, last })
}

fn protect_anchor_references(
    formula: &[u8],
    anchor: CellAddress,
    area: FormulaArea,
    current_sheet: &str,
    sheet_name_to_file: &HashMap<Vec<u8>, String>,
    protected: &mut ProtectedWorkbookRefs,
    reference_tokens: &Regex,
) -> Option<String> {
    let formula = object_references::decode_entities(formula);
    for token in reference_tokens.find_iter(&formula) {
        let (sheet_prefix, raw) = split_sheet_prefix_bytes(token.as_bytes());
        let Some((first, last)) = parse_formula_token(raw) else {
            return Some("shared formula contains an unsupported reference".into());
        };
        let target = match sheet_prefix {
            Some(sheet) => resolve_sheet_name(sheet, sheet_name_to_file),
            None => Some(current_sheet),
        };
        let target = refs_for_target_sheet(target, protected);
        let Some((first_col, last_col)) = shifted_axis_bounds(
            first.col,
            last.col,
            area.first.col as i64 - anchor.col as i64,
            area.last.col as i64 - anchor.col as i64,
            MAX_EXCEL_COL,
        ) else {
            return Some("shared formula reference moves outside the Excel grid".into());
        };
        let Some((first_row, last_row)) = shifted_axis_bounds(
            first.row,
            last.row,
            area.first.row as i64 - anchor.row as i64,
            area.last.row as i64 - anchor.row as i64,
            MAX_EXCEL_ROW,
        ) else {
            return Some("shared formula reference moves outside the Excel grid".into());
        };
        target.insert_range(first_col, first_row, last_col, last_row);
    }
    None
}

fn parse_formula_token(text: &[u8]) -> Option<(FormulaReference, FormulaReference)> {
    let (first, last) = match memchr::memchr(b':', text) {
        Some(index) => (&text[..index], &text[index + 1..]),
        None => (text, text),
    };
    Some((
        parse_formula_reference(first)?,
        parse_formula_reference(last)?,
    ))
}

fn parse_formula_reference(text: &[u8]) -> Option<FormulaReference> {
    let mut cursor = 0;
    let leading_dollar = text.first() == Some(&b'$');
    if leading_dollar {
        cursor += 1;
    }
    let col_start = cursor;
    let mut col = 0u32;
    while let Some(byte) = text.get(cursor).filter(|byte| byte.is_ascii_alphabetic()) {
        col = col
            .checked_mul(26)?
            .checked_add((byte.to_ascii_uppercase() - b'A' + 1) as u32)?;
        cursor += 1;
    }
    let has_col = cursor > col_start;
    let row_absolute = if has_col && text.get(cursor) == Some(&b'$') {
        cursor += 1;
        true
    } else {
        !has_col && leading_dollar
    };
    let row_start = cursor;
    let mut row = 0u32;
    while let Some(byte) = text.get(cursor).filter(|byte| byte.is_ascii_digit()) {
        row = row.checked_mul(10)?.checked_add((byte - b'0') as u32)?;
        cursor += 1;
    }
    let has_row = cursor > row_start;
    if cursor != text.len()
        || (!has_col && !has_row)
        || (row_absolute && !has_row)
        || (has_col && !(1..=MAX_EXCEL_COL).contains(&col))
        || (has_row && !(1..=MAX_EXCEL_ROW).contains(&row))
    {
        return None;
    }
    Some(FormulaReference {
        col: has_col.then_some(AxisReference {
            value: col,
            absolute: leading_dollar,
        }),
        row: has_row.then_some(AxisReference {
            value: row,
            absolute: row_absolute,
        }),
    })
}

fn shifted_axis_bounds(
    first: Option<AxisReference>,
    last: Option<AxisReference>,
    min_shift: i64,
    max_shift: i64,
    maximum: u32,
) -> Option<(u32, u32)> {
    let (Some(first), Some(last)) = (first, last) else {
        return (first.is_none() && last.is_none()).then_some((1, maximum));
    };
    let shifted = [
        first.value as i64 + if first.absolute { 0 } else { min_shift },
        first.value as i64 + if first.absolute { 0 } else { max_shift },
        last.value as i64 + if last.absolute { 0 } else { min_shift },
        last.value as i64 + if last.absolute { 0 } else { max_shift },
    ];
    if shifted
        .iter()
        .any(|value| *value < 1 || *value > maximum as i64)
    {
        return None;
    }
    Some((*shifted.iter().min()? as u32, *shifted.iter().max()? as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shared_formula_range() {
        let area = parse_formula_area(b"D166:J166").unwrap();
        assert_eq!((area.first.col, area.first.row), (4, 166));
        assert_eq!((area.last.col, area.last.row), (10, 166));
    }
}
