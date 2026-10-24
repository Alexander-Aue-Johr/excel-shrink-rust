#[cfg(not(windows))]
fn wsl_unc_path(path: &str, current_distro: Option<&str>) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let normalized = normalized
        .strip_prefix("//?/UNC/")
        .map(|path| format!("//{path}"))
        .unwrap_or(normalized);
    let rest = normalized.strip_prefix("//")?;
    let (server, rest) = rest.split_once('/')?;
    if !server.eq_ignore_ascii_case("wsl.localhost") && !server.eq_ignore_ascii_case("wsl$") {
        return None;
    }
    let (distro, tail) = rest.split_once('/').unwrap_or((rest, ""));
    if distro.is_empty()
        || current_distro.is_some_and(|current| !current.eq_ignore_ascii_case(distro))
    {
        return None;
    }
    current_distro?;
    Some(format!("/{tail}"))
}

fn recalculation_environment_key(work_dir: &Path) -> anyhow::Result<String> {
    static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(key) = KEY.get() {
        return Ok(key.clone());
    }
    let transfer = RecalculationTransfer::new()?;
    let output = transfer.0.join("environment.json");
    let script = transfer.0.join("environment.ps1");
    const SOURCE: &str = include_str!("excel_render_environment.ps1");
    // This probe can be the first operation in a new selection-recalc tree.
    // A warm process cache used to hide the missing-directory failure.
    fs::create_dir_all(work_dir)?;
    fs::write(work_dir.join("excel-render-environment.ps1"), SOURCE)?;
    fs::write(&script, SOURCE)?;
    run_power_shell_script(&script, &[("OutputPath", power_shell_path(&output)?)])?;
    let environment = fs::read(&output)?;
    anyhow::ensure!(
        !environment.is_empty(),
        "Excel environment probe returned no data"
    );
    let mut material = b"recalc-environment-v1".to_vec();
    material.extend_from_slice(SOURCE.as_bytes());
    material.extend_from_slice(&environment);
    let key = super::artifact_cache::sha256(&material);
    let _ = KEY.set(key.clone());
    Ok(key)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExcelProgressLineKind {
    RecalcStarted,
    RecalcFinished,
    Normal,
}

fn excel_progress_line_kind(line: &str) -> ExcelProgressLineKind {
    if line.contains("recalc CalculateFullRebuild start") {
        ExcelProgressLineKind::RecalcStarted
    } else if line.contains("recalc CalculateFullRebuild finished") {
        ExcelProgressLineKind::RecalcFinished
    } else {
        ExcelProgressLineKind::Normal
    }
}

struct ExcelProgressTail {
    offset: u64,
    pending: Vec<u8>,
    phase: visual_diagnostics::Phase,
    recalc_started: Option<Instant>,
    spinner_frame: usize,
    spinner_visible: bool,
}
impl ExcelProgressTail {
    fn new(label: &str) -> Self {
        Self {
            offset: 0,
            pending: Vec::new(),
            phase: visual_diagnostics::phase("excel.automation", label),
            recalc_started: None,
            spinner_frame: 0,
            spinner_visible: false,
        }
    }

    fn spinner_enabled() -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stderr())
            && std::env::var_os("CI").is_none()
            && !std::env::var("TERM").is_ok_and(|term| term.eq_ignore_ascii_case("dumb"))
    }

    fn tick_recalc_spinner(&mut self) {
        let Some(started) = self.recalc_started else {
            return;
        };
        if !Self::spinner_enabled() {
            return;
        }

        const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

        let frame = FRAMES[self.spinner_frame % FRAMES.len()];
        self.spinner_frame = self.spinner_frame.wrapping_add(1);

        eprint!(
            "\r\x1b[2K{frame} Excel CalculateFullRebuild … {:.1} s",
            started.elapsed().as_secs_f64()
        );
        let _ = std::io::stderr().flush();
        self.spinner_visible = true;
    }

    fn clear_recalc_spinner(&mut self) {
        if self.spinner_visible && Self::spinner_enabled() {
            eprint!("\r\x1b[2K");
            let _ = std::io::stderr().flush();
        }
        self.spinner_visible = false;
    }

    fn start_recalc(&mut self, line: &str) {
        self.clear_recalc_spinner();
        self.recalc_started = Some(Instant::now());
        self.spinner_frame = 0;

        self.phase.set_detail_silent("Excel CalculateFullRebuild");

        tracing::info!(event = "excel_recalc_started", "[excel-progress] {line}");

        self.tick_recalc_spinner();
    }

    fn finish_recalc(&mut self, line: &str) {
        let elapsed = self.recalc_started.take().map(|started| started.elapsed());

        self.clear_recalc_spinner();
        self.phase.set_detail_silent(line.to_owned());

        if let Some(elapsed) = elapsed {
            tracing::info!(
                event = "excel_recalc_finished",
                elapsed_ms = elapsed.as_millis() as u64,
                elapsed_seconds = elapsed.as_secs_f64(),
                "[excel-progress] Excel CalculateFullRebuild finished"
            );
        } else {
            tracing::info!(event = "excel_recalc_finished", "[excel-progress] {line}");
        }
    }

    fn abort_recalc_spinner_if_active(&mut self) {
        if self.recalc_started.take().is_some() {
            self.clear_recalc_spinner();
        }
    }

    fn recalculation_elapsed(&self) -> Option<Duration> {
        self.recalc_started.map(|started| started.elapsed())
    }

    fn poll(&mut self, path: &Path) -> io::Result<bool> {
        use std::io::{Seek, SeekFrom};
        // poll() already runs every 500 ms, so Rust can animate the spinner
        // while the synchronous Excel COM call blocks PowerShell.
        self.tick_recalc_spinner();
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let length = file.metadata()?.len();
        if length < self.offset {
            self.offset = 0;
            self.pending.clear();
            self.abort_recalc_spinner_if_active();
        }
        if length == self.offset {
            return Ok(false);
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let read = file.read_to_end(&mut self.pending)?;
        self.offset += read as u64;
        let mut consumed = 0;
        let mut lines = Vec::new();
        for i in 0..self.pending.len() {
            if self.pending[i] != b'\n' {
                continue;
            }
            let line = String::from_utf8_lossy(&self.pending[consumed..i])
                .trim()
                .trim_start_matches('\u{feff}')
                .to_owned();
            consumed = i + 1;
            if !line.is_empty() {
                lines.push(line);
            }
        }
        self.pending.drain(..consumed);

        for line in lines {
            match excel_progress_line_kind(&line) {
                ExcelProgressLineKind::RecalcStarted => {
                    self.start_recalc(&line);
                    continue;
                }

                ExcelProgressLineKind::RecalcFinished => {
                    self.finish_recalc(&line);
                    continue;
                }

                ExcelProgressLineKind::Normal => {}
            }

            self.phase.detail(line.clone());
            if line.contains("recalc ") || line.contains("error=") {
                tracing::info!("[excel-progress] {line}");
            } else {
                tracing::debug!("[excel-progress] {line}");
            }
        }
        Ok(read > 0)
    }
}
impl Drop for ExcelProgressTail {
    fn drop(&mut self) {
        self.clear_recalc_spinner();
    }
}

#[cfg(test)]
mod excel_progress_tail_tests {
    use super::*;

    #[test]

    fn classifies_synchronous_recalculation_boundaries() {
        assert_eq!(
            excel_progress_line_kind("2026-09-24T18:49:00+02:00 recalc CalculateFullRebuild start"),
            ExcelProgressLineKind::RecalcStarted
        );

        assert_eq!(
            excel_progress_line_kind(
                "2026-09-24T18:49:18+02:00 recalc CalculateFullRebuild finished"
            ),
            ExcelProgressLineKind::RecalcFinished
        );

        assert_eq!(
            excel_progress_line_kind("2026-09-24T18:49:19+02:00 recalc SaveAs start"),
            ExcelProgressLineKind::Normal
        );
    }
}

/// Owns only the PowerShell process and Excel PIDs announced by that process.
/// Errors while reading progress must not leave an invisible Excel instance behind.
struct ManagedAutomationChild {
    process: Child,
    progress_path: Option<PathBuf>,
    finished: bool,
}
impl ManagedAutomationChild {
    fn finish(&mut self, success: bool) {
        if !success {
            if let Some(path) = &self.progress_path {
                terminate_excel_processes_from_progress_log(path);
            }
        }
        self.finished = true;
    }
    fn terminate(&mut self) {
        if self.finished {
            return;
        }
        terminate_child_process(&mut self.process);
        if let Some(path) = &self.progress_path {
            terminate_excel_processes_from_progress_log(path);
        }
        self.finished = true;
    }
}
impl Drop for ManagedAutomationChild {
    fn drop(&mut self) {
        self.terminate();
    }
}
