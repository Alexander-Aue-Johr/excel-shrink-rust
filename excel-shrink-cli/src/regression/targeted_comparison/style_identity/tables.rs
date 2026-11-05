//! Borrowed style-table extents; unknown direct children retain the raw fallback.
use crate::regression::xml_views::{Attributes, Kind, Reader};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::ops::Range;

pub(super) struct Table {
    pub range: Range<usize>,
    pub entries: Vec<Range<usize>>,
    pub supported: bool,
    pub(super) name: &'static [u8],
    attributes: [u8; 32],
}

impl Table {
    pub fn read(bytes: &[u8], name: &'static [u8], child: &[u8]) -> Result<Option<Self>> {
        let mut reader = Reader::<128>::new(bytes)?;
        let mut table = None;
        let mut entry = None;
        let mut supported = true;
        let mut active = false;
        while let Some(tag) = reader.next()? {
            if tag.depth == 1 && tag.name == name {
                match tag.kind {
                    Kind::Start | Kind::Empty => {
                        ensure!(table.is_none(), "Duplicate style table");
                        let mut attributes = Sha256::new();
                        for attribute in Attributes::new(tag.attrs) {
                            let (key, value) = attribute?;
                            // Count is bookkeeping; every actual entry is inspected.
                            if key != b"count" {
                                attributes.update((key.len() as u64).to_le_bytes());
                                attributes.update(key);
                                attributes.update((value.len() as u64).to_le_bytes());
                                attributes.update(value);
                            }
                        }
                        active = tag.kind == Kind::Start;
                        table = Some(Self {
                            range: tag.start..tag.end,
                            entries: Vec::new(),
                            name,
                            supported: true,
                            attributes: attributes.finalize().into(),
                        });
                    }
                    Kind::End => {
                        table
                            .as_mut()
                            .context("Unexpected style table end")?
                            .range
                            .end = tag.end;
                        active = false;
                    }
                }
            } else if active && tag.depth == 2 {
                if let Some(table) = &mut table {
                    if tag.name != child {
                        supported = false;
                    }
                    match tag.kind {
                        Kind::Start => entry = Some(tag.start),
                        Kind::Empty => table.entries.push(tag.start..tag.end),
                        Kind::End => table
                            .entries
                            .push(entry.take().context("Unexpected style entry end")?..tag.end),
                    }
                }
            }
        }
        if let Some(table) = &mut table {
            table.supported = supported;
        }
        Ok(table)
    }
}

/// Index-table order/count and named-style labels do not describe an applied format.
/// Base definitions are instead incorporated into each referencing cell-format key.
pub(super) fn shared_context(bytes: &[u8], tables: &[&Table]) -> Vec<u8> {
    let mut tables = tables.to_vec();
    tables.sort_by_key(|table| table.range.start);
    let mut result = Vec::new();
    let mut offset = 0;
    for table in tables {
        result.extend_from_slice(&bytes[offset..table.range.start]);
        result.extend_from_slice(table.name);
        result.extend_from_slice(&table.attributes);
        offset = table.range.end;
    }
    result.extend_from_slice(&bytes[offset..]);
    result
}

/// Unknown extensions may carry extra index dependencies; retain the raw safeguard.
pub(super) fn supported_format(bytes: &[u8]) -> Result<bool> {
    let mut reader = Reader::<128>::new(bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End {
            continue;
        }
        let allowed: &[&[u8]] = match (tag.depth, tag.name) {
            (0, b"xf") => &[
                b"numFmtId",
                b"fontId",
                b"fillId",
                b"borderId",
                b"xfId",
                b"applyNumberFormat",
                b"applyFont",
                b"applyFill",
                b"applyBorder",
                b"applyAlignment",
                b"applyProtection",
                b"pivotButton",
                b"quotePrefix",
            ],
            (1, b"alignment") => &[
                b"horizontal",
                b"vertical",
                b"textRotation",
                b"wrapText",
                b"shrinkToFit",
                b"indent",
                b"relativeIndent",
                b"justifyLastLine",
                b"readingOrder",
                b"mergeCell",
            ],
            (1, b"protection") => &[b"locked", b"hidden"],
            _ => return Ok(false),
        };
        for attribute in Attributes::new(tag.attrs) {
            if !allowed.contains(&attribute?.0) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn supported_named_style(bytes: &[u8], revision_ids: bool) -> Result<bool> {
    let mut reader = Reader::<128>::new(bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End {
            continue;
        }
        if tag.depth != 0 || tag.name != b"cellStyle" {
            return Ok(false);
        }
        for attribute in Attributes::new(tag.attrs) {
            let (key, _) = attribute?;
            // Excel's revision GUID identifies an editing record, not a format.
            if revision_ids && key == b"xr:uid" {
                continue;
            }
            if ![
                b"name".as_slice(),
                b"xfId",
                b"builtinId",
                b"iLevel",
                b"hidden",
                b"customBuiltin",
            ]
            .contains(&key)
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Require known format links before removing any index table from the global context.
pub(super) fn supported(
    bytes: &[u8],
    cells: &Table,
    bases: Option<&Table>,
    names: Option<&Table>,
) -> Result<bool> {
    if !cells.supported
        || bases
            .iter()
            .chain(names.iter())
            .any(|table| !table.supported)
    {
        return Ok(false);
    }
    for range in cells
        .entries
        .iter()
        .chain(bases.into_iter().flat_map(|table| &table.entries))
    {
        if !supported_format(&bytes[range.clone()])? {
            return Ok(false);
        }
    }
    let mut reader = Reader::<128>::new(bytes)?;
    let root = reader.next()?.context("Missing stylesheet root")?;
    let revision_ids = root.attr(b"xmlns:xr")?
        == Some(b"http://schemas.microsoft.com/office/spreadsheetml/2014/revision".as_slice());
    for range in names.into_iter().flat_map(|table| &table.entries) {
        if !supported_named_style(&bytes[range.clone()], revision_ids)? {
            return Ok(false);
        }
    }
    Ok(true)
}
