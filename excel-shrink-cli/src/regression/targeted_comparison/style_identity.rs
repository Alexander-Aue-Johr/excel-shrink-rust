//! Resolve cell-format indices without treating a table permutation as a visual change.
use crate::regression::xml_views::{Kind, Reader};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};

#[derive(Default)]
pub(super) struct StyleIdentity {
    formats: Vec<[u8; 32]>,
    pub context: Vec<u8>,
}

impl StyleIdentity {
    /// Keep every format byte and every other stylesheet component in the evidence.
    /// Only the order of complete cellXfs entries is normalized; their references
    /// to fonts, fills, borders and base styles remain unchanged.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let mut result = Self::default();
        let mut reader = Reader::<128>::new(bytes)?;
        let mut container = None;
        let mut entry = None;
        let mut body = None;
        let mut body_range = None;
        while let Some(tag) = reader.next()? {
            if tag.name == b"cellXfs" && tag.depth == 1 {
                match tag.kind {
                    Kind::Start => {
                        ensure!(container.is_none(), "Duplicate cellXfs table");
                        container = Some(tag.depth);
                        body = Some(tag.end);
                    }
                    Kind::End => {
                        let start = body.context("Unexpected cellXfs end")?;
                        body_range = Some((start, tag.start));
                        container = None;
                    }
                    Kind::Empty => {}
                }
            } else if container.is_some_and(|depth| tag.depth == depth + 1) {
                ensure!(tag.name == b"xf", "Unknown cellXfs child");
                match tag.kind {
                    Kind::Start => entry = Some(tag.start),
                    Kind::Empty => result
                        .formats
                        .push(Sha256::digest(&bytes[tag.start..tag.end]).into()),
                    Kind::End => {
                        let start = entry.take().context("Unexpected cell format end")?;
                        result
                            .formats
                            .push(Sha256::digest(&bytes[start..tag.end]).into());
                    }
                }
            }
        }
        if let Some((start, end)) = body_range {
            // A matching xf is meaningful only while its font/fill/border/base-style
            // definitions and stylesheet namespace context also remain unchanged.
            let mut dependencies = Sha256::new();
            dependencies.update(&bytes[..start]);
            dependencies.update(&bytes[end..]);
            let dependencies = dependencies.finalize();
            for identity in &mut result.formats {
                let mut hash = Sha256::new();
                hash.update(*identity);
                hash.update(dependencies);
                *identity = hash.finalize().into();
            }
            result.context.extend_from_slice(&bytes[..start]);
            let mut ordered = result.formats.clone();
            ordered.sort_unstable();
            ordered.dedup();
            for identity in ordered {
                result.context.extend_from_slice(&identity);
            }
            result.context.extend_from_slice(&bytes[end..]);
        } else {
            result.context.extend_from_slice(bytes);
        }
        Ok(result)
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
