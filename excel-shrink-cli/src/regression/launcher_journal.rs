//! Retire launcher identities after their original child handle confirms exit.
use std::io;
use std::process::{Child, Output};

pub(super) fn mark_exited(child: &mut Child) -> io::Result<()> {
    #[cfg(windows)]
    if let Some(registry) = std::env::var_os("EXCEL_SHRINK_ARCHIVE_OWNERS") {
        return mark_in_registry(child, std::path::Path::new(&registry));
    }
    let _ = child;
    Ok(())
}

pub(super) fn wait_with_output(child: Child) -> io::Result<Output> {
    #[cfg(windows)]
    {
        let registry = std::env::var_os("EXCEL_SHRINK_ARCHIVE_OWNERS");
        return wait_with_registry(child, registry.as_deref().map(std::path::Path::new));
    }
    #[cfg(not(windows))]
    child.wait_with_output()
}

#[cfg(windows)]
fn mark_in_registry(child: &mut Child, registry: &std::path::Path) -> io::Result<()> {
    if child.try_wait()?.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "launcher has not exited",
        ));
    }
    Completion::capture(child, registry)?.publish()
}

#[cfg(windows)]
fn wait_with_registry(child: Child, registry: Option<&std::path::Path>) -> io::Result<Output> {
    // wait_with_output consumes the Child. Preserve its exact native identity
    // before it drops that handle, then publish only after the wait succeeds.
    let completion = registry
        .map(|path| Completion::capture(&child, path))
        .transpose()?;
    let output = child.wait_with_output()?;
    if let Some(completion) = completion {
        completion.publish()?;
    }
    Ok(output)
}

#[cfg(windows)]
struct Completion {
    path: std::path::PathBuf,
    pid: u32,
    started: String,
}

#[cfg(windows)]
impl Completion {
    fn capture(child: &Child, registry: &std::path::Path) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        let (mut created, mut exited, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
        // Child retains the handle returned for this process. Looking the PID
        // up again here could inspect an unrelated replacement process.
        if unsafe {
            GetProcessTimes(
                child.as_raw_handle(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // The existing PowerShell journal stores DateTime UTC ticks, not FILETIME.
        let ticks = created
            .checked_add(504_911_232_000_000_000)
            .filter(|_| created > 0)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid launcher creation time")
            })?;
        let mut path = registry.as_os_str().to_os_string();
        path.push(".exited");
        Ok(Self {
            path: path.into(),
            pid: child.id(),
            started: ticks.to_string(),
        })
    }

    fn publish(&self) -> io::Result<()> {
        use std::io::Write;
        let mut line =
            serde_json::to_vec(&serde_json::json!({"pid":self.pid,"started":self.started}))?;
        line.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?
            .write_all(&line)
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetProcessTimes(
        process: *mut std::ffi::c_void,
        created: *mut u64,
        exited: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;
    use std::process::{Command, Stdio};

    fn child(hang: bool) -> io::Result<Child> {
        Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "regression::launcher_journal::tests::fixture_child",
                "--nocapture",
            ])
            .env(
                "EXCEL_SHRINK_LAUNCHER_FIXTURE",
                if hang { "hang" } else { "exit" },
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }

    #[test]
    fn fixture_child() {
        match std::env::var("EXCEL_SHRINK_LAUNCHER_FIXTURE").as_deref() {
            Ok("hang") => std::thread::sleep(std::time::Duration::from_secs(30)),
            Ok("exit") => {
                println!("launcher fixture output");
                eprintln!("launcher fixture error");
            }
            _ => {}
        }
    }

    #[test]
    fn running_child_cannot_be_retired() -> anyhow::Result<()> {
        let root = super::super::artifact_cache::unique_path(
            &std::env::temp_dir(),
            "launcher-retirement",
        )?;
        fs::create_dir(&root)?;
        let registry = root.join("owners.jsonl");
        let mut child = child(true)?;
        let result = mark_in_registry(&mut child, &registry);
        child.kill()?;
        child.wait()?;
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert!(!root.join("owners.jsonl.exited").exists());
        mark_in_registry(&mut child, &registry)?;
        let recorded: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("owners.jsonl.exited"))?)?;
        assert_eq!(recorded["pid"], child.id());
        assert!(
            recorded["started"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
                > 504_911_232_000_000_000
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn output_wait_retires_the_exact_child_and_preserves_output() -> anyhow::Result<()> {
        let root =
            super::super::artifact_cache::unique_path(&std::env::temp_dir(), "launcher-output")?;
        fs::create_dir(&root)?;
        let registry = root.join("owners.jsonl");
        let child = child(false)?;
        let identity = Completion::capture(&child, &registry)?;
        let result = wait_with_registry(child, Some(&registry))?;
        assert!(result.status.success());
        assert!(String::from_utf8_lossy(&result.stdout).contains("launcher fixture output"));
        assert!(String::from_utf8_lossy(&result.stderr).contains("launcher fixture error"));
        let recorded: serde_json::Value = serde_json::from_slice(&fs::read(&identity.path)?)?;
        assert_eq!(recorded["pid"], identity.pid);
        assert_eq!(recorded["started"], identity.started);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn native_birth_time_matches_the_existing_powershell_journal() -> anyhow::Result<()> {
        let root =
            super::super::artifact_cache::unique_path(&std::env::temp_dir(), "launcher-birth")?;
        fs::create_dir(&root)?;
        let registry = root.join("owners.jsonl");
        let child = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "[pscustomobject]@{pid=$PID;started=[string]([Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime().Ticks)} | ConvertTo-Json -Compress"])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let identity = Completion::capture(&child, &registry)?;
        let output = wait_with_registry(child, Some(&registry))?;
        assert!(output.status.success());
        let powershell: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let receipt: serde_json::Value = serde_json::from_slice(&fs::read(&identity.path)?)?;
        assert_eq!(receipt, powershell);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
