// Included in excel_automation.rs; private helpers share its existing cache types.
// Package identities use SHA-256 member manifests. Image/geometry cache helpers
// retain their compact FNV-1a-derived identity inside the current cache namespace.
fn canonical_workbook_hash(path: &Path) -> anyhow::Result<String> {
    Ok(super::artifact_cache::manifest(path)?.package_sha256)
}

fn atomic_cache_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cache path has no parent"))?;
    fs::create_dir_all(parent)?;
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let pending = parent.join(format!(".pending-{}-{stamp}", std::process::id()));
    let result = (|| -> anyhow::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        {
            // MoveFileEx replaces an existing cache manifest without an unlink gap.
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
            }
            let parent = fs::canonicalize(parent)?;
            let from: Vec<u16> = parent
                .join(pending.file_name().unwrap())
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            let to: Vec<u16> = parent
                .join(path.file_name().unwrap())
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } == 0 {
                return Err(io::Error::last_os_error().into());
            }
        }
        #[cfg(not(windows))]
        fs::rename(&pending, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(pending);
    }
    result
}

fn capture_geometry_matches(a: &WorkbookScreenshotCapture, b: &WorkbookScreenshotCapture) -> bool {
    fn same(a: Option<&ScreenshotCaptureInfo>, b: Option<&ScreenshotCaptureInfo>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => {
                a.start_row == b.start_row
                    && a.start_column == b.start_column
                    && a.rows == b.rows
                    && a.columns == b.columns
            }
            _ => false,
        }
    }
    same(a.capture_info.as_ref(), b.capture_info.as_ref())
        && a.tiles.len() == b.tiles.len()
        && a.tiles.iter().zip(&b.tiles).all(|(a, b)| {
            a.tile_index == b.tile_index && same(Some(&a.capture_info), Some(&b.capture_info))
        })
}
fn cached_pair_geometry_is_compatible(
    a: &WorkbookScreenshotCache,
    b: &WorkbookScreenshotCache,
) -> anyhow::Result<bool> {
    let left = a.valid_current_sheet_captures()?;
    let right = b.valid_current_sheet_captures()?;
    Ok(cached_capture_maps_are_compatible(&left, &right))
}

fn cache_capture_to_windows_paths(capture: &mut WorkbookScreenshotCapture) -> anyhow::Result<()> {
    if let Some(image) = &mut capture.image {
        *image = PathBuf::from(power_shell_path(image)?);
    }
    for tile in &mut capture.tiles {
        if let Some(image) = &mut tile.image {
            *image = PathBuf::from(power_shell_path(image)?);
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn wsl_unc_path(path: &str, current_distro: Option<&str>) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let normalized = normalized
        .strip_prefix("//?/UNC/")
        .map(|p| format!("//{p}"))
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
    // Never silently map another distribution's /home into this distribution.
    // Without WSL_DISTRO_NAME, wslpath is used instead of guessing the mount.
    current_distro?;
    Some(format!("/{tail}"))
}

fn render_environment_key(work_dir: &Path) -> anyhow::Result<String> {
    static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(key) = KEY.get() {
        return Ok(key.clone());
    }
    let _phase = visual_diagnostics::phase(
        "screenshot.renderer_environment",
        "Excel version, Windows fonts, DPI, locale, printer",
    );
    let transfer = RecalculationTransfer::new()?;
    let output = transfer.0.join("environment.json");
    let script = transfer.0.join("environment.ps1");
    fs::write(
        work_dir.join("excel-render-environment.ps1"),
        include_str!("excel_render_environment.ps1"),
    )?;
    fs::write(&script, include_str!("excel_render_environment.ps1"))?;
    run_power_shell_script(&script, &[("OutputPath", power_shell_path(&output)?)])?;
    let environment = fs::read(&output)?;
    anyhow::ensure!(
        !environment.is_empty(),
        "Excel renderer environment probe returned no data"
    );
    let mut hash = fnv_hash_update(fnv_offset_basis(), &SCREENSHOT_CACHE_VERSION.to_le_bytes());
    hash = fnv_hash_update(hash, SCREENSHOT_COMPARE_SCRIPT.as_bytes());
    hash = fnv_hash_update(hash, include_str!("excel_visual_bounds.ps1").as_bytes());
    hash = fnv_hash_update(hash, include_str!("excel_visual_clipboard.ps1").as_bytes());
    hash = fnv_hash_update(hash, include_str!("excel_visual_plan.cs").as_bytes());
    hash = fnv_hash_update(hash, &environment);
    hash = fnv_hash_update(
        hash,
        std::env::var("EXCEL_SHRINK_SCREENSHOT_ENVIRONMENT")
            .unwrap_or_default()
            .as_bytes(),
    );
    let key = format!("v{SCREENSHOT_CACHE_VERSION}-{hash:016x}");
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
            if let Some(step) = line.split("visual step phase=").nth(1) {
                if let (Some(name), Some(milliseconds)) = (
                    step.split_whitespace().next(),
                    step.split_whitespace()
                        .find_map(|word| word.strip_prefix("duration_ms=")?.parse::<u64>().ok()),
                ) {
                    visual_diagnostics::completed(name, &line, Duration::from_millis(milliseconds));
                }
            }
            if line.contains("sheet start")
                || line.contains("sheet done")
                || line.contains("opened ")
                || line.contains("range expanded")
                || line.contains("recalc ")
                || line.contains("cache hit")
                || line.contains("error=")
                || line.contains("capture finished")
                || line.contains("bounds scan")
                || line.contains("bounds shapes summary")
                || line.contains("visual step phase=")
            {
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

/// A tile manifest is complete only when non-overlapping tile rectangles cover
/// the entire capture rectangle. Valid image files alone are not enough.
fn capture_tile_coverage_is_complete(capture: &WorkbookScreenshotCapture) -> bool {
    fn rect(info: &ScreenshotCaptureInfo) -> Option<(u64, u64, u64, u64)> {
        if info.start_row == 0 || info.start_column == 0 || info.rows == 0 || info.columns == 0 {
            return None;
        }
        let row = u64::from(info.start_row);
        let column = u64::from(info.start_column);
        let bottom = row + u64::from(info.rows);
        let right = column + u64::from(info.columns);
        if bottom > 1_048_577 || right > 16_385 {
            return None;
        }
        Some((row, column, bottom, right))
    }
    let Some(full) = capture.capture_info.as_ref().and_then(rect) else {
        return false;
    };
    if capture.tiles.is_empty() {
        return true;
    }
    let mut events = Vec::new();
    let mut area = 0_u128;
    for (index, tile) in capture.tiles.iter().enumerate() {
        let Some((top, left, bottom, right)) = rect(&tile.capture_info) else {
            return false;
        };
        if top < full.0 || left < full.1 || bottom > full.2 || right > full.3 {
            return false;
        }
        area += u128::from(bottom - top) * u128::from(right - left);
        events.push((top, 1_u8, left, right, index));
        events.push((bottom, 0_u8, left, right, index));
    }
    if area != u128::from(full.2 - full.0) * u128::from(full.3 - full.1) {
        return false;
    }
    events.sort_unstable(); // At a row boundary, remove ending intervals first.
    let mut active: BTreeMap<(u64, usize), u64> = BTreeMap::new();
    for (_, kind, left, right, index) in events {
        let key = (left, index);
        if kind == 0 {
            active.remove(&key);
            continue;
        }
        if active
            .range(..key)
            .next_back()
            .is_some_and(|(_, end)| *end > left)
            || active
                .range(key..)
                .next()
                .is_some_and(|((start, _), _)| *start < right)
        {
            return false;
        }
        active.insert(key, right);
    }
    active.is_empty()
}

fn cached_capture_maps_are_compatible(
    left: &BTreeMap<usize, WorkbookScreenshotCapture>,
    right: &BTreeMap<usize, WorkbookScreenshotCapture>,
) -> bool {
    !left.is_empty()
        && left.len() == right.len()
        && left.iter().all(|(i, a)| {
            right
                .get(i)
                .is_some_and(|b| a.sheet_name == b.sheet_name && capture_geometry_matches(a, b))
        })
}
fn write_visual_capture_support_files(directory: &Path) -> anyhow::Result<()> {
    for (name, source) in [
        (
            "excel_visual_bounds.ps1",
            include_str!("excel_visual_bounds.ps1"),
        ),
        (
            "excel_visual_clipboard.ps1",
            include_str!("excel_visual_clipboard.ps1"),
        ),
        ("excel_visual_plan.cs", include_str!("excel_visual_plan.cs")),
    ] {
        fs::write(directory.join(name), source)?;
    }
    Ok(())
}
