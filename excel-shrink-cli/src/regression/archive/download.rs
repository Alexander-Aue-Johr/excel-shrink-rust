use super::*;
use std::io::Write;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub urls: Vec<String>,
    /// Visit a private sharing link once to obtain download cookies.
    pub access_url: Option<String>,
}

fn quote(value: &str) -> Result<String> {
    ensure!(
        !value.chars().any(char::is_control),
        "control character in download setting"
    );
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn curl(root: &Path, url: &str, target: &Path, resume: bool) -> Result<()> {
    ensure!(
        url.starts_with("https://") || url.starts_with("http://"),
        "expected an HTTP(S) URL"
    );
    fs::create_dir_all(root)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(".download.lock"))?;
    lock.lock()
        .context("lock download settings and cookie jar")?;
    let config = root.join("download.curl");
    let cookies = root.join("cookies.txt");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&config)?;
    writeln!(
        file,
        "url = {}\noutput = {}\ncookie = {}\ncookie-jar = {}",
        quote(url)?,
        quote(&target.to_string_lossy())?,
        quote(&cookies.to_string_lossy())?,
        quote(&cookies.to_string_lossy())?
    )?;
    drop(file);
    let mut command = Command::new("curl");
    command.args(["--config"]).arg(&config).args([
        "--location",
        "--fail",
        "--silent",
        "--show-error",
        "--proto",
        "=http,https",
        "--proto-redir",
        "=http,https",
        "--retry",
        "4",
        "--retry-delay",
        "2",
        "--connect-timeout",
        "30",
        "--speed-limit",
        "1024",
        "--speed-time",
        "120",
        "--max-time",
        "14400",
    ]);
    if resume {
        command.args(["--continue-at", "-"]);
    }
    let log_path = root.join("download.log");
    command
        .stdout(Stdio::null())
        .stderr(Stdio::from(fs::File::create(&log_path)?));
    let mut child = command
        .spawn()
        .context("start curl (required for archive downloads)")?;
    let mut progress = Instant::now();
    let result = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if progress.elapsed() >= Duration::from_secs(10) {
            let bytes = fs::metadata(target).map(|m| m.len()).unwrap_or(0);
            eprintln!(
                "[archive] Download: {:.1} MiB received",
                bytes as f64 / 1_048_576.0
            );
            progress = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let _ = fs::remove_file(config);
    // curl diagnostics can contain a private redirect URL. Keep them local.
    ensure!(
        result.success(),
        "download failed ({}); see {}",
        result,
        log_path.display()
    );
    Ok(())
}

pub(super) fn authorize(root: &Path, url: &str) -> Result<()> {
    let page = root.join("access.html");
    let result = curl(root, url, &page, false);
    let _ = fs::remove_file(page);
    result
}

pub(super) fn archive(root: &Path, url: &str) -> Result<PathBuf> {
    let key = artifact_cache::sha256(url.as_bytes());
    let target = root.join(format!("{}.zip", &key[..20]));
    if target.is_file() {
        ZipArchive::new(fs::File::open(&target)?).context("cached ZIP is invalid")?;
        return Ok(target);
    }
    let partial = target.with_extension("zip.partial");
    if partial.is_file() {
        let mut magic = [0; 4];
        let count = fs::File::open(&partial)?.read(&mut magic)?;
        if count > 0 && magic != *b"PK\x03\x04" && magic != *b"PK\x05\x06" {
            fs::remove_file(&partial).context("discard non-ZIP partial download")?;
        }
    }
    curl(root, url, &partial, true)?;
    ZipArchive::new(fs::File::open(&partial)?)
        .context("download is not a ZIP archive (check access permissions)")?;
    fs::rename(partial, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_urls_remain_a_single_curl_setting() {
        assert_eq!(
            quote("https://example.test/a?x=\"b\"\\c").unwrap(),
            "\"https://example.test/a?x=\\\"b\\\"\\\\c\""
        );
        assert!(quote("x\nurl = secret").is_err());
    }
}
