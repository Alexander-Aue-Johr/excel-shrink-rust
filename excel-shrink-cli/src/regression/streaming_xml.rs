//! Bounded byte-token stream for validation preflight. Ordinary text is borrowed
//! from the input chunk; only one unfinished tag and the open names are retained.
//! CDATA text is streamed without treating its markup characters as tags.
use super::xml_views::{Attributes, Kind, Tag};
use anyhow::{ensure, Result};
use memchr::{memchr, memchr3, memmem};
use std::io::{self, Read};

pub(super) const READ_BYTES: usize = 128 * 1024;
const MAX_TAG_BYTES: usize = 64 * 1024;
const MAX_NAME_BYTES: usize = 256;
const MAX_DEPTH: usize = 128;

pub(super) enum Event<'a> {
    // start/end are offsets within the temporary tag, not the whole document.
    Tag(Tag<'a>),
    Text(&'a [u8]),
    Cdata(&'a [u8]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Text,
    Open,
    Tag,
    Comment(usize),
    ProcessingInstruction(usize),
    Cdata(usize),
}

pub(super) struct Scanner {
    state: State,
    quote: Option<u8>,
    token: Vec<u8>,
    // Slots are reused when siblings close/open: no allocation per row or cell.
    names: Vec<Vec<u8>>,
    depth: usize,
    roots: usize,
    source_offsets: bool,
    bytes_received: usize,
    tag_start: usize,
}

impl Scanner {
    pub(super) fn new() -> Self {
        Self {
            state: State::Text,
            quote: None,
            token: Vec::new(),
            names: Vec::new(),
            depth: 0,
            roots: 0,
            source_offsets: false,
            bytes_received: 0,
            tag_start: 0,
        }
    }

    pub(super) fn feed(
        &mut self,
        mut bytes: &[u8],
        mut emit: impl FnMut(Event<'_>) -> Result<()>,
    ) -> Result<()> {
        ensure!(
            memchr3(0, 0xff, 0xfe, bytes).is_none(),
            "UTF-16/NUL or invalid UTF-8 bytes in streaming XML"
        );
        let chunk_start = self.bytes_received;
        let chunk_length = bytes.len();
        self.bytes_received = self.bytes_received.checked_add(chunk_length)
            .ok_or_else(|| anyhow::anyhow!("XML byte offset overflow"))?;
        while !bytes.is_empty() {
            match self.state {
                State::Text => {
                    let end = memchr(b'<', bytes).unwrap_or(bytes.len());
                    if end != 0 {
                        emit(Event::Text(&bytes[..end]))?;
                    }
                    bytes = &bytes[end..];
                    if !bytes.is_empty() {
                        self.tag_start = chunk_start + chunk_length - bytes.len();
                        self.append(b"<")?;
                        self.state = State::Open;
                        bytes = &bytes[1..];
                    }
                }
                State::Open => {
                    self.append(&bytes[..1])?;
                    bytes = &bytes[1..];
                    match self.token.as_slice() {
                        b"<!" | b"<!-" | b"<![" | b"<![C" | b"<![CD" | b"<![CDA" | b"<![CDAT" | b"<![CDATA" => {}
                        b"<![CDATA[" => {
                            self.token.clear();
                            self.state = State::Cdata(0);
                        }
                        b"<!--" => {
                            self.token.clear();
                            self.state = State::Comment(0);
                        }
                        b"<?" => {
                            self.token.clear();
                            self.state = State::ProcessingInstruction(0);
                        }
                        token => {
                            ensure!(
                                !token.starts_with(b"<!"),
                                "DTD/CDATA/declarations are not supported in streaming XML"
                            );
                            ensure!(
                                !token[1].is_ascii_whitespace()
                                    && !matches!(token[1], b'>' | b'\'' | b'"' | b'<'),
                                "invalid XML tag opening"
                            );
                            self.state = State::Tag;
                        }
                    }
                }
                State::Tag => {
                    let special = match self.quote {
                        Some(quote) => memchr(quote, bytes),
                        None => memchr3(b'>', b'\'', b'"', bytes),
                    };
                    let count = special.map_or(bytes.len(), |index| index + 1);
                    self.append(&bytes[..count])?;
                    if let Some(index) = special {
                        if self.quote.is_some() {
                            self.quote = None;
                        } else if bytes[index] == b'>' {
                            self.emit_tag(&mut emit)?;
                            self.token.clear();
                            self.state = State::Text;
                        } else {
                            self.quote = Some(bytes[index]);
                        }
                    }
                    bytes = &bytes[count..];
                }
                State::Cdata(matched) => {
                    let (remaining, pending, complete) = scan_cdata(bytes, matched, &mut emit)?;
                    bytes = remaining;
                    self.state = if complete { State::Text } else { State::Cdata(pending) };
                }
                State::Comment(matched) | State::ProcessingInstruction(matched) => {
                    let comment = matches!(self.state, State::Comment(_));
                    let delimiter = if comment { b"-->".as_slice() } else { b"?>".as_slice() };
                    let (count, matched, complete) = skip_delimited(bytes, delimiter, matched);
                    bytes = &bytes[count..];
                    self.state = if complete {
                        State::Text
                    } else if comment {
                        State::Comment(matched)
                    } else {
                        State::ProcessingInstruction(matched)
                    };
                }
            }
        }
        Ok(())
    }


    fn append(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(
            bytes.len() <= MAX_TAG_BYTES - self.token.len(),
            "XML tag exceeds streaming limit of {MAX_TAG_BYTES} bytes"
        );
        self.token.extend_from_slice(bytes);
        Ok(())
    }

    fn emit_tag(&mut self, emit: &mut impl FnMut(Event<'_>) -> Result<()>) -> Result<()> {
        let bytes = &self.token;
        let closing = bytes[1] == b'/';
        let empty = !closing && bytes[bytes.len() - 2] == b'/';
        let start = if closing { 2 } else { 1 };
        let end = bytes.len() - if empty { 2 } else { 1 };
        ensure!(start < end, "empty XML tag name");
        let name_end = start
            + bytes[start..end]
                .iter()
                .position(u8::is_ascii_whitespace)
                .unwrap_or(end - start);
        let name = &bytes[start..name_end];
        ensure!(
            !name.is_empty()
                && name.len() <= MAX_NAME_BYTES
                && !name.iter().any(|&b| matches!(b, b'<' | b'>' | b'/' | b'=' | b'\'' | b'"')),
            "invalid/overlong XML tag name"
        );
        let attrs = &bytes[name_end..end];
        let depth;
        let kind;
        if closing {
            ensure!(attrs.iter().all(u8::is_ascii_whitespace), "attributes on XML closing tag");
            ensure!(self.depth > 0, "unexpected XML closing tag");
            self.depth -= 1;
            ensure!(self.names[self.depth].as_slice() == name, "mismatched XML closing tag");
            depth = self.depth;
            kind = Kind::End;
        } else {
            // Validate even attributes that a particular consumer does not use.
            for attribute in Attributes::new(attrs) {
                attribute?;
            }
            depth = self.depth;
            if depth == 0 {
                self.roots += 1;
                ensure!(self.roots == 1, "multiple XML roots");
            }
            kind = if empty { Kind::Empty } else { Kind::Start };
            if !empty {
                ensure!(depth < MAX_DEPTH, "XML nesting exceeds streaming limit of {MAX_DEPTH}");
                if self.names.len() == depth {
                    self.names.push(Vec::new());
                }
                self.names[depth].clear();
                self.names[depth].extend_from_slice(name);
                self.depth += 1;
            }
        }
        let start = if self.source_offsets { self.tag_start } else { 0 };
        emit(Event::Tag(Tag { name, attrs, start, end: start + bytes.len(), depth, kind }))
    }

    pub(super) fn finish(&self) -> Result<()> {
        ensure!(self.state == State::Text, "truncated XML markup");
        ensure!(self.depth == 0, "truncated XML: unclosed element");
        ensure!(self.roots == 1, "missing XML root");
        Ok(())
    }
}

/// Keep at most two trailing `]` bytes so `]]>` can cross input chunks.
fn scan_cdata<'a>(
    mut bytes: &'a [u8],
    mut pending: usize,
    emit: &mut impl FnMut(Event<'_>) -> Result<()>,
) -> Result<(&'a [u8], usize, bool)> {
    while !bytes.is_empty() {
        if pending > 0 {
            if bytes[0] == b']' {
                if pending == 2 {
                    emit(Event::Cdata(b"]"))?;
                } else {
                    pending += 1;
                }
                bytes = &bytes[1..];
                continue;
            }
            if pending == 2 && bytes[0] == b'>' {
                return Ok((&bytes[1..], 0, true));
            }
            emit(Event::Cdata(&b"]]"[..pending]))?;
            pending = 0;
        }
        let end = memchr(b']', bytes).unwrap_or(bytes.len());
        if end > 0 {
            emit(Event::Cdata(&bytes[..end]))?;
            bytes = &bytes[end..];
        }
        if !bytes.is_empty() {
            pending = 1;
            bytes = &bytes[1..];
        }
    }
    Ok((bytes, pending, false))
}

/// Skip arbitrarily long comments/PIs with only a delimiter prefix as state.
fn skip_delimited(bytes: &[u8], delimiter: &[u8], matched: usize) -> (usize, usize, bool) {
    let mut boundary = [0u8; 4];
    let head = bytes.len().min(delimiter.len() - 1);
    boundary[..matched].copy_from_slice(&delimiter[..matched]);
    boundary[matched..matched + head].copy_from_slice(&bytes[..head]);
    let boundary = &boundary[..matched + head];
    if let Some(index) = memmem::find(boundary, delimiter) {
        return (index + delimiter.len() - matched, 0, true);
    }
    if let Some(index) = memmem::find(bytes, delimiter) {
        return (index + delimiter.len(), 0, true);
    }
    let tail = if bytes.len() < delimiter.len() - 1 { boundary } else { bytes };
    let matched = (1..delimiter.len())
        .rev()
        .find(|&length| tail.ends_with(&delimiter[..length]))
        .unwrap_or(0);
    (bytes.len(), matched, false)
}

pub(super) fn read_chunk(input: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        match input.read(buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => return result,
        }
    }
}

pub(super) fn scan(
    input: impl Read,
    emit: impl FnMut(Event<'_>) -> Result<()>,
) -> Result<()> {
    scan_internal(input, false, emit)
}

/// Emit absolute uncompressed tag offsets so selected raw subtrees can be retained verbatim.
/// The ordinary scanner keeps its existing temporary-tag offsets.
pub(super) fn scan_with_offsets(
    input: impl Read,
    emit: impl FnMut(Event<'_>) -> Result<()>,
) -> Result<()> {
    scan_internal(input, true, emit)
}

fn scan_internal(
    mut input: impl Read,
    source_offsets: bool,
    mut emit: impl FnMut(Event<'_>) -> Result<()>,
) -> Result<()> {
    let mut scanner = Scanner::new();
    scanner.source_offsets = source_offsets;
    let mut buffer = [0u8; READ_BYTES];
    loop {
        let count = read_chunk(&mut input, &mut buffer)?;
        if count == 0 {
            break;
        }
        scanner.feed(&buffer[..count], &mut emit)?;
    }
    scanner.finish()
}

#[cfg(test)]
#[path = "streaming_xml_tests.rs"]
mod tests;
