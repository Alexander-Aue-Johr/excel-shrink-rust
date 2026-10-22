use std::io::Read;

const SPREADSHEETML: &[u8] = b"http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const STRICT_SPREADSHEETML: &[u8] = b"http://purl.oclc.org/ooxml/spreadsheetml/main";

/// The cleanup parsers match literal, unprefixed SpreadsheetML element names.
/// Preserve a workbook if any XML part declares another spelling they cannot
/// safely interpret. This is deliberately conservative: even an unused prefix
/// or an escaped namespace URI opts out, rather than guessing at XML semantics.
pub fn requires_namespace_passthrough(mut input: impl Read) -> std::io::Result<bool> {
    let mut guard = NamespaceGuard::default();
    let mut chunk = [0; 64 * 1024];
    loop {
        let count = input.read(&mut chunk)?;
        if count == 0 {
            return Ok(guard.has_incomplete_declaration());
        }
        if guard.check(&chunk[..count]) {
            return Ok(true);
        }
    }
}

#[derive(Default)]
pub(crate) struct NamespaceGuard {
    pending: Vec<u8>,
}

impl NamespaceGuard {
    pub(crate) fn check(&mut self, bytes: &[u8]) -> bool {
        // UTF-16 and other NUL-containing representations are also unsupported
        // by the byte-oriented cleanup parsers.
        if bytes.contains(&0) {
            return true;
        }
        bytes.chunks(64 * 1024).any(|chunk| self.check_chunk(chunk))
    }

    fn check_chunk(&mut self, bytes: &[u8]) -> bool {
        self.pending.extend_from_slice(bytes);
        let mut cursor = 0;
        while let Some(relative) = super::find_bytes(&self.pending[cursor..], b"xmlns") {
            let start = cursor + relative;
            let mut end = start + 5;
            let prefixed = self.pending.get(end) == Some(&b':');
            if prefixed {
                end += 1;
                while self
                    .pending
                    .get(end)
                    .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'=')
                {
                    end += 1;
                }
            }
            while self.pending.get(end).is_some_and(u8::is_ascii_whitespace) {
                end += 1;
            }
            if end == self.pending.len() {
                return self.retain_declaration(start);
            }
            if self.pending[end] != b'=' {
                cursor = start + 5;
                continue;
            }
            end += 1;
            while self.pending.get(end).is_some_and(u8::is_ascii_whitespace) {
                end += 1;
            }
            let Some(&quote) = self.pending.get(end) else {
                return self.retain_declaration(start);
            };
            if quote != b'\'' && quote != b'"' {
                return true;
            }
            let value_start = end + 1;
            let Some(value_length) = self.pending[value_start..].iter().position(|b| *b == quote)
            else {
                return self.retain_declaration(start);
            };
            let value_end = value_start + value_length;
            let value = &self.pending[value_start..value_end];
            if value.contains(&b'&')
                || value == STRICT_SPREADSHEETML
                || (prefixed && value == SPREADSHEETML)
            {
                return true;
            }
            cursor = value_end + 1;
        }
        // A declaration can begin across any input/decompression boundary.
        let keep_from = cursor.max(self.pending.len().saturating_sub(4));
        self.pending.drain(..keep_from);
        false
    }

    fn retain_declaration(&mut self, start: usize) -> bool {
        self.pending.drain(..start);
        // Do not allocate without a bound for malformed/exceptional declarations.
        self.pending.len() > 64 * 1024
    }

    pub(crate) fn has_incomplete_declaration(&self) -> bool {
        self.pending.starts_with(b"xmlns")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_prefixed_main_namespace_at_every_chunk_boundary() {
        for xml in [
            br#"<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#.as_slice(),
            b"<row xmlns:local \n = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'>",
            br#"<si><p:t xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/main">text</p:t></si>"#,
        ] {
            for size in 1..=xml.len() {
                let mut guard = NamespaceGuard::default();
                assert!(xml.chunks(size).any(|chunk| guard.check(chunk)), "chunk size {size}");
            }
        }
    }

    #[test]
    fn allows_default_main_namespace_and_normal_extension_prefixes() {
        let xml = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac"><sheetData/></worksheet>"#;
        for size in 1..=xml.len() {
            let mut guard = NamespaceGuard::default();
            assert!(!xml.chunks(size).any(|chunk| guard.check(chunk)));
            assert!(!guard.has_incomplete_declaration());
        }
    }

    #[test]
    fn preserves_namespace_representations_that_cannot_be_checked() {
        for xml in [
            br#"<worksheet xmlns="http://purl.oclc.org/ooxml/spreadsheetml/main"/>"#.as_slice(),
            br#"<worksheet xmlns:p="http://schemas.openxmlformats.org/spreadsheetml/2006/ma&#105;n"/>"#,
            b"<worksheet xmlns:p=\"unfinished",
            b"<\0w\0o\0r\0k\0s\0h\0e\0e\0t\0",
        ] {
            assert!(requires_namespace_passthrough(xml).unwrap());
        }
    }
}
