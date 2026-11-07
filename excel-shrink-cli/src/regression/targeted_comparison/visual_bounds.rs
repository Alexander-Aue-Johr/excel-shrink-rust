//! Visible formatting contributes to the validation extent even without cell values.
use super::content_bounds::{include, Rectangle};
use crate::regression::xml_views::{Kind, Tag};
use anyhow::{Context, Result};
use excel_shrink_core::models::{CenteredTextSpan, ColumnDefinition, ColumnRangeMap, Xf};
use std::collections::BTreeSet;

pub(super) struct VisualBounds<'a> {
    styles: &'a [Xf],
    has_centered_styles: bool,
    centered_span: CenteredTextSpan,
    pending_cell: Option<PendingCell>,
    columns: ColumnRangeMap,
    column_bounds: Option<Rectangle>,
    explicit_cells: Option<Rectangle>,
    row_styles: BTreeSet<usize>,
    row_styles_known: bool,
    row_bounds: Option<Rectangle>,
    columns_known: bool,
    pub bounds: Option<Rectangle>,
}

struct PendingCell {
    row: u32,
    column: u32,
    centered: bool,
    has_content: bool,
}

impl<'a> VisualBounds<'a> {
    pub fn new(styles: &'a [Xf]) -> Self {
        Self {
            styles,
            has_centered_styles: styles.iter().any(|style| style.centers_across_selection),
            centered_span: CenteredTextSpan::default(),
            pending_cell: None,
            columns: ColumnRangeMap::default(),
            column_bounds: None,
            explicit_cells: None,
            row_styles: BTreeSet::new(),
            row_styles_known: true,
            row_bounds: None,
            columns_known: true,
            bounds: None,
        }
    }

    /// Row and column formats apply beyond explicitly serialized cells.
    /// Unknown styles remain potential visual content instead of proving an empty tail.
    pub fn observe(&mut self, tag: Tag<'_>) -> Result<()> {
        if self.has_centered_styles {
            self.observe_centered_span(tag)?;
        }
        if tag.kind == Kind::End {
            return Ok(());
        }
        match tag.local() {
            b"col" => {
                self.columns_known &= tag.name == b"col" && tag.kind == Kind::Empty;
                let number = |name| -> Result<u32> {
                    Ok(
                        std::str::from_utf8(tag.attr(name)?.context("Missing column boundary")?)?
                            .parse()?,
                    )
                };
                let style = tag
                    .attr(b"style")?
                    .map(|value| -> Result<usize> { Ok(std::str::from_utf8(value)?.parse()?) })
                    .transpose()?;
                self.columns.add(ColumnDefinition {
                    min: number(b"min")?,
                    max: number(b"max")?,
                    cell_xf: style.and_then(|index| self.styles.get(index)).cloned(),
                    hidden: crate::regression::xml_views::truth(tag.attr(b"hidden")?),
                });
            }
            b"row" => {
                if let Some(index) = tag.attr(b"s")? {
                    self.row_styles.insert(std::str::from_utf8(index)?.parse()?);
                } else if tag.attr(b"customFormat")?.is_some() {
                    self.row_styles_known = false;
                }
            }
            b"c" => {
                let address =
                    std::str::from_utf8(tag.attr(b"r")?.context("Missing cell coordinate")?)?;
                let (row, column) = super::conditional_formatting::cell_address(address)
                    .context("Invalid cell coordinate")?;
                include(&mut self.explicit_cells, row, column);
            }
            _ => {}
        }
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
                include(&mut self.row_bounds, row, 1);
                include(&mut self.row_bounds, row, 16_384);
            }
            b"col" => {
                let left: u32 =
                    std::str::from_utf8(tag.attr(b"min")?.context("Missing column start")?)?
                        .parse()?;
                let right: u32 =
                    std::str::from_utf8(tag.attr(b"max")?.context("Missing column end")?)?
                        .parse()?;
                include(&mut self.column_bounds, 1, left);
                include(&mut self.column_bounds, 1_048_576, right);
            }
            _ => {}
        }
        Ok(())
    }

    /// Reuse the shrinker's span rule so centered text can extend past value cells.
    fn observe_centered_span(&mut self, tag: Tag<'_>) -> Result<()> {
        if tag.local() == b"c" {
            if tag.kind == Kind::End {
                self.finish_centered_cell();
                return Ok(());
            }
            let address = std::str::from_utf8(tag.attr(b"r")?.context("Missing cell coordinate")?)?;
            let (row, column) = super::conditional_formatting::cell_address(address)
                .context("Invalid cell coordinate")?;
            let style = tag
                .attr(b"s")?
                .map(|index| -> Result<usize> { Ok(std::str::from_utf8(index)?.parse()?) })
                .transpose()?;
            self.pending_cell = Some(PendingCell {
                row,
                column,
                has_content: false,
                centered: style
                    .and_then(|index| self.styles.get(index))
                    .is_some_and(|style| style.centers_across_selection),
            });
            if tag.kind == Kind::Empty {
                self.finish_centered_cell();
            }
        } else if tag.kind != Kind::End && matches!(tag.local(), b"v" | b"f" | b"is") {
            if let Some(cell) = &mut self.pending_cell {
                cell.has_content = true;
            }
        }
        Ok(())
    }

    fn finish_centered_cell(&mut self) {
        if let Some(cell) = self.pending_cell.take() {
            if self.centered_span.observe_cell(
                cell.row,
                cell.column,
                cell.centered,
                cell.has_content,
            ) {
                include(&mut self.bounds, cell.row, cell.column);
            }
        }
    }

    pub fn include_rectangle(&mut self, top: u32, left: u32, bottom: u32, right: u32) {
        include(&mut self.bounds, top, left);
        include(&mut self.bounds, bottom, right);
    }

    /// A proven solid background has no finite lower or right edge to capture.
    /// Keep every serialized cell, merge/content extent and a background sample.
    pub fn finish_columns(
        &mut self,
        solid_fills: &BTreeSet<u32>,
        finite: Rectangle,
        allow_uniform: bool,
    ) {
        self.columns.finalize();
        let uniform = allow_uniform
            && self.columns_known
            && self.row_styles_known
            && self
                .columns
                .uniform_background_fill()
                .is_some_and(|fill| {
                    solid_fills.contains(&fill)
                        && self.row_styles.iter().all(|index| {
                            self.styles.get(*index).is_some_and(|style| {
                                style.fill_id == Some(fill)
                                    && style.has_visible_fill
                                    && !style.has_visible_border
                            })
                        })
                });
        let bounds = if uniform {
            self.include_rectangle(finite.top, finite.left, finite.bottom, finite.right);
            self.explicit_cells
        } else {
            self.column_bounds
        };
        if let Some(bounds) = bounds {
            self.include_rectangle(bounds.top, bounds.left, bounds.bottom, bounds.right);
        }
        if !uniform {
            if let Some(bounds) = self.row_bounds {
                self.include_rectangle(bounds.top, bounds.left, bounds.bottom, bounds.right);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regression::xml_views::Reader;

    #[test]
    fn uniform_background_keeps_explicit_overrides_and_conservative_exceptions() -> Result<()> {
        let base = Xf {
            fill_id: Some(2),
            apply_fill: true,
            has_visible_fill: true,
            ..Default::default()
        };
        for (columns, row, solid, allow, finite_expected) in [
            (
                r#"<col min="1" max="16384" style="1"/>"#,
                r#"<row r="3" s="1" customFormat="1"/>"#,
                true,
                true,
                true,
            ),
            (
                r#"<col min="1" max="16384" style="1"/>"#,
                "",
                true,
                true,
                true,
            ),
            (
                r#"<col min="2" max="16384" style="1"/>"#,
                "",
                true,
                true,
                false,
            ),
            (
                r#"<col min="1" max="16384" style="1"/>"#,
                r#"<row r="3" s="0"/>"#,
                true,
                true,
                false,
            ),
            (
                r#"<col min="1" max="16384" style="1"/>"#,
                "",
                false,
                true,
                false,
            ),
            (
                r#"<col min="1" max="16384" style="1"/>"#,
                "",
                true,
                false,
                false,
            ),
        ] {
            let styles = [Xf::default(), base.clone()];
            let mut bounds = VisualBounds::new(&styles);
            let xml = format!("{columns}{row}<c r=\"Z40\" s=\"0\"/>");
            let mut reader = Reader::<32>::new(xml.as_bytes())?;
            while let Some(tag) = reader.next()? {
                bounds.observe(tag)?;
            }
            let fills = if solid {
                BTreeSet::from([2])
            } else {
                BTreeSet::new()
            };
            bounds.finish_columns(
                &fills,
                Rectangle {
                    top: 1,
                    left: 1,
                    bottom: 10,
                    right: 6,
                },
                allow,
            );
            let extent = bounds.bounds.unwrap();
            if finite_expected {
                assert_eq!(
                    extent.address(),
                    "A1:Z40",
                    "explicit no-fill cells remain visible evidence"
                );
            } else {
                assert_eq!(extent.bottom, 1_048_576);
                assert_eq!(extent.right, 16_384);
            }
        }
        let styles = [Xf {
            has_visible_border: true,
            ..base
        }];
        let mut bounds = VisualBounds::new(&styles);
        let mut reader = Reader::<32>::new(br#"<col min="1" max="16384" style="0"/>"#)?;
        bounds.observe(reader.next()?.unwrap())?;
        bounds.finish_columns(
            &BTreeSet::from([2]),
            Rectangle {
                top: 1,
                left: 1,
                bottom: 10,
                right: 6,
            },
            true,
        );
        assert_eq!(bounds.bounds.unwrap().bottom, 1_048_576);
        Ok(())
    }

    #[test]
    fn centered_text_extends_the_envelope_without_retaining_unanchored_formats() -> Result<()> {
        let styles = [
            Xf::default(),
            Xf {
                centers_across_selection: true,
                ..Default::default()
            },
        ];
        let mut bounds = VisualBounds::new(&styles);
        let mut reader = Reader::<32>::new(br#"<row r="2"><c r="D2" s="1"><v>1</v></c><c r="E2" s="1"/><c r="F2" s="1"/><c r="G2" s="1"/><c r="I2" s="1"/></row><row r="3"><c r="XFD3" s="1"/></row>"#)?;
        while let Some(tag) = reader.next()? {
            bounds.observe(tag)?;
        }
        assert_eq!(bounds.bounds.unwrap().address(), "D2:G2");
        Ok(())
    }

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
        bounds.finish_columns(
            &BTreeSet::new(),
            Rectangle {
                top: 1,
                left: 1,
                bottom: 20,
                right: 9,
            },
            false,
        );
        assert_eq!(bounds.bounds.unwrap().address(), "C1:I1048576");
        Ok(())
    }
}
