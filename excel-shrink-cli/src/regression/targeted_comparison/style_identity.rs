//! Resolve cell-format indices without treating a table permutation as a visual change.
use crate::regression::xml_views::{Attributes, Kind, Reader};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

mod tables;

#[derive(Default)]
pub(super) struct StyleIdentity {
    formats: Vec<[u8; 32]>,
    pub context: Vec<u8>,
    pub conditional_context: Vec<u8>,
}

impl StyleIdentity {
    /// Resolve applied formats through their base xf; unrelated named templates
    /// cannot change the identity of every cell or every conditional-format rule.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let Some(cells) = tables::Table::read(bytes, b"cellXfs", b"xf")? else {
            return Ok(Self::raw_fallback(bytes));
        };
        let bases = tables::Table::read(bytes, b"cellStyleXfs", b"xf")?;
        let names = tables::Table::read(bytes, b"cellStyles", b"cellStyle")?;
        if !tables::supported(bytes, &cells, bases.as_ref(), names.as_ref())? {
            return Ok(Self::raw_fallback(bytes));
        }
        let sections: Vec<_> = std::iter::once(&cells)
            .chain(bases.iter())
            .chain(names.iter())
            .collect();
        let shared: [u8; 32] = Sha256::digest(tables::shared_context(bytes, &sections)).into();
        let bases = Self::base_formats(bytes, bases.as_ref(), shared);
        let formats = cells
            .entries
            .iter()
            .map(|range| Self::resolved_format(&bytes[range.clone()], &bases, shared))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::from_resolved_formats(shared, formats))
    }

    fn raw_fallback(bytes: &[u8]) -> Self {
        Self {
            context: bytes.to_vec(),
            conditional_context: bytes.to_vec(),
            ..Self::default()
        }
    }

    fn base_formats(bytes: &[u8], table: Option<&tables::Table>, shared: [u8; 32]) -> Self {
        let mut result = Self::default();
        for range in table.into_iter().flat_map(|table| &table.entries) {
            let mut hash = Sha256::new();
            hash.update(&bytes[range.clone()]);
            hash.update(shared);
            result.formats.push(hash.finalize().into());
        }
        result
    }

    /// Preserve every XF property while replacing its base index with that base's identity.
    fn resolved_format(entry: &[u8], bases: &Self, shared: [u8; 32]) -> Result<[u8; 32]> {
        let mut reader = Reader::<128>::new(entry)?;
        let opening = reader.next()?.context("Missing cell format element")?;
        let mut hash = Sha256::new();
        hash.update(&entry[..opening.name.len() + 1]);
        bases.hash_attributes(&mut hash, opening.attrs, b"xfId")?;
        let attributes_end =
            opening.attrs.as_ptr() as usize - entry.as_ptr() as usize + opening.attrs.len();
        hash.update(&entry[attributes_end..]);
        if opening.attr(b"xfId")?.is_none() {
            if let Some(default_base) = bases.formats.first() {
                hash.update(default_base);
            }
        }
        hash.update(shared);
        Ok(hash.finalize().into())
    }

    fn from_resolved_formats(shared: [u8; 32], formats: Vec<[u8; 32]>) -> Self {
        let mut result = Self {
            formats,
            ..Self::default()
        };
        result.context.extend_from_slice(&shared);
        let mut unique = result.formats.clone();
        unique.sort_unstable();
        unique.dedup();
        for identity in unique {
            result.context.extend_from_slice(&identity);
        }
        // Only constant cellIs proofs use this narrower rendering context.
        // Other CF families retain the complete style-context safeguard.
        result.conditional_context.extend_from_slice(&shared);
        if let Some(default) = result.formats.first() {
            result.conditional_context.extend_from_slice(default);
        }
        result
    }

    /// Hash the original attribute bytes, replacing only a resolved numeric index.
    /// Unknown or absent style tables retain the raw conservative representation.
    pub fn attributes(&self, attrs: &[u8], key: &[u8]) -> Result<Vec<u8>> {
        let mut output = Vec::with_capacity(attrs.len() + 32);
        self.visit_attributes(attrs, key, |bytes| output.extend_from_slice(bytes))?;
        Ok(output)
    }

    pub fn hash_attributes(&self, hash: &mut Sha256, attrs: &[u8], key: &[u8]) -> Result<()> {
        self.visit_attributes(attrs, key, |bytes| hash.update(bytes))
    }

    /// Cells without an explicit s attribute still inherit the implicit default.
    pub fn hash_cell_attributes(&self, hash: &mut Sha256, attrs: &[u8]) -> Result<()> {
        self.hash_attributes(hash, attrs, b"s")?;
        for attribute in Attributes::new(attrs) {
            if attribute?.0 == b"s" {
                return Ok(());
            }
        }
        if let Some(default) = self.formats.first() {
            hash.update(default);
        }
        Ok(())
    }

    pub fn index_text(&self, value: &[u8]) -> Result<String> {
        if self.formats.is_empty() {
            return Ok(std::str::from_utf8(value)?.into());
        }
        let index: usize = std::str::from_utf8(value)?.parse()?;
        let identity = self
            .formats
            .get(index)
            .context("Cell format index outside cellXfs")?;
        Ok(String::from_utf8(hex(identity).to_vec())?)
    }

    fn visit_attributes(
        &self,
        attrs: &[u8],
        key: &[u8],
        mut emit: impl FnMut(&[u8]),
    ) -> Result<()> {
        let tag = crate::regression::xml_views::Tag {
            name: b"",
            attrs,
            start: 0,
            end: 0,
            depth: 0,
            kind: Kind::Empty,
        };
        if self.formats.is_empty() {
            emit(attrs);
            return Ok(());
        }
        let Some(value) = tag.attr(key)? else {
            emit(attrs);
            return Ok(());
        };
        let index: usize = std::str::from_utf8(value)?.parse()?;
        let identity = self
            .formats
            .get(index)
            .context("Cell format index outside cellXfs")?;
        let offset = value.as_ptr() as usize - attrs.as_ptr() as usize;
        emit(&attrs[..offset]);
        emit(&hex(identity));
        emit(&attrs[offset + value.len()..]);
        Ok(())
    }
}

fn hex(identity: &[u8; 32]) -> [u8; 64] {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut result = [0; 64];
    for (index, byte) in identity.iter().enumerate() {
        result[index * 2] = DIGITS[usize::from(byte >> 4)];
        result[index * 2 + 1] = DIGITS[usize::from(byte & 15)];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_format_indices_resolve_to_properties_and_unused_templates_can_disappear() -> Result<()>
    {
        let left = StyleIdentity::read(br#"<styleSheet><cellStyleXfs count="3"><xf fontId="0"/><xf fontId="1"/><xf fontId="2"/></cellStyleXfs><cellXfs count="2"><xf xfId="0"/><xf xfId="1"/></cellXfs><cellStyles count="2"><cellStyle name="Normal" xfId="0"/><cellStyle name="Unused" xfId="2"/></cellStyles></styleSheet>"#)?;
        let right = StyleIdentity::read(br#"<styleSheet><cellStyleXfs count="2"><xf fontId="1"/><xf fontId="0"/></cellStyleXfs><cellXfs count="2"><xf xfId="1"/><xf xfId="0"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="1"/></cellStyles></styleSheet>"#)?;
        assert_eq!(left.index_text(b"0")?, right.index_text(b"0")?);
        assert_eq!(left.index_text(b"1")?, right.index_text(b"1")?);
        assert_eq!(left.context, right.context);
        assert_eq!(left.conditional_context, right.conditional_context);
        Ok(())
    }

    #[test]
    fn removed_unused_cell_format_does_not_change_constant_rule_context() -> Result<()> {
        let left = StyleIdentity::read(br#"<styleSheet><cellXfs count="2"><xf fontId="0"/><xf fontId="1"/></cellXfs></styleSheet>"#)?;
        let right = StyleIdentity::read(
            br#"<styleSheet><cellXfs count="1"><xf fontId="0"/></cellXfs></styleSheet>"#,
        )?;
        assert_ne!(left.context, right.context);
        assert_eq!(left.conditional_context, right.conditional_context);
        Ok(())
    }

    #[test]
    fn changed_used_base_and_implicit_default_are_not_hidden() -> Result<()> {
        let left = StyleIdentity::read(br#"<styleSheet><cellStyleXfs count="1"><xf fontId="0"/></cellStyleXfs><cellXfs count="1"><xf xfId="0"/></cellXfs></styleSheet>"#)?;
        let right = StyleIdentity::read(br#"<styleSheet><cellStyleXfs count="1"><xf fontId="1"/></cellStyleXfs><cellXfs count="1"><xf xfId="0"/></cellXfs></styleSheet>"#)?;
        assert_ne!(left.index_text(b"0")?, right.index_text(b"0")?);
        assert_ne!(left.conditional_context, right.conditional_context);
        let mut before = Sha256::new();
        let mut after = Sha256::new();
        left.hash_cell_attributes(&mut before, br#" r="A1""#)?;
        right.hash_cell_attributes(&mut after, br#" r="A1""#)?;
        assert_ne!(before.finalize(), after.finalize());
        Ok(())
    }

    #[test]
    fn unknown_style_table_children_keep_the_raw_fallback() -> Result<()> {
        let xml =
            br#"<styleSheet><cellXfs count="1"><unknown/><xf fontId="0"/></cellXfs></styleSheet>"#;
        let identity = StyleIdentity::read(xml)?;
        assert!(identity.formats.is_empty());
        assert_eq!(identity.context, xml);
        assert_eq!(identity.conditional_context, xml);
        assert!(StyleIdentity::read(br#"<styleSheet><cellXfs/><cellXfs/></styleSheet>"#).is_err());
        for xml in [
            br#"<styleSheet><cellXfs><xf><extLst><ext/></extLst></xf></cellXfs></styleSheet>"#.as_slice(),
            br#"<styleSheet><cellXfs><xf mystery="1"/></cellXfs></styleSheet>"#,
            br#"<styleSheet><cellStyleXfs><unknown/></cellStyleXfs><cellXfs><xf/></cellXfs></styleSheet>"#,
            br#"<styleSheet><cellXfs><xf/></cellXfs><cellStyles><cellStyle name="Normal"><extLst/></cellStyle></cellStyles></styleSheet>"#,
        ] {
            let identity = StyleIdentity::read(xml)?;
            assert!(identity.formats.is_empty());
            assert_eq!(identity.context, xml);
            assert_eq!(identity.conditional_context, xml);
        }
        Ok(())
    }

    #[test]
    fn reordered_formats_keep_identity_but_changed_definitions_do_not() -> Result<()> {
        let a = StyleIdentity::read(br#"<styleSheet><fonts count="1"/><cellXfs count="2"><xf fontId="0"/><xf fontId="1"><alignment wrapText="1"/></xf></cellXfs></styleSheet>"#)?;
        let b = StyleIdentity::read(br#"<styleSheet><fonts count="1"/><cellXfs count="2"><xf fontId="1"><alignment wrapText="1"/></xf><xf fontId="0"/></cellXfs></styleSheet>"#)?;
        assert_eq!(a.context, b.context);
        assert_eq!(
            a.attributes(br#" r="A1" s="0" t="n""#, b"s")?,
            b.attributes(br#" r="A1" s="1" t="n""#, b"s")?
        );
        assert_ne!(
            a.attributes(br#" s="0""#, b"s")?,
            b.attributes(br#" s="0""#, b"s")?
        );
        let changed = StyleIdentity::read(br#"<styleSheet><fonts count="1"/><cellXfs count="2"><xf fontId="0"/><xf fontId="2"><alignment wrapText="1"/></xf></cellXfs></styleSheet>"#)?;
        assert_ne!(a.context, changed.context);
        assert!(a.attributes(br#" s="99""#, b"s").is_err());
        Ok(())
    }

    #[test]
    fn shared_style_dependencies_cannot_be_hidden_by_matching_xf_bytes() -> Result<()> {
        let left = StyleIdentity::read(br#"<styleSheet><fonts><font><sz val="11"/></font></fonts><cellXfs count="1"><xf fontId="0"/></cellXfs></styleSheet>"#)?;
        let right = StyleIdentity::read(br#"<styleSheet><fonts><font><sz val="20"/></font></fonts><cellXfs count="1"><xf fontId="0"/></cellXfs></styleSheet>"#)?;
        assert_ne!(left.context, right.context);
        assert_ne!(left.index_text(b"0")?, right.index_text(b"0")?);
        Ok(())
    }

    #[test]
    fn duplicate_entries_do_not_change_format_meaning() -> Result<()> {
        let left = StyleIdentity::read(br#"<styleSheet><cellXfs count="3"><xf fontId="0"/><xf fontId="0"/><xf fontId="1"/></cellXfs></styleSheet>"#)?;
        let right = StyleIdentity::read(br#"<styleSheet><cellXfs count="3"><xf fontId="0"/><xf fontId="1"/><xf fontId="1"/></cellXfs></styleSheet>"#)?;
        assert_eq!(left.context, right.context);
        assert_eq!(left.index_text(b"2")?, right.index_text(b"1")?);
        Ok(())
    }
}
