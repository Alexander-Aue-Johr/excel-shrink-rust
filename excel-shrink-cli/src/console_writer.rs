use std::io::{self, IsTerminal, Write};

/// Keep log lines aligned even if a Windows child changes WSL terminal modes.
/// Redirected stderr retains its original bytes.
pub(crate) fn stderr() -> ConsoleWriter<io::Stderr> {
    let stream = io::stderr();
    let terminal = stream.is_terminal();
    ConsoleWriter { inner: stream, terminal, previous_cr: false }
}

pub(crate) struct ConsoleWriter<W> {
    inner: W,
    terminal: bool,
    previous_cr: bool,
}

impl<W: Write> Write for ConsoleWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !self.terminal {
            return self.inner.write(bytes);
        }
        let mut output = Vec::with_capacity(bytes.len());
        let mut previous_cr = self.previous_cr;
        for &byte in bytes {
            if byte == b'\n' && !previous_cr {
                output.push(b'\r');
            }
            output.push(byte);
            previous_cr = byte == b'\r';
        }
        self.inner.write_all(&output)?;
        self.previous_cr = previous_cr;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_normalizes_lf_and_preserves_split_crlf() {
        let mut writer = ConsoleWriter {
            inner: Vec::new(), terminal: true, previous_cr: false,
        };
        writer.write_all(b"first\nsecond\r").unwrap();
        writer.write_all(b"\n\x1b[32mthird\x1b[0m\n").unwrap();
        assert_eq!(writer.inner, b"first\r\nsecond\r\n\x1b[32mthird\x1b[0m\r\n");
    }

    #[test]
    fn redirected_output_is_unchanged() {
        let mut writer = ConsoleWriter {
            inner: Vec::new(), terminal: false, previous_cr: false,
        };
        let input = b"first\nsecond\r\n";
        writer.write_all(input).unwrap();
        assert_eq!(writer.inner, input);
    }
}
