//! A cheap rejection hint, not validation of the rest of an XML part. Normal
//! parsers must still reject unsupported declarations encountered in its body.
use super::NamespaceGuard;
use std::io::Read;

const MAX_HEADER_BYTES: usize = 64 * 1024;
const READ_BYTES: usize = 512;
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

/// Inspect only the root opening tag, skipping an optional UTF-8 BOM, processing
/// instructions and comments. An ambiguous or overlong header opts out. Ordinary
/// roots need at most one small read regardless of the XML body's size.
pub fn requires_namespace_root_passthrough(mut input: impl Read) -> std::io::Result<bool> {
    let mut header = Vec::with_capacity(READ_BYTES);
    let mut parser = HeaderParser::default();
    let mut chunk = [0; READ_BYTES];
    loop {
        let remaining = MAX_HEADER_BYTES - header.len();
        if remaining == 0 {
            return Ok(true);
        }
        let count = match input.read(&mut chunk[..remaining.min(READ_BYTES)]) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if count == 0 {
            return Ok(true);
        }
        header.extend_from_slice(&chunk[..count]);
        if let Some(unsupported) = parser.inspect(&header) {
            return Ok(unsupported);
        }
    }
}

#[derive(Clone, Copy, Default)]
enum State {
    #[default]
    Prolog,
    Comment,
    ProcessingInstruction,
    Root {
        start: usize,
        quote: u8,
    },
}

#[derive(Default)]
struct HeaderParser {
    bom_checked: bool,
    cursor: usize,
    state: State,
}

impl HeaderParser {
    fn inspect(&mut self, bytes: &[u8]) -> Option<bool> {
        if !self.bom_checked {
            if bytes.len() < UTF8_BOM.len() && UTF8_BOM.starts_with(bytes) {
                return None;
            }
            self.cursor = if bytes.starts_with(UTF8_BOM) { 3 } else { 0 };
            self.bom_checked = true;
        }
        while self.cursor < bytes.len() {
            match self.state {
                State::Prolog => {
                    let byte = bytes[self.cursor];
                    if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
                        self.cursor += 1;
                        continue;
                    }
                    if byte != b'<' {
                        return Some(true);
                    }
                    let rest = &bytes[self.cursor..];
                    if rest.len() < 2 {
                        return None;
                    }
                    if rest.starts_with(b"<?") {
                        self.cursor += 2;
                        self.state = State::ProcessingInstruction;
                    } else if rest.starts_with(b"<!--") {
                        self.cursor += 4;
                        self.state = State::Comment;
                    } else if b"<!--".starts_with(rest) {
                        return None;
                    } else if rest[1].is_ascii_alphabetic() || rest[1] == b'_' {
                        self.state = State::Root {
                            start: self.cursor,
                            quote: 0,
                        };
                        self.cursor += 2;
                    } else {
                        // DTD/entity declarations, unsupported encodings and
                        // malformed roots are outside the fast-path contract.
                        return Some(true);
                    }
                }
                State::Comment => {
                    if bytes[self.cursor] == b'-' {
                        if bytes.len() - self.cursor < 3 {
                            return None;
                        }
                        if bytes[self.cursor..].starts_with(b"-->") {
                            self.cursor += 3;
                            self.state = State::Prolog;
                            continue;
                        }
                    }
                    self.cursor += 1;
                }
                State::ProcessingInstruction => {
                    if bytes[self.cursor] == b'?' {
                        if bytes.len() - self.cursor < 2 {
                            return None;
                        }
                        if bytes[self.cursor + 1] == b'>' {
                            self.cursor += 2;
                            self.state = State::Prolog;
                            continue;
                        }
                    }
                    self.cursor += 1;
                }
                State::Root { start, quote } => {
                    let byte = bytes[self.cursor];
                    if quote != 0 {
                        if byte == quote {
                            self.state = State::Root { start, quote: 0 };
                        }
                    } else if matches!(byte, b'\'' | b'"') {
                        self.state = State::Root { start, quote: byte };
                    } else if byte == b'>' {
                        let mut guard = NamespaceGuard::default();
                        return Some(
                            guard.check(&bytes[start..=self.cursor])
                                || guard.has_incomplete_declaration(),
                        );
                    } else if byte == b'<' || byte == 0 {
                        return Some(true);
                    }
                    self.cursor += 1;
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Chunked<'a> {
        input: Cursor<&'a [u8]>,
        maximum: usize,
    }
    impl Read for Chunked<'_> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let count = output.len().min(self.maximum);
            self.input.read(&mut output[..count])
        }
    }

    #[test]
    fn supports_excel_headers_and_ignores_prolog_text_at_every_read_boundary() {
        let headers: &[&[u8]] = &[
            br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
            b"\xef\xbb\xbf \r\n<?xml version='1.0' encoding='UTF-8'?>\n<!-- <p:worksheet xmlns:p='http://schemas.openxmlformats.org/spreadsheetml/2006/main'> --><?probe more?><worksheet label='a > b'/>",
            br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">"#,
        ];
        for header in headers {
            for maximum in 1..=header.len() {
                assert!(
                    !requires_namespace_root_passthrough(Chunked {
                        input: Cursor::new(header),
                        maximum,
                    })
                    .unwrap(),
                    "read limit {maximum}: {}",
                    String::from_utf8_lossy(header)
                );
            }
        }
    }

    #[test]
    fn rejects_unsupported_or_ambiguous_headers() {
        let headers: &[&[u8]] = &[
            br#"<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
            br#"<worksheet xmlns="http://purl.oclc.org/ooxml/spreadsheetml/main"/>"#,
            br#"<worksheet xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/ma&#105;n"/>"#,
            br#"<worksheet xmlns:p=unquoted/>"#,
            br#"<worksheet xmlns="unfinished>"#,
            b"\xff\xfe<\0w\0o\0r\0k\0s\0h\0e\0e\0t\0",
            b"<\0w\0o\0r\0k\0s\0h\0e\0e\0t\0",
            b"<!DOCTYPE worksheet><worksheet/>",
            b"<?unfinished",
            b"<!-- unfinished",
            b"<worksheet",
            b"\xef\xbb",
            b"",
        ];
        for header in headers {
            for maximum in 1..=header.len().max(1) {
                assert!(
                    requires_namespace_root_passthrough(Chunked {
                        input: Cursor::new(header),
                        maximum,
                    })
                    .unwrap(),
                    "read limit {maximum}: {}",
                    String::from_utf8_lossy(header)
                );
            }
        }
    }

    struct LargeBody<'a> {
        header: &'a [u8],
        length: usize,
        read: usize,
        calls: usize,
    }
    impl Read for LargeBody<'_> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            self.calls += 1;
            let count = output.len().min(self.length.saturating_sub(self.read));
            output[..count].fill(b'x');
            if self.read < self.header.len() {
                let copy = count.min(self.header.len() - self.read);
                output[..copy].copy_from_slice(&self.header[self.read..self.read + copy]);
            }
            self.read += count;
            Ok(count)
        }
    }

    #[test]
    fn ordinary_root_cost_is_constant_even_for_a_gigabyte_body() {
        for length in [1024, 1024 * 1024 * 1024] {
            let mut input = LargeBody {
                header: b"<worksheet>",
                length,
                read: 0,
                calls: 0,
            };
            assert!(!requires_namespace_root_passthrough(&mut input).unwrap());
            assert_eq!(input.read, READ_BYTES);
            assert_eq!(input.calls, 1);
        }
    }

    #[test]
    fn body_declarations_are_left_for_the_normal_stream_guard() {
        let xml = br#"<worksheet><row xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><p:c/></row></worksheet>"#;
        assert!(!requires_namespace_root_passthrough(xml.as_slice()).unwrap());
        assert!(super::super::requires_namespace_passthrough(xml.as_slice()).unwrap());
    }

    #[test]
    fn overlong_header_is_rejected_without_reading_beyond_its_budget() {
        let mut header = b"<worksheet note='".to_vec();
        header.resize(MAX_HEADER_BYTES + 100, b'x');
        header.extend_from_slice(b"'>");
        let mut input = LargeBody {
            header: &header,
            length: header.len(),
            read: 0,
            calls: 0,
        };
        assert!(requires_namespace_root_passthrough(&mut input).unwrap());
        assert_eq!(input.read, MAX_HEADER_BYTES);
        assert_eq!(input.calls, MAX_HEADER_BYTES / READ_BYTES);
    }
}
