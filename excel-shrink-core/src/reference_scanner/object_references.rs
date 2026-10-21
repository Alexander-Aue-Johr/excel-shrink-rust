use super::*;
use std::borrow::Cow;

static FORMULAS: OnceLock<Regex> = OnceLock::new();
static LINKS: OnceLock<Regex> = OnceLock::new();
static PIVOT_SOURCES: OnceLock<Regex> = OnceLock::new();
static PIVOT_LOCATIONS: OnceLock<Regex> = OnceLock::new();

impl PreparedReferenceScanner<'_> {
    /// GETPIVOTDATA refers to the pivot anchor, but consumes values throughout
    /// its result area. Those cells can reside on a hidden worksheet.
    pub fn scan_pivot_result_references(
        &self,
        xml: &[u8],
        sheet_file: Option<&str>,
        protected: &mut ProtectedWorkbookRefs,
    ) {
        let locations = PIVOT_LOCATIONS
            .get_or_init(|| Regex::new(r"(?s)<(?:[\w.-]+:)?location\b([^>]*)>").unwrap());
        for captures in locations.captures_iter(xml) {
            if let Some(reference) =
                extract_quoted_attr_value(captures.get(1).unwrap().as_bytes(), b"ref")
            {
                extract_reference_token_for_sheet_bytes(
                    reference,
                    sheet_file,
                    &self.sheet_name_to_file,
                    protected,
                );
            }
        }
    }

    /// Only data consumers belong here. Drawing anchors, worksheet dimensions and
    /// filter ranges describe layout; they do not make a cell's value necessary.
    pub fn scan_object_references(
        &self,
        xml: &[u8],
        protected: &mut ProtectedWorkbookRefs,
    ) -> ScanResult {
        let formulas = FORMULAS.get_or_init(|| {
            Regex::new(r"(?is)<(?:[\w.-]+:)?(?:f|FmlaLink|FmlaRange)\b[^>]*>([^<]*)</").unwrap()
        });
        let links = LINKS.get_or_init(|| {
            Regex::new(r#"(?i)\b(?:textlink|fmlaLink|fmlaRange)\s*=\s*(?:"([^"]*)"|'([^']*)')"#)
                .unwrap()
        });
        for captures in formulas.captures_iter(xml) {
            if let ScanResult::Unsafe(reason) =
                self.scan_object_formula(captures.get(1).unwrap().as_bytes(), protected)
            {
                return ScanResult::Unsafe(reason);
            }
        }
        for captures in links.captures_iter(xml) {
            let formula = captures.get(1).or_else(|| captures.get(2)).unwrap();
            if let ScanResult::Unsafe(reason) =
                self.scan_object_formula(formula.as_bytes(), protected)
            {
                return ScanResult::Unsafe(reason);
            }
        }
        let sources = PIVOT_SOURCES
            .get_or_init(|| Regex::new(r"(?s)<(?:[\w.-]+:)?worksheetSource\b([^>]*)>").unwrap());
        for captures in sources.captures_iter(xml) {
            let attrs = captures.get(1).unwrap().as_bytes();
            let Some(reference) = extract_quoted_attr_value(attrs, b"ref") else {
                // A source name can refer to a defined name or a table. Until the
                // latter is resolved, its values must not be called unreferenced.
                if extract_quoted_attr_value(attrs, b"name").is_some() {
                    return ScanResult::Unsafe("named pivot source".into());
                }
                continue;
            };
            let sheet = extract_quoted_attr_value(attrs, b"sheet").map(decode_entities);
            let target = sheet
                .as_deref()
                .and_then(|name| self.sheet_name_to_file.get(name).map(String::as_str));
            extract_reference_token_for_sheet_bytes(
                reference,
                target,
                &self.sheet_name_to_file,
                protected,
            );
        }
        ScanResult::Safe
    }

    fn scan_object_formula(
        &self,
        formula: &[u8],
        protected: &mut ProtectedWorkbookRefs,
    ) -> ScanResult {
        let formula = decode_entities(formula);
        if let Some(function) = reference_scan_blocker(&formula, self.patterns.unsafe_function) {
            return ScanResult::Unsafe(function);
        }
        // Object formulas normally carry a sheet name. For an unresolved owner,
        // conservatively protect these coordinates on every sheet.
        extract_tokens_for_sheet(
            &formula,
            None,
            &self.sheet_name_to_file,
            protected,
            formula_reference_pattern(),
        );
        ScanResult::Safe
    }
}

pub(super) fn decode_entities(value: &[u8]) -> Cow<'_, [u8]> {
    if !value.contains(&b'&') {
        return Cow::Borrowed(value);
    }
    let mut decoded = Vec::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.iter().position(|byte| *byte == b'&') {
        decoded.extend_from_slice(&remaining[..start]);
        remaining = &remaining[start..];
        let Some(end) = remaining.iter().position(|byte| *byte == b';') else {
            break;
        };
        let entity = &remaining[1..end];
        let character = match entity {
            b"amp" => Some('&'),
            b"apos" => Some('\''),
            b"quot" => Some('"'),
            b"lt" => Some('<'),
            b"gt" => Some('>'),
            _ => std::str::from_utf8(entity).ok().and_then(|text| {
                text.strip_prefix("#x")
                    .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                    .or_else(|| {
                        text.strip_prefix('#')
                            .and_then(|digits| digits.parse().ok())
                    })
                    .and_then(char::from_u32)
            }),
        };
        if let Some(character) = character {
            decoded.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
        } else {
            decoded.extend_from_slice(&remaining[..=end]);
        }
        remaining = &remaining[end + 1..];
    }
    decoded.extend_from_slice(remaining);
    Cow::Owned(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(xml: &[u8]) -> ProtectedWorkbookRefs {
        let names = HashMap::from([
            ("Basic Interface".into(), "sheet1".into()),
            ("O'Brien & Sons".into(), "sheet2".into()),
        ]);
        let scanner = PreparedReferenceScanner::new(&[], Some(&names), None);
        let mut protected = ProtectedWorkbookRefs::default();
        assert!(matches!(
            scanner.scan_object_references(xml, &mut protected),
            ScanResult::Safe
        ));
        protected
    }

    #[test]
    fn chart_series_keep_hidden_data_without_protecting_other_cells() {
        let refs = scan(
            br#"<c:numRef><c:f>'Basic Interface'!$A$13:$Y$14</c:f></c:numRef>
            <cx:f>'Basic Interface'!$A$19:$U$21</cx:f>
            <dimension ref="A1:XFD1048576"/><xdr:from><xdr:row>900</xdr:row></xdr:from>"#,
        );
        assert!(refs.contains("sheet1", 25, 14));
        assert!(refs.contains("sheet1", 21, 21));
        assert!(!refs.contains("sheet1", 26, 14));
        assert!(!refs.contains("sheet2", 25, 14));
        assert!(!refs.contains("sheet1", 1, 900));
    }

    #[test]
    fn linked_drawing_text_and_controls_decode_sheet_names() {
        let refs = scan(
            br#"<xdr:sp textlink="'O''Brien &amp; Sons'!$C$7"/>
            <x:FmlaLink>'Basic Interface'!$D$4</x:FmlaLink>
            <formControlPr fmlaRange="'Basic Interface'!$A$2:$A$8"/>"#,
        );
        assert!(refs.contains("sheet2", 3, 7));
        assert!(!refs.contains("sheet1", 3, 7));
        assert!(refs.contains("sheet1", 4, 4));
        assert!(refs.contains("sheet1", 1, 8));
    }

    #[test]
    fn pivot_sources_protect_the_input_range_only() {
        let refs = scan(
            br#"<worksheetSource ref="B2:D9" sheet="Basic Interface"/>
            <location ref="A1:Z200"/>"#,
        );
        assert!(refs.contains("sheet1", 3, 8));
        assert!(!refs.contains("sheet1", 1, 1));
        assert!(!refs.contains("sheet1", 3, 10));
        assert!(!refs.contains("sheet2", 3, 8));
    }

    #[test]
    fn pivot_results_are_scoped_to_the_owning_sheet() {
        let scanner = PreparedReferenceScanner::new(&[], None, None);
        let mut refs = ProtectedWorkbookRefs::default();
        scanner.scan_pivot_result_references(
            br#"<pivotTableDefinition><location ref="A7:B94"/></pivotTableDefinition>"#,
            Some("sheet1"),
            &mut refs,
        );
        assert!(refs.contains("sheet1", 2, 94));
        assert!(!refs.contains("sheet1", 3, 94));
        assert!(!refs.contains("sheet2", 2, 94));
    }

    #[test]
    fn whole_column_series_are_kept_without_expanding_every_cell() {
        let refs = scan(br#"<c:f>'Basic Interface'!$B:$D</c:f>"#);
        assert!(refs.contains("sheet1", 3, 1048576));
        assert!(!refs.contains("sheet1", 5, 1));
    }

    #[test]
    fn dynamic_object_references_disable_reference_based_removal() {
        let scanner = PreparedReferenceScanner::new(&[], None, None);
        assert!(matches!(
            scanner.scan_object_references(
                br#"<c:f>OFFSET(Sheet1!A1,1,0)</c:f>"#,
                &mut ProtectedWorkbookRefs::default()
            ),
            ScanResult::Unsafe(_)
        ));
    }

    #[test]
    fn unresolved_named_pivot_sources_keep_candidates() {
        let scanner = PreparedReferenceScanner::new(&[], None, None);
        assert!(matches!(
            scanner.scan_object_references(
                br#"<worksheetSource name="Table1"/>"#,
                &mut ProtectedWorkbookRefs::default()
            ),
            ScanResult::Unsafe(_)
        ));
    }
}
