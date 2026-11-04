//! Independent idle watchdog; only persisted capture receipts extend the deadline.
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

struct OwnedLauncher(Child);
impl Drop for OwnedLauncher {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_some() {
            return;
        }
        let _ = self.0.kill();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.0.try_wait().ok().flatten().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
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
    let mut clock = CaptureClock::new(Instant::now());
    loop {
        if let Some(status) = child
            .0
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
        if clock.expired(Instant::now(), idle) {
            bail!(
                "No successful capture for {} seconds; owned launcher stopped",
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

    #[cfg(windows)]
    #[test]
    fn stalled_attempt_does_not_block_the_next_attempt() -> Result<()> {
        let root = std::env::temp_dir().join(format!("capture-idle-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root)?;
        let telemetry = root.join("telemetry.json");
        let mut stalled = Command::new("powershell.exe");
        stalled.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"]);
        let started = Instant::now();
        assert!(run(&mut stalled, &telemetry, Duration::from_millis(200)).is_err());
        assert!(started.elapsed() < Duration::from_secs(10));
        let mut next = Command::new("powershell.exe");
        next.args(["-NoProfile", "-Command", "exit 0"]);
        assert!(run(&mut next, &telemetry, Duration::from_secs(10))?.success());
        fs::remove_dir(&root)?;
        Ok(())
    }
}
