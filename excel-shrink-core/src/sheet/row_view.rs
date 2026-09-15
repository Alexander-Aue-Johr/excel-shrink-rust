use crate::models::{CleanSheetParameters, Xf};
use crate::xml::attributes::Attributes;

/// A row header borrowed from the current XML buffer and the workbook style
/// table. No owned byte buffers, allocation, or state surviving the row.
#[derive(Debug, Clone, Copy)]
pub struct RowView<'xml, 'styles> {
    pub(crate) raw_attributes: &'xml [u8],
    pub(crate) reference: &'xml [u8],
    pub(crate) number: Option<u32>,
    pub(crate) style: Option<&'styles Xf>,
    pub(crate) hidden: bool,
    pub(crate) has_height: bool,
    pub(crate) has_custom_height: bool,
    pub(crate) spans: Option<(usize, usize)>,
}

impl<'xml, 'styles> RowView<'xml, 'styles> {
    /// Scan each attribute once. An invalid header is left untouched by callers.
    pub fn parse(
        raw_attributes: &'xml [u8],
        params: &'styles CleanSheetParameters,
    ) -> Option<Self> {
        let mut row = Self {
            raw_attributes,
            reference: b"",
            number: None,
            style: None,
            hidden: false,
            has_height: false,
            has_custom_height: false,
            spans: None,
        };
        // Preserve first-value semantics for repeated recognized attributes.
        let mut seen = 0u8;
        for attribute in Attributes::new(raw_attributes) {
            let attribute = attribute.ok()?;
            let bit = match attribute.name {
                b"r" => 1,
                b"s" => 2,
                b"hidden" => 4,
                b"ht" => 8,
                b"customHeight" => 16,
                b"spans" => 32,
                _ => continue,
            };
            if seen & bit != 0 {
                continue;
            }
            seen |= bit;
            match attribute.name {
                b"r" => {
                    row.reference = attribute.value;
                    row.number = std::str::from_utf8(attribute.value).ok()?.parse().ok();
                }
                b"s" => {
                    let id = crate::utils::digits_byte_string_to_integer(attribute.value);
                    row.style = params.patterns.xfs_by_index.get(&id);
                }
                b"hidden" => row.hidden = attribute.value == b"1",
                b"ht" => row.has_height = true,
                b"customHeight" => row.has_custom_height = attribute.value == b"1",
                b"spans" => row.spans = Some((attribute.value_start, attribute.value_end)),
                _ => unreachable!(),
            }
        }
        Some(row)
    }
}
