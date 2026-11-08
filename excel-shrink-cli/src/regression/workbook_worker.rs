//! Rust ownership and bounded transport for a warm native workbook worker.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::{
    fs,
    io::{self, BufRead, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const MAX_FRAME_BYTES: usize = 65536;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    seed: PathBuf,
    excel_executable: PathBuf,
    capture_root: PathBuf,
    profile_root: PathBuf,
    timeout_seconds: u64,
}

struct Worker {
    child: Child,
    requests: mpsc::SyncSender<Vec<u8>>,
    written: mpsc::Receiver<Result<()>>,
    replies: mpsc::Receiver<Result<Value>>,
    job: Option<crate::process_job::Job>,
    identity: Option<(i64, i64)>,
}

mod pool;
pub(crate) use pool::run as run_pool;

impl Worker {
    fn start(settings: &Settings) -> Result<Self> {
        ensure!(
            (1..=600).contains(&settings.timeout_seconds),
            "Invalid workbook worker timeout"
        );
        for path in [
            &settings.seed,
            &settings.excel_executable,
            &settings.capture_root,
            &settings.profile_root,
        ] {
            ensure!(path.is_absolute(), "Workbook worker paths must be absolute");
        }
        ensure!(
            settings.seed.is_file() && settings.excel_executable.is_file(),
            "Missing workbook worker input"
        );
        let script = settings.capture_root.join("workbook-worker.ps1");
        ensure!(script.is_file(), "Missing workbook worker bootstrap");
        // A worker's profile is exclusive for its complete lifetime.
        fs::create_dir(&settings.profile_root).context("Reserve new workbook worker profile")?;
        let job = crate::process_job::Job::new()?;
        let mut command = super::excel_automation::power_shell_script_command(
            &script,
            &[
                ("Seed", super::power_shell_path(&settings.seed)?),
                (
                    "ExcelExecutable",
                    super::power_shell_path(&settings.excel_executable)?,
                ),
                ("Startup", "owned".into()),
            ],
        )?;
        let mut child = command
            .env("WEBVIEW2_USER_DATA_FOLDER", &settings.profile_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Start native workbook worker")?;
        if let Err(error) = job.add(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let mut input = BufWriter::new(child.stdin.take().context("Worker stdin unavailable")?);
        let output = child.stdout.take().context("Worker stdout unavailable")?;
        let (requests, pending) = mpsc::sync_channel::<Vec<u8>>(1);
        let (acknowledge, written) = mpsc::sync_channel(1);
        thread::spawn(move || {
            while let Ok(frame) = pending.recv() {
                let result = input
                    .write_all(&frame)
                    .and_then(|_| input.flush())
                    .context("Write workbook worker command");
                let failed = result.is_err();
                if acknowledge.send(result).is_err() || failed {
                    break;
                }
            }
        });
        let (send, replies) = mpsc::sync_channel(4);
        thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                match read_frame(&mut output) {
                    Ok(Some(value)) => {
                        if send.send(Ok(value)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = send.send(Err(error));
                        break;
                    }
                }
            }
        });
        let worker = Self {
            child,
            requests,
            written,
            replies,
            job: Some(job),
            identity: None,
        };
        // Excel may start only after the helper belongs to the Rust owner's job.
        worker.send(
            b"start\n".to_vec(),
            Instant::now() + Duration::from_secs(settings.timeout_seconds),
        )?;
        Ok(worker)
    }

    fn send(&self, frame: Vec<u8>, deadline: Instant) -> Result<()> {
        self.requests
            .try_send(frame)
            .context("Workbook worker command queue unavailable")?;
        self.written
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .context("Workbook worker write did not finish before deadline")?
    }

    fn receive(&self, deadline: Instant) -> Result<Value> {
        self.replies
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .context("Workbook worker reply unavailable before deadline")?
    }

    fn verify_identity(&mut self, value: &Value) -> Result<()> {
        let pid = value["pid"]
            .as_i64()
            .filter(|pid| *pid > 0)
            .context("Missing Excel PID")?;
        let started = chrono::DateTime::parse_from_rfc3339(
            value["started_utc"]
                .as_str()
                .context("Missing Excel birth time")?,
        )?
        .timestamp_millis();
        ensure!(
            crate::native_process::process_alive(pid, started) == Some(true),
            "Excel identity is not live"
        );
        self.identity = Some((pid, started));
        Ok(())
    }

    fn verify_reply(&self, request: &Value, reply: &Value) -> Result<()> {
        ensure!(
            reply["id"] == request["id"],
            "Workbook worker reply identity differs"
        );
        ensure!(
            reply["ok"].is_boolean(),
            "Workbook worker reply has no status"
        );
        if reply["ok"] == true {
            ensure!(
                reply["pid"].as_i64() == self.identity.map(|id| id.0),
                "Workbook worker changed Excel PID"
            );
        }
        Ok(())
    }

    fn initialize(&mut self, timeout: Duration) -> Result<Value> {
        let deadline = Instant::now() + timeout;
        let mut identity = self.receive(deadline)?;
        ensure!(
            identity["event_type"] == "process_started",
            "Missing workbook worker startup identity"
        );
        self.verify_identity(&identity)?;
        let ready = self.receive(deadline)?;
        ensure!(
            ready["event_type"] == "ready" && ready["pid"] == identity["pid"],
            "Workbook worker did not become ready"
        );
        identity["background_error_checking_disabled"] = ready["background_error_checking_disabled"].clone();
        Ok(identity)
    }

    fn command(&mut self, request: &Value, timeout: Duration) -> Result<Value> {
        uuid::Uuid::parse_str(
            request["id"]
                .as_str()
                .context("Missing worker command ID")?,
        )?;
        let deadline = Instant::now() + timeout;
        let mut frame = Vec::new();
        write_frame(&mut frame, request)?;
        self.send(frame, deadline)?;
        let reply = self.receive(deadline)?;
        self.verify_reply(request, &reply)?;
        Ok(reply)
    }

    fn stop(&mut self) -> Result<()> {
        // Closing this job also terminates a helper blocked inside a COM call.
        drop(self.job.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let helper_ended = self.child.try_wait()?.is_some();
            let excel_ended = self.identity.is_none_or(|(pid, started)| {
                crate::native_process::process_alive(pid, started) == Some(false)
            });
            if helper_ended && excel_ended {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "Workbook worker cleanup unconfirmed"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("Workbook worker cleanup failed: {error:#}");
        }
    }
}

/// Internal JSON-lines entry point. The corpus scheduler can own the same worker transport.
pub(crate) fn run(settings_path: &Path) -> Result<()> {
    let settings: Settings = serde_json::from_slice(&fs::read(settings_path)?)?;
    let mut worker = Worker::start(&settings)?;
    let timeout = Duration::from_secs(settings.timeout_seconds);
    let startup_deadline = Instant::now() + timeout;
    let identity = worker.receive(startup_deadline)?;
    ensure!(
        identity["event_type"] == "process_started",
        "Missing workbook worker startup identity"
    );
    worker.verify_identity(&identity)?;
    let mut output = io::stdout().lock();
    write_frame(&mut output, &identity)?;
    let ready = worker.receive(startup_deadline)?;
    ensure!(
        ready["event_type"] == "ready" && ready["pid"] == identity["pid"],
        "Workbook worker did not become ready"
    );
    write_frame(&mut output, &ready)?;
    let mut input = io::stdin().lock();
    while let Some(request) = read_frame(&mut input)? {
        uuid::Uuid::parse_str(
            request["id"]
                .as_str()
                .context("Missing worker command ID")?,
        )?;
        let deadline = Instant::now() + timeout;
        let mut frame = Vec::new();
        write_frame(&mut frame, &request)?;
        worker.send(frame, deadline)?;
        let reply = worker.receive(deadline)?;
        worker.verify_reply(&request, &reply)?;
        write_frame(&mut output, &reply)?;
        if reply["ok"] == false || request["kind"] == "stop" {
            break;
        }
    }
    worker.stop()
}

fn read_frame(reader: &mut impl BufRead) -> Result<Option<Value>> {
    let mut bytes = Vec::new();
    let count = reader
        .take((MAX_FRAME_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(None);
    }
    ensure!(count <= MAX_FRAME_BYTES, "Worker frame exceeds byte limit");
    ensure!(bytes.last() == Some(&b'\n'), "Incomplete worker frame");
    Ok(Some(
        serde_json::from_slice(&bytes).context("Invalid worker JSON frame")?,
    ))
}

fn write_frame(writer: &mut impl Write, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() < MAX_FRAME_BYTES,
        "Worker frame exceeds byte limit"
    );
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn framing_preserves_unicode_and_rejects_truncation_and_oversize() -> Result<()> {
        let value = json!({"path":"V:\\prüfen.xlsx"});
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &value)?;
        assert_eq!(read_frame(&mut io::Cursor::new(bytes))?, Some(value));
        assert_eq!(read_frame(&mut io::Cursor::new(Vec::<u8>::new()))?, None);
        assert!(read_frame(&mut io::Cursor::new(b"{}".to_vec())).is_err());
        assert!(read_frame(&mut io::Cursor::new(vec![b'x'; MAX_FRAME_BYTES + 1])).is_err());
        assert!(read_frame(&mut io::Cursor::new(b"not-json\n".to_vec())).is_err());
        Ok(())
    }

    #[test]
    fn stalled_mock_helper_has_bounded_reads_writes_and_owned_cleanup() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("workbook-transport-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        fs::write(
            root.join("seed.xlsx"),
            b"transport fixture, never opened by Excel",
        )?;
        fs::write(
            root.join("workbook-worker.ps1"),
            r#"
param($Seed, $ExcelExecutable, $Startup)
if ([Console]::ReadLine() -ne 'start') { throw 'Missing ownership gate' }
$process = [Diagnostics.Process]::GetCurrentProcess()
[Console]::WriteLine((@{event_type='process_started';pid=$PID;started_utc=$process.StartTime.ToUniversalTime().ToString('o')} | ConvertTo-Json -Compress))
[Console]::WriteLine((@{event_type='ready';pid=$PID} | ConvertTo-Json -Compress))
[Console]::Out.Flush()
Start-Sleep -Seconds 30
"#,
        )?;
        let settings = Settings {
            seed: root.join("seed.xlsx"),
            excel_executable: PathBuf::from(
                std::env::var_os("SystemRoot").context("Missing Windows directory")?,
            )
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
            capture_root: root.clone(),
            profile_root: root.join("profile"),
            timeout_seconds: 10,
        };
        let mut worker = Worker::start(&settings)?;
        let identity = worker.receive(Instant::now() + Duration::from_secs(10))?;
        worker.verify_identity(&identity)?;
        worker.receive(Instant::now() + Duration::from_secs(10))?;
        assert!(worker
            .receive(Instant::now() + Duration::from_millis(50))
            .is_err());
        assert!(worker
            .send(
                // Exceed the OS pipe buffer without depending on its default size.
                vec![b'x'; 8 * 1024 * 1024],
                Instant::now() + Duration::from_millis(50)
            )
            .is_err());
        worker.stop()?;
        assert_eq!(
            crate::native_process::process_alive(
                worker.identity.unwrap().0,
                worker.identity.unwrap().1
            ),
            Some(false)
        );
        drop(worker);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
