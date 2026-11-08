//! Resume a corpus study using its immutable executable and saved launch contract.

use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
};

#[derive(Deserialize)]
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

/// Validate the recorded identities before spawning. The study runner acquires
/// the authoritative output lock again, closing the preview-to-spawn race.
pub(crate) fn resume(path: PathBuf) -> Result<(Child, String)> {
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
    Ok((child, receipt.control_room))
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
                    Some("127.0.0.1" | "localhost" | "deltasieve.localhost" | "observatory.deltasieve.localhost")
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
            assert!(value.validate().is_err(), "Unexpected allowed address {address}");
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
