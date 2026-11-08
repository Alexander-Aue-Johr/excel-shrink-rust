//! Coordinator ownership of a gated pool process and its native descendants.
use super::*;
use serde_json::json;
use std::process::Command;

pub(crate) struct PoolOwner {
    child: Child,
    job: Option<crate::process_job::Job>,
    pub connection: PathBuf,
    pub groups: usize,
}

impl PoolOwner {
    /// Start two exclusive workers per admitted comparison group.
    pub fn start(output: &Path, groups: usize, timeout_seconds: u64) -> Result<Self> {
        ensure!(
            (1..=32).contains(&groups),
            "Warm pool supports 1 through 32 groups"
        );
        ensure!(
            (1..=600).contains(&timeout_seconds),
            "Invalid warm pool timeout"
        );
        let root = output
            .join(".warm-workers")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root)?;
        let assets = crate::validation::capture::capture_directory()?;
        validate_cleanup_assets(&assets)?;
        let job = crate::process_job::Job::new()?;
        let excel = resolve_excel(&assets, &root, &job)?;
        let seed = root.join("seed.xlsx");
        crate::regression::legacy_conversion::write_seed(&seed)?;
        let ready = root.join("ready.json");
        let settings = root.join("settings.json");
        let token = uuid::Uuid::new_v4().to_string();
        fs::write(
            &settings,
            serde_json::to_vec_pretty(&json!({
                "worker":{"seed":seed,"excel_executable":excel,"capture_root":assets,
                    "profile_root":root.join("workers"),"timeout_seconds":timeout_seconds},
                "workers":groups*2,"token":token,"port":0,"ready_path":ready,"owned_startup":true
            }))?,
        )?;
        let mut command = Command::new(std::env::current_exe()?);
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
        let mut child = command
            .args(["capture-host", "workbook-pool"])
            .arg(&settings)
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(fs::File::create(root.join("stdout.log"))?)
            .stderr(fs::File::create(root.join("stderr.log"))?)
            .spawn()?;
        if let Err(error) = job.add(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let mut owner = Self {
            child,
            job: Some(job),
            connection: root.join("connection.json"),
            groups,
        };
        owner
            .child
            .stdin
            .take()
            .context("Pool startup stdin missing")?
            .write_all(b"start\n")?;
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds + 30);
        loop {
            ensure!(
                owner.child.try_wait()?.is_none(),
                "Warm pool exited during startup; inspect {}",
                root.display()
            );
            if ready.is_file() {
                let snapshot: Value = serde_json::from_slice(&fs::read(&ready)?)?;
                ensure!(
                    snapshot["owner_pid"] == owner.child.id(),
                    "Warm pool startup identity differs"
                );
                let birth = snapshot["owner_started_ms"]
                    .as_i64()
                    .context("Missing warm pool birth time")?;
                ensure!(
                    crate::native_process::process_alive(owner.child.id() as i64, birth)
                        == Some(true),
                    "Warm pool native identity is unconfirmed"
                );
                fs::write(
                    &owner.connection,
                    serde_json::to_vec(&json!({"url":snapshot["url"],"token":token}))?,
                )?;
                return Ok(owner);
            }
            ensure!(
                Instant::now() < deadline,
                "Warm pool startup timed out; inspect {}",
                root.display()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn check_alive(&mut self) -> Result<()> {
        ensure!(
            self.child.try_wait()?.is_none(),
            "Owned warm workbook pool exited"
        );
        Ok(())
    }
}

/// Reject incomplete deployments before starting Excel or admitting comparisons.
fn validate_cleanup_assets(assets: &Path) -> Result<()> {
    for name in ["archive_processes.ps1", "ArchiveProcesses.cs"] {
        let path = assets.join("../src/regression").join(name);
        ensure!(path.is_file(), "Incomplete capture runtime: missing {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod asset_tests {
    use super::*;

    #[test]
    fn warm_pool_rejects_missing_cleanup_dependencies() -> Result<()> {
        let root = std::env::temp_dir().join(format!("pool-assets-{}", uuid::Uuid::new_v4()));
        let assets = root.join("office-capture");
        fs::create_dir_all(&assets)?;
        let result = (|| -> Result<()> {
            assert!(validate_cleanup_assets(&assets).unwrap_err().to_string().contains("archive_processes.ps1"));
            let helpers = root.join("src/regression");
            fs::create_dir_all(&helpers)?;
            fs::write(helpers.join("archive_processes.ps1"), "fixture")?;
            assert!(validate_cleanup_assets(&assets).unwrap_err().to_string().contains("ArchiveProcesses.cs"));
            fs::write(helpers.join("ArchiveProcesses.cs"), "fixture")?;
            validate_cleanup_assets(&assets)
        })();
        fs::remove_dir_all(&root)?;
        result
    }
}

impl Drop for PoolOwner {
    fn drop(&mut self) {
        drop(self.job.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                _ => {
                    tracing::error!("Warm workbook pool process exit is unconfirmed");
                    break;
                }
            }
        }
    }
}

fn resolve_excel(assets: &Path, root: &Path, job: &crate::process_job::Job) -> Result<PathBuf> {
    let script = root.join("resolve-excel.ps1");
    fs::write(
        &script,
        br#"$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Import-Module (Join-Path $env:EXCEL_SHRINK_CAPTURE_DIR 'CaptureSetup.psm1') -DisableNameChecking
[Console]::WriteLine((Find-ExcelExecutable))
"#,
    )?;
    let stdout = root.join("excel-path.txt");
    let mut command = super::super::excel_automation::power_shell_script_command(&script, &[])?;
    let mut child = command
        .env(
            "EXCEL_SHRINK_CAPTURE_DIR",
            super::super::power_shell_path(assets)?,
        )
        .stdout(fs::File::create(&stdout)?)
        .stderr(fs::File::create(root.join("resolve.stderr.log"))?)
        .spawn()?;
    if let Err(error) = job.add(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(
                status.success(),
                "Cannot resolve installed Excel; inspect {}",
                root.display()
            );
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Installed Excel resolution timed out"
        );
        thread::sleep(Duration::from_millis(50));
    }
    let path = PathBuf::from(fs::read_to_string(stdout)?.trim());
    ensure!(
        path.is_absolute() && path.is_file(),
        "Installed Excel executable is unavailable"
    );
    Ok(path)
}
