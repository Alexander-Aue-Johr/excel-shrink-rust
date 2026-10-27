//! Manual inspection uses immutable prepared inputs, never a capture client's
//! workbook. Each click gets its own copy so edits cannot affect a later test.
use super::snapshots::{confined_path, safe_id};
use anyhow::{ensure, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) const PROFILES: [&str; 4] = [
    "original",
    "default",
    "maximum-cleaning",
    "too-aggressive-cleaning",
];

fn bundle_path(root: &Path, id: &str) -> Result<PathBuf> {
    safe_id(id)?;
    let study = root.parent().context("Missing study directory.")?;
    confined_path(study, &format!("prepared/{id}.zip"))
}

pub(super) fn available_profiles(root: &Path, id: &str) -> Result<Vec<&'static str>> {
    let path = bundle_path(root, id)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    Ok(PROFILES
        .into_iter()
        .filter(|profile| archive.by_name(&format!("{profile}.xlsx")).is_ok())
        .collect())
}

pub(super) fn copy_workbook(
    root: &Path,
    id: &str,
    profile: &str,
    destination: &Path,
) -> Result<PathBuf> {
    ensure!(PROFILES.contains(&profile), "Unknown inspection profile.");
    let file = fs::File::open(bundle_path(root, id)?)
        .context("No prepared workbook copy is available for this group.")?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut member = archive
        .by_name(&format!("{profile}.xlsx"))
        .context("This workbook variant is not available in the prepared bundle.")?;
    ensure!(!member.is_dir(), "Prepared workbook is not a file.");
    fs::create_dir_all(destination)?;
    let path = destination.join(format!("{id}-{profile}.xlsx"));
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    // Copy only the allowlisted member, not paths supplied by the archive.
    std::io::copy(&mut member, &mut output)?;
    Ok(path)
}

pub(super) fn open_copy(root: &Path, id: &str, profile: &str) -> Result<PathBuf> {
    let directory = std::env::temp_dir()
        .join("excel-shrink-inspection")
        .join(uuid::Uuid::new_v4().to_string());
    let path = copy_workbook(root, id, profile, &directory)?;
    open_visible_excel(&path)?;
    Ok(path)
}

#[cfg(windows)]
fn open_visible_excel(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let wide = |value: &std::ffi::OsStr| value.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let operation = wide(std::ffi::OsStr::new("open"));
    let executable = wide(std::ffi::OsStr::new("excel.exe"));
    let parameters = wide(std::ffi::OsStr::new(&format!("/x \"{}\"", path.display())));
    // The shell resolves Excel's registered App Path. /x creates a visible
    // inspection instance independent of the hidden automation workbooks.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            executable.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    ensure!(
        result > 32,
        "Windows could not launch visible Excel (shell error {result}). Inspection copy: {}",
        path.display()
    );
    Ok(())
}

#[cfg(not(windows))]
fn open_visible_excel(_path: &Path) -> Result<()> {
    anyhow::bail!("Opening inspection copies in Excel requires Windows.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn inspection_copies_exact_variants_without_modifying_prepared_inputs() -> Result<()> {
        let base = std::env::temp_dir().join(format!("inspection-test-{}", uuid::Uuid::new_v4()));
        let root = base.join("study/control-room");
        fs::create_dir_all(&root)?;
        fs::create_dir_all(base.join("study/prepared"))?;
        let bundle = bundle_path(&root, "workbook")?;
        let mut zip = zip::ZipWriter::new(fs::File::create(&bundle)?);
        for (name, bytes) in [
            ("original.xlsx", b"original".as_slice()),
            ("maximum-cleaning.xlsx", b"maximum".as_slice()),
            ("../outside.xlsx", b"outside".as_slice()),
        ] {
            zip.start_file(name, zip::write::FileOptions::default())?;
            zip.write_all(bytes)?;
        }
        zip.finish()?;
        let before = fs::read(&bundle)?;
        assert_eq!(
            available_profiles(&root, "workbook")?,
            vec!["original", "maximum-cleaning"]
        );
        let copy = copy_workbook(&root, "workbook", "maximum-cleaning", &base.join("copies"))?;
        assert_eq!(fs::read(&copy)?, b"maximum");
        fs::write(&copy, "inspection edits")?;
        assert_eq!(fs::read(&bundle)?, before);
        assert!(
            copy_workbook(&root, "workbook", "maximum-cleaning", &base.join("copies")).is_err()
        );
        assert!(copy_workbook(&root, "../escape", "original", &base.join("other")).is_err());
        assert!(copy_workbook(&root, "workbook", "../outside", &base.join("other")).is_err());
        assert!(copy_workbook(&root, "workbook", "default", &base.join("other")).is_err());
        assert!(available_profiles(&root, "absent")?.is_empty());
        fs::remove_dir_all(base)?;
        Ok(())
    }
}
