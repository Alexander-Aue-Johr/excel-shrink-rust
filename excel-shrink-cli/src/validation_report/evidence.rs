//! Copy only report-addressed evidence and publish its integrity manifest last.

use super::{difference_count, read_json_file, PackagedComparison, FORMAT_VERSION};
use anyhow::{bail, ensure, Context, Result};
use image::ImageEncoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct FileIdentity {
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct Manifest {
    version: u32,
    report: FileIdentity,
    pub images: BTreeMap<String, FileIdentity>,
}

pub(super) struct ImageStore {
    source: PathBuf,
    destination: PathBuf,
    images: BTreeMap<String, FileIdentity>,
}

impl ImageStore {
    pub fn create(source: &Path, destination: &Path) -> Result<Self> {
        let source = source
            .canonicalize()
            .context("Resolve comparison evidence directory")?;
        ensure!(source.is_dir(), "Comparison evidence must be a directory");
        let destination = create_destination(&source, destination)?;
        fs::create_dir(destination.join("evidence"))?;
        Ok(Self {
            source,
            destination,
            images: BTreeMap::new(),
        })
    }

    /// Keep both sides, charts, legacy projections, and existing retention
    /// references. Paths that describe workbooks or cache provenance stay data.
    pub fn retain_report_images(&mut self, report: &mut Value) -> Result<()> {
        let mut references = BTreeSet::new();
        collect_image_references(report, &mut references)?;
        let mut aliases = BTreeMap::new();
        for original in references {
            let source = self.source_image(&original)?;
            let retained = self.retain_image(&source)?;
            aliases.insert(original, retained);
        }
        replace_references(report, &aliases);
        Ok(())
    }

    fn source_image(&self, recorded: &str) -> Result<PathBuf> {
        let recorded = Path::new(recorded);
        // Raw targeted runs may have recorded paths relative to the invoking
        // working directory. Portable reports always use report-relative paths.
        let candidates = if recorded.is_absolute() {
            vec![recorded.to_owned()]
        } else {
            vec![self.source.join(recorded), recorded.to_owned()]
        };
        for candidate in candidates {
            if let Ok(path) = candidate.canonicalize() {
                ensure!(
                    path.starts_with(&self.source),
                    "Image escapes its comparison directory: {}",
                    candidate.display()
                );
                ensure!(
                    path.is_file(),
                    "Image reference is not a regular file: {}",
                    candidate.display()
                );
                return Ok(path);
            }
        }
        bail!(
            "Referenced comparison image is missing: {}",
            recorded.display()
        )
    }

    fn retain_image(&mut self, source: &Path) -> Result<String> {
        let reader = image::ImageReader::open(source)?.with_guessed_format()?;
        let extension = match reader.format() {
            Some(image::ImageFormat::Png) => "png",
            Some(image::ImageFormat::Jpeg) => "jpg",
            _ => bail!(
                "Expected retained PNG or JPEG evidence: {}",
                source.display()
            ),
        };
        let identity = file_identity(source)?;
        let relative = format!("evidence/{}.{}", identity.sha256, extension);
        if !self.images.contains_key(&relative) {
            let destination = self.destination.join(&relative);
            copy_verified_file(source, &destination, &identity)?;
            self.images.insert(relative.clone(), identity);
        }
        Ok(relative)
    }

    pub fn retain_overlay(&mut self, overlay: &image::RgbaImage) -> Result<String> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes).write_image(
            overlay.as_raw(),
            overlay.width(),
            overlay.height(),
            image::ExtendedColorType::Rgba8,
        )?;
        let identity = bytes_identity(&bytes);
        let relative = format!("evidence/{}.png", identity.sha256);
        if !self.images.contains_key(&relative) {
            write_new(&self.destination.join(&relative), &bytes)?;
            self.images.insert(relative.clone(), identity);
        }
        Ok(relative)
    }

    pub fn path(&self, relative: &str) -> Result<PathBuf> {
        ensure!(
            self.images.contains_key(relative),
            "Image was not retained by this package"
        );
        Ok(self.destination.join(relative))
    }

    /// The manifest is the completion marker. No caller may regard a directory
    /// left behind without it as a successfully retained comparison.
    pub fn publish(self, report: &Value) -> Result<PackagedComparison> {
        let bytes = serde_json::to_vec(report)?;
        let manifest = Manifest {
            version: FORMAT_VERSION,
            report: bytes_identity(&bytes),
            images: self.images,
        };
        write_new(&self.destination.join("report.json"), &bytes)?;
        write_new(
            &self.destination.join("bundle.json"),
            &serde_json::to_vec_pretty(&manifest)?,
        )?;
        Ok(PackagedComparison {
            report_path: self.destination.join("report.json"),
            image_count: manifest.images.len(),
            different_regions: difference_count(report),
        })
    }
}

pub(super) fn verify_package(directory: &Path) -> Result<Manifest> {
    let root = directory
        .canonicalize()
        .context("Resolve retained comparison")?;
    let manifest_path = confined_file(&root, "bundle.json")?;
    let manifest: Manifest = serde_json::from_value(read_json_file(&manifest_path)?)?;
    ensure!(
        manifest.version == FORMAT_VERSION,
        "Unsupported portable comparison version"
    );
    let report_path = confined_file(&root, "report.json")?;
    verify_file(&report_path, &manifest.report)?;
    let report = read_json_file(&report_path)?;
    ensure!(
        report["portable_report"]["version"] == FORMAT_VERSION,
        "Missing portable report metadata"
    );
    let mut references = BTreeSet::new();
    collect_image_references(&report, &mut references)?;
    ensure!(
        references
            .iter()
            .all(|name| manifest.images.contains_key(name)),
        "Report references an image not listed in its integrity manifest"
    );
    for (relative, identity) in &manifest.images {
        ensure!(
            is_package_image_path(relative),
            "Unexpected file in evidence manifest: {relative}"
        );
        verify_file(&confined_file(&root, relative)?, identity)?;
    }
    Ok(manifest)
}

pub(super) fn copy_verified_package(
    source: &Path,
    destination: &Path,
) -> Result<PackagedComparison> {
    let manifest = verify_package(source)?;
    let source = source.canonicalize()?;
    let destination = create_destination(&source, destination)?;
    fs::create_dir(destination.join("evidence"))?;
    for (relative, identity) in &manifest.images {
        copy_verified_file(
            &confined_file(&source, relative)?,
            &destination.join(relative),
            identity,
        )?;
    }
    copy_verified_file(
        &source.join("report.json"),
        &destination.join("report.json"),
        &manifest.report,
    )?;
    write_new(
        &destination.join("bundle.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    let report = read_json_file(&destination.join("report.json"))?;
    Ok(PackagedComparison {
        report_path: destination.join("report.json"),
        image_count: manifest.images.len(),
        different_regions: difference_count(&report),
    })
}

/// Preserve the selected-part fingerprint alongside an exported baseline. This
/// is separate from the immutable image bundle because it belongs to the run's
/// explicit reuse policy, not to the earlier scientific observation.
pub(super) fn copy_identity(
    source: &Path,
    destination: &Path,
    relative: &str,
    expected_sha256: Option<&str>,
) -> Result<()> {
    ensure!(
        relative.starts_with("identities/") && relative.ends_with(".json"),
        "Unexpected identity metadata path: {relative}"
    );
    let path = confined_file(source, relative)?;
    ensure!(
        read_json_file(&path)?.is_object(),
        "Identity metadata must be an object"
    );
    let identity = file_identity(&path)?;
    if let Some(expected) = expected_sha256 {
        ensure!(
            identity.sha256 == expected,
            "Selected-part identity changed after publication: {relative}"
        );
    }
    let target = destination.join(relative_path(relative)?);
    fs::create_dir_all(target.parent().context("Identity metadata has no parent")?)?;
    copy_verified_file(&path, &target, &identity)
}

fn collect_image_references(value: &Value, references: &mut BTreeSet<String>) -> Result<()> {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                if key.ends_with("_png")
                    || matches!(key.as_str(), "left_image" | "right_image" | "overlay_url")
                {
                    add_reference(child, references)?;
                } else if key == "images" && child.is_object() {
                    for image in child.as_object().unwrap().values() {
                        add_reference(image, references)?;
                    }
                } else if key == "file" && child.as_str().is_some_and(is_image_path) {
                    add_reference(child, references)?;
                } else {
                    collect_image_references(child, references)?;
                }
            }
        }
        Value::Array(children) => {
            for child in children {
                collect_image_references(child, references)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn add_reference(value: &Value, references: &mut BTreeSet<String>) -> Result<()> {
    if value.is_null() {
        return Ok(());
    }
    let path = value
        .as_str()
        .filter(|path| !path.is_empty())
        .context("Invalid retained image reference")?;
    references.insert(path.to_owned());
    Ok(())
}

fn is_image_path(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".image"]
        .iter()
        .any(|extension| value.ends_with(extension))
}

fn is_package_image_path(value: &str) -> bool {
    let Some(file) = value.strip_prefix("evidence/") else {
        return false;
    };
    let Some((hash, extension)) = file.rsplit_once('.') else {
        return false;
    };
    matches!(extension, "png" | "jpg")
        && hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn replace_references(value: &mut Value, aliases: &BTreeMap<String, String>) {
    match value {
        Value::String(path) => {
            if let Some(retained) = aliases.get(path) {
                *path = retained.clone();
            }
        }
        Value::Object(fields) => {
            for child in fields.values_mut() {
                replace_references(child, aliases);
            }
        }
        Value::Array(children) => {
            for child in children {
                replace_references(child, aliases);
            }
        }
        _ => {}
    }
}

/// Report manifests use URL-like relative paths on every platform. Reject
/// traversal and Windows path syntax before applying filesystem confinement.
pub(super) fn relative_path(value: &str) -> Result<&Path> {
    ensure!(
        !value.is_empty()
            && !value.contains(['\\', ':'])
            && !value.chars().any(char::is_control)
            && value
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "Invalid report-relative path: {value:?}"
    );
    let path = Path::new(value);
    ensure!(
        path.components()
            .all(|part| matches!(part, Component::Normal(_))),
        "Report path must be relative: {value:?}"
    );
    Ok(path)
}

/// Resolve beneath an already canonical root, rejecting links at every level.
pub(super) fn confined_file(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = relative_path(relative)?;
    let mut path = root.to_owned();
    for part in relative.components() {
        path.push(part.as_os_str());
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "Report paths must not traverse links"
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "Report paths must not traverse reparse points"
            );
        }
    }
    let path = path.canonicalize()?;
    ensure!(
        path.starts_with(root) && path.is_file(),
        "Report file escapes its root or is not a file"
    );
    Ok(path)
}

pub(super) fn create_destination(source: &Path, destination: &Path) -> Result<PathBuf> {
    ensure!(
        !destination.try_exists()?,
        "Report destination already exists: {}",
        destination.display()
    );
    let absolute = std::path::absolute(destination)?;
    let parent = absolute
        .parent()
        .context("Report destination has no parent")?;
    // Resolve the nearest existing ancestor before creating anything. A rejected
    // nested export must not create directories inside its read-only source.
    let mut ancestor = parent;
    let mut missing = Vec::new();
    while !ancestor.try_exists()? {
        missing.push(ancestor.file_name().context("Invalid report parent")?);
        ancestor = ancestor
            .parent()
            .context("Report destination has no existing ancestor")?;
    }
    let mut parent = ancestor.canonicalize()?;
    for name in missing.into_iter().rev() {
        parent.push(name);
    }
    let destination = parent.join(absolute.file_name().context("Invalid report destination")?);
    ensure!(
        !destination.starts_with(source),
        "Retained reports must be outside the disposable source directory"
    );
    fs::create_dir_all(&parent)?;
    fs::create_dir(&destination)?;
    Ok(destination)
}

fn bytes_identity(bytes: &[u8]) -> FileIdentity {
    FileIdentity {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        bytes: bytes.len() as u64,
    }
}

fn file_identity(path: &Path) -> Result<FileIdentity> {
    let mut source = File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        bytes += count as u64;
    }
    Ok(FileIdentity {
        sha256: format!("{:x}", digest.finalize()),
        bytes,
    })
}

fn verify_file(path: &Path, expected: &FileIdentity) -> Result<()> {
    ensure!(
        expected.sha256.len() == 64 && expected.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid retained file digest"
    );
    let actual = file_identity(path)?;
    ensure!(
        actual.sha256 == expected.sha256 && actual.bytes == expected.bytes,
        "Retained evidence failed its integrity check: {}",
        path.display()
    );
    Ok(())
}

fn copy_verified_file(source: &Path, destination: &Path, identity: &FileIdentity) -> Result<()> {
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    verify_file(destination, identity)
}

pub(super) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    Ok(())
}
