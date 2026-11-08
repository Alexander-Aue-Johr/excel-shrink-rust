//! Check the extracted public API against the existing version-one persisted contract.
use anyhow::Result;
use deltasieve_engine::reuse::{PairFingerprint, PartSelection};
use std::{fs, io::Write};
use zip::{write::FileOptions, ZipWriter};

#[test]
fn public_api_retains_version_one_fingerprint_content() -> Result<()> {
    let root = std::env::temp_dir().join(format!("deltasieve-contract-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root)?;
    for (name, content) in [("original.xlsx", b"a"), ("candidate.xlsx", b"b")] {
        let mut archive = ZipWriter::new(fs::File::create(root.join(name))?);
        archive.start_file("xl/worksheets/sheet1.xml", FileOptions::default())?;
        archive.write_all(content)?;
        archive.finish()?;
    }
    let current = PairFingerprint::read(
        &root.join("original.xlsx"),
        &root.join("candidate.xlsx"),
        &PartSelection::default(),
    )?;
    let frozen = include_str!("fixtures/fingerprint-v1.json");
    let previous: PairFingerprint = serde_json::from_str(frozen)?;
    assert!(current.matches_previous(&previous));
    assert_eq!(
        serde_json::to_value(current)?,
        serde_json::from_str::<serde_json::Value>(frozen)?,
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
