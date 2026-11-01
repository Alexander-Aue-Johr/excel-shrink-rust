//! Bounded command transport. The host owns a kill-on-close job for Excel.
use super::telemetry::{LaneId, Lifecycle, Telemetry};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub(super) struct ExcelHost {
    child: Child,
    input: BufWriter<ChildStdin>,
    responses: mpsc::Receiver<Result<Value>>,
    timeout: Duration,
}

impl ExcelHost {
    /// Open one workbook and return the PID confirmed by the command response.
    pub(super) fn open_workbook(
        &mut self,
        lane: LaneId,
        path: &Path,
        telemetry: &Telemetry,
    ) -> Result<u32> {
        let result = self.call(
            json!({"kind":"open", "lane":lane.key(), "path":path}),
            telemetry,
        )?;
        result["pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .context("Excel host returned no PID")
    }

    /// Save the open working copy without adding a recalculation command.
    pub(super) fn save_workbook(&mut self, lane: LaneId, telemetry: &Telemetry) -> Result<()> {
        self.call(json!({"kind":"save", "lane":lane.key()}), telemetry)?;
        Ok(())
    }

    /// Request the same full rebuild used by the existing comparison protocol.
    pub(super) fn recalculate_workbook(
        &mut self,
        lane: LaneId,
        telemetry: &Telemetry,
    ) -> Result<()> {
        self.call(json!({"kind":"recalculate", "lane":lane.key()}), telemetry)?;
        Ok(())
    }

    /// Require confirmed Excel exit before a normal close can complete.
    pub(super) fn close_workbook(&mut self, lane: LaneId, telemetry: &Telemetry) -> Result<()> {
        let result = self.call(json!({"kind":"close", "lane":lane.key()}), telemetry)?;
        ensure!(
            result["closed"] == true,
            "Excel exit was not confirmed: {}",
            lane.key()
        );
        Ok(())
    }

    /// Retain the host's measured geometry without reinterpreting its fields.
    pub(super) fn measure_range(
        &mut self,
        lane: LaneId,
        sheet: &str,
        range: &str,
        telemetry: &Telemetry,
    ) -> Result<Value> {
        self.call(
            json!({"kind":"measure", "lane":lane.key(), "sheet":sheet, "range":range}),
            telemetry,
        )
    }

    /// Capture one requested rectangle into its evidence file.
    pub(super) fn capture_range(
        &mut self,
        lane: LaneId,
        sheet: &str,
        range: &str,
        output: &Path,
        telemetry: &Telemetry,
    ) -> Result<()> {
        self.call(
            json!({"kind":"capture", "lane":lane.key(), "sheet":sheet, "range":range, "output":output}),
            telemetry,
        )?;
        Ok(())
    }

    /// Start one host and read its event/response stream independently of Excel.
    pub fn start(host_script: Option<&Path>, timeout: Duration) -> Result<Self> {
        let repository_script =
            Path::new("excel-shrink-cli/office-capture/targeted-comparison-host.ps1");
        let package_script = Path::new("office-capture/targeted-comparison-host.ps1");
        let default_script = if repository_script.exists() {
            repository_script.to_owned()
        } else if package_script.exists() {
            package_script.to_owned()
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("office-capture/targeted-comparison-host.ps1")
        };
        let mut child = Command::new("powershell.exe")
            .args(["-STA", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(host_script.unwrap_or(&default_script))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Start persistent Excel command host")?;
        let input = BufWriter::new(child.stdin.take().context("Excel host stdin")?);
        let output = child.stdout.take().context("Excel host stdout")?;
        let (send, responses) = mpsc::sync_channel(16);
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let message = line.context("Read Excel host response").and_then(|line| {
                    serde_json::from_str(&line).context("Invalid Excel host JSON")
                });
                if send.send(message).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            responses,
            timeout,
        })
    }

    /// Deliver startup events immediately; apply one overall deadline per command.
    fn call(&mut self, command: Value, telemetry: &Telemetry) -> Result<Value> {
        let timeout = if command["kind"] == "close" {
            self.timeout.min(Duration::from_secs(15))
        } else {
            self.timeout
        };
        serde_json::to_writer(&mut self.input, &command)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let deadline = Instant::now() + timeout;
        loop {
            ensure!(
                Instant::now() < deadline,
                "Excel command {} exceeded its {} second deadline",
                command["kind"],
                timeout.as_secs()
            );
            let remaining = deadline.saturating_duration_since(Instant::now());
            let response = match self.responses.recv_timeout(remaining) {
                Ok(response) => response?,
                Err(mpsc::RecvTimeoutError::Timeout) => anyhow::bail!(
                    "Excel command {} did not reply within {} seconds",
                    command["kind"],
                    timeout.as_secs()
                ),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("Excel host exited before replying to {}", command["kind"])
                }
            };
            if response["event"] == "process_started" {
                observe_started_process(&response, telemetry)?;
                continue;
            }
            ensure!(response["ok"] == true, "Excel host: {}", response["error"]);
            return Ok(response["result"].clone());
        }
    }
}

impl Drop for ExcelHost {
    fn drop(&mut self) {
        // Normal close is an explicit phase. Error cleanup makes no COM calls.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Bind liveness sampling to the PID and birth time reported before opening.
fn observe_started_process(event: &Value, telemetry: &Telemetry) -> Result<()> {
    let id = match event["lane"].as_str() {
        Some("O") => LaneId::O,
        Some("SD") => LaneId::SD,
        _ => anyhow::bail!("Unknown Excel process lane"),
    };
    let pid = event["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid > 0)
        .context("Missing Excel PID")?;
    let started = event["started_at"]
        .as_i64()
        .context("Missing Excel process creation time")?;
    telemetry.update(|state| {
        let lane = state.lane(id);
        lane.pid = Some(pid);
        lane.process_started_at = Some(started);
        lane.lifecycle = Lifecycle::Opening;
    })
}
