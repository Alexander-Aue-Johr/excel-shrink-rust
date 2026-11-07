//! Select layout changes in the compared states independently of save-control diagnostics.
use super::{content_bounds::ContentBounds, xml_changes::Snapshot, Candidate};
use anyhow::{ensure, Result};
use std::collections::BTreeSet;

/// Changed inherited formats remain candidates even when every cell XML is unchanged.
/// Effective heights are checked separately against the live Excel workbooks.
pub(super) fn select(
    before: &Snapshot,
    after: &Snapshot,
    content: &ContentBounds,
    candidates: &mut BTreeSet<Candidate>,
) -> Result<()> {
    ensure!(
        before.sheets.keys().eq(after.sheets.keys()),
        "Layout partner sheets are missing"
    );
    for (name, left) in &before.sheets {
        let right = &after.sheets[name];
        let Some(bounds) = content.envelope(name) else {
            continue;
        };
        let mut add = |area: super::content_bounds::Rectangle| {
            candidates.insert(Candidate {
                sheet: name.clone(),
                cell: area.address(),
            });
        };
        if before.layout_context != after.layout_context
            || left.defaults != right.defaults
            || left.cf_layout != right.cf_layout
        {
            add(bounds);
            continue;
        }
        let Some(columns) = left
            .cf_columns
            .changed_spans(&right.cf_columns, bounds.right)
        else {
            add(bounds);
            continue;
        };
        for (first, last) in columns {
            if first < bounds.left {
                // Earlier columns can move all later content, including its first visible cell.
                add(bounds);
            } else {
                add(super::content_bounds::Rectangle {
                    left: first,
                    right: last,
                    ..bounds
                });
            }
        }
        let rows: BTreeSet<_> = left.rows.keys().chain(right.rows.keys()).copied().collect();
        for row in rows {
            if row < bounds.top || row > bounds.bottom {
                continue;
            }
            let changed = ["s", "customFormat", "hidden"]
                .into_iter()
                .any(|attribute| {
                    left.rows.get(&row).and_then(|attrs| attrs.get(attribute))
                        != right.rows.get(&row).and_then(|attrs| attrs.get(attribute))
                });
            if changed {
                add(super::content_bounds::Rectangle {
                    top: row,
                    bottom: row,
                    ..bounds
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn snapshot() -> Snapshot {
        let sheet = super::super::xml_changes::Sheet {
            visual_bounds: Some(super::super::content_bounds::Rectangle {
                top: 10,
                bottom: 30,
                left: 3,
                right: 5,
            }),
            cf_columns: super::super::column_layout::ColumnLayout::new(),
            ..Default::default()
        };
        Snapshot {
            sheets: [("Data".into(), sheet)].into(),
            context: "same".into(),
            layout_context: "same".into(),
            cf_context: "same".into(),
            sheet_names: None,
            defined_names: None,
        }
    }

    #[test]
    fn inherited_row_style_changes_select_the_actual_content_row() -> Result<()> {
        let before = snapshot();
        let mut after = before.clone();
        Arc::make_mut(&mut after.sheets.get_mut("Data").unwrap().rows)
            .insert(20, [("s".into(), "different actual format".into())].into());
        let mut bounds = ContentBounds::default();
        bounds.include(&before);
        bounds.include(&after);
        let mut candidates = BTreeSet::new();
        select(&before, &after, &bounds, &mut candidates)?;
        assert_eq!(
            candidates,
            [Candidate {
                sheet: "Data".into(),
                cell: "C20:E20".into()
            }]
            .into()
        );
        Ok(())
    }

    #[test]
    fn unknown_layout_and_global_context_require_content_coverage() -> Result<()> {
        let before = snapshot();
        let mut after = before.clone();
        after.layout_context = "different global rendering context".into();
        let mut bounds = ContentBounds::default();
        bounds.include(&before);
        let mut candidates = BTreeSet::new();
        select(&before, &after, &bounds, &mut candidates)?;
        assert_eq!(
            candidates,
            [Candidate {
                sheet: "Data".into(),
                cell: "C10:E30".into()
            }]
            .into()
        );
        Ok(())
    }

    #[test]
    fn column_changes_outside_content_are_excluded_and_interior_changes_are_local() -> Result<()> {
        let before = snapshot();
        let mut after = before.clone();
        let mut columns = super::super::column_layout::ColumnLayout::new();
        let mut reader = crate::regression::xml_views::Reader::<16>::new(
            br#"<cols><col min="4" max="4" width="22"/><col min="9" max="100" width="30"/></cols>"#,
        )?;
        while let Some(tag) = reader.next()? {
            if tag.name == b"col" && tag.kind == crate::regression::xml_views::Kind::Empty {
                columns.observe(tag, &super::super::style_identity::StyleIdentity::default())?;
            }
        }
        after.sheets.get_mut("Data").unwrap().cf_columns = columns;
        let mut bounds = ContentBounds::default();
        bounds.include(&before);
        let mut candidates = BTreeSet::new();
        select(&before, &after, &bounds, &mut candidates)?;
        assert_eq!(
            candidates,
            [Candidate {
                sheet: "Data".into(),
                cell: "D10:D30".into()
            }]
            .into()
        );
        Ok(())
    }
    #[test]
    fn unused_format_catalogue_changes_do_not_expand_layout_coverage() -> Result<()> {
        let before = snapshot();
        let mut after = before.clone();
        after.context = "unused format was removed".into();
        let mut bounds = ContentBounds::default();
        bounds.include(&before);
        let mut candidates = BTreeSet::new();
        select(&before, &after, &bounds, &mut candidates)?;
        assert!(candidates.is_empty());
        Ok(())
    }

    /// Replay retained Excel saves without launching or changing Excel.
    #[test]
    #[ignore = "requires EXCEL_SHRINK_LAYOUT_PAIR_FIXTURE containing rebuilt workbooks"]
    fn retained_unused_hyperlink_format_does_not_select_the_whole_sheet() -> Result<()> {
        let root = std::path::PathBuf::from(std::env::var("EXCEL_SHRINK_LAYOUT_PAIR_FIXTURE")?);
        let before = Snapshot::read(&root.join("OR-rebuilt.xlsx"))?;
        let after = Snapshot::read(&root.join("SDR-rebuilt.xlsx"))?;
        assert_ne!(before.context, after.context);
        assert_eq!(before.layout_context, after.layout_context);
        let mut bounds = ContentBounds::default();
        bounds.include(&before);
        bounds.include(&after);
        let mut candidates = super::super::xml_changes::cell_candidates(
            &before.content_changes(&after)?.0,
        );
        select(&before, &after, &bounds, &mut candidates)?;
        let (selected, _) = bounds.select(&candidates)?;
        println!("{} selected regions", selected.len());
        assert!(!selected.is_empty(), "deleted styled cells still need evidence");
        assert!(selected.len() < 20, "unused formats must not select a million-row sheet");
        Ok(())
    }

}
