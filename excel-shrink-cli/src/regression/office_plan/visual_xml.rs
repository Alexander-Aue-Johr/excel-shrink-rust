//! Borrowed XML reader for workbook capture bounds.
use anyhow::{bail, ensure, Context, Result};
use memchr::{memchr, memmem};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Start,
    Empty,
    End,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Tag<'a> {
    pub name: &'a [u8],
    pub attrs: &'a [u8],
    pub start: usize,
    pub end: usize,
    pub depth: usize,
    pub kind: Kind,
}

impl<'a> Tag<'a> {
    pub fn local(self) -> &'a [u8] {
        local_name(self.name)
    }

    pub fn attr(self, key: &[u8]) -> Result<Option<&'a [u8]>> {
        let mut found = None;
        for entry in Attributes::new(self.attrs) {
            let (name, value) = entry?;
            if name == key {
                ensure!(
                    found.is_none(),
                    "duplicate XML attribute {}",
                    String::from_utf8_lossy(key)
                );
                found = Some(value);
            }
        }
        Ok(found)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Element<'a> {
    pub body: &'a [u8],
    pub raw: &'a [u8],
}

pub(crate) fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|&byte| byte == b':').next().unwrap_or(name)
}

pub(crate) struct Attributes<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Attributes<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
}

impl<'a> Iterator for Attributes<'a> {
    type Item = Result<(&'a [u8], &'a [u8])>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        if self.pos >= self.bytes.len() {
            return None;
        }

        Some((|| {
            let name_start = self.pos;
            while self.pos < self.bytes.len()
                && !self.bytes[self.pos].is_ascii_whitespace()
                && self.bytes[self.pos] != b'='
            {
                self.pos += 1;
            }
            ensure!(self.pos > name_start, "empty XML attribute name");
            let name_end = self.pos;

            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            ensure!(
                self.bytes.get(self.pos) == Some(&b'='),
                "XML attribute has no '='"
            );
            self.pos += 1;
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }

            let quote = *self
                .bytes
                .get(self.pos)
                .ok_or_else(|| anyhow::anyhow!("missing XML attribute quote"))?;
            ensure!(matches!(quote, b'\'' | b'"'), "XML attribute is not quoted");
            self.pos += 1;
            let value_start = self.pos;
            let relative = memchr(quote, &self.bytes[self.pos..])
                .ok_or_else(|| anyhow::anyhow!("unclosed XML attribute"))?;
            let value_end = self.pos + relative;
            self.pos = value_end + 1;
            Ok((
                &self.bytes[name_start..name_end],
                &self.bytes[value_start..value_end],
            ))
        })())
    }
}

pub(crate) struct Reader<'a, const N: usize> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
    stack: [(usize, usize); N],
}

impl<'a, const N: usize> Reader<'a, N> {
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        ensure!(
            !bytes.starts_with(&[0xff, 0xfe]) && !bytes.starts_with(&[0xfe, 0xff]),
            "UTF-16 XML is unsupported"
        );
        ensure!(
            memmem::find(bytes, b"<!DOCTYPE").is_none(),
            "DTD XML is unsupported"
        );
        Ok(Self {
            bytes,
            pos: 0,
            depth: 0,
            stack: [(0, 0); N],
        })
    }

    pub fn next(&mut self) -> Result<Option<Tag<'a>>> {
        loop {
            let Some(relative) = memchr(b'<', &self.bytes[self.pos..]) else {
                self.pos = self.bytes.len();
                return Ok(None);
            };
            let start = self.pos + relative;

            if self.bytes[start..].starts_with(b"<!--") {
                let close = memmem::find(&self.bytes[start + 4..], b"-->")
                    .ok_or_else(|| anyhow::anyhow!("unclosed XML comment"))?;
                self.pos = start + 4 + close + 3;
                continue;
            }
            if self.bytes[start..].starts_with(b"<?") {
                let close = memmem::find(&self.bytes[start + 2..], b"?>")
                    .ok_or_else(|| anyhow::anyhow!("unclosed XML processing instruction"))?;
                self.pos = start + 2 + close + 2;
                continue;
            }
            if self.bytes[start..].starts_with(b"<![CDATA[") {
                let close = memmem::find(&self.bytes[start + 9..], b"]]>")
                    .ok_or_else(|| anyhow::anyhow!("unclosed XML CDATA"))?;
                self.pos = start + 9 + close + 3;
                continue;
            }
            if self.bytes[start..].starts_with(b"<!") {
                bail!("unsupported XML declaration near byte {start}");
            }

            let mut p = start + 1;
            let closing = self.bytes.get(p) == Some(&b'/');
            if closing {
                p += 1;
            }
            let name_start = p;
            while let Some(&byte) = self.bytes.get(p) {
                if byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>') {
                    break;
                }
                p += 1;
            }
            ensure!(p > name_start, "empty XML tag name");
            let name_end = p;

            let mut quote = None;
            let mut tag_end = None;
            while let Some(&byte) = self.bytes.get(p) {
                match quote {
                    Some(active) if byte == active => quote = None,
                    Some(_) => {}
                    None if matches!(byte, b'\'' | b'"') => quote = Some(byte),
                    None if byte == b'>' => {
                        tag_end = Some(p);
                        break;
                    }
                    None => {}
                }
                p += 1;
            }
            let p = tag_end.ok_or_else(|| anyhow::anyhow!("unclosed XML tag"))?;
            ensure!(quote.is_none(), "unclosed XML quote");

            let mut attrs_end = p;
            while attrs_end > name_end && self.bytes[attrs_end - 1].is_ascii_whitespace() {
                attrs_end -= 1;
            }
            let self_closing =
                !closing && attrs_end > name_end && self.bytes[attrs_end - 1] == b'/';
            if self_closing {
                attrs_end -= 1;
                while attrs_end > name_end && self.bytes[attrs_end - 1].is_ascii_whitespace() {
                    attrs_end -= 1;
                }
            }

            let name = &self.bytes[name_start..name_end];
            let attrs = &self.bytes[name_end..attrs_end];
            let (depth, kind) = if closing {
                ensure!(self.depth > 0, "unexpected XML closing tag");
                self.depth -= 1;
                let (open_start, open_end) = self.stack[self.depth];
                ensure!(
                    &self.bytes[open_start..open_end] == name,
                    "mismatched XML closing tag {}",
                    String::from_utf8_lossy(name)
                );
                (self.depth, Kind::End)
            } else if self_closing {
                (self.depth, Kind::Empty)
            } else {
                ensure!(
                    self.depth < N,
                    "XML nesting exceeds configured reader limit"
                );
                let depth = self.depth;
                self.stack[self.depth] = (name_start, name_end);
                self.depth += 1;
                (depth, Kind::Start)
            };

            self.pos = p + 1;
            return Ok(Some(Tag {
                name,
                attrs,
                start,
                end: p + 1,
                depth,
                kind,
            }));
        }
    }

    pub fn element(&mut self, tag: Tag<'a>) -> Result<Element<'a>> {
        ensure!(tag.kind != Kind::End, "cannot read closing tag as element");
        if tag.kind == Kind::Empty {
            return Ok(Element {
                body: &self.bytes[tag.end..tag.end],
                raw: &self.bytes[tag.start..tag.end],
            });
        }
        while let Some(close) = self.next()? {
            if close.kind == Kind::End && close.depth == tag.depth {
                return Ok(Element {
                    body: &self.bytes[tag.end..close.start],
                    raw: &self.bytes[tag.start..close.end],
                });
            }
        }
        bail!("unclosed XML element")
    }
}

pub(crate) fn decimal(bytes: &[u8]) -> Result<u32> {
    ensure!(!bytes.is_empty(), "empty decimal integer");
    let mut value = 0u32;
    for &byte in bytes {
        ensure!(byte.is_ascii_digit(), "invalid decimal integer");
        value = value
            .checked_mul(10)
            .and_then(|current| current.checked_add(u32::from(byte - b'0')))
            .ok_or_else(|| anyhow::anyhow!("decimal integer overflow"))?;
    }
    Ok(value)
}

pub(crate) fn decode_attr(bytes: &[u8]) -> Result<String> {
    let mut out = String::with_capacity(bytes.len());
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] != b'&' {
            let start = pos;
            while pos < bytes.len() && bytes[pos] != b'&' {
                pos += 1;
            }
            out.push_str(std::str::from_utf8(&bytes[start..pos])?);
            continue;
        }
        let semi = memchr(b';', &bytes[pos + 1..])
            .map(|relative| pos + 1 + relative)
            .ok_or_else(|| anyhow::anyhow!("unterminated XML entity"))?;
        let entity = &bytes[pos + 1..semi];
        match entity {
            b"amp" => out.push('&'),
            b"lt" => out.push('<'),
            b"gt" => out.push('>'),
            b"quot" => out.push('"'),
            b"apos" => out.push('\''),
            _ if entity.starts_with(b"#x") => {
                let value = u32::from_str_radix(std::str::from_utf8(&entity[2..])?, 16)?;
                out.push(
                    char::from_u32(value)
                        .ok_or_else(|| anyhow::anyhow!("invalid XML code point"))?,
                );
            }
            _ if entity.starts_with(b"#") => {
                let value: u32 = std::str::from_utf8(&entity[1..])?.parse()?;
                out.push(
                    char::from_u32(value)
                        .ok_or_else(|| anyhow::anyhow!("invalid XML code point"))?,
                );
            }
            _ => bail!(
                "unsupported XML entity &{};",
                String::from_utf8_lossy(entity)
            ),
        }
        pos = semi + 1;
    }
    Ok(out)
}

/// Decode the worksheet-name Xstring after XML entities, consuming each escape once.
/// An escaped underscore protects a following literal `_xHHHH_` from a second decode.
pub(crate) fn decode_sheet_name(bytes: &[u8]) -> Result<String> {
    let value = decode_attr(bytes)?;
    if !value.contains("_x") {
        return Ok(value);
    }
    let mut units = Vec::with_capacity(value.len());
    let mut remaining = value.as_str();
    while !remaining.is_empty() {
        let escape = remaining.as_bytes().get(..7).filter(|bytes| {
            bytes.starts_with(b"_x")
                && bytes[6] == b'_'
                && bytes[2..6].iter().all(u8::is_ascii_hexdigit)
        });
        if let Some(escape) = escape {
            units.push(u16::from_str_radix(std::str::from_utf8(&escape[2..6])?, 16)?);
            remaining = &remaining[7..];
        } else {
            let character = remaining.chars().next().expect("nonempty worksheet name");
            let mut buffer = [0; 2];
            units.extend_from_slice(character.encode_utf16(&mut buffer));
            remaining = &remaining[character.len_utf8()..];
        }
    }
    String::from_utf16(&units).context("Invalid UTF-16 in worksheet name")
}

#[cfg(test)]
mod sheet_name_tests {
    use super::decode_sheet_name;

    #[test]
    fn decodes_entities_and_xstring_controls_once() {
        assert_eq!(decode_sheet_name(b"A &amp; B_x0009_").unwrap(), "A & B\t");
        assert_eq!(decode_sheet_name(b"A_x000A_B_x000D_C").unwrap(), "A\nB\rC");
        assert_eq!(decode_sheet_name(b"A_x005F_x0009_").unwrap(), "A_x0009_");
        for literal in ["A_xZZZZ_", "A_x123_", "A_x12345_", "A_X0009_"] {
            assert_eq!(decode_sheet_name(literal.as_bytes()).unwrap(), literal);
        }
    }

    #[test]
    fn retains_unicode_and_checks_surrogate_pairs() {
        assert_eq!(decode_sheet_name("Größe 🚀".as_bytes()).unwrap(), "Größe 🚀");
        assert_eq!(decode_sheet_name(b"A_xD83D__xDE80_").unwrap(), "A🚀");
        assert!(decode_sheet_name(b"A_xD83D_").is_err());
    }
}
