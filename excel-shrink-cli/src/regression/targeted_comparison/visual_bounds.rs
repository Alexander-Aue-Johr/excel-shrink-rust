//! Visible formatting contributes to the validation extent even without cell values.
use super::content_bounds::{include, Rectangle};
use crate::regression::xml_views::Tag;
use anyhow::{Context, Result};
use excel_shrink_core::models::Xf;

pub(super) struct VisualBounds<'a> {
    styles: &'a [Xf],
    pub bounds: Option<Rectangle>,
}

impl<'a> VisualBounds<'a> {
    pub fn new(styles: &'a [Xf]) -> Self {
        Self {
            styles,
            bounds: None,
        }
    }

    /// Row and column formats apply beyond explicitly serialized cells.
    /// Unknown styles remain potential visual content instead of proving an empty tail.
    pub fn observe(&mut self, tag: Tag<'_>) -> Result<()> {
        let attribute = if tag.local() == b"col" {
            b"style".as_slice()
        } else {
            b"s"
        };
        let Some(index) = tag.attr(attribute)? else {
            return Ok(());
        };
        let index: usize = std::str::from_utf8(index)?.parse()?;
        if self
            .styles
            .get(index)
            .is_some_and(|xf| !xf.has_visible_fill && !xf.has_visible_border)
        {
            return Ok(());
        }
        match tag.local() {
            b"c" => {
                let address = std::str::from_utf8(
                    tag.attr(b"r")?.context("Missing styled cell coordinate")?,
                )?;
                let (row, column) = super::conditional_formatting::cell_address(address)
                    .filter(|(row, column)| *row > 0 && *column > 0)
                    .context("Invalid styled cell coordinate")?;
                include(&mut self.bounds, row, column);
            }
            b"row" if tag.attr(b"customFormat")? == Some(b"1") => {
                let row: u32 =
                    std::str::from_utf8(tag.attr(b"r")?.context("Missing styled row")?)?.parse()?;
                self.include_rectangle(row, 1, row, 16_384);
            }
            b"col" => {
                let left: u32 =
                    std::str::from_utf8(tag.attr(b"min")?.context("Missing column start")?)?
                        .parse()?;
                let right: u32 =
                    std::str::from_utf8(tag.attr(b"max")?.context("Missing column end")?)?
                        .parse()?;
                self.include_rectangle(1, left, 1_048_576, right);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn include_rectangle(&mut self, top: u32, left: u32, bottom: u32, right: u32) {
        include(&mut self.bounds, top, left);
        include(&mut self.bounds, bottom, right);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regression::xml_views::Reader;

    #[test]
    fn fill_border_and_inherited_formats_extend_bounds_but_font_only_cells_do_not() -> Result<()> {
        let styles = [
            Xf::default(),
            Xf {
                has_visible_fill: true,
                ..Default::default()
            },
            Xf {
                has_visible_border: true,
                ..Default::default()
            },
        ];
        let mut bounds = VisualBounds::new(&styles);
        let mut reader =
            Reader::<32>::new(br#"<c r="C10" s="1"/><c r="E20" s="2"/><c r="XFD999999" s="0"/>"#)?;
        while let Some(tag) = reader.next()? {
            bounds.observe(tag)?;
        }
        assert_eq!(bounds.bounds.unwrap().address(), "C10:E20");
        let mut reader = Reader::<32>::new(br#"<col min="8" max="9" style="1"/>"#)?;
        bounds.observe(reader.next()?.unwrap())?;
        assert_eq!(bounds.bounds.unwrap().address(), "C1:I1048576");
        Ok(())
    }
}
