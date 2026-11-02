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
        let plan = crate::regression::office_plan::build_workbook_visual_plan(path)?;
        let sheets: Vec<_> = plan.sheets.iter().map(|sheet| json!({
            "index":sheet.sheet_index, "name":sheet.sheet, "bottom_row":sheet.bottom_row,
            "base_right_column":sheet.base_right_column, "right_column":sheet.base_right_column,
            "horizontal_overflow_required":false, "drawing_fallback_required":false,
        })).collect();
        let result = self.call(
            json!({"kind":"open", "lane":lane.key(), "path":path, "sheets":sheets,
                "host_executable":std::env::current_exe()?, "timeout_seconds":self.timeout.as_secs()}),
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

    /// Require a positive native completion response, not merely an opened pane.
    pub(super) fn optimize_workbook(&mut self, telemetry: &Telemetry) -> Result<Value> {
        let result = self.call(
            json!({"kind":"optimize", "lane":"CP", "timeout_seconds":self.timeout.as_secs()}),
            telemetry,
        )?;
        ensure!(
            matches!(
                result["status"].as_str(),
                Some("optimization_confirmed" | "no_optimization_needed")
            ),
            "Check Performance returned no confirmed outcome: {result}"
        );
        Ok(result)
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

    /// Measure verified height runs in the same Office.js workbook used for captures.
    pub(super) fn height_runs(
        &mut self,
        lane: LaneId,
        sheet: &str,
        spans: &[(u32, u32)],
        telemetry: &Telemetry,
    ) -> Result<Value> {
        let spans: Vec<_> = spans
            .iter()
            .map(|&(start, end)| json!({"start":start,"end":end}))
            .collect();
        let result = self.call(
            json!({"kind":"heights", "lane":lane.key(), "sheet":sheet,
            "spans":spans, "budget_ms":self.timeout.as_millis().min(30000)}),
            telemetry,
        )?;
        ensure!(result["complete"] == true, "Height coverage is incomplete");
        Ok(result)
    }

    /// Capture pixels and geometry in one Office.js synchronization.
    pub(super) fn capture_range(
        &mut self,
        lane: LaneId,
        sheet: &str,
        range: &str,
        output: &Path,
        telemetry: &Telemetry,
    ) -> Result<Value> {
        let response = self.call(
            json!({"kind":"capture", "lane":lane.key(), "sheet":sheet, "range":range, "output":output}),
            telemetry,
        )?;
        measured_geometry(&response["geometry"])
    }

    /// Submit one outstanding capture per open lane and require every corresponding result.
    pub(super) fn capture_ranges(
        &mut self,
        jobs: &[(LaneId, super::Candidate, std::path::PathBuf)],
        telemetry: &Telemetry,
    ) -> Result<Vec<Value>> {
        let response = self.call(json!({"kind":"capture_batch","jobs":jobs.iter().map(|(lane,candidate,output)|
            json!({"kind":"capture","lane":lane.key(),"sheet":candidate.sheet,"range":candidate.cell,"output":output}))
            .collect::<Vec<_>>()}), telemetry)?;
        let images = response["images"]
            .as_array()
            .context("Missing capture batch responses")?;
        ensure!(
            images.len() == jobs.len(),
            "Capture batch response count differs from its requests"
        );
        images
            .iter()
            .map(|image| measured_geometry(&image["geometry"]))
            .collect()
    }

    /// Enumerate embedded charts and same-resident chart sheets independently of XML changes.
    pub(super) fn charts(
        &mut self,
        lane: LaneId,
        telemetry: &Telemetry,
    ) -> Result<Vec<super::charts::Chart>> {
        let response = self.call(json!({"kind":"charts", "lane":lane.key()}), telemetry)?;
        serde_json::from_value(response["charts"].clone()).context("Invalid chart inventory")
    }

    /// Export the whole chart and its placement, without a worksheet-sized capture.
    pub(super) fn capture_chart(
        &mut self,
        lane: LaneId,
        chart: &super::charts::Chart,
        output: &Path,
        telemetry: &Telemetry,
    ) -> Result<Value> {
        let response = self.call(
            json!({"kind":"capture_chart", "lane":lane.key(),
            "sheet":chart.sheet, "chart":chart.name, "chart_kind":chart.kind, "output":output}),
            telemetry,
        )?;
        ensure!(response["geometry"].is_object(), "Missing chart geometry");
        Ok(response)
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
    fn call(&mut self, mut command: Value, telemetry: &Telemetry) -> Result<Value> {
        normalize_windows_paths(&mut command)?;
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

/// PowerShell's filesystem provider cannot resolve Rust's canonical verbatim drive paths.
fn normalize_windows_paths(command: &mut Value) -> Result<()> {
    if let Some(jobs) = command.get_mut("jobs").and_then(Value::as_array_mut) {
        for job in jobs {
            normalize_windows_paths(job)?;
        }
    }
    for field in ["path", "output", "host_executable"] {
        if let Some(path) = command[field].as_str() {
            command[field] = json!(crate::regression::excel_automation::power_shell_path(
                Path::new(path)
            )?);
        }
    }
    Ok(())
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
        Some("CP") => LaneId::CP,
        Some("OR") => LaneId::OR,
        Some("SDR") => LaneId::SDR,
        Some("CPR") => LaneId::CPR,
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

/// Transport timings and resource samples are not workbook geometry.
fn measured_geometry(response: &Value) -> Result<Value> {
    let mut geometry = serde_json::Map::new();
    for key in ["top", "left", "width", "height"] {
        let value = response[key]
            .as_f64()
            .filter(|value| value.is_finite())
            .with_context(|| format!("Missing or invalid measured {key}"))?;
        geometry.insert(key.into(), json!(value));
    }
    Ok(Value::Object(geometry))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn canonical_paths_cross_the_powershell_boundary_without_verbatim_prefixes() -> Result<()> {
        let mut command = json!({"kind":"capture","path":r"\\?\V:\study\original.xlsx",
            "output":r"\\?\UNC\server\share\tile.png", "range":"A1:B2"});
        normalize_windows_paths(&mut command)?;
        assert_eq!(command["path"], r"V:\study\original.xlsx");
        assert_eq!(command["output"], r"\\server\share\tile.png");
        assert_eq!(command["range"], "A1:B2");
        Ok(())
    }
    #[test]
    fn protocol_telemetry_is_not_a_geometry_difference() {
        let a = json!({"top":0,"left":0,"width":90,"height":15,"elapsed_ms":1});
        let b = json!({"top":0,"left":0,"width":90,"height":15,"elapsed_ms":100});
        assert_eq!(
            measured_geometry(&a).unwrap(),
            measured_geometry(&b).unwrap()
        );
        assert!(measured_geometry(&json!({"top":0})).is_err());
    }
}
