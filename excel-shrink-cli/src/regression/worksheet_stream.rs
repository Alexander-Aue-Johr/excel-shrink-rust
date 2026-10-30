//! Bounded byte framing for worksheet children and individual sheetData rows.
//! The existing XML view reader validates the framed elements and their contents.
use crate::regression::xml_views::{Kind, Reader};
use anyhow::{ensure, Context, Result};
use std::io::{BufRead, BufReader, Read};

const MAX_ELEMENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_DEPTH: usize = 128;

pub(crate) struct WorksheetStream<R: Read> {
    input: BufReader<R>,
    markup: Vec<u8>,
    root: Option<Vec<u8>>,
    sheet_data: Option<Vec<u8>>,
    data_seen: bool,
    dimension_seen: bool,
    finished: bool,
}

impl<R: Read> WorksheetStream<R> {
    pub(crate) fn new(input: R) -> Self {
        Self {
            input: BufReader::with_capacity(64 * 1024, input),
            markup: Vec::new(),
            root: None,
            sheet_data: None,
            data_seen: false,
            dimension_seen: false,
            finished: false,
        }
    }

    // Read through the markup terminator, including quoted '>' characters,
    // comments, CDATA and processing instructions split across input buffers.
    fn markup(&mut self) -> Result<bool> {
        self.markup.clear();
        loop {
            let remaining = MAX_ELEMENT_BYTES.saturating_sub(self.markup.len());
            let count = self
                .input
                .by_ref()
                .take((remaining + 1) as u64)
                .read_until(b'>', &mut self.markup)?;
            ensure!(
                self.markup.len() <= MAX_ELEMENT_BYTES,
                "worksheet markup exceeds streaming buffer limit"
            );
            if count == 0 {
                ensure!(
                    self.markup.iter().all(u8::is_ascii_whitespace),
                    "truncated worksheet XML"
                );
                return Ok(false);
            }
            let Some(start) = memchr::memchr(b'<', &self.markup) else {
                continue;
            };
            let tag = &self.markup[start..];
            let complete = if tag.starts_with(b"<!--") {
                tag.ends_with(b"-->")
            } else if tag.starts_with(b"<![CDATA[") {
                tag.ends_with(b"]]>")
            } else if tag.starts_with(b"<?") {
                tag.ends_with(b"?>")
            } else {
                ensure!(
                    !tag.starts_with(b"<!"),
                    "DTD/declarations are not supported in worksheet streams"
                );
                let mut quote = None;
                let mut complete = false;
                for &byte in &tag[1..] {
                    match quote {
                        Some(value) if byte == value => quote = None,
                        Some(_) => {}
                        None if matches!(byte, b'\'' | b'"') => quote = Some(byte),
                        None if byte == b'>' => {
                            complete = true;
                            break;
                        }
                        None => {}
                    }
                }
                complete
            };
            if complete {
                return Ok(true);
            }
        }
    }

    fn finish(&mut self) -> Result<bool> {
        while self.markup()? {
            let start = memchr::memchr(b'<', &self.markup).context("unexpected trailing XML")?;
            ensure!(
                self.markup[..start].iter().all(u8::is_ascii_whitespace)
                    && (self.markup[start..].starts_with(b"<!--")
                        || self.markup[start..].starts_with(b"<?")),
                "unexpected content after worksheet"
            );
        }
        self.finished = true;
        Ok(false)
    }

    /// Supply a self-contained worksheet fragment in a reusable caller buffer.
    /// Only one row or one non-sheetData child is buffered at a time.
    pub(crate) fn next(&mut self, output: &mut Vec<u8>) -> Result<bool> {
        if self.finished {
            return Ok(false);
        }
        loop {
            ensure!(self.markup()?, "worksheet ended before its closing tag");
            let start = memchr::memchr(b'<', &self.markup).context("missing worksheet tag")?;
            let prefix = &self.markup[..start];
            ensure!(
                prefix.iter().all(u8::is_ascii_whitespace)
                    || (self.root.is_none() && prefix == [0xef, 0xbb, 0xbf]),
                "unexpected text outside worksheet elements"
            );
            let raw = &self.markup[start..];
            if raw.starts_with(b"<?") || raw.starts_with(b"<!--") {
                continue;
            }
            if raw.starts_with(b"</") {
                let name = closing_name(raw)?;
                if let Some(data) = &self.sheet_data {
                    ensure!(name == data.as_slice(), "unexpected sheetData closing tag");
                    self.sheet_data = None;
                    continue;
                }
                ensure!(
                    self.root.as_deref() == Some(name),
                    "unexpected worksheet closing tag"
                );
                return self.finish();
            }
            let mut reader = Reader::<MAX_DEPTH>::new(raw)?;
            let tag = reader.next()?.context("missing worksheet element")?;
            if self.root.is_none() {
                ensure!(tag.local() == b"worksheet", "unsupported worksheet root");
                self.root = Some(tag.name.to_vec());
                if tag.kind == Kind::Empty {
                    return self.finish();
                }
                continue;
            }
            if self.sheet_data.is_none() && tag.local() == b"sheetData" {
                ensure!(!self.data_seen, "duplicate sheetData");
                self.data_seen = true;
                if tag.kind == Kind::Start {
                    self.sheet_data = Some(tag.name.to_vec());
                }
                continue;
            }
            if self.sheet_data.is_none() && tag.local() == b"cols" {
                ensure!(!self.data_seen, "cols after sheetData is unsupported");
            }
            if self.sheet_data.is_none() && tag.local() == b"dimension" {
                ensure!(!self.dimension_seen, "duplicate worksheet dimension");
                self.dimension_seen = true;
            }
            if self.sheet_data.is_some() {
                ensure!(tag.local() == b"row", "unexpected sheetData child");
            }
            let mut depth = usize::from(tag.kind == Kind::Start);
            output.clear();
            output.extend_from_slice(b"<worksheet>");
            if self.sheet_data.is_some() {
                output.extend_from_slice(b"<sheetData>");
            }
            output.extend_from_slice(raw);
            while depth > 0 {
                ensure!(self.markup()?, "truncated worksheet element");
                ensure!(
                    output.len() + self.markup.len() <= MAX_ELEMENT_BYTES,
                    "worksheet element exceeds streaming buffer limit"
                );
                output.extend_from_slice(&self.markup);
                let start = memchr::memchr(b'<', &self.markup).context("missing element markup")?;
                let raw = &self.markup[start..];
                if raw.starts_with(b"<!--")
                    || raw.starts_with(b"<?")
                    || raw.starts_with(b"<![CDATA[")
                {
                    continue;
                }
                if raw.starts_with(b"</") {
                    depth -= 1;
                } else {
                    // Full QName matching and attributes are validated by the
                    // byte reader over this completed fragment, not twice here.
                    depth += usize::from(!raw.ends_with(b"/>"));
                    ensure!(
                        depth <= MAX_DEPTH,
                        "worksheet nesting exceeds streaming limit"
                    );
                }
            }
            if self.sheet_data.is_some() {
                output.extend_from_slice(b"</sheetData>");
            }
            output.extend_from_slice(b"</worksheet>");
            return Ok(true);
        }
    }
}

fn closing_name(raw: &[u8]) -> Result<&[u8]> {
    let body = raw
        .strip_prefix(b"</")
        .and_then(|raw| raw.strip_suffix(b">"))
        .context("invalid closing tag")?;
    let end = body
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(body.len());
    ensure!(
        end > 0 && body[end..].iter().all(u8::is_ascii_whitespace),
        "invalid closing tag name"
    );
    Ok(&body[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_single_rows_fail_with_bounded_buffers() {
        let prefix = b"<worksheet><sheetData><row r='1'><c r='A1'><v>";
        let input = prefix
            .as_slice()
            .chain(std::io::repeat(b'x').take(MAX_ELEMENT_BYTES as u64 + 1));
        let mut stream = WorksheetStream::new(input);
        let mut buffer = Vec::new();
        let error = stream.next(&mut buffer).unwrap_err();
        assert!(error.to_string().contains("streaming buffer limit"));
        assert!(buffer.capacity() <= 2 * MAX_ELEMENT_BYTES);
        assert!(stream.markup.capacity() <= 2 * MAX_ELEMENT_BYTES);
    }

    #[test]
    fn worksheet_structure_is_validated_across_fragments() {
        for xml in [
            b"<worksheet><sheetData/><sheetData/></worksheet>".as_slice(),
            b"<worksheet><sheetData/><cols/></worksheet>",
            b"<worksheet><dimension ref='A1'/><dimension ref='B2'/></worksheet>",
        ] {
            let mut stream = WorksheetStream::new(xml);
            let mut buffer = Vec::new();
            loop {
                match stream.next(&mut buffer) {
                    Ok(true) => {}
                    Ok(false) => panic!("malformed worksheet accepted"),
                    Err(_) => break,
                }
            }
        }
    }
}
