//! Bounded command transport. The host owns a kill-on-close job for Excel.
use super::telemetry::{LaneId, Lifecycle, Telemetry};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

// Successful retained rebuild-and-save samples stayed below four seconds.
// Use this shorter budget only for continuously responsive Pending with external links.
const LINKED_PENDING_TIMEOUT_MS: u64 = 15_000;

#[path = "profile_pool.rs"]
mod profile_pool;
use profile_pool::ProfileLease;

pub(super) struct ExcelHost {
    child: Child,
    input: BufWriter<ChildStdin>,
    responses: mpsc::Receiver<Result<Value>>,
    timeout: Duration,
    residents: BTreeSet<PathBuf>,
    profiles: BTreeMap<String, ProfileLease>,
    pub(super) operation_measurements: Vec<Value>,
}

impl ExcelHost {
    /// Inspect the installation and recipe without another Excel instance.
    pub(super) fn cache_environment(&mut self, telemetry: &Telemetry) -> Result<Value> {
        self.call(json!({"kind":"cache_environment"}), telemetry)
    }

    /// Read existing launcher observations with their original measurement scope.
    pub(super) fn measurements(&mut self, lane: LaneId, telemetry: &Telemetry) -> Result<Value> {
        self.call(json!({"kind":"measurements", "lane":lane.key()}), telemetry)
    }
    /// Open one workbook and return the PID confirmed by the command response.
    pub(super) fn open_workbook(
        &mut self,
        lane: LaneId,
        path: &Path,
        telemetry: &Telemetry,
    ) -> Result<u32> {
        self.remember_resident(lane, path)?;
        let command = self.open_command(lane, path)?;
        let result = self.call(command, telemetry)?;
        telemetry.update(|state| state.lane(lane).measurements = result["measurements"].clone())?;
        confirmed_pid(&result)
    }

    /// Launch independent inputs before awaiting either add-in's readiness.
    pub(super) fn open_workbooks(
        &mut self,
        jobs: &[(LaneId, std::path::PathBuf)],
        telemetry: &Telemetry,
    ) -> Result<Vec<u32>> {
        for (lane, path) in jobs {
            self.remember_resident(*lane, path)?;
        }
        let commands = jobs
            .iter()
            .map(|(lane, path)| self.open_command(*lane, path))
            .collect::<Result<Vec<_>>>()?;
        let response = self.call(json!({"kind":"open_batch", "jobs":commands}), telemetry)?;
        let results = response["results"]
            .as_array()
            .context("Missing opening results")?;
        ensure!(results.len() == jobs.len(), "Incomplete opening batch");
        telemetry.update(|state| {
            for ((lane, _), result) in jobs.iter().zip(results) {
                state.lane(*lane).measurements = result["measurements"].clone();
            }
        })?;
        results.iter().map(confirmed_pid).collect()
    }

    /// Keep single and parallel starts on the same workbook planning path.
    fn open_command(&mut self, lane: LaneId, path: &Path) -> Result<Value> {
        let plan = crate::regression::office_plan::build_workbook_visual_plan(path)?;
        let sheets: Vec<_> = plan.sheets.iter().map(|sheet| json!({
            "index":sheet.sheet_index, "name":sheet.sheet, "bottom_row":sheet.bottom_row,
            "base_right_column":sheet.base_right_column, "right_column":sheet.base_right_column,
            "horizontal_overflow_required":false, "drawing_fallback_required":false,
        })).collect();
        let pool = std::env::var_os("EXCEL_SHRINK_WEBVIEW_PROFILE_POOL")
            .filter(|root| !root.is_empty());
        let profile_root = if let Some(root) = pool {
            ensure!(
                !self.profiles.contains_key(lane.key()),
                "Lane already owns a profile"
            );
            let lease = ProfileLease::acquire(Path::new(&root))?;
            let directory = lease.directory.clone();
            self.profiles.insert(lane.key().to_owned(), lease);
            Some(directory)
        } else {
            None
        };
        Ok(
            json!({"kind":"open", "lane":lane.key(), "path":path, "sheets":sheets,
                "host_executable":std::env::current_exe()?, "timeout_seconds":self.timeout.as_secs(),
                "manual_calculation":true, "profile_root":profile_root,
                "external_link_parts":external_workbook_link_parts(path)?}),
        )
    }

    /// Retain session locations before startup so failed opens can still request cleanup.
    fn remember_resident(&mut self, lane: LaneId, path: &Path) -> Result<()> {
        let root = path.parent().context("Workbook has no working directory")?;
        self.residents.insert(
            root.join(format!("resident-{}", lane.key()))
                .join("session"),
        );
        Ok(())
    }

    /// Save the open working copy without adding a recalculation command.
    pub(super) fn save_workbook(&mut self, lane: LaneId, telemetry: &Telemetry) -> Result<()> {
        self.call(json!({"kind":"save", "lane":lane.key()}), telemetry)?;
        Ok(())
    }

    pub(super) fn save_as_workbook(
        &mut self,
        lane: LaneId,
        output: &Path,
        telemetry: &Telemetry,
    ) -> Result<()> {
        self.call(
            json!({"kind":"save_as", "lane":lane.key(), "output":output}),
            telemetry,
        )?;
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

    /// Transfer an existing session to its rebuilt reporting state without reopening Excel.
    pub(super) fn move_to_rebuild(
        &mut self,
        source: LaneId,
        target: LaneId,
        output: &Path,
        telemetry: &Telemetry,
    ) -> Result<u32> {
        let result = self.call(
            json!({
                "kind": "move_lane", "lane": source.key(), "target_lane": target.key(), "output": output
            }),
            telemetry,
        )?;
        ensure!(
            result["reopened"] == false,
            "In-place transition reopened the workbook"
        );
        if let Some(profile) = self.profiles.remove(source.key()) {
            self.profiles.insert(target.key().to_owned(), profile);
        }
        confirmed_pid(&result)
    }
    /// Request the same full rebuild used by the existing comparison protocol.
    pub(super) fn recalculate_workbook(
        &mut self,
        lane: LaneId,
        telemetry: &Telemetry,
    ) -> Result<()> {
        self.call(json!({"kind":"recalculate", "lane":lane.key(), "timeout_seconds":self.timeout.as_secs(),
            "linked_pending_timeout_ms":LINKED_PENDING_TIMEOUT_MS}), telemetry)?;
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
        if let Some(profile) = self.profiles.remove(lane.key()) {
            ensure!(
                result["profile_reusable"] == true,
                "Whole-job exit unconfirmed; profile quarantined"
            );
            profile.release_after_job_exit()?;
        }
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
            "spans":spans, "budget_ms":self.timeout.as_millis()}),
            telemetry,
        )?;
        ensure!(result["complete"] == true, "Height coverage is incomplete");
        Ok(result)
    }

    /// Capture after the Office.js visible-area preflight; return the observed geometry.
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

    /// Measure bounded same-sheet ranges with one Office.js synchronization per lane.
    pub(super) fn measure_ranges(
        &mut self,
        jobs: &[(LaneId, Vec<super::Candidate>)],
        telemetry: &Telemetry,
    ) -> Result<Vec<Vec<super::capture_layout::Geometry>>> {
        let mut commands = Vec::new();
        for (lane, candidates) in jobs {
            let first = candidates.first().context("Empty geometry batch")?;
            ensure!(
                candidates.len() <= crate::capture_settings::MAX_GEOMETRY_RANGES,
                "Geometry batch exceeds range limit"
            );
            ensure!(
                candidates
                    .iter()
                    .all(|candidate| candidate.sheet == first.sheet),
                "Geometry batch spans multiple sheets"
            );
            commands.push(json!({"kind":"measure_batch", "lane":lane.key(),
                "sheet":first.sheet, "ranges":candidates.iter().map(|c| &c.cell).collect::<Vec<_>>()}));
        }
        let response = self.call(json!({"kind":"capture_batch", "jobs":commands}), telemetry)?;
        let results = response["images"]
            .as_array()
            .context("Missing geometry responses")?;
        ensure!(
            results.len() == jobs.len(),
            "Incomplete geometry lane coverage"
        );
        let geometries = jobs
            .iter()
            .zip(results)
            .map(|((_, candidates), result)| decode_geometry_batch(candidates, result))
            .collect::<Result<Vec<_>>>()?;
        telemetry.progress(|state| {
            state.capture_progress_sequence +=
                geometries.iter().map(Vec::len).sum::<usize>() as u64;
        })?;
        Ok(geometries)
    }

    /// Capture bounded same-sheet packages concurrently across live Excel lanes.
    pub(super) fn capture_ranges(
        &mut self,
        jobs: &[(LaneId, super::Candidate, std::path::PathBuf)],
        core_ranges: &std::collections::BTreeMap<super::Candidate, String>,
        megapixels: usize,
        telemetry: &Telemetry,
    ) -> Result<Vec<Value>> {
        let mut lanes = std::collections::BTreeMap::<LaneId, Vec<usize>>::new();
        for (index, (lane, _, _)) in jobs.iter().enumerate() {
            lanes.entry(*lane).or_default().push(index);
        }
        let mut commands = Vec::new();
        for (lane, indices) in &lanes {
            ensure!(
                indices.len() <= crate::capture_settings::MAX_CAPTURE_RANGES,
                "Capture package exceeds its range limit"
            );
            let sheet = &jobs[indices[0]].1.sheet;
            ensure!(
                indices.iter().all(|&index| jobs[index].1.sheet == *sheet),
                "Capture package spans multiple sheets"
            );
            let ranges: Vec<_> = indices
                .iter()
                .map(|&index| {
                    let (_, candidate, output) = &jobs[index];
                    json!({"range":candidate.cell, "output":output,
                    "comparison_core_range":core_ranges.get(candidate)})
                })
                .collect();
            commands.push(
                json!({"kind":"capture_many", "lane":lane.key(), "sheet":sheet, "ranges":ranges,
                    "batch_megapixels":megapixels}),
            );
        }
        let response = self.call(json!({"kind":"capture_batch", "jobs":commands}), telemetry)?;
        let results = response["images"]
            .as_array()
            .context("Missing capture lane responses")?;
        ensure!(
            results.len() == lanes.len(),
            "Incomplete capture lane coverage"
        );
        let mut geometries = vec![Value::Null; jobs.len()];
        for ((_, indices), result) in lanes.iter().zip(results) {
            let images = result["images"]
                .as_array()
                .context("Missing capture images")?;
            ensure!(
                images.len() == indices.len(),
                "Incomplete capture range coverage"
            );
            for (&index, image) in indices.iter().zip(images) {
                ensure!(
                    image["range"].as_str() == Some(jobs[index].1.cell.as_str()),
                    "Capture range identity changed"
                );
                geometries[index] = measured_geometry(&image["geometry"])?;
            }
        }
        Ok(geometries)
    }

    /// Enumerate embedded charts and same-resident chart sheets independently of XML changes.
    pub(super) fn charts(
        &mut self,
        lane: LaneId,
        telemetry: &Telemetry,
    ) -> Result<Vec<super::charts::Inventory>> {
        let response = self.call(json!({"kind":"charts", "lane":lane.key()}), telemetry)?;
        serde_json::from_value(response["charts"].clone()).context("Invalid chart inventory")
    }

    /// Inventory native object rectangles independently of XML candidate selection.
    pub(super) fn reference_objects(
        &mut self,
        lane: LaneId,
        sheet: &str,
        telemetry: &Telemetry,
    ) -> Result<Value> {
        self.call(json!({"kind":"reference_objects", "lane":lane.key(), "sheet":sheet}), telemetry)
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
            "sheet":chart.sheet, "chart":chart.name, "chart_index":chart.index, "chart_kind":chart.kind, "output":output}),
            telemetry,
        )?;
        ensure!(response["geometry"].is_object(), "Missing chart geometry");
        Ok(response)
    }

    /// Start one host and read its event/response stream independently of Excel.
    pub fn start(host_script: Option<&Path>, timeout: Duration) -> Result<Self> {
        let script = match host_script {
            Some(path) => path.to_owned(),
            None => crate::validation::capture::capture_directory()?
                .join("targeted-comparison-host.ps1"),
        };
        let mut child = Command::new("powershell.exe")
            .args(["-STA", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(crate::regression::excel_automation::power_shell_path(&script)?)
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
            operation_measurements: Vec::new(),
            residents: BTreeSet::new(),
            profiles: BTreeMap::new(),
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
        let started = chrono::Utc::now().timestamp_millis();
        let operation = json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "kind": command["kind"], "lane": command["lane"],
            "sheet": command["sheet"], "range": command["range"],
            "job_count": command["jobs"].as_array().map(Vec::len),
            "jobs": command["jobs"].as_array().map(|jobs| jobs.iter().map(|job| json!({
                "lane":job["lane"], "kind":job["kind"], "sheet":job["sheet"],
            })).collect::<Vec<_>>()),
            "started_at": started, "deadline_at": started + timeout.as_millis() as i64,
            "state": "awaiting_host_reply"
        });
        telemetry.update(|state| state.current_operation = operation.clone())?;
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
            if !response["operation_measurements"].is_null() {
                self.operation_measurements
                    .push(response["operation_measurements"].clone());
            }
            telemetry.update(|state| {
                state.record_operation(
                    &command, &response,
                    chrono::Utc::now().timestamp_millis().saturating_sub(started).max(0) as u64,
                );
                if response["ok"] == true {
                    state.last_successful_operation = operation.clone();
                    state.last_successful_operation["state"] = json!("completed");
                    state.last_successful_operation["finished_at"] =
                        json!(chrono::Utc::now().timestamp_millis());
                    state.current_operation = Value::Null;
                }
            })?;
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
    for field in ["path", "output", "host_executable", "profile_root"] {
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
        // Give resident launchers a bounded opportunity to restore add-in registrations.
        // Forced host termination otherwise kills their jobs before their finally blocks run.
        for session in &self.residents {
            if session.is_dir() {
                if let Err(error) = std::fs::write(session.join("finish.json"), b"{}") {
                    tracing::warn!(path=%session.display(), %error, "Cannot request resident shutdown");
                }
            }
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while self
            .residents
            .iter()
            .any(|session| session.is_dir() && !session.join("launcher-result.json").is_file())
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(50));
        }
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
/// Require ordered range identities and complete, finite geometry before caching any batch.
fn decode_geometry_batch(
    candidates: &[super::Candidate],
    response: &Value,
) -> Result<Vec<super::capture_layout::Geometry>> {
    let ranges = response["ranges"]
        .as_array()
        .context("Missing geometry ranges")?;
    ensure!(
        ranges.len() == candidates.len(),
        "Incomplete geometry range coverage"
    );
    candidates
        .iter()
        .zip(ranges)
        .map(|(candidate, result)| {
            ensure!(
                result["range"].as_str() == Some(candidate.cell.as_str()),
                "Geometry range identity changed"
            );
            let geometry: super::capture_layout::Geometry = serde_json::from_value(result.clone())?;
            geometry.validate()
        })
        .collect()
}

fn measured_geometry(response: &Value) -> Result<Value> {
    let mut geometry = serde_json::Map::new();
    for key in ["top", "left", "width", "height"] {
        let value = response[key]
            .as_f64()
            .filter(|value| value.is_finite())
            .with_context(|| format!("Missing or invalid measured {key}"))?;
        geometry.insert(key.into(), json!(value));
    }
    if response["comparison_core"].is_object() {
        geometry.insert(
            "comparison_core".into(),
            measured_geometry(&response["comparison_core"])?,
        );
    }
    Ok(Value::Object(geometry))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_batch_receipts_require_every_requested_range_in_order() -> Result<()> {
        let candidates = ["A1:P128", "A129:P256"].map(|cell| super::super::Candidate {
            sheet: "Data".into(),
            cell: cell.into(),
        });
        let first = json!({"range":"A1:P128","top":0.,"left":0.,"width":300.,"height":0.});
        let second = json!({"range":"A129:P256","top":0.,"left":0.,"width":300.,"height":15.});
        let decoded = decode_geometry_batch(&candidates, &json!({"ranges":[first, second]}))?;
        assert_eq!(decoded.len(), 2);
        assert!(decoded[0].empty());
        assert!(decode_geometry_batch(&candidates, &json!({"ranges":[first]})).is_err());
        assert!(decode_geometry_batch(&candidates, &json!({"ranges":[second, first]})).is_err());
        let mut invalid = second;
        invalid["height"] = json!(-1);
        assert!(decode_geometry_batch(&candidates, &json!({"ranges":[first, invalid]})).is_err());
        Ok(())
    }

    #[test]
    fn external_link_inventory_reads_package_entries_without_parsing_xml() -> Result<()> {
        let path = crate::regression::artifact_cache::unique_path(&std::env::temp_dir(), "linked-package")?;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path)?);
        for name in ["xl/externalLinks/externalLink1.xml", "xl/externalLinks/externalLink2.xml",
            "xl/externalLinks/_rels/externalLink1.xml.rels", "xl/worksheets/sheet1.xml"] {
            zip.start_file(name, zip::write::FileOptions::default())?;
            zip.write_all(b"deliberately not XML")?;
        }
        zip.finish()?;
        assert_eq!(external_workbook_link_parts(&path)?, 2);
        std::fs::remove_file(path)?;
        Ok(())
    }

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
    #[cfg(windows)]
    #[test]
    fn height_measurement_uses_the_configured_command_budget() -> Result<()> {
        use clap::Parser;
        let root = std::env::temp_dir().join(format!("height-budget-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root)?;
        let script = root.join("host.ps1");
        std::fs::write(
            &script,
            r#"
$command = [Console]::ReadLine() | ConvertFrom-Json
$resolved = Join-Path $PSScriptRoot 'verified'
@{ok=$true;result=@{complete=$true;received_budget_ms=$command.budget_ms;script_root=$resolved}} | ConvertTo-Json -Compress
"#,
        )?;
        let args = super::super::Args::try_parse_from([
            "test",
            "--original",
            "unused.xlsx",
            "--default",
            "unused-default.xlsx",
            "--output",
            root.to_str().context("test path encoding")?,
        ])?;
        let telemetry = Telemetry::start(&args)?;
        let canonical_script = std::fs::canonicalize(&script)?;
        let mut host = ExcelHost::start(Some(&canonical_script), Duration::from_secs(90))?;
        let result = host.height_runs(LaneId::O, "Data", &[(1, 100)], &telemetry)?;
        assert_eq!(result["received_budget_ms"], 90_000);
        assert!(result["script_root"].as_str().context("script root")?.ends_with("verified"));
        drop(host);
        drop(telemetry);
        std::fs::remove_dir_all(root)?;
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

/// Require a real process identity for every successful opening response.
/// Count standard external-link parts from ZIP metadata without reading worksheet XML.
/// Presence is a diagnostic clue, not proof that a particular link caused a failure.
fn external_workbook_link_parts(path: &Path) -> Result<usize> {
    let archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    Ok(archive.file_names().filter(|name| {
        name.strip_prefix("xl/externalLinks/")
            .is_some_and(|leaf| !leaf.contains('/') && leaf.ends_with(".xml"))
    }).count())
}

fn confirmed_pid(result: &Value) -> Result<u32> {
    result["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .context("Excel host returned no PID")
}
