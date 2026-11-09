//! Honor a CRC-bound Info-ZIP Unicode path when the legacy name differs.
use anyhow::{ensure, Result};
use zip::read::ZipFile;

pub(super) fn matches(member: &ZipFile<'_>, expected: &str) -> Result<bool> {
    if member.name() == expected {
        return Ok(true);
    }
    Ok(unicode_path(member.name_raw(), member.extra_data())? == Some(expected))
}

/// APPNOTE 4.6.9 binds version-one UTF-8 names to the raw header name by CRC32.
/// Stale checksums and unknown versions retain the ordinary header-name path.
fn unicode_path<'a>(raw_name: &[u8], mut extra: &'a [u8]) -> Result<Option<&'a str>> {
    let mut result = None;
    while !extra.is_empty() {
        ensure!(extra.len() >= 4, "Truncated ZIP extra-field header");
        let kind = u16::from_le_bytes([extra[0], extra[1]]);
        let size = usize::from(u16::from_le_bytes([extra[2], extra[3]]));
        extra = &extra[4..];
        ensure!(extra.len() >= size, "Truncated ZIP extra-field body");
        let (body, remaining) = extra.split_at(size);
        extra = remaining;
        if kind != 0x7075 {
            continue;
        }
        ensure!(!body.is_empty(), "Missing ZIP Unicode-path version");
        if body[0] != 1 {
            continue;
        }
        ensure!(body.len() >= 5, "Truncated ZIP Unicode-path checksum");
        let checksum = u32::from_le_bytes(body[1..5].try_into()?);
        if checksum != crc32fast::hash(raw_name) {
            continue;
        }
        let name = std::str::from_utf8(&body[5..])?;
        ensure!(result.is_none(), "Ambiguous ZIP Unicode-path fields");
        result = Some(name);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(raw: &[u8], name: &[u8]) -> Vec<u8> {
        let mut extra = Vec::new();
        extra.extend_from_slice(&0x7075u16.to_le_bytes());
        extra.extend_from_slice(&u16::try_from(name.len() + 5).unwrap().to_le_bytes());
        extra.push(1);
        extra.extend_from_slice(&crc32fast::hash(raw).to_le_bytes());
        extra.extend_from_slice(name);
        extra
    }

    #[test]
    fn resolves_only_the_crc_bound_utf8_name() -> Result<()> {
        let raw = b"Ramh\x9bj.xlsx";
        let expected = "Ramhøj.xlsx";
        let valid = field(raw, expected.as_bytes());
        assert_eq!(unicode_path(raw, &valid)?, Some(expected));
        assert_eq!(unicode_path(b"renamed.xlsx", &valid)?, None);
        let mut future = valid.clone();
        future[4] = 2;
        assert_eq!(unicode_path(raw, &future)?, None);
        let mut unrelated = vec![0xfe, 0xca, 1, 0, 9];
        unrelated.extend_from_slice(&valid);
        assert_eq!(unicode_path(raw, &unrelated)?, Some(expected));
        assert_eq!(unicode_path(raw, &[])?, None);
        Ok(())
    }

    #[test]
    fn rejects_truncation_invalid_utf8_and_ambiguous_names() {
        let raw = b"legacy.xlsx";
        let valid = field(raw, b"canonical.xlsx");
        let mut duplicate = valid.clone();
        duplicate.extend_from_slice(&field(raw, b"other.xlsx"));
        for extra in [
            vec![0x75],
            vec![0x75, 0x70, 8, 0, 1],
            field(raw, &[0xff]),
            duplicate,
        ] {
            assert!(unicode_path(raw, &extra).is_err());
        }
    }
}
