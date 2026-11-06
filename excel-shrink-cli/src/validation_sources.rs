//! Input acquisition for SheetScope, independent of workbook processing.
//!
//! A source is an immutable set of named workbooks. Local directories are read
//! in place; ZIP members and completed HTTP(S) downloads have separate lifetimes.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use walkdir::WalkDir;
use zip::{write::FileOptions, CompressionMethod, ZipArchive, ZipWriter};

#[cfg(test)]
mod tests;

const MAX_ARCHIVE_MEMBERS: usize = 100_000;
const MAX_EXPANDED_BYTES: u64 = 1 << 40; // One TiB; checked before extraction.
const DOWNLOAD_SCHEMA: u32 = 1;

/// Portable relative names remain stable when a corpus is moved to another disk.
/// `files` is authoritative: a later failed stage may have other files under `root`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MaterializedSource {
    pub root: PathBuf,
    pub files: BTreeMap<String, PathBuf>,
    /// Present only when the original ZIP represents this complete input set.
    pub archive: Option<PathBuf>,
}

/// Resolve a directory, a single workbook, a local ZIP, or an HTTP(S) ZIP URL.
/// ZIP extraction creates a unique child of `work_root`; the caller owns its
/// lifetime. Completed downloads remain in `cache_root` across runs.
pub(crate) fn materialize(
    source: &str,
    work_root: &Path,
    cache_root: &Path,
    refresh_download: bool,
) -> Result<MaterializedSource> {
    ensure!(!source.trim().is_empty(), "An input source is required");
    if source.starts_with("http://") || source.starts_with("https://") {
        let archive = cached_download(source, cache_root, refresh_download)?;
        return extract_source(&archive, work_root);
    }
    ensure!(!source.contains("://"), "Only HTTP(S) archive URLs are supported");
    let path = Path::new(source);
    require_regular_source(path)?;
    let path = fs::canonicalize(path).with_context(|| format!("Resolve input {}", path.display()))?;
    if path.is_dir() {
        return discover(&path);
    }
    if is_workbook(&path) {
        let root = path.parent().context("Workbook has no parent directory")?.to_path_buf();
        let name = relative_name(path.file_name().context("Workbook has no filename")?.to_str()
            .context("Workbook filename is not valid UTF-8")?)?;
        return Ok(MaterializedSource { root, files: BTreeMap::from([(name, path)]), archive: None });
    }
    ensure!(path.extension().and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip")),
        "Input archives currently require ZIP; received {}", path.display());
    extract_source(&path, work_root)
}

/// Enumerate supported workbook formats without following symlinks or junctions.
/// Excel lock files (`~$...`) are not workbooks in the evaluation corpus.
pub(crate) fn discover(root: &Path) -> Result<MaterializedSource> {
    require_regular_source(root)?;
    let root = fs::canonicalize(root)?;
    ensure!(root.is_dir(), "Expected a source directory: {}", root.display());
    let mut files = BTreeMap::new();
    let mut names = PortableNames::default();
    for entry in WalkDir::new(&root).follow_links(false).min_depth(1) {
        let entry = entry.with_context(|| format!("Enumerate {}", root.display()))?;
        require_regular_source(entry.path())?;
        let relative = portable_relative(entry.path().strip_prefix(&root)?)?;
        names.insert(&relative, entry.file_type().is_dir())?;
        if entry.file_type().is_file() && is_workbook(entry.path()) {
            files.insert(relative, entry.into_path());
        }
    }
    Ok(MaterializedSource { root, files, archive: None })
}

fn extract_source(archive: &Path, work_root: &Path) -> Result<MaterializedSource> {
    fs::create_dir_all(work_root)?;
    let destination = work_root.join(format!("source-{}", uuid::Uuid::new_v4()));
    let mut source = extract_archive(archive, &destination)?;
    source.archive = Some(fs::canonicalize(archive)?);
    Ok(source)
}

/// Extract only workbook members, after validating every ZIP member name.
/// The destination must not exist. Partial extraction is removed on failure;
/// existing directories and user files are never used as cleanup targets.
pub(crate) fn extract_archive(archive_path: &Path, destination: &Path) -> Result<MaterializedSource> {
    let mut archive = ZipArchive::new(File::open(archive_path)?)
        .with_context(|| format!("Open ZIP {}", archive_path.display()))?;
    let entries = inspect_archive(&mut archive)?;
    fs::create_dir(destination)
        .with_context(|| format!("Create new extraction directory {}", destination.display()))?;
    let result = extract_workbooks(&mut archive, entries, destination);
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn inspect_archive(archive: &mut ZipArchive<File>) -> Result<Vec<(usize, String)>> {
    ensure!(archive.len() <= MAX_ARCHIVE_MEMBERS,
        "ZIP contains more than {MAX_ARCHIVE_MEMBERS} members");
    let mut names = PortableNames::default();
    let mut declared = BTreeSet::new();
    let mut expanded_bytes = 0u64;
    let mut workbooks = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let directory = entry.is_dir();
        let member = if directory { entry.name().strip_suffix('/').unwrap_or(entry.name()) } else { entry.name() };
        let name = relative_name(member)?;
        ensure!(declared.insert(name.to_lowercase()), "Duplicate ZIP member: {name}");
        names.insert(&name, directory)?;
        ensure!(regular_zip_mode(entry.unix_mode()), "ZIP links and special files are unsupported: {name}");
        expanded_bytes = expanded_bytes.checked_add(entry.size()).context("ZIP size overflow")?;
        ensure!(expanded_bytes <= MAX_EXPANDED_BYTES, "ZIP expands beyond one TiB");
        if !directory && is_workbook(Path::new(&name)) {
            workbooks.push((index, name));
        }
    }
    Ok(workbooks)
}

fn regular_zip_mode(mode: Option<u32>) -> bool {
    matches!(mode.map(|mode| mode & 0o170000), None | Some(0 | 0o100000 | 0o040000))
}

fn extract_workbooks(
    archive: &mut ZipArchive<File>,
    entries: Vec<(usize, String)>,
    destination: &Path,
) -> Result<MaterializedSource> {
    let root = fs::canonicalize(destination)?;
    let mut files = BTreeMap::new();
    for (index, name) in entries {
        let target = root.join(&name);
        fs::create_dir_all(target.parent().context("ZIP member has no parent")?)?;
        let file = OpenOptions::new().write(true).create_new(true).open(&target)?;
        let mut output = BufWriter::with_capacity(128 * 1024, file);
        let mut entry = archive.by_index(index)?;
        let declared_bytes = entry.size();
        let copied = io::copy(&mut entry.by_ref().take(declared_bytes + 1), &mut output)
            .with_context(|| format!("Extract ZIP member {name}"))?;
        ensure!(copied == declared_bytes, "ZIP member size differs from its declaration: {name}");
        output.flush()?;
        files.insert(name, target);
    }
    Ok(MaterializedSource { root, files, archive: None })
}

/// Write exactly the accepted workbook set to a new ZIP, preserving relative names.
/// XLSX/XLSM/XLSB inputs are already compressed; Store avoids recompressing them.
/// Publication occurs only after ZIP finalisation and a durable file flush.
pub(crate) fn archive_files(files: &BTreeMap<String, PathBuf>, target: &Path) -> Result<()> {
    ensure!(!target.exists(), "Output archive already exists: {}", target.display());
    let parent = target.parent().context("Archive output has no parent directory")?;
    fs::create_dir_all(parent)?;
    let pending = parent.join(format!(".archive-{}.partial", uuid::Uuid::new_v4()));
    let result = write_archive(files, &pending).and_then(|()| {
        ensure!(!target.exists(), "Output archive appeared during packaging: {}", target.display());
        fs::rename(&pending, target).context("Publish output ZIP")
    });
    if result.is_err() {
        let _ = fs::remove_file(&pending);
    }
    result
}

fn write_archive(files: &BTreeMap<String, PathBuf>, pending: &Path) -> Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(pending)?;
    let mut archive = ZipWriter::new(BufWriter::with_capacity(128 * 1024, file));
    let mut names = PortableNames::default();
    for (name, source) in files {
        let name = relative_name(name)?;
        names.insert(&name, false)?;
        require_regular_file(source)?;
        let mut input = File::open(source)?;
        let options = FileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(input.metadata()?.len() >= u64::from(u32::MAX));
        archive.start_file(name, options)?;
        io::copy(&mut input, &mut archive)?;
    }
    let mut output = archive.finish()?;
    output.flush()?;
    output.get_ref().sync_all()?;
    Ok(())
}

/// Validate a name once for both Windows and Unix; never normalise away ambiguity.
pub(crate) fn relative_name(value: &str) -> Result<String> {
    ensure!(!value.is_empty(), "Empty relative workbook name");
    for component in value.split('/') {
        ensure!(!component.is_empty() && component != "." && component != "..",
            "Unsafe relative workbook path: {value}");
        ensure!(!component.ends_with(['.', ' '])
            && !component.chars().any(|c| c.is_control() || "\\<>:\"|?*".contains(c)),
            "Non-portable workbook path: {value}");
        let stem = component.split('.').next().unwrap_or("").to_ascii_uppercase();
        let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4 && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        ensure!(!device, "Reserved Windows filename: {value}");
    }
    Ok(value.to_owned())
}

fn portable_relative(path: &Path) -> Result<String> {
    let value = path.components().map(|part| part.as_os_str().to_str()
        .context("Workbook path is not valid UTF-8")).collect::<Result<Vec<_>>>()?.join("/");
    relative_name(&value)
}

/// Detect case aliases and file/directory shadowing before any output is written.
#[derive(Default)]
pub(crate) struct PortableNames {
    entries: BTreeMap<String, (String, bool)>,
}

impl PortableNames {
    pub(crate) fn insert(&mut self, name: &str, directory: bool) -> Result<()> {
        relative_name(name)?;
        let parts: Vec<_> = name.split('/').collect();
        for end in 1..=parts.len() {
            let prefix = parts[..end].join("/");
            let is_directory = end < parts.len() || directory;
            let folded = prefix.to_lowercase();
            if let Some((existing, existing_directory)) = self.entries.get(&folded) {
                ensure!(existing == &prefix && *existing_directory && is_directory,
                    "Case-insensitive path collision: {existing} and {prefix}");
            } else {
                self.entries.insert(folded, (prefix, is_directory));
            }
        }
        Ok(())
    }
}

pub(crate) fn is_workbook(path: &Path) -> bool {
    if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("~$")) {
        return false;
    }
    path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| {
        ["xlsx", "xls", "xlsm", "xlsb"].iter().any(|supported| extension.eq_ignore_ascii_case(supported))
    })
}

pub(crate) fn require_regular_file(path: &Path) -> Result<()> {
    require_regular_source(path)?;
    ensure!(path.is_file(), "Expected a regular file: {}", path.display());
    Ok(())
}

fn require_regular_source(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Inspect input {}", path.display()))?;
    ensure!(!metadata.file_type().is_symlink(), "Symlinks are unsupported: {}", path.display());
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(metadata.file_attributes() & 0x400 == 0,
            "Reparse points are unsupported: {}", path.display());
    }
    ensure!(metadata.is_file() || metadata.is_dir(), "Special files are unsupported: {}", path.display());
    Ok(())
}

/// A streaming digest is also used by processing manifests; never load a corpus
/// workbook into memory merely to establish its content identity.
pub(crate) fn hash_file(path: &Path) -> Result<String> {
    let mut input = File::open(path).with_context(|| format!("Hash {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[derive(Serialize, Deserialize)]
struct DownloadRecord {
    schema: u32,
    file: String,
    sha256: String,
    bytes: u64,
}

fn cached_download(url: &str, cache_root: &Path, refresh: bool) -> Result<PathBuf> {
    ensure!(!url.chars().any(char::is_control), "Control character in archive URL");
    let uri: axum::http::Uri = url.parse().context("Invalid archive URL")?;
    ensure!(matches!(uri.scheme_str(), Some("http" | "https")) && uri.host().is_some(),
        "Expected an HTTP(S) archive URL");
    let url_key = format!("{:x}", Sha256::digest(url.as_bytes()));
    let directory = cache_root.join("downloads").join(url_key);
    fs::create_dir_all(&directory)?;
    let lock = OpenOptions::new().create(true).truncate(false).write(true).open(directory.join(".lock"))?;
    lock.lock().context("Lock archive download cache")?;
    let index = directory.join("current.json");
    if !refresh && index.is_file() {
        return read_cached_archive(&index, &directory);
    }
    let temporary = directory.join(format!("download-{}.partial", uuid::Uuid::new_v4()));
    let result = download_and_publish(url, &temporary, &directory, &index);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_cached_archive(index: &Path, directory: &Path) -> Result<PathBuf> {
    let record: DownloadRecord = serde_json::from_slice(&fs::read(index)?)
        .context("Read download cache; use --refresh-downloads to replace a damaged cache entry")?;
    ensure!(record.schema == DOWNLOAD_SCHEMA
        && record.sha256.len() == 64
        && record.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        && record.file == format!("{}.zip", record.sha256), "Invalid download cache record");
    let path = directory.join(record.file);
    require_regular_file(&path)?;
    ensure!(fs::metadata(&path)?.len() == record.bytes && hash_file(&path)? == record.sha256,
        "Cached archive changed; use --refresh-downloads to download it again");
    ZipArchive::new(File::open(&path)?).context("Cached download is not a readable ZIP")?;
    Ok(path)
}

fn download_and_publish(url: &str, temporary: &Path, directory: &Path, index: &Path) -> Result<PathBuf> {
    download(url, temporary, directory)?;
    ZipArchive::new(File::open(temporary)?).context("Downloaded source is not a readable ZIP")?;
    let digest = hash_file(temporary)?;
    let target = directory.join(format!("{digest}.zip"));
    if target.exists() {
        ensure!(hash_file(&target)? == digest, "Content-addressed download cache is damaged");
        fs::remove_file(temporary)?;
    } else {
        fs::rename(temporary, &target)?;
    }
    let record = DownloadRecord {
        schema: DOWNLOAD_SCHEMA,
        file: target.file_name().unwrap().to_string_lossy().into_owned(),
        sha256: digest,
        bytes: fs::metadata(&target)?.len(),
    };
    crate::regression::write_atomic_artifact(index, &serde_json::to_vec_pretty(&record)?)?;
    Ok(target)
}

fn download(url: &str, output: &Path, directory: &Path) -> Result<()> {
    let configuration = directory.join(format!(".curl-{}.conf", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&configuration)?;
    writeln!(file, "url = {}\noutput = {}", curl_string(url)?,
        curl_string(output.to_str().context("Download cache path is not valid UTF-8")?)?)?;
    drop(file);
    // Private redirect URLs may occur in curl diagnostics. They remain in the
    // local download cache and are not copied into a portable scientific report.
    let log = directory.join("download.log");
    let result = Command::new("curl")
        .args(["--disable", "--config"]).arg(&configuration)
        .args(["--location", "--fail", "--silent", "--show-error", "--proto", "=http,https",
            "--proto-redir", "=http,https", "--connect-timeout", "30", "--speed-limit", "1024",
            "--speed-time", "120", "--max-time", "14400", "--retry", "3"])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(File::create(&log)?)
        .status().context("Start curl, required for HTTP(S) ZIP sources");
    let _ = fs::remove_file(configuration);
    let status = result?;
    if !status.success() {
        bail!("Archive download failed ({status}); see {}", log.display());
    }
    OpenOptions::new().write(true).open(output)?.sync_all().context("Flush completed archive download")
}

fn curl_string(value: &str) -> Result<String> {
    ensure!(!value.chars().any(char::is_control), "Control character in download setting");
    Ok(format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
}
