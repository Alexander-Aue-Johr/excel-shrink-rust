//! Resume a corpus study using its immutable executable and saved launch contract.

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
};

mod configuration;
pub(crate) use configuration::{create, Configuration};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct Receipt {
    executable: PathBuf,
    binary_sha256: String,
    output_root: PathBuf,
    request_file: PathBuf,
    arguments: Vec<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    control_room: String,
}

/// Use the same creation and resume contracts as the web configurator. The
/// scientific runner remains independent and writes its own durable evidence.
pub(crate) fn run(config: Option<PathBuf>, receipt: Option<PathBuf>, open: bool) -> Result<()> {
    let receipt = match (config, receipt) {
        (Some(path), None) => create(Configuration::from_file(&path)?)?,
        (None, Some(path)) => path,
        _ => anyhow::bail!("Choose either a new study configuration or a launch receipt"),
    };
    let (mut child, url, output) = resume(receipt.clone())?;
    eprintln!("Study launch receipt: {}", receipt.display());
    eprintln!("Study Observatory: {url}");
    if open {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(&url, &output) && std::time::Instant::now() < deadline {
            if child.try_wait()?.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        if ready(&url, &output) {
            crate::validation::open_browser(std::ffi::OsStr::new(&url))?;
        }
    }
    let status = child.wait()?;
    ensure!(
        status.success(),
        "Study runner exited with {status}; inspect {}/web-launches",
        output.display()
    );
    Ok(())
}

/// Validate the recorded identities before spawning. The study runner acquires
/// the authoritative output lock again, closing the preview-to-spawn race.
pub(crate) fn resume(path: PathBuf) -> Result<(Child, String, PathBuf)> {
    let receipt: Receipt =
        serde_json::from_slice(&fs::read(&path)?).context("Read study launch receipt")?;
    receipt.validate()?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .open(receipt.output_root.join(".run.lock"))
        .context("Open the existing study lock")?;
    lock.try_lock()
        .context("This study is already running or its output cannot be locked")?;
    let hash = format!("{:x}", Sha256::digest(fs::read(&receipt.executable)?));
    ensure!(
        hash.eq_ignore_ascii_case(&receipt.binary_sha256),
        "The recorded study executable has changed"
    );
    let launch = receipt
        .output_root
        .join("web-launches")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&launch)?;
    let mut command = Command::new(&receipt.executable);
    command
        .args(&receipt.arguments)
        .envs(&receipt.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::from(fs::File::create(launch.join("stdout.log"))?))
        .stderr(Stdio::from(fs::File::create(launch.join("stderr.log"))?));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    // Release our probe lock. The child must independently acquire this lock.
    drop(lock);
    let child = command.spawn().context("Start the recorded study runner")?;
    Ok((child, receipt.control_room, receipt.output_root))
}

/// Probe only the already validated local study address, requiring its exact
/// output identity before a browser can leave the configuration page.
pub(crate) fn ready(url: &str, output: &std::path::Path) -> bool {
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, SocketAddr, TcpStream};
    use std::time::Duration;
    let probe = || -> Result<bool> {
        let uri: axum::http::Uri = url.parse()?;
        let port = uri.port_u16().context("Missing study port")?;
        let mut stream = TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            Duration::from_millis(300),
        )?;
        stream.set_read_timeout(Some(Duration::from_millis(500)))?;
        stream.set_write_timeout(Some(Duration::from_millis(500)))?;
        write!(
            stream,
            "GET /health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )?;
        let mut bytes = Vec::new();
        stream.take(16 * 1024).read_to_end(&mut bytes)?;
        ensure!(
            bytes.starts_with(b"HTTP/1.0 200 ") || bytes.starts_with(b"HTTP/1.1 200 "),
            "Study health endpoint is not ready"
        );
        let body = bytes
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .context("Incomplete study health response")?
            + 4;
        let health: serde_json::Value = serde_json::from_slice(&bytes[body..])?;
        Ok(health["kind"] == "excel-shrink-control-room"
            && health["root"].as_str().map(PathBuf::from) == Some(output.join("control-room")))
    };
    probe().unwrap_or(false)
}

impl Receipt {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.executable.is_absolute()
                && self.output_root.is_absolute()
                && self.request_file.is_absolute(),
            "Study receipt paths must be absolute"
        );
        ensure!(
            self.output_root.is_dir() && self.request_file.is_file(),
            "Study files are missing"
        );
        ensure!(
            self.arguments.first().map(String::as_str) == Some("regression")
                && self.arguments.get(1).map(String::as_str) == Some("study"),
            "Receipt must invoke regression study"
        );
        self.require_argument("--output-root", &self.output_root)?;
        self.require_argument("--request-file", &self.request_file)?;
        let uri: axum::http::Uri = self.control_room.parse()?;
        ensure!(
            uri.scheme_str() == Some("http")
                && matches!(
                    uri.host(),
                    Some(
                        "127.0.0.1"
                            | "localhost"
                            | "deltasieve.localhost"
                            | "observatory.deltasieve.localhost"
                    )
                )
                && uri.port_u16().is_some(),
            "Study Observatory must use a local HTTP address"
        );
        Ok(())
    }

    fn require_argument(&self, flag: &str, expected: &std::path::Path) -> Result<()> {
        let matches: Vec<_> = self
            .arguments
            .iter()
            .enumerate()
            .filter(|(_, argument)| argument.as_str() == flag)
            .collect();
        ensure!(matches.len() == 1, "Receipt requires exactly one {flag}");
        let value = self
            .arguments
            .get(matches[0].0 + 1)
            .context("Missing study argument value")?;
        ensure!(
            std::path::Path::new(value) == expected,
            "Receipt disagrees with {flag}"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt() -> Receipt {
        Receipt {
            executable: PathBuf::from("runner.exe"),
            binary_sha256: String::new(),
            output_root: PathBuf::from("output"),
            request_file: PathBuf::from("request.json"),
            arguments: vec!["--output-root".into(), "output".into()],
            environment: BTreeMap::new(),
            control_room: "http://127.0.0.1:18777/".into(),
        }
    }

    #[test]
    fn rejects_missing_duplicate_and_mismatched_output_arguments() {
        let mut value = receipt();
        assert!(value
            .require_argument("--output-root", &value.output_root)
            .is_ok());
        value.arguments.push("--output-root".into());
        assert!(value
            .require_argument("--output-root", &value.output_root)
            .is_err());
        value.arguments = vec!["--output-root".into()];
        assert!(value
            .require_argument("--output-root", &value.output_root)
            .is_err());
        value.arguments.push("other-output".into());
        assert!(value
            .require_argument("--output-root", &value.output_root)
            .is_err());
        value.arguments.clear();
        assert!(value
            .require_argument("--output-root", &value.output_root)
            .is_err());
    }

    #[test]
    fn rejects_relative_receipt_paths() {
        assert!(receipt()
            .validate()
            .unwrap_err()
            .to_string()
            .contains("absolute"));
    }

    #[test]
    fn readiness_requires_success_and_the_exact_study_identity() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let output = std::env::temp_dir().join("expected-study");
        for (status, matching_root, expected) in
            [(200, true, true), (200, false, false), (503, true, false)]
        {
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let health = serde_json::json!({
                "kind":"excel-shrink-control-room",
                "root":if matching_root { output.join("control-room") } else { output.join("foreign") },
            });
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 1024];
                stream.read(&mut request).unwrap();
                let body = serde_json::to_vec(&health).unwrap();
                write!(
                    stream,
                    "HTTP/1.0 {status} Test\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            });
            assert_eq!(
                ready(
                    &format!("http://observatory.deltasieve.localhost:{port}/"),
                    &output
                ),
                expected
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn named_study_address_remains_local_and_explicitly_ported() {
        let root = std::env::temp_dir().join(format!("study-address-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let request = root.join("request.json");
        fs::write(&request, b"{}").unwrap();
        let mut value = receipt();
        value.executable = std::env::current_exe().unwrap();
        value.output_root = root.clone();
        value.request_file = request.clone();
        value.arguments = vec![
            "regression".into(),
            "study".into(),
            "--output-root".into(),
            root.to_string_lossy().into_owned(),
            "--request-file".into(),
            request.to_string_lossy().into_owned(),
        ];
        for address in [
            "http://127.0.0.1:18777/",
            "http://localhost:18777/",
            "http://deltasieve.localhost:18777/",
            "http://observatory.deltasieve.localhost:18777/",
        ] {
            value.control_room = address.into();
            value.validate().unwrap();
        }
        for address in [
            "http://observatory.deltasieve.localhost.example.com:18777/",
            "http://unlisted.localhost:18777/",
            "http://observatory.deltasieve.localhost/",
            "https://observatory.deltasieve.localhost:18777/",
        ] {
            value.control_room = address.into();
            assert!(
                value.validate().is_err(),
                "Unexpected allowed address {address}"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_a_busy_study_before_starting_any_executable() {
        let root = std::env::temp_dir().join(format!("study-resume-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let request = root.join("request.json");
        fs::write(&request, b"{}").unwrap();
        let lock = fs::File::create(root.join(".run.lock")).unwrap();
        lock.try_lock().unwrap();
        let receipt_path = root.join("launch.json");
        let executable = root.join("missing-runner.exe");
        let value = serde_json::json!({
            "Executable": executable, "BinarySha256": "invalid", "OutputRoot": root,
            "RequestFile":request,"Arguments":["regression","study","--output-root",root,
                "--request-file",request],"ControlRoom":"http://127.0.0.1:18777/"
        });
        fs::write(&receipt_path, serde_json::to_vec(&value).unwrap()).unwrap();
        let error = resume(receipt_path).unwrap_err();
        assert!(error.to_string().contains("already running"), "{error:#}");
        assert!(!root.join("web-launches").exists());
        drop(lock);
        fs::write(&executable, b"changed runner").unwrap();
        let error = resume(root.join("launch.json")).unwrap_err();
        assert!(
            error.to_string().contains("executable has changed"),
            "{error:#}"
        );
        assert!(!root.join("web-launches").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
