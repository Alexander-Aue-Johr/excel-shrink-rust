//! Own helper handles and verify Excel identity before forced cleanup.
use super::*;
use std::process::{Child, Stdio};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER},
    System::Threading::{
        OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    },
};

pub(super) struct Worker {
    pub directory: PathBuf,
    child: Child,
    started: Instant,
    retiring: Option<Instant>,
    stopped: bool,
    readiness_observed: bool,
    excel: Option<(windows_sys::Win32::Foundation::HANDLE, i64)>,
}

impl Worker {
    /// Start asynchronously; readiness is published only after Excel is configured.
    pub fn start(
        root: &Path,
        script: &Path,
        owner_started_ms: u64,
        job: &job::Job,
    ) -> Result<Self> {
        let directory = root.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&directory)?;
        let mut command = crate::shell::power_shell_script_command(
            script,
            &[
                ("Session", crate::shell::power_shell_path(&directory)?),
                ("OwnerPid", std::process::id().to_string()),
                ("OwnerStartedMs", owner_started_ms.to_string()),
            ],
        )?;
        let temporary = root.join("temp");
        fs::create_dir_all(&temporary)?;
        // .NET Framework's compiler rejects extended-length paths in TEMP.
        let temporary = crate::shell::power_shell_path(&temporary)?;
        command.env("TEMP", &temporary).env("TMP", &temporary);
        command.stdout(Stdio::from(File::create(directory.join("stdout.log"))?));
        command.stderr(Stdio::from(File::create(directory.join("stderr.log"))?));
        let mut child = command
            .spawn()
            .context("start persistent Excel converter")?;
        if let Err(error) = job.add(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(Self {
            directory,
            child,
            started: Instant::now(),
            retiring: None,
            stopped: false,
            readiness_observed: false,
            excel: None,
        })
    }

    pub fn ready(&self) -> bool {
        self.directory.join("ready.json").is_file()
            && !self.directory.join("startup-error.json").exists()
    }

    pub fn became_ready(&mut self) -> bool {
        if self.readiness_observed || !self.ready() {
            return false;
        }
        self.readiness_observed = true;
        true
    }

    pub fn retire(&mut self) -> Result<()> {
        fs::write(self.directory.join("retire"), b"")?;
        if self.retiring.is_none() {
            self.retiring = Some(Instant::now());
        }
        Ok(())
    }

    pub fn is_retiring(&self) -> bool {
        self.retiring.is_some()
    }

    /// Check retirement only while idle; an active workbook keeps its own deadline.
    pub fn should_stop(&mut self) -> Result<bool> {
        self.retain_excel_handle()?;
        if self.child.try_wait()?.is_some()
            || self.directory.join("cancel").exists()
            || self.directory.join("startup-error.json").exists()
        {
            return Ok(true);
        }
        if self.directory.join("active.json").exists()
            || self.directory.join("request.json").exists()
        {
            let lease = open_lease(&self.directory)?;
            if lease.try_lock().is_ok() {
                // A submitted request without its client lease is abandoned.
                return Ok(true);
            }
        }
        if !self.ready() && self.started.elapsed() > Duration::from_secs(120) {
            fs::write(
                self.directory.join("startup-error.json"),
                br#"{"error":"Excel startup exceeded 120 seconds"}"#,
            )?;
            return Ok(true);
        }
        if self.retiring.is_some() {
            let lease = open_lease(&self.directory)?;
            if lease.try_lock().is_err() {
                self.retiring = Some(Instant::now());
            } else if self
                .retiring
                .is_some_and(|time| time.elapsed() > Duration::from_secs(8))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Retain identity-checked handles before Excel exits, including during teardown.
    fn retain_excel_handle(&mut self) -> Result<()> {
        if self.excel.is_some() {
            return Ok(());
        }
        let Some(identity) = read_json(&self.directory.join("identity.json")) else {
            return Ok(());
        };
        let pid = identity["pid"].as_u64().context("Excel PID missing")? as u32;
        let started_ms = identity["started_ms"]
            .as_i64()
            .context("Excel creation time missing")?;
        // SAFETY: OpenProcess returns one owned handle. Creation time is checked below.
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                return Ok(());
            }
            return Err(std::io::Error::last_os_error().into());
        }
        if crate::native_process::running_handle_matches(handle, started_ms).is_none() {
            unsafe { CloseHandle(handle) };
            anyhow::bail!("cannot verify conversion Excel identity");
        }
        self.excel = Some((handle, started_ms));
        Ok(())
    }

    /// Reap the exact helper and recorded Excel handle before releasing clients.
    pub fn stop(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        let mut identity = self.retain_excel_handle();
        let mut termination_error = None;
        if self.child.try_wait()?.is_none() {
            if let Err(error) = self.child.kill() {
                termination_error = Some(error);
            }
        }
        let exit_wait = Instant::now();
        while self
            .child
            .try_wait()
            .context("inspect converter helper exit")?
            .is_none()
        {
            anyhow::ensure!(
                exit_wait.elapsed() < Duration::from_secs(5),
                "converter helper exit unconfirmed: {:?}",
                termination_error
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        self.child.wait().context("reap converter helper")?;
        // An already-exited process can briefly reject OpenProcess during teardown.
        let started = Instant::now();
        while identity.is_err() && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(50));
            identity = self.retain_excel_handle();
        }
        identity.context("retain conversion Excel identity during cleanup")?;
        if let Some((handle, started_ms)) = self.excel {
            stop_excel(handle, started_ms).context("stop conversion Excel handle")?;
        }
        fs::write(self.directory.join("stopped"), b"")?;
        self.stopped = true;
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("Conversion worker cleanup failed: {error:#}");
        }
        if let Some((handle, _)) = self.excel.take() {
            // SAFETY: Worker owns this retained process handle.
            unsafe { CloseHandle(handle) };
        }
    }
}

fn stop_excel(handle: windows_sys::Win32::Foundation::HANDLE, started_ms: i64) -> Result<()> {
    match crate::native_process::running_handle_matches(handle, started_ms) {
        Some(false) => return Ok(()),
        Some(true) => {}
        None => anyhow::bail!("cannot verify conversion Excel identity"),
    }
    // SAFETY: Creation time matched on this retained handle, ruling out PID reuse.
    if unsafe { TerminateProcess(handle, 1) } == 0 {
        let error = std::io::Error::last_os_error();
        // Job closure may already have begun termination. Access denied in
        // that window is resolved by the retained handle's exit signal.
        if unsafe { WaitForSingleObject(handle, 5000) } == 0 {
            return Ok(());
        }
        return Err(error).context("terminate verified conversion Excel");
    }
    anyhow::ensure!(
        unsafe { WaitForSingleObject(handle, 5000) } == 0,
        "conversion Excel exit remains unconfirmed"
    );
    Ok(())
}
