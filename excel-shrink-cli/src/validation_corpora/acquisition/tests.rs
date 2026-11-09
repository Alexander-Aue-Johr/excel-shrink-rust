use super::*;
use crate::validation_corpora::{Archive, Workbook};
use std::io::Write;
use zip::{write::FileOptions, ZipWriter};

struct Fixture {
    root: PathBuf,
    artifacts: PathBuf,
    archive: PathBuf,
    profile: Profile,
}

impl Fixture {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().join(format!("corpus-acquire-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let archive = root.join("originals.zip");
        let mut writer = ZipWriter::new(File::create(&archive)?);
        let mut workbooks = Vec::new();
        for (index, name) in ["a.xlsx", "nested/b.xlsx"].into_iter().enumerate() {
            let content = format!("original-{index}");
            writer.start_file(name, FileOptions::default())?;
            writer.write_all(content.as_bytes())?;
            let source = root.join(format!("hash-{index}"));
            fs::write(&source, &content)?;
            workbooks.push(Workbook {
                index,
                name: name.into(),
                bytes: content.len() as u64,
                sha256: hash_file(&source)?,
                crc32: crc32fast::hash(content.as_bytes()),
            });
        }
        writer.finish()?;
        let profile = Profile {
            version: 1,
            id: "fixture".into(),
            name: "Fixture".into(),
            archive: Archive {
                url: "https://example.invalid/never-download.zip".into(),
                sha256: hash_file(&archive)?,
                bytes: fs::metadata(&archive)?.len(),
            },
            workbooks,
        };
        Ok(Self {
            artifacts: root.join("artifacts"),
            root,
            archive,
            profile,
        })
    }

    fn prepare(&self, archive: Option<&Path>) -> Result<Value> {
        prepare_profile(
            self.profile.clone(),
            &self.artifacts,
            &self.root.join("cache"),
            archive,
            &|_| {},
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn restores_only_missing_members_and_reuses_complete_files_without_archive() -> Result<()> {
    let fixture = Fixture::new()?;
    fs::create_dir(&fixture.artifacts)?;
    fs::write(fixture.artifacts.join("a.xlsx"), b"original-0")?;
    let original_time = fs::metadata(fixture.artifacts.join("a.xlsx"))?.modified()?;
    let result = fixture.prepare(Some(&fixture.archive))?;
    assert_eq!(result["added"], 1);
    assert_eq!(result["inspection"]["complete"], true);
    assert_eq!(
        fs::metadata(fixture.artifacts.join("a.xlsx"))?.modified()?,
        original_time
    );
    fs::remove_file(&fixture.archive)?;
    let result = fixture.prepare(None)?;
    assert_eq!(result["added"], 0);
    assert_eq!(result["archive_used"], false);
    assert!(!fixture.root.join("cache").exists());
    Ok(())
}

#[test]
fn acquires_the_registered_unicode_path_when_the_legacy_header_differs() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let raw = b"Ramh\x9bj.xlsx";
    let expected = "Ramhøj.xlsx";
    let placeholder = b"RamhXj.xlsx";
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer.start_file_with_extra_data("RamhXj.xlsx", FileOptions::default())?;
    // Independent fixture encoding of version-one Info-ZIP Unicode path metadata.
    // ZipWriter reserves 0x7075 behind an optional feature. Emit its documented
    // test field ID first and patch the two field IDs in the completed fixture.
    writer.write_all(&[0xef, 0xbe, 17, 0, 1])?;
    writer.write_all(&crc32fast::hash(raw).to_le_bytes())?;
    writer.write_all(expected.as_bytes())?;
    writer.end_extra_data()?;
    writer.write_all(b"original-0")?;
    let mut bytes = writer.finish()?.into_inner();
    let fields: Vec<_> = bytes
        .windows(5)
        .enumerate()
        .filter_map(|(position, value)| (value == [0xef, 0xbe, 17, 0, 1]).then_some(position))
        .collect();
    assert_eq!(fields.len(), 2);
    for position in fields {
        bytes[position] = 0x75;
        bytes[position + 1] = 0x70;
    }
    let positions: Vec<_> = bytes
        .windows(placeholder.len())
        .enumerate()
        .filter_map(|(position, value)| (value == placeholder).then_some(position))
        .collect();
    assert_eq!(positions.len(), 2);
    for position in positions {
        bytes[position + 4] = 0x9b;
    }
    fs::write(&fixture.archive, &bytes)?;
    fixture.profile.workbooks.truncate(1);
    fixture.profile.workbooks[0].name = expected.into();
    fixture.profile.archive.bytes = bytes.len() as u64;
    fixture.profile.archive.sha256 = hash_file(&fixture.archive)?;
    {
        let mut archive = ZipArchive::new(std::io::Cursor::new(&bytes))?;
        let member = archive.by_index(0)?;
        assert_eq!(member.name_raw(), raw);
        assert_ne!(member.name(), expected);
        assert!(member_name::matches(&member, expected)?);
        assert!(!member_name::matches(&member, "different.xlsx")?);
    }
    let result = fixture.prepare(Some(&fixture.archive))?;
    assert_eq!(result["inspection"]["complete"], true);
    assert_eq!(fs::read(fixture.artifacts.join(expected))?, b"original-0");
    Ok(())
}

#[test]
#[ignore = "Reads the private pinned Data.gov source archive from an explicit environment path"]
fn inspect_pinned_datagov_archive_member_names() -> Result<()> {
    let path = PathBuf::from(
        std::env::var_os("DELTASIEVE_DATAGOV_ARCHIVE").context("Set DELTASIEVE_DATAGOV_ARCHIVE")?,
    );
    let profile = super::super::profile("datagov")?;
    ensure!(
        fs::metadata(&path)?.len() == profile.archive.bytes,
        "Archive size differs"
    );
    ensure!(
        hash_file(&path)? == profile.archive.sha256,
        "Archive SHA-256 differs"
    );
    let mut archive = ZipArchive::new(File::open(&path)?)?;
    let mut unicode_aliases = 0;
    for workbook in &profile.workbooks {
        let member = archive.by_index_raw(workbook.index)?;
        ensure!(
            member_name::matches(&member, &workbook.name)?,
            "Name differs at {}",
            workbook.index
        );
        ensure!(
            member.size() == workbook.bytes && member.crc32() == workbook.crc32,
            "Member metadata differs at {}",
            workbook.index
        );
        unicode_aliases += usize::from(member.name() != workbook.name);
    }
    println!(
        "Verified {} pinned archive members, {unicode_aliases} Unicode path aliases",
        profile.workbooks.len()
    );
    Ok(())
}

#[test]
fn conflicting_originals_stop_before_downloading_or_replacing() -> Result<()> {
    let fixture = Fixture::new()?;
    fs::create_dir(&fixture.artifacts)?;
    fs::write(fixture.artifacts.join("a.xlsx"), b"user-data")?;
    let error = fixture.prepare(None).unwrap_err();
    assert!(error.to_string().contains("differ from the manifest"));
    assert_eq!(fs::read(fixture.artifacts.join("a.xlsx"))?, b"user-data");
    assert!(!fixture.artifacts.join("nested/b.xlsx").exists());
    assert!(!fixture.root.join("cache").exists());
    Ok(())
}

#[test]
fn progress_reports_verification_and_successfully_published_members() -> Result<()> {
    let fixture = Fixture::new()?;
    let events = std::sync::Mutex::new(Vec::new());
    prepare_profile(
        fixture.profile.clone(),
        &fixture.artifacts,
        &fixture.root.join("cache"),
        Some(&fixture.archive),
        &|event| events.lock().unwrap().push(event),
    )?;
    let events = events.into_inner().unwrap();
    assert_eq!(events.first().unwrap()["phase"], "checking_originals");
    assert_eq!(events.last().unwrap()["phase"], "verifying_originals");
    let extracted: Vec<_> = events
        .iter()
        .filter(|event| event["phase"] == "extracting")
        .collect();
    assert_eq!(extracted.len(), 2);
    assert_eq!(extracted[1]["added"], 2);
    assert_eq!(extracted[1]["member"], "nested/b.xlsx");
    Ok(())
}

#[test]
fn rejects_wrong_archive_and_preserves_verified_partial_progress() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.profile.archive.sha256 = "0".repeat(64);
    assert!(fixture
        .prepare(Some(&fixture.archive))
        .unwrap_err()
        .to_string()
        .contains("Archive bytes"));
    assert!(!fixture.artifacts.join("a.xlsx").exists());
    fixture.profile.archive.sha256 = hash_file(&fixture.archive)?;
    fixture.profile.workbooks[1].sha256 = "0".repeat(64);
    assert!(fixture
        .prepare(Some(&fixture.archive))
        .unwrap_err()
        .to_string()
        .contains("checksum differs"));
    assert_eq!(fs::read(fixture.artifacts.join("a.xlsx"))?, b"original-0");
    assert!(!fixture.artifacts.join("nested/b.xlsx").exists());
    assert_eq!(fs::read_dir(fixture.artifacts.join("nested"))?.count(), 0);
    fixture.profile.workbooks[1].sha256 = hash_file(&fixture.root.join("hash-1"))?;
    assert_eq!(fixture.prepare(Some(&fixture.archive))?["added"], 1);
    Ok(())
}

#[test]
fn refuses_concurrent_acquisition_and_create_only_publication() -> Result<()> {
    let fixture = Fixture::new()?;
    fs::create_dir(&fixture.artifacts)?;
    let lock = File::create(fixture.artifacts.join(".corpus-acquisition.lock"))?;
    lock.lock()?;
    assert!(fixture
        .prepare(Some(&fixture.archive))
        .unwrap_err()
        .to_string()
        .contains("already active"));
    drop(lock);
    let destination = fixture.artifacts.join("a.xlsx");
    fs::write(&destination, b"arrived-after-preflight")?;
    let temporary = fixture.artifacts.join("staging.partial");
    let mut member = &b"original-0"[..];
    assert!(publish_member(
        &mut member,
        &temporary,
        &destination,
        &fixture.profile.workbooks[0]
    )
    .is_err());
    assert_eq!(fs::read(destination)?, b"arrived-after-preflight");
    Ok(())
}
