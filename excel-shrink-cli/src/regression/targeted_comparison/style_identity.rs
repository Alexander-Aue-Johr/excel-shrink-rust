//! Resolve cell-format indices without treating a table permutation as a visual change.
use crate::regression::xml_views::{Attributes, Kind, Reader};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

mod components;
mod tables;

/// Identify simple solid fills for the uniform-background extent proof.
/// Patterned, gradient and extended fill records remain conservative.
pub(super) fn solid_fill_ids(bytes: &[u8]) -> Result<std::collections::BTreeSet<u32>> {
    let mut result = std::collections::BTreeSet::new();
    let Some(fills) = tables::Table::read(bytes, b"fills", b"fill")? else {
        return Ok(result);
    };
    if !fills.supported {
        return Ok(result);
    }
    for (index, range) in fills.entries.iter().enumerate() {
        let mut reader = Reader::<32>::new(&bytes[range.clone()])?;
        let mut solid = false;
        let mut patterns = 0;
        let mut supported = true;
        while let Some(tag) = reader.next()? {
            if tag.kind == Kind::End {
                continue;
            }
            match (tag.depth, tag.name) {
                (0, b"fill") => supported &= tag.attrs.iter().all(u8::is_ascii_whitespace),
                (1, b"patternFill") => {
                    patterns += 1;
                    solid = tag.attr(b"patternType")? == Some(b"solid");
                    for attribute in Attributes::new(tag.attrs) {
                        supported &= attribute?.0 == b"patternType";
                    }
                }
                (2, b"fgColor" | b"bgColor") if tag.kind == Kind::Empty => {
                    for attribute in Attributes::new(tag.attrs) {
                        supported &= matches!(
                            attribute?.0,
                            b"rgb" | b"indexed" | b"theme" | b"tint" | b"auto"
                        );
                    }
                }
                _ => supported = false,
            }
        }
        if solid && supported && patterns == 1 {
            result.insert(index as u32);
        }
    }
    Ok(result)
}

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
        let Some(components) = components::Components::read(bytes)? else {
            return Ok(Self::raw_fallback(bytes));
        };
        let sections: Vec<_> = std::iter::once(&cells)
            .chain(bases.iter())
            .chain(names.iter())
            .chain(components.tables.iter())
            .collect();
        let shared: [u8; 32] = Sha256::digest(tables::shared_context(bytes, &sections)).into();
        let bases = Self::base_formats(bytes, bases.as_ref(), &components, shared)?;
        let formats = cells
            .entries
            .iter()
            .map(|range| {
                let entry = components.resolve(bytes, &bytes[range.clone()])?;
                Self::resolved_format(&entry, &bases, shared)
            })
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

    fn base_formats(
        bytes: &[u8],
        table: Option<&tables::Table>,
        components: &components::Components,
        shared: [u8; 32],
    ) -> Result<Self> {
        let mut result = Self::default();
        for range in table.into_iter().flat_map(|table| &table.entries) {
            let mut hash = Sha256::new();
            hash.update(components.resolve(bytes, &bytes[range.clone()])?);
            hash.update(shared);
            result.formats.push(hash.finalize().into());
        }
        Ok(result)
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
        // Input-dependent proofs use this context plus their actual cell changes.
        // Input-independent and mixed proofs retain the complete format catalogue.
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
    fn background_proof_accepts_only_plain_solid_fills() -> anyhow::Result<()> {
        let xml = br#"<styleSheet><fills count="5"><fill><patternFill patternType="solid"><fgColor rgb="FFFFFFFF"/></patternFill></fill><fill><patternFill patternType="darkGrid"/></fill><fill><gradientFill/></fill><fill><patternFill patternType="solid"><ext/></patternFill></fill><fill><patternFill patternType="solid"><fgColor future="1"/></patternFill></fill></fills></styleSheet>"#;
        assert_eq!(
            super::solid_fill_ids(xml)?,
            std::collections::BTreeSet::from([0])
        );
        Ok(())
    }

    /// CALHR exports use an explicit closing numFmt tag, including for IDs
    /// below 164. Its spelling must not prevent comparison or hide its code.
    #[test]
    fn paired_empty_number_formats_preserve_identity() -> Result<()> {
        let xml = br#"<styleSheet><numFmts><numFmt numFmtId="82" formatCode="General"/></numFmts><cellXfs><xf numFmtId="82"/></cellXfs></styleSheet>"#;
        let paired = String::from_utf8(xml.to_vec())?
            .replace("formatCode=\"General\"/>", "formatCode=\"General\"> \n</numFmt>");
        let left = StyleIdentity::read(xml)?;
        let right = StyleIdentity::read(paired.as_bytes())?;
        assert_eq!(left.index_text(b"0")?, right.index_text(b"0")?);
        let changed = paired.replace("General", "0.00");
        assert_ne!(
            left.index_text(b"0")?,
            StyleIdentity::read(changed.as_bytes())?.index_text(b"0")?
        );
        let unexpected_body = paired.replace(" \n</numFmt>", "unexpected</numFmt>");
        assert!(StyleIdentity::read(unexpected_body.as_bytes()).is_err());
        Ok(())
    }

    #[test]
    fn component_reindexing_preserves_properties_but_not_actual_changes() -> Result<()> {
        let before = concat!(
            "<styleSheet><fonts><font><sz val=\"11\"/></font><font><b/></font></fonts>",
            "<fills><fill><patternFill patternType=\"none\"/></fill></fills>",
            "<borders><border/></borders>",
            "<numFmts><numFmt numFmtId=\"164\" formatCode=\"0.00\"/></numFmts>",
            "<cellStyleXfs><xf fontId=\"1\"/></cellStyleXfs>",
            "<cellXfs><xf fontId=\"1\" numFmtId=\"164\" xfId=\"0\"/></cellXfs></styleSheet>",
        );
        let after = before
            .replace(
                "<font><sz val=\"11\"/></font><font><b/></font>",
                "<font><b/></font><font><sz val=\"11\"/></font>",
            )
            .replace("fontId=\"1\"", "fontId=\"0\"")
            .replace("numFmtId=\"164\"", "numFmtId=\"180\"");
        let left = StyleIdentity::read(before.as_bytes())?;
        let right = StyleIdentity::read(after.as_bytes())?;
        assert_eq!(left.index_text(b"0")?, right.index_text(b"0")?);
        assert_eq!(left.context, right.context);
        for changed in [
            after.replace("<b/>", "<i/>"),
            after.replace("formatCode=\"0.00\"", "formatCode=\"0%\""),
            after.replace("patternType=\"none\"", "patternType=\"solid\""),
            after.replace("<border/>", "<border><left style=\"thin\"/></border>"),
        ] {
            assert_ne!(
                left.index_text(b"0")?,
                StyleIdentity::read(changed.as_bytes())?.index_text(b"0")?
            );
        }
        // An explicit definition can override an otherwise built-in number ID.
        let builtin_override = before.replace("164", "14");
        let changed_override = builtin_override.replace("0.00", "0%");
        assert_ne!(
            StyleIdentity::read(builtin_override.as_bytes())?.index_text(b"0")?,
            StyleIdentity::read(changed_override.as_bytes())?.index_text(b"0")?
        );
        Ok(())
    }

    #[test]
    fn implicit_component_zero_and_unknown_extension_links_remain_guarded() -> Result<()> {
        let before = "<styleSheet><fonts><font><sz val=\"11\"/></font></fonts><cellXfs><xf/></cellXfs></styleSheet>";
        let after = before.replace("11", "20");
        assert_ne!(
            StyleIdentity::read(before.as_bytes())?.index_text(b"0")?,
            StyleIdentity::read(after.as_bytes())?.index_text(b"0")?
        );
        let unknown = before.replace(
            "</styleSheet>",
            "<extLst><ext uri=\"unknown\"><fontLink fontId=\"0\"/></ext></extLst></styleSheet>",
        );
        assert!(StyleIdentity::read(unknown.as_bytes())?.formats.is_empty());
        assert!(StyleIdentity::read(b"<styleSheet><fonts><font/></fonts><cellXfs><xf fontId=\"99\"/></cellXfs></styleSheet>").is_err());
        Ok(())
    }

    #[test]
    fn named_style_revision_ids_require_the_actual_revision_namespace() -> Result<()> {
        let xml = r#"<styleSheet xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision"><cellXfs><xf/></cellXfs><cellStyles><cellStyle name="Normal" xfId="0" xr:uid="abc"/></cellStyles></styleSheet>"#;
        let identity = StyleIdentity::read(xml.as_bytes())?;
        assert!(!identity.formats.is_empty());
        assert_eq!(
            identity.context,
            StyleIdentity::read(xml.replace("abc", "def").as_bytes())?.context
        );
        let unknown = xml.replace(
            "http://schemas.microsoft.com/office/spreadsheetml/2014/revision",
            "urn:unknown",
        );
        assert!(StyleIdentity::read(unknown.as_bytes())?.formats.is_empty());
        let known_extension = xml.replace("</styleSheet>",
            r#"<extLst><ext uri="{EB79DEF2-80B8-43e5-95BD-54CBDDF9020C}"><x14:slicerStyles defaultSlicerStyle="SlicerStyleLight1" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"/></ext></extLst></styleSheet>"#);
        assert!(!StyleIdentity::read(known_extension.as_bytes())?
            .formats
            .is_empty());
        let unknown_property = known_extension.replace("defaultSlicerStyle=", "fontId=");
        assert!(StyleIdentity::read(unknown_property.as_bytes())?
            .formats
            .is_empty());
        Ok(())
    }

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
        let a = StyleIdentity::read(br#"<styleSheet><cellXfs count="2"><xf fontId="0"/><xf fontId="1"><alignment wrapText="1"/></xf></cellXfs></styleSheet>"#)?;
        let b = StyleIdentity::read(br#"<styleSheet><cellXfs count="2"><xf fontId="1"><alignment wrapText="1"/></xf><xf fontId="0"/></cellXfs></styleSheet>"#)?;
        assert_eq!(a.context, b.context);
        assert_eq!(
            a.attributes(br#" r="A1" s="0" t="n""#, b"s")?,
            b.attributes(br#" r="A1" s="1" t="n""#, b"s")?
        );
        assert_ne!(
            a.attributes(br#" s="0""#, b"s")?,
            b.attributes(br#" s="0""#, b"s")?
        );
        let changed = StyleIdentity::read(br#"<styleSheet><cellXfs count="2"><xf fontId="0"/><xf fontId="2"><alignment wrapText="1"/></xf></cellXfs></styleSheet>"#)?;
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
