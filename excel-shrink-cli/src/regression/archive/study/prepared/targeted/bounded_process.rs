//! Independent idle watchdog for validated capture and geometry receipts.
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    fmt,
    fs,
    path::Path,
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const CLEANUP_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The owned launcher could not be confirmed exited after an attempt failed.
/// Callers must retain its working evidence instead of treating a kill request
/// or a destructor's best effort as a successful cleanup receipt.
#[derive(Debug)]
pub(super) struct CleanupUnconfirmed {
    failure: anyhow::Error,
    cleanup: anyhow::Error,
}

impl fmt::Display for CleanupUnconfirmed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Owned capture launcher cleanup is unconfirmed; attempt failed: {:#}; cleanup: {:#}",
            self.failure, self.cleanup
        )
    }
}

impl std::error::Error for CleanupUnconfirmed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cleanup.as_ref())
    }
}

struct OwnedLauncher(Child);

impl OwnedLauncher {
    fn stop_and_wait(&mut self) -> Result<ExitStatus> {
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        if let Ok(Some(status)) = self.0.try_wait() {
            return Ok(status);
        }
        // A failed observation must not prevent the termination attempt. A
        // failed kill can race with natural exit, so only try_wait confirms it.
        let kill_error = self.0.kill().err();
        let observation_error = loop {
            let observation_error = match self.0.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => None,
                Err(error) => Some(error),
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break observation_error;
            }
            thread::sleep(CLEANUP_POLL_INTERVAL.min(remaining));
        };
        bail!(
            "Owned launcher exit was not confirmed within {} seconds; termination: {}; observation: {}",
            CLEANUP_TIMEOUT.as_secs(),
            kill_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "requested".into()),
            observation_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "still running".into()),
        )
    }
}

impl Drop for OwnedLauncher {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_some() {
            return;
        }
        // Normal failures explicitly stop and reap above. This fallback covers
        // unwinding without extending a failed cleanup's bounded wait or
        // claiming that termination has been observed.
        let _ = self.0.kill();
        let _ = self.0.try_wait();
    }
}

struct CaptureClock {
    sequence: u64,
    last_success: Instant,
}

impl CaptureClock {
    fn new(now: Instant) -> Self {
        Self {
            sequence: 0,
            last_success: now,
        }
    }

    fn observe(&mut self, sequence: u64, now: Instant) {
        if sequence > self.sequence {
            self.sequence = sequence;
            self.last_success = now;
        }
    }

    fn expired(&self, now: Instant, idle: Duration) -> bool {
        now.duration_since(self.last_success) >= idle
    }
}

/// Read only the local receipt of this unique attempt. A heartbeat, phase change,
/// missing/partial JSON or a reset lane counter cannot keep an idle attempt alive.
pub(super) fn run(command: &mut Command, telemetry: &Path, idle: Duration) -> Result<ExitStatus> {
    let mut child = OwnedLauncher(
        command
            .spawn()
            .context("Start targeted private-desktop comparison")?,
    );
    let outcome = observe_until_exit(&mut child.0, telemetry, idle);
    finish_attempt(outcome, || child.stop_and_wait())
}

/// Error classification follows the cleanup receipt, never just the request
/// to stop. The closure is invoked only when the attempt did not observe exit.
fn finish_attempt(
    outcome: Result<ExitStatus>,
    stop: impl FnOnce() -> Result<ExitStatus>,
) -> Result<ExitStatus> {
    match outcome {
        Ok(status) => Ok(status),
        Err(failure) => match stop() {
            Ok(_) => Err(failure.context("Owned capture launcher exit confirmed after failure")),
            Err(cleanup) => Err(CleanupUnconfirmed { failure, cleanup }.into()),
        },
    }
}

fn observe_until_exit(child: &mut Child, telemetry: &Path, idle: Duration) -> Result<ExitStatus> {
    let mut clock = CaptureClock::new(Instant::now());
    let mut progress_clock = CaptureClock::new(Instant::now());
    loop {
        if let Some(status) = child
            .try_wait()
            .context("Inspect targeted comparison launcher")?
        {
            return Ok(status);
        }
        let progress = fs::read(telemetry)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        if let Some(sequence) = progress
            .as_ref()
            .and_then(|value| value["capture_success_sequence"].as_u64())
        {
            clock.observe(sequence, Instant::now());
        }
        if let Some(sequence) = progress
            .as_ref()
            .and_then(|value| value["capture_progress_sequence"].as_u64())
        {
            progress_clock.observe(sequence, Instant::now());
        }
        if clock.expired(Instant::now(), idle) && progress_clock.expired(Instant::now(), idle) {
            bail!(
                "No verified capture or geometry progress for {} seconds",
                idle.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_can_extend_an_attempt_indefinitely_but_heartbeats_cannot() {
        let start = Instant::now();
        let mut clock = CaptureClock::new(start);
        let idle = Duration::from_secs(2);
        for sequence in 1..100 {
            let now = start + Duration::from_secs(sequence);
            assert!(!clock.expired(now, idle));
            clock.observe(sequence, now);
        }
        let last = start + Duration::from_secs(99);
        clock.observe(99, last + Duration::from_secs(1)); // heartbeat/same count
        clock.observe(0, last + Duration::from_secs(1)); // lane reset
        assert!(clock.expired(last + Duration::from_secs(2), idle));
    }

    #[test]
    fn stalled_attempt_does_not_block_the_next_attempt() -> Result<()> {
        let telemetry = std::env::temp_dir()
            .join(format!("capture-idle-{}.json", uuid::Uuid::new_v4()));
        let mut stalled = launcher_fixture_command("wait")?;
        let started = Instant::now();
        let error = run(&mut stalled, &telemetry, Duration::from_millis(200)).unwrap_err();
        assert!(error.downcast_ref::<CleanupUnconfirmed>().is_none());
        assert!(format!("{error:#}").contains("exit confirmed"));
        assert!(started.elapsed() < Duration::from_secs(10));
        let mut next = launcher_fixture_command("exit")?;
        assert!(run(&mut next, &telemetry, Duration::from_secs(10))?.success());
        Ok(())
    }

    #[test]
    fn observation_failure_still_stops_and_reaps_the_owned_launcher() -> Result<()> {
        let mut child = OwnedLauncher(launcher_fixture_command("wait")?.spawn()?);
        let failure = anyhow::anyhow!("Cannot inspect launcher status");
        let error = finish_attempt(Err(failure), || child.stop_and_wait()).unwrap_err();
        assert!(
            child.0.try_wait()?.is_some(),
            "The failure must not return before exit is observed"
        );
        assert!(error.downcast_ref::<CleanupUnconfirmed>().is_none());
        assert!(format!("{error:#}").contains("Cannot inspect launcher status"));
        Ok(())
    }

    #[test]
    fn unsuccessful_cleanup_remains_typed_through_callers_context() {
        let error = finish_attempt(
            Err(anyhow::anyhow!("No capture progress")),
            || Err(anyhow::anyhow!("Access denied while observing launcher exit")),
        )
        .unwrap_err()
        .context("Capture failed");
        assert!(error.downcast_ref::<CleanupUnconfirmed>().is_some());
        let message = format!("{error:#}");
        assert!(message.contains("No capture progress"));
        assert!(message.contains("Access denied"));
        assert!(!message.contains("exit confirmed"));
    }

    /// The test executable is its own sleeping child; no Excel, shell or
    /// PowerShell installation is needed to exercise process ownership.
    #[test]
    fn launcher_fixture() {
        if std::env::var("EXCEL_SHRINK_WATCHDOG_FIXTURE").as_deref() == Ok("wait") {
            thread::sleep(Duration::from_secs(60));
        }
    }

    fn launcher_fixture_command(mode: &str) -> Result<Command> {
        let module = module_path!().split_once("::").unwrap().1;
        let test = format!("{module}::launcher_fixture");
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["--exact", &test, "--nocapture"])
            .env("EXCEL_SHRINK_WATCHDOG_FIXTURE", mode)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        Ok(command)
    }
}
