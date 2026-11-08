use super::*;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("sheet-scope-source-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let mut archive = ZipWriter::new(File::create(path).unwrap());
    for (name, bytes) in entries {
        archive.start_file(*name, FileOptions::default()).unwrap();
        archive.write_all(bytes).unwrap();
    }
    archive.finish().unwrap();
}

#[test]
fn local_directories_are_not_copied_and_excel_lock_files_are_ignored() -> Result<()> {
    let test = TestDirectory::new();
    let input = test.0.join("input");
    fs::create_dir_all(input.join("nested"))?;
    fs::write(input.join("nested/old.xls"), b"legacy workbook fixture")?;
    fs::write(input.join("current.xlsx"), b"xlsx fixture")?;
    fs::write(input.join("~$current.xlsx"), b"lock fixture")?;
    fs::write(input.join("notes.txt"), b"not a workbook")?;
    let unused_work = test.0.join("not-created");
    let source = materialize(
        input.to_str().unwrap(),
        &unused_work,
        &test.0.join("cache"),
        false,
    )?;
    assert_eq!(source.root, fs::canonicalize(&input)?);
    assert_eq!(
        source.files.keys().cloned().collect::<Vec<_>>(),
        ["current.xlsx", "nested/old.xls"]
    );
    assert!(!unused_work.exists());
    assert!(source.archive.is_none());
    Ok(())
}

#[test]
fn corpus_archives_preserve_relative_names_and_only_extract_workbooks() -> Result<()> {
    let test = TestDirectory::new();
    let archive = test.0.join("input.zip");
    make_zip(
        &archive,
        &[
            ("nested/a.xlsx", b"first"),
            ("b.xls", b"second"),
            ("readme.txt", b"notes"),
        ],
    );
    let source = materialize(
        archive.to_str().unwrap(),
        &test.0.join("work"),
        &test.0.join("cache"),
        false,
    )?;
    assert_eq!(source.archive, Some(fs::canonicalize(&archive)?));
    assert_eq!(fs::read(&source.files["nested/a.xlsx"])?, b"first");
    assert!(!source.root.join("readme.txt").exists());
    let published = test.0.join("published.zip");
    archive_files(&source.files, &published)?;
    let unpacked = extract_archive(&published, &test.0.join("roundtrip"))?;
    assert_eq!(fs::read(&unpacked.files["b.xls"])?, b"second");
    assert!(archive_files(&source.files, &published).is_err());
    Ok(())
}

#[test]
fn unsafe_or_case_ambiguous_members_are_rejected_before_extraction() {
    for entries in [
        vec![("../outside.xlsx", b"bad".as_slice())],
        vec![
            ("safe.xlsx", b"one".as_slice()),
            ("SAFE.xlsx", b"two".as_slice()),
        ],
        vec![
            ("folder/a.xlsx", b"one".as_slice()),
            ("Folder/b.xlsx", b"two".as_slice()),
        ],
        vec![
            ("folder", b"file".as_slice()),
            ("folder/b.xlsx", b"two".as_slice()),
        ],
        vec![("nested\\bad.xlsx", b"bad".as_slice())],
        vec![
            ("safe.xlsx", b"one".as_slice()),
            ("../unrelated.txt", b"bad".as_slice()),
        ],
    ] {
        let test = TestDirectory::new();
        let archive = test.0.join("input.zip");
        make_zip(&archive, &entries);
        let destination = test.0.join("destination");
        assert!(
            extract_archive(&archive, &destination).is_err(),
            "{entries:?}"
        );
        assert!(!destination.exists());
        assert!(!test.0.join("outside.xlsx").exists());
    }
}

#[test]
fn extraction_never_cleans_an_existing_directory() -> Result<()> {
    let test = TestDirectory::new();
    let archive = test.0.join("input.zip");
    make_zip(&archive, &[("a.xlsx", b"new")]);
    let destination = test.0.join("existing");
    fs::create_dir(&destination)?;
    fs::write(destination.join("keep.txt"), b"user data")?;
    assert!(extract_archive(&archive, &destination).is_err());
    assert_eq!(fs::read(destination.join("keep.txt"))?, b"user data");
    Ok(())
}

#[test]
fn portable_names_reject_devices_and_links_without_interpreting_them() {
    for path in [
        "/absolute.xlsx",
        "C:/book.xlsx",
        "a//b.xlsx",
        "a/./b.xlsx",
        "NUL.xlsx",
        "COM1/file.xlsx",
        "a./b.xlsx",
    ] {
        assert!(relative_name(path).is_err(), "{path}");
    }
    assert!(relative_name("public tables/Jahresübersicht.xlsx").is_ok());
    assert!(regular_zip_mode(Some(0o100644)));
    assert!(regular_zip_mode(Some(0o040755)));
    assert!(!regular_zip_mode(Some(0o120777)));
    assert!(!regular_zip_mode(Some(0o010644)));
}

#[test]
fn download_settings_cannot_inject_another_curl_option() {
    assert_eq!(
        curl_string("https://example.test/x?a=\"b\"\\c").unwrap(),
        "\"https://example.test/x?a=\\\"b\\\"\\\\c\""
    );
    assert!(curl_string("x\nurl = another").is_err());
}

#[test]
fn cached_download_reuse_checks_the_published_content() -> Result<()> {
    let test = TestDirectory::new();
    let original = test.0.join("download.zip");
    make_zip(&original, &[("a.xlsx", b"first")]);
    let digest = hash_file(&original)?;
    let target = test.0.join(format!("{digest}.zip"));
    fs::rename(original, &target)?;
    let index = test.0.join("current.json");
    let record = DownloadRecord {
        schema: DOWNLOAD_SCHEMA,
        file: target.file_name().unwrap().to_string_lossy().into_owned(),
        sha256: digest,
        bytes: fs::metadata(&target)?.len(),
    };
    fs::write(&index, serde_json::to_vec(&record)?)?;
    assert_eq!(read_cached_archive(&index, &test.0)?, target);
    make_zip(&target, &[("a.xlsx", b"other")]);
    assert!(read_cached_archive(&index, &test.0).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn directory_sources_reject_symlinks_instead_of_following_them() -> Result<()> {
    let test = TestDirectory::new();
    fs::write(test.0.join("real.xlsx"), b"source")?;
    std::os::unix::fs::symlink(test.0.join("real.xlsx"), test.0.join("alias.xlsx"))?;
    assert!(discover(&test.0).is_err());
    Ok(())
}
