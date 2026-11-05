//! Resolve the component indices used by cell and base formats.
use super::tables::Table;
use crate::regression::xml_views::{Attributes, Kind, Reader};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};

pub(super) struct Components {
    pub tables: Vec<Table>,
}

impl Components {
    /// Keep unknown table shapes on the raw-context path rather than hiding links.
    pub fn read(bytes: &[u8]) -> Result<Option<Self>> {
        if !known_global_links(bytes)? {
            return Ok(None);
        }
        let mut tables = Vec::new();
        for (name, child) in [
            (b"fonts".as_slice(), b"font".as_slice()),
            (b"fills", b"fill"),
            (b"borders", b"border"),
            (b"numFmts", b"numFmt"),
        ] {
            if let Some(table) = Table::read(bytes, name, child)? {
                if !table.supported {
                    return Ok(None);
                }
                for range in &table.entries {
                    if !supported_component(&bytes[range.clone()])? {
                        return Ok(None);
                    }
                }
                tables.push(table);
            }
        }
        Ok(Some(Self { tables }))
    }

    /// Replace only outer XF references; all apply flags and child properties survive.
    pub fn resolve(&self, bytes: &[u8], entry: &[u8]) -> Result<Vec<u8>> {
        let opening = Reader::<128>::new(entry)?
            .next()?
            .context("Missing style component owner")?;
        let mut output = Vec::with_capacity(entry.len() + 128);
        let mut copied = 0;
        let mut present = Vec::new();
        for attribute in Attributes::new(opening.attrs) {
            let (key, value) = attribute?;
            present.push(key);
            let (table_name, custom_number) = match key {
                b"fontId" => (b"fonts".as_slice(), false),
                b"fillId" => (b"fills".as_slice(), false),
                b"borderId" => (b"borders".as_slice(), false),
                b"numFmtId" => (b"numFmts".as_slice(), true),
                _ => continue,
            };
            let Some(table) = self.tables.iter().find(|table| table.name == table_name) else {
                continue;
            };
            let index: usize = std::str::from_utf8(value)?.parse()?;
            let component = if custom_number {
                match Self::number_format(bytes, table, value)? {
                    Some(definition) => definition,
                    None if index < 164 => continue,
                    None => anyhow::bail!("Missing custom number format definition"),
                }
            } else {
                let range = table
                    .entries
                    .get(index)
                    .context("Style component index outside table")?;
                bytes[range.clone()].to_vec()
            };
            let offset = value.as_ptr() as usize - entry.as_ptr() as usize;
            output.extend_from_slice(&entry[copied..offset]);
            output.extend_from_slice(&super::hex(&Sha256::digest(component).into()));
            copied = offset + value.len();
        }
        let attributes_end =
            opening.attrs.as_ptr() as usize - entry.as_ptr() as usize + opening.attrs.len();
        output.extend_from_slice(&entry[copied..attributes_end]);
        for (key, table_name) in [
            (b"fontId".as_slice(), b"fonts".as_slice()),
            (b"fillId", b"fills"),
            (b"borderId", b"borders"),
        ] {
            if present.contains(&key) {
                continue;
            }
            if let Some(table) = self.tables.iter().find(|table| table.name == table_name) {
                let range = table
                    .entries
                    .first()
                    .context("Missing implicit style component")?;
                output.push(b' ');
                output.extend_from_slice(key);
                output.extend_from_slice(b"=\"");
                output
                    .extend_from_slice(&super::hex(&Sha256::digest(&bytes[range.clone()]).into()));
                output.push(b'"');
            }
        }
        output.extend_from_slice(&entry[attributes_end..]);
        Ok(output)
    }

    /// Custom number-format IDs are arbitrary; retain the complete remaining attributes.
    fn number_format(bytes: &[u8], table: &Table, id: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut found = None;
        for range in &table.entries {
            let mut reader = Reader::<128>::new(&bytes[range.clone()])?;
            let tag = reader.next()?.context("Missing number format")?;
            if tag.attr(b"numFmtId")? != Some(id) {
                continue;
            }
            ensure!(found.is_none(), "Duplicate custom number format ID");
            ensure!(
                tag.kind == Kind::Empty,
                "Unsupported custom number format body"
            );
            let mut definition = Vec::new();
            for attribute in Attributes::new(tag.attrs) {
                let (key, value) = attribute?;
                if key != b"numFmtId" {
                    definition.extend_from_slice(&(key.len() as u64).to_le_bytes());
                    definition.extend_from_slice(key);
                    definition.extend_from_slice(&(value.len() as u64).to_le_bytes());
                    definition.extend_from_slice(value);
                }
            }
            found = Some(definition);
        }
        Ok(found)
    }
}

/// Recognized component properties cannot contain hidden links to another index table.
fn supported_component(bytes: &[u8]) -> Result<bool> {
    let mut reader = Reader::<128>::new(bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End {
            continue;
        }
        let allowed: &[&[u8]] = match tag.name {
            b"font" | b"fill" => &[],
            b"name" | b"sz" | b"family" | b"charset" | b"scheme" | b"u" | b"vertAlign" | b"b"
            | b"i" | b"strike" | b"outline" | b"shadow" | b"condense" | b"extend" => &[b"val"],
            b"color" | b"fgColor" | b"bgColor" => &[b"rgb", b"indexed", b"auto", b"theme", b"tint"],
            b"patternFill" => &[b"patternType"],
            b"gradientFill" => &[b"type", b"degree", b"left", b"right", b"top", b"bottom"],
            b"stop" => &[b"position"],
            b"border" => &[b"diagonalUp", b"diagonalDown", b"outline"],
            b"left" | b"right" | b"top" | b"bottom" | b"diagonal" | b"vertical" | b"horizontal"
            | b"start" | b"end" => &[b"style"],
            b"numFmt" => &[b"numFmtId", b"formatCode"],
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

/// Unknown extensions could reference component indices outside an XF.
/// Accept only the observed empty built-in slicer/timeline default declarations.
fn known_global_links(bytes: &[u8]) -> Result<bool> {
    let mut reader = Reader::<128>::new(bytes)?;
    let mut extension = false;
    while let Some(tag) = reader.next()? {
        if tag.depth == 1 {
            extension = tag.name == b"extLst" && tag.kind == Kind::Start;
            if ![
                b"numFmts".as_slice(),
                b"fonts",
                b"fills",
                b"borders",
                b"cellStyleXfs",
                b"cellXfs",
                b"cellStyles",
                b"dxfs",
                b"tableStyles",
                b"colors",
                b"extLst",
            ]
            .contains(&tag.name)
            {
                return Ok(false);
            }
        } else if extension && tag.kind != Kind::End {
            let allowed = match (tag.depth, tag.local(), tag.kind) {
                (2, b"ext", Kind::Start) => b"uri".as_slice(),
                (3, b"slicerStyles", Kind::Empty) => b"defaultSlicerStyle".as_slice(),
                (3, b"timelineStyles", Kind::Empty) => b"defaultTimelineStyle".as_slice(),
                _ => return Ok(false),
            };
            for attribute in Attributes::new(tag.attrs) {
                let (key, _) = attribute?;
                // Declarations remain in the shared raw extension bytes.
                if key != allowed && key != b"xmlns" && !key.starts_with(b"xmlns:") {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}
