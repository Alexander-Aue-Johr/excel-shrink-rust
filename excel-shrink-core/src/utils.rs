/// Convert a column-letter bytes object (e.g. b"A", b"B", b"AA")
/// into its 1-based column index (A=1, B=2, ... AA=27, etc.).
#[inline]
pub fn letters_byte_string_to_integer(col_bytes: &[u8]) -> u32 {
    let mut col_index = 0;
    for &b in col_bytes {
        if b >= b'A' && b <= b'Z' {
            col_index = col_index * 26 + (b - b'A' + 1) as u32;
        } else if b >= b'a' && b <= b'z' {
            col_index = col_index * 26 + (b - b'a' + 1) as u32;
        }
    }
    col_index
}

/// Convert row bytes (e.g. b"80") directly into an integer without decoding to string.
#[inline]
pub fn digits_byte_string_to_integer(row_bytes: &[u8]) -> u32 {
    let mut row_index = 0;
    for &b in row_bytes {
        if b >= b'0' && b <= b'9' {
            row_index = row_index * 10 + (b - b'0') as u32;
        }
    }
    row_index
}

/// Parse unsigned decimal XML values without a UTF-8 intermediate.
/// Matches unsigned Rust parsing, including optional '+' and checked overflow.
pub fn parse_unsigned_bytes<T: TryFrom<u64>>(bytes: &[u8]) -> Option<T> {
    let digits = bytes.strip_prefix(b"+").unwrap_or(bytes);
    if digits.is_empty() {
        return None;
    }
    let mut value = 0u64;
    for &digit in digits {
        if !digit.is_ascii_digit() {
            return None;
        }
        value = value
            .checked_mul(10)?
            .checked_add(u64::from(digit - b'0'))?;
    }
    T::try_from(value).ok()
}

#[cfg(test)]
mod byte_number_tests {
    #[test]
    fn matches_unsigned_string_parsing() {
        for bytes in [
            b"".as_slice(),
            b"+",
            b"+1",
            b"001",
            b"-1",
            b" 1",
            b"1 ",
            b"4294967295",
            b"4294967296",
            b"18446744073709551615",
            b"18446744073709551616",
            b"\xff",
        ] {
            assert_eq!(
                super::parse_unsigned_bytes::<u32>(bytes),
                std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|s| s.parse::<u32>().ok())
            );
            assert_eq!(
                super::parse_unsigned_bytes::<usize>(bytes),
                std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
            );
        }
    }
}
