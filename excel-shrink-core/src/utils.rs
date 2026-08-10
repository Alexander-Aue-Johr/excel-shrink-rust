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
