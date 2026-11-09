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
