//! Preserve archive member identities when a profile's originals already exist on disk.

use super::*;

pub(super) fn load(
    corpus: &Corpus,
    root: &Path,
    part: Option<selection::SizePart>,
) -> Result<Source> {
    let profile_id = corpus
        .profile
        .as_deref()
        .context("Missing corpus profile")?;
    let profile = crate::validation_corpora::profile(profile_id)?;
    ensure!(
        corpus_id(corpus)? == profile.archive.sha256,
        "Profile corpus identity must match its pinned archive SHA-256"
    );
    let path = resolve_path(root, PathBuf::from(&corpus.url));
    crate::validation_sources::require_regular_source(&path)?;
    ensure!(path.is_dir(), "Profile originals require a directory");
    let inspection = crate::validation_corpora::inspect(profile_id, &path)?;
    ensure!(
        inspection["complete"] == true,
        "Profile originals are missing or changed"
    );
    from_profile(corpus, profile, path, part)
}

fn from_profile(
    corpus: &Corpus,
    mut profile: crate::validation_corpora::Profile,
    path: PathBuf,
    part: Option<selection::SizePart>,
) -> Result<Source> {
    profile.workbooks.sort_by_key(|entry| entry.index);
    let members: Vec<_> = profile
        .workbooks
        .iter()
        .map(|entry| (entry.index, entry.name.clone()))
        .collect();
    // Keep the exact ZIP catalog representation so partitions and resume fingerprints agree.
    let catalog: Vec<_> = profile
        .workbooks
        .iter()
        .map(|entry| (&entry.name, Some((entry.crc32, entry.bytes))))
        .collect();
    let fingerprint = artifact_cache::sha256(&serde_json::to_vec(&catalog)?);
    let sized = profile
        .workbooks
        .iter()
        .map(|entry| selection::CatalogMember {
            archive_index: entry.index,
            member_name: entry.name.clone(),
            bytes: Some(entry.bytes),
            crc32: Some(entry.crc32),
        })
        .collect();
    let member_sha256 = profile
        .workbooks
        .into_iter()
        .map(|entry| (entry.index, entry.sha256))
        .collect();
    Ok(Source {
        id: corpus_id(corpus)?,
        name: corpus.name.clone(),
        path,
        members,
        fingerprint,
        selection: selection::Plan::new(sized, part)?,
        member_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation_corpora::{Archive, Profile, Workbook};
    use std::io::Write;

    #[test]
    fn profile_source_preserves_zip_fingerprint_member_indices_and_size_partitions() -> Result<()> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in [
            ("README.txt", &b"readme"[..]),
            ("a.xlsx", &b"a"[..]),
            ("b.xlsx", &b"bbbb"[..]),
        ] {
            writer.start_file(name, zip::write::FileOptions::default())?;
            writer.write_all(bytes)?;
        }
        let archive = writer.finish()?.into_inner();
        let hash = artifact_cache::sha256(&archive);
        let mut zip = ZipArchive::new(std::io::Cursor::new(&archive))?;
        let mut workbooks = Vec::new();
        let mut zip_catalog = Vec::new();
        let mut sized = Vec::new();
        for (index, content) in [(1, &b"a"[..]), (2, &b"bbbb"[..])] {
            let entry = zip.by_index(index)?;
            let name = entry.name().to_owned();
            zip_catalog.push((name.clone(), Some((entry.crc32(), entry.size()))));
            sized.push(selection::CatalogMember {
                archive_index: index,
                member_name: name.clone(),
                bytes: Some(entry.size()),
                crc32: Some(entry.crc32()),
            });
            workbooks.push(Workbook {
                index,
                name,
                bytes: entry.size(),
                crc32: entry.crc32(),
                sha256: artifact_cache::sha256(content),
            });
        }
        let mut profile = Profile {
            version: 1,
            id: "fixture".into(),
            name: "Fixture".into(),
            workbooks,
            archive: Archive {
                url: "https://example.invalid/source.zip".into(),
                sha256: hash.clone(),
                bytes: archive.len() as u64,
            },
        };
        profile.workbooks.reverse();
        let corpus = Corpus {
            name: "Fixture".into(),
            url: "originals".into(),
            id: Some(hash),
            total_workbooks: Some(2),
            profile: Some("fixture".into()),
        };
        for part in [
            None,
            Some(selection::SizePart { part: 1, parts: 2 }),
            Some(selection::SizePart { part: 2, parts: 2 }),
        ] {
            let source = from_profile(&corpus, profile.clone(), PathBuf::from("originals"), part)?;
            assert_eq!(
                source.fingerprint,
                artifact_cache::sha256(&serde_json::to_vec(&zip_catalog)?)
            );
            assert_eq!(
                source.members,
                vec![(1, "a.xlsx".into()), (2, "b.xlsx".into())]
            );
            assert_eq!(source.member_sha256[&1], artifact_cache::sha256(b"a"));
            let expected = selection::Plan::new(sized.clone(), part)?;
            assert_eq!(
                serde_json::to_value(&source.selection)?,
                serde_json::to_value(expected)?
            );
        }
        Ok(())
    }
}
