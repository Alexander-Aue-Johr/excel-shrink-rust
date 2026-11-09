//! ZIP-directory measurements; worksheet XML is never decompressed or parsed here.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs::File, path::Path};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorksheetSize {
    pub part: String,
    pub compressed_bytes: u64,
    pub uncompressed_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PackageSizes {
    pub file_bytes: u64,
    pub compressed_payload_bytes: u64,
    pub uncompressed_payload_bytes: u64,
    pub worksheets: Vec<WorksheetSize>,
}

pub fn inspect(path: &Path) -> Result<PackageSizes> {
    let file =
        File::open(path).with_context(|| format!("Open package sizes for {}", path.display()))?;
    let file_bytes = file.metadata()?.len();
    let mut archive = zip::ZipArchive::new(file)?;
    let mut result = PackageSizes {
        file_bytes,
        compressed_payload_bytes: 0,
        uncompressed_payload_bytes: 0,
        worksheets: Vec::new(),
    };
    for index in 0..archive.len() {
        let member = archive.by_index_raw(index)?;
        if member.is_dir() {
            continue;
        }
        result.compressed_payload_bytes = result
            .compressed_payload_bytes
            .checked_add(member.compressed_size())
            .context("Compressed package size overflow")?;
        result.uncompressed_payload_bytes = result
            .uncompressed_payload_bytes
            .checked_add(member.size())
            .context("Uncompressed package size overflow")?;
        if member.name().starts_with("xl/worksheets/") && member.name().ends_with(".xml") {
            result.worksheets.push(WorksheetSize {
                part: member.name().to_owned(),
                compressed_bytes: member.compressed_size(),
                uncompressed_bytes: member.size(),
            });
        }
    }
    result
        .worksheets
        .sort_by(|left, right| left.part.cmp(&right.part));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn measures_xml_bytes_without_requiring_valid_xml() -> Result<()> {
        let path = std::env::temp_dir().join(format!("zip-sizes-{}.xlsx", uuid::Uuid::new_v4()));
        let mut writer = zip::ZipWriter::new(File::create(&path)?);
        writer.start_file(
            "xl/worksheets/sheet1.xml",
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )?;
        writer.write_all(&vec![b'x'; 8192])?;
        writer.start_file(
            "xl/worksheets/_rels/sheet1.xml.rels",
            zip::write::FileOptions::default(),
        )?;
        writer.write_all(b"relationship")?;
        writer.finish()?;
        let sizes = inspect(&path)?;
        assert_eq!(sizes.worksheets.len(), 1);
        assert_eq!(sizes.worksheets[0].uncompressed_bytes, 8192);
        assert!(sizes.worksheets[0].compressed_bytes < 8192);
        assert_eq!(sizes.uncompressed_payload_bytes, 8192 + 12);
        assert!(sizes.file_bytes > sizes.compressed_payload_bytes);
        std::fs::remove_file(path)?;
        Ok(())
    }
}
