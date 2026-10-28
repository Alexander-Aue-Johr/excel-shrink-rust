//! Borrowed XML token/element views. No DOM and no allocation per tag.
//! DTDs and UTF-16 are explicitly rejected rather than partially read.
use anyhow::{bail, ensure, Result};
use memchr::{memchr, memmem};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind { Start, Empty, End }

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
    pub fn local(self) -> &'a [u8] { local_name(self.name) }
    pub fn attr(self, key: &[u8]) -> Result<Option<&'a [u8]>> {
        let mut found = None;
        for entry in Attributes::new(self.attrs) {
            let (name, value) = entry?;
            if name == key {
                ensure!(found.is_none(), "duplicate XML attribute {}", String::from_utf8_lossy(key));
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
    name.rsplit(|&b| b == b':').next().unwrap_or(name)
}

pub(crate) struct Attributes<'a> { bytes: &'a [u8], pos: usize }
impl<'a> Attributes<'a> {
    pub fn new(bytes: &'a [u8]) -> Self { Self { bytes, pos: 0 } }
}
impl<'a> Iterator for Attributes<'a> {
    type Item = Result<(&'a [u8], &'a [u8])>;
    fn next(&mut self) -> Option<Self::Item> {
        let result = (|| -> Result<Option<(&'a [u8], &'a [u8])>> {
            let b = self.bytes;
            while self.pos < b.len() && b[self.pos].is_ascii_whitespace() { self.pos += 1; }
            if self.pos == b.len() { return Ok(None); }
            let start = self.pos;
            while self.pos < b.len() && !b[self.pos].is_ascii_whitespace() && b[self.pos] != b'=' { self.pos += 1; }
            ensure!(self.pos > start, "empty XML attribute name");
            let name = &b[start..self.pos];
            while self.pos < b.len() && b[self.pos].is_ascii_whitespace() { self.pos += 1; }
            ensure!(b.get(self.pos) == Some(&b'='), "XML attribute without equals");
            self.pos += 1;
            while self.pos < b.len() && b[self.pos].is_ascii_whitespace() { self.pos += 1; }
            let quote = *b.get(self.pos).ok_or_else(|| anyhow::anyhow!("truncated XML attribute"))?;
            ensure!(quote == b'\'' || quote == b'"', "unquoted XML attribute");
            self.pos += 1;
            let start = self.pos;
            self.pos += memchr(quote, &b[start..]).ok_or_else(|| anyhow::anyhow!("unclosed XML attribute"))?;
            let value = &b[start..self.pos];
            ensure!(!value.contains(&b'<'), "unescaped less-than in XML attribute");
            self.pos += 1;
            ensure!(self.pos == b.len() || b[self.pos].is_ascii_whitespace(), "missing XML attribute separator");
            Ok(Some((name, value)))
        })();
        match result {
            Ok(Some(value)) => Some(Ok(value)),
            Ok(None) => None,
            Err(error) => { self.pos = self.bytes.len(); Some(Err(error)) }
        }
    }
}

/// The fixed stack stores offsets, not copied names. 128 nesting levels is a
/// deliberate validation limit, not a silent truncation of the input.
pub(crate) struct Reader<'a, const DEPTH: usize = 128> {
    bytes: &'a [u8],
    pos: usize,
    stack: [(usize, usize); DEPTH],
    depth: usize,
}
impl<'a, const DEPTH: usize> Reader<'a, DEPTH> {
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        ensure!(!bytes.starts_with(&[0xff, 0xfe]) && !bytes.starts_with(&[0xfe, 0xff])
            && !bytes.starts_with(&[0, b'<']) && !bytes.starts_with(&[b'<', 0]), "UTF-16 XML is not supported by the byte-view validator");
        Ok(Self { bytes, pos: 0, stack: [(0, 0); DEPTH], depth: 0 })
    }
    pub fn next(&mut self) -> Result<Option<Tag<'a>>> {
        loop {
            let Some(offset) = memchr(b'<', &self.bytes[self.pos..]) else {
                ensure!(self.depth == 0, "truncated XML: unclosed element");
                self.pos = self.bytes.len();
                return Ok(None);
            };
            let start = self.pos + offset;
            let tail = &self.bytes[start..];
            if tail.starts_with(b"<!--") {
                self.pos = start + 4 + memmem::find(&tail[4..], b"-->").ok_or_else(|| anyhow::anyhow!("unclosed XML comment"))? + 3;
                continue;
            }
            if tail.starts_with(b"<?") {
                self.pos = start + 2 + memmem::find(&tail[2..], b"?>").ok_or_else(|| anyhow::anyhow!("unclosed XML processing instruction"))? + 2;
                continue;
            }
            if tail.starts_with(b"<![CDATA[") {
                self.pos = start + 9 + memmem::find(&tail[9..], b"]]>")
                    .ok_or_else(|| anyhow::anyhow!("unclosed XML CDATA"))? + 3;
                continue;
            }
            ensure!(!tail.starts_with(b"<!"), "DTD/declarations are not supported in byte-view XML");
            let closing = tail.get(1) == Some(&b'/');
            let name_start = start + if closing { 2 } else { 1 };
            let mut p = name_start;
            while p < self.bytes.len() && !self.bytes[p].is_ascii_whitespace() && !matches!(self.bytes[p], b'/' | b'>') { p += 1; }
            ensure!(p > name_start && p < self.bytes.len(), "missing/truncated XML tag name");
            let name_end = p;
            let mut quote = None;
            while p < self.bytes.len() {
                let ch = self.bytes[p];
                if let Some(q) = quote { if ch == q { quote = None; } }
                else if ch == b'\'' || ch == b'"' { quote = Some(ch); }
                else if ch == b'>' { break; }
                p += 1;
            }
            ensure!(p < self.bytes.len(), "unclosed XML tag");
            let empty = !closing && self.bytes[p - 1] == b'/';
            let attrs_end = if empty { p - 1 } else { p };
            let name = &self.bytes[name_start..name_end];
            let attrs = &self.bytes[name_end..attrs_end];
            self.pos = p + 1;
            let depth;
            let kind;
            if closing {
                ensure!(attrs.iter().all(u8::is_ascii_whitespace), "attributes on XML closing tag");
                ensure!(self.depth > 0, "unexpected XML closing tag");
                self.depth -= 1;
                let (a, b) = self.stack[self.depth];
                ensure!(&self.bytes[a..b] == name, "mismatched XML closing tag");
                depth = self.depth;
                kind = Kind::End;
            } else {
                depth = self.depth;
                if empty { kind = Kind::Empty; }
                else {
                    ensure!(self.depth < self.stack.len(), "XML nesting exceeds configured view-reader limit");
                    self.stack[self.depth] = (name_start, name_end);
                    self.depth += 1;
                    kind = Kind::Start;
                }
            }
            return Ok(Some(Tag { name, attrs, start, end: p + 1, depth, kind }));
        }
    }
    /// Consume this element and return a slice into the existing XML buffer.
    pub fn element(&mut self, tag: Tag<'a>) -> Result<Element<'a>> {
        ensure!(tag.kind != Kind::End, "cannot read closing tag as element");
        if tag.kind == Kind::Empty {
            return Ok(Element { body: &self.bytes[tag.end..tag.end], raw: &self.bytes[tag.start..tag.end] });
        }
        while let Some(close) = self.next()? {
            if close.kind == Kind::End && close.depth == tag.depth {
                return Ok(Element { body: &self.bytes[tag.end..close.start], raw: &self.bytes[tag.start..close.end] });
            }
        }
        bail!("unclosed XML element")
    }
}

pub(crate) fn decimal(bytes: &[u8]) -> Result<u32> {
    ensure!(!bytes.is_empty(), "empty decimal integer");
    let mut value = 0u32;
    for &b in bytes {
        ensure!(b.is_ascii_digit(), "invalid decimal integer");
        value = value.checked_mul(10).and_then(|v| v.checked_add(u32::from(b - b'0')))
            .ok_or_else(|| anyhow::anyhow!("decimal integer overflow"))?;
    }
    Ok(value)
}

pub(crate) fn truth(bytes: Option<&[u8]>) -> bool {
    bytes.is_some_and(|b| b == b"1" || b.eq_ignore_ascii_case(b"true"))
}

/// XML character/reference decoding without allocating Strings. Spreadsheet
/// _xHHHH_ escape spellings remain literal, conservatively; no Unicode folding.
pub(crate) struct TextBytes<'a> {
    raw: &'a [u8], pos: usize, pending: [u8; 4], read: usize, len: usize,
}
impl<'a> TextBytes<'a> {
    pub fn new(raw: &'a [u8]) -> Self { Self { raw, pos: 0, pending: [0; 4], read: 0, len: 0 } }
}
impl Iterator for TextBytes<'_> {
    type Item = Result<u8>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.read < self.len { let b = self.pending[self.read]; self.read += 1; return Some(Ok(b)); }
        if self.pos >= self.raw.len() { return None; }
        let result = (|| -> Result<u8> {
            let b = self.raw[self.pos]; self.pos += 1;
            if b == b'\r' {
                if self.raw.get(self.pos) == Some(&b'\n') { self.pos += 1; }
                return Ok(b'\n');
            }
            if b != b'&' { return Ok(b); }
            let end = self.pos + memchr(b';', &self.raw[self.pos..]).ok_or_else(|| anyhow::anyhow!("unclosed XML entity"))?;
            let entity = &self.raw[self.pos..end]; self.pos = end + 1;
            let scalar = match entity {
                b"amp" => '&', b"lt" => '<', b"gt" => '>', b"quot" => '"', b"apos" => '\'',
                _ => {
                    let (digits, radix) = if let Some(n) = entity.strip_prefix(b"#x") { (n, 16) }
                        else if let Some(n) = entity.strip_prefix(b"#") { (n, 10) }
                        else { bail!("unknown XML entity"); };
                    let value = u32::from_str_radix(std::str::from_utf8(digits)?, radix)?;
                    ensure!(matches!(value, 9 | 10 | 13 | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff), "invalid XML character");
                    char::from_u32(value).ok_or_else(|| anyhow::anyhow!("invalid Unicode scalar"))?
                }
            };
            self.len = scalar.encode_utf8(&mut self.pending).len(); self.read = 1;
            Ok(self.pending[0])
        })();
        if result.is_err() { self.pos = self.raw.len(); }
        Some(result)
    }
}
