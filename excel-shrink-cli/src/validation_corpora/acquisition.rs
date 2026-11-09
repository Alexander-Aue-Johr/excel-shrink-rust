//! Resume manifest-backed acquisition without replacing existing originals.

use super::{profile, Profile};
use crate::validation_sources::{cached_download, hash_file, require_regular_source};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
};
use zip::ZipArchive;

/// Reuse complete originals before consulting an optional archive or the network.
/// A conflicting original is an error. Successfully published members survive retries.
pub(crate) fn prepare(
    id: &str,
    artifacts: &Path,
    cache: &Path,
    archive: Option<&Path>,
) -> Result<Value> {
    prepare_profile(profile(id)?, artifacts, cache, archive)
}

fn prepare_profile(
    profile: Profile,
    artifacts: &Path,
    cache: &Path,
    archive: Option<&Path>,
) -> Result<Value> {
    fs::create_dir_all(artifacts)?;
    require_regular_source(artifacts)?;
    let artifacts = fs::canonicalize(artifacts)?;
    let lock_path = artifacts.join(".corpus-acquisition.lock");
    match fs::symlink_metadata(&lock_path) {
        Ok(_) => require_regular_source(&lock_path)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    lock.try_lock()
        .context("Corpus acquisition is already active")?;
    let before = super::inspect_profile(profile.clone(), &artifacts)?;
    ensure!(
        before["changed_count"] == 0,
        "Existing corpus files differ from the manifest; preserve or relocate them before acquisition"
    );
    if before["complete"] == true {
        return Ok(json!({"inspection":before,"added":0,"archive_used":false}));
    }
    let archive = match archive {
        Some(path) => path.to_path_buf(),
        None => cached_download(&profile.archive.url, cache, false)?,
    };
    crate::validation_sources::require_regular_file(&archive)?;
    ensure!(
        fs::metadata(&archive)?.len() == profile.archive.bytes
            && hash_file(&archive)?.eq_ignore_ascii_case(&profile.archive.sha256),
        "Archive bytes or SHA-256 differ from the versioned corpus profile"
    );
    let mut zip = ZipArchive::new(File::open(&archive)?)?;
    let mut added = 0;
    for workbook in &profile.workbooks {
        let destination = artifacts.join(&workbook.name);
        match fs::symlink_metadata(&destination) {
            Ok(_) => continue, // The locked preflight verified present originals; the final pass checks races.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("Inspect corpus destination"),
        }
        let mut member = zip.by_index(workbook.index)?;
        ensure!(
            member.name() == workbook.name && member.size() == workbook.bytes && !member.is_dir(),
            "Archive member identity differs for {}",
            workbook.name
        );
        let parent = confined_parent(&artifacts, &workbook.name)?;
        let temporary = parent.join(format!(".corpus-{}.partial", uuid::Uuid::new_v4()));
        let result = publish_member(&mut member, &temporary, &destination, workbook);
        // This is our unique staging file. Published originals are never cleanup targets.
        let cleanup = if temporary.exists() {
            fs::remove_file(&temporary).context("Remove corpus staging file")
        } else {
            Ok(())
        };
        result?;
        cleanup?;
        added += 1;
    }
    let inspection = super::inspect_profile(profile, &artifacts)?;
    ensure!(
        inspection["complete"] == true,
        "Corpus changed during acquisition"
    );
    Ok(json!({"inspection":inspection,"added":added,"archive_used":true}))
}

/// Reject links and junctions before creating each manifest-owned parent directory.
fn confined_parent(root: &Path, name: &str) -> Result<PathBuf> {
    let name = crate::validation_sources::relative_name(name)?;
    let mut parent = root.to_path_buf();
    for component in Path::new(&name)
        .parent()
        .context("Corpus member parent")?
        .components()
    {
        parent.push(component);
        if !parent.exists() {
            fs::create_dir(&parent)?;
        }
        require_regular_source(&parent)?;
        ensure!(
            parent.is_dir() && fs::canonicalize(&parent)?.starts_with(root),
            "Corpus member parent escapes its artifacts directory"
        );
    }
    Ok(parent)
}

fn publish_member(
    member: &mut impl Read,
    temporary: &Path,
    destination: &Path,
    workbook: &super::Workbook,
) -> Result<()> {
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temporary)?;
    let bytes = io::copy(
        &mut member.take(workbook.bytes.saturating_add(1)),
        &mut output,
    )?;
    output.sync_all()?;
    drop(output);
    ensure!(
        bytes == workbook.bytes && hash_file(temporary)?.eq_ignore_ascii_case(&workbook.sha256),
        "Extracted workbook checksum differs for {}",
        workbook.name
    );
    // Reuse the host's create-only publication, including Windows volumes without hard links.
    crate::capture_host::publish_new(temporary, destination).with_context(|| {
        format!(
            "Publish original without replacement: {}",
            destination.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests;
