use super::visual_diagnostics;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const EXCEL_AUTOMATION_POLL_INTERVAL: Duration = Duration::from_millis(500);
include!("visual_cache_support.rs");
include!("baseline_recalculation.rs");

#[derive(Debug, Clone, Serialize)]
pub struct ExcelSavedWorkbook {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExcelOpenMetrics {
    pub open_seconds: f64,
    pub working_set_bytes: u64,
    pub private_bytes: u64,
    pub calculation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub macros_removed: Option<bool>,
}

pub fn measure_workbook_open(input: &Path, work_dir: &Path) -> anyhow::Result<ExcelOpenMetrics> {
    open_workbook(input, None, work_dir)
}

pub fn convert_legacy_workbook(
    input: &Path,
    output: &Path,
    work_dir: &Path,
) -> anyhow::Result<ExcelOpenMetrics> {
    anyhow::ensure!(input != output, "conversion input and output must differ");
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    open_workbook(input, Some(output), work_dir)
}

fn open_workbook(
    input: &Path,
    output: Option<&Path>,
    work_dir: &Path,
) -> anyhow::Result<ExcelOpenMetrics> {
    fs::create_dir_all(work_dir)?;
    let transfer = RecalculationTransfer::new()?;
    let local_input = transfer.0.join(format!("input.{}", workbook_extension(input)));
    fs::copy(input, &local_input)?;
    let script = transfer.0.join("open.ps1");
    fs::write(&script, include_str!("excel_open.ps1"))?;
    fs::write(transfer.0.join("archive_processes.ps1"), include_str!("archive_processes.ps1"))?;
    fs::write(transfer.0.join("CaptureProcesses.cs"), include_str!("../../office-capture/CaptureProcesses.cs"))?;
    let progress = transfer.0.join("progress.log");
    let metrics = transfer.0.join("metrics.json");
    let converted = transfer.0.join("converted.xlsx");
    let mut args = vec![
        ("InputPath", power_shell_path(&local_input)?),
        ("MetricsPath", power_shell_path(&metrics)?),
        ("ProgressPath", power_shell_path(&progress)?),
    ];
    if output.is_some() {
        args.push(("OutputPath", power_shell_path(&converted)?));
    }
    let result = run_power_shell_script(&script, &args);
    if progress.is_file() {
        fs::copy(&progress, work_dir.join("excel-open-progress.log"))?;
    }
    result?;
    let measured: ExcelOpenMetrics = serde_json::from_slice(&fs::read(&metrics)?)?;
    fs::copy(&metrics, work_dir.join("excel-open-metrics.json"))?;
    if let Some(output) = output {
        anyhow::ensure!(converted.is_file(), "Excel did not create a converted workbook");
        fs::copy(&converted, output)?;
    }
    Ok(measured)
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ExcelScreenshotDifferenceSummary {
    pub total_differences: usize,
    pub shift_only_differences: usize,
    pub entropy_changed_differences: usize,
    pub screenshot_error_differences: usize,
    pub diff_pixels: u64,
    pub shift_only_diff_pixels: u64,
    pub entropy_changed_diff_pixels: u64,
    pub shift_movement_px: u64,
    pub max_shift_movement_px: u32,
    #[serde(default)]
    pub row_height_differences: usize,
}

struct RecalculationTransfer(PathBuf);

impl RecalculationTransfer {
    fn new() -> io::Result<Self> {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::var_os("EXCEL_SHRINK_TRANSFER_ROOT")
            .map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let path = root.join(format!("es-recalc-{}-{unique}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for RecalculationTransfer {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn save_recalculated_copy(
    input_path: &Path,
    output_path: &Path,
    work_dir: &Path,
) -> anyhow::Result<ExcelSavedWorkbook> {
    let _visual_phase = visual_diagnostics::phase(
        "excel.recalculate_and_save",
        input_path.display().to_string(),
    );
    let before = super::artifact_cache::snapshot(input_path)?;
    anyhow::ensure!(
        input_path != output_path,
        "recalculation input and output must differ"
    );
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir_all(work_dir)?;
    fs::remove_file(output_path).ok();

    let script_path = work_dir.join("excel_recalculate_and_save.ps1");
    fs::write(&script_path, RECALCULATE_AND_SAVE_SCRIPT)?;

    // Keep both Copy-Item endpoints short too: Windows PowerShell can reject
    // long UNC paths even though Excel itself only sees the local staged paths.
    let transfer = RecalculationTransfer::new()?;
    let transfer_input = transfer.0.join("input.xlsx");
    let transfer_output = transfer.0.join("output.xlsx");
    fs::copy(input_path, &transfer_input)?;
    let local_script = transfer.0.join("recalculate.ps1");
    let local_progress = transfer.0.join("recalc-progress.log");
    fs::write(&local_script, RECALCULATE_AND_SAVE_SCRIPT)?;
    for directory in [&transfer.0, work_dir] {
        fs::write(directory.join("archive_processes.ps1"), include_str!("archive_processes.ps1"))?;
        fs::write(directory.join("CaptureProcesses.cs"), include_str!("../../office-capture/CaptureProcesses.cs"))?;
    }
    let result = run_power_shell_script(
        &local_script,
        &[
            ("InputPath", power_shell_path(&transfer_input)?),
            ("OutputPath", power_shell_path(&transfer_output)?),
            ("ProgressPath", power_shell_path(&local_progress)?),
        ],
    );
    if local_progress.is_file() {
        fs::copy(&local_progress, work_dir.join("excel-recalc-progress.log"))?;
    }
    result?;
    anyhow::ensure!(
        transfer_output.is_file(),
        "Excel did not create recalculated transfer workbook: {}",
        transfer_output.display()
    );
    fs::copy(&transfer_output, output_path).map_err(|error| {
        anyhow::anyhow!(
            "failed to copy recalculated workbook to '{}': {error}",
            output_path.display()
        )
    })?;

    anyhow::ensure!(
        output_path.exists(),
        "Excel did not create recalculated workbook: {}",
        output_path.display()
    );

    super::artifact_cache::assert_snapshot_unchanged(
        &before,
        input_path,
        &work_dir.join("source-mutation"),
    )?;
    super::artifact_cache::record_stage_change(
        &before,
        output_path,
        &work_dir.join("recalc-package-changes"),
    )?;
    replace_workbook_with_shared_cache_link(output_path, work_dir)?;

    Ok(ExcelSavedWorkbook {
        path: output_path.to_path_buf(),
    })
}

fn file_content_hash(path: &Path) -> anyhow::Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut buffer = [0u8; 64 * 1024];
    let mut hash = fnv_offset_basis();

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hash = fnv_hash_update(hash, &buffer[..bytes_read]);
    }

    Ok(format!("{hash:016x}"))
}

fn fnv_offset_basis() -> u64 {
    0xcbf29ce484222325
}

fn fnv_hash_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn replace_workbook_with_shared_cache_link(
    workbook_path: &Path,
    anchor_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let destination_dir = workbook_path.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "workbook path has no parent directory: {}",
            workbook_path.display()
        )
    })?;
    let destination_file_name = workbook_path.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "workbook path has no file name: {}",
            workbook_path.display()
        )
    })?;

    link_workbook_artifact_to_shared_cache(
        workbook_path,
        anchor_dir,
        destination_dir,
        destination_file_name,
    )
}

pub fn link_workbook_artifact_to_shared_cache(
    source_path: &Path,
    anchor_dir: &Path,
    destination_dir: &Path,
    destination_file_name: &OsStr,
) -> anyhow::Result<PathBuf> {
    let shared_path = ensure_shared_workbook(source_path, anchor_dir)?;
    let destination = destination_dir.join(destination_file_name);
    materialize_workbook_reference(&shared_path, &destination)?;
    Ok(destination)
}

fn ensure_shared_workbook(source_path: &Path, anchor_dir: &Path) -> anyhow::Result<PathBuf> {
    let _visual_phase = visual_diagnostics::phase(
        "regression.shared_workbook",
        source_path.display().to_string(),
    );
    let workbook_hash = file_content_hash(source_path)?;
    let extension = workbook_extension(source_path);
    let shared_path = shared_workbook_root(anchor_dir)
        .join(&workbook_hash)
        .join(format!("workbook.{extension}"));

    if shared_path == source_path {
        return Ok(shared_path);
    }

    if shared_path.exists() {
        if file_content_hash(&shared_path)? == workbook_hash {
            return Ok(shared_path);
        }
        fs::remove_file(&shared_path)?;
    }

    if let Some(parent) = shared_path.parent() {
        fs::create_dir_all(parent)?;
    }

    match fs::hard_link(source_path, &shared_path) {
        Ok(()) => Ok(shared_path),
        Err(_) => {
            fs::copy(source_path, &shared_path).map_err(|error| {
                anyhow::anyhow!(
                    "failed to materialize shared workbook '{}' from '{}': {error}",
                    shared_path.display(),
                    source_path.display()
                )
            })?;
            Ok(shared_path)
        }
    }
}

fn materialize_workbook_reference(shared_path: &Path, destination: &Path) -> anyhow::Result<()> {
    if shared_path == destination {
        return Ok(());
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    if destination.exists() {
        fs::remove_file(destination)?;
    }

    match fs::hard_link(shared_path, destination) {
        Ok(()) => Ok(()),
        Err(hard_link_error) => match symlink_file(shared_path, destination) {
            Ok(()) => Ok(()),
            Err(symlink_error) => {
                fs::copy(shared_path, destination).map_err(|copy_error| {
                    anyhow::anyhow!(
                        "failed to link shared workbook '{}' to '{}' (hardlink: {}; symlink: {}; copy: {})",
                        shared_path.display(),
                        destination.display(),
                        hard_link_error,
                        symlink_error,
                        copy_error
                    )
                })?;
                Ok(())
            }
        },
    }
}

#[cfg(windows)]
fn symlink_file(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(source, destination)
}

#[cfg(unix)]
fn symlink_file(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(not(any(windows, unix)))]
fn symlink_file(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file symlinks are not supported on this platform",
    ))
}

fn workbook_extension(path: &Path) -> String {
    path.extension()
        .and_then(OsStr::to_str)
        .filter(|extension| !extension.is_empty())
        .unwrap_or("xlsx")
        .to_ascii_lowercase()
}

fn shared_workbook_root(anchor_dir: &Path) -> PathBuf {
    if let Some(root) = std::env::var_os("EXCEL_SHRINK_SHARED_WORKBOOK_DIR") {
        return PathBuf::from(root);
    }
    for ancestor in anchor_dir.ancestors() {
        if ancestor.file_name() == Some(std::ffi::OsStr::new("regression")) {
            if let Some(repo_root) = ancestor.parent() {
                return repo_root.join(".tmp").join("workbooks").join("shared");
            }
        }
    }

    std::env::current_dir()
        .unwrap_or_else(|_| anchor_dir.to_path_buf())
        .join(".tmp")
        .join("workbooks")
        .join("shared")
}

fn normalize_automation_path(path: &PathBuf) -> PathBuf {
    #[cfg(not(windows))]
    {
        let text = path.to_string_lossy();
        let distro = std::env::var("WSL_DISTRO_NAME").ok();
        if let Some(converted) = wsl_unc_path(&text, distro.as_deref()) {
            return PathBuf::from(converted);
        }
        if let Some(converted) = windows_path_to_wsl_mnt_path(&text) {
            return PathBuf::from(converted);
        }
        // Let WSL resolve non-standard drive mounts and UNC shares. Cache these
        // results: a report may reference the same image many times.
        if text.starts_with(r"\\") {
            static PATHS: std::sync::OnceLock<std::sync::Mutex<BTreeMap<String, PathBuf>>> =
                std::sync::OnceLock::new();
            let paths = PATHS.get_or_init(Default::default);
            if let Some(converted) = paths.lock().unwrap().get(text.as_ref()).cloned() {
                return converted;
            }
            if let Ok(output) = Command::new("wslpath").args(["-u", text.as_ref()]).output() {
                if output.status.success() {
                    let converted = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                    if converted.starts_with('/') {
                        let converted = PathBuf::from(converted);
                        paths
                            .lock()
                            .unwrap()
                            .insert(text.into_owned(), converted.clone());
                        return converted;
                    }
                }
            }
        }
    }
    path.clone()
}

#[cfg(not(windows))]
fn windows_path_to_wsl_mnt_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || bytes[2] != b'\\' {
        return None;
    }
    let drive = bytes[0] as char;
    if !drive.is_ascii_alphabetic() {
        return None;
    }
    Some(format!(
        "/mnt/{}/{}",
        drive.to_ascii_lowercase(),
        path[3..].replace('\\', "/")
    ))
}

fn run_power_shell_script(script_path: &Path, args: &[(&str, String)]) -> anyhow::Result<()> {
    let status = run_power_shell_script_with_status(script_path, args)?;
    anyhow::ensure!(
        status.success(),
        "Excel automation script failed: {}",
        script_path.display()
    );
    Ok(())
}

fn run_power_shell_script_with_status(
    script_path: &Path,
    args: &[(&str, String)],
) -> anyhow::Result<std::process::ExitStatus> {
    let mut progress = ExcelProgressTail::new(&script_path.display().to_string());
    let progress_path = args
        .iter()
        .find(|(name, _)| *name == "ProgressPath")
        .map(|(_, value)| normalize_automation_path(&PathBuf::from(value)));
    if let Some(path) = &progress_path {
        let _ = fs::remove_file(path);
    }
    let process = power_shell_script_command(script_path, args)?
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to start PowerShell automation: {e}"))?;
    let mut child = ManagedAutomationChild {
        process,
        progress_path: progress_path.clone(),
        finished: false,
    };
    let automation_timeout =
        visual_diagnostics::seconds_from_env("EXCEL_SHRINK_AUTOMATION_STALL_SECONDS", 1800.0);
    let recalculation_timeout =
        visual_diagnostics::seconds_from_env("EXCEL_SHRINK_RECALC_TIMEOUT_SECONDS", 1800.0);
    let mut last_progress = Instant::now();
    loop {
        if let Some(path) = &progress_path {
            if progress.poll(path)? {
                last_progress = Instant::now();
            }
        }
        if let Some(status) = child.process.try_wait()? {
            child.finish(status.success());
            if let Some(path) = &progress_path {
                let _ = progress.poll(path);
            }
            return Ok(status);
        }
        if let Some(elapsed) = progress.recalculation_elapsed() {
            if elapsed >= recalculation_timeout {
                child.terminate();
                anyhow::bail!(
                    "Excel recalculation exceeded {:.1}s: {}",
                    recalculation_timeout.as_secs_f64(),
                    script_path.display()
                );
            }
        } else if last_progress.elapsed() >= automation_timeout {
            child.terminate();
            anyhow::bail!(
                "Excel automation stalled for {:.1}s: {}",
                automation_timeout.as_secs_f64(),
                script_path.display()
            );
        }
        thread::sleep(EXCEL_AUTOMATION_POLL_INTERVAL);
    }
}

fn power_shell_script_command(
    script_path: &Path,
    args: &[(&str, String)],
) -> anyhow::Result<Command> {
    let Some(shell) = find_power_shell() else {
        anyhow::bail!(
            "PowerShell is not available. Install Excel-accessible PowerShell or run on Windows/WSL with powershell.exe."
        );
    };

    let mut command = Command::new(shell);
    command.arg("-NoProfile");
    let program = command.get_program().to_string_lossy().to_ascii_lowercase();
    if program.contains("powershell") {
        command.arg("-STA");
        command.arg("-ExecutionPolicy").arg("Bypass");
    }
    let script_arg = if cfg!(not(windows)) && program.ends_with(".exe") {
        power_shell_path(script_path)?
    } else {
        script_path.to_string_lossy().to_string()
    };
    command.arg("-File").arg(script_arg);
    for (name, value) in args {
        command.arg(format!("-{name}")).arg(value);
    }

    Ok(command)
}

fn terminate_child_process(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn terminate_excel_processes_from_progress_log(progress_path: &Path) {
    for pid in excel_pids_from_progress_log(progress_path) {
        eprintln!("[excel-screenshot] terminating stalled Excel automation process pid={pid}");
        terminate_process_by_pid(pid);
    }
}

fn read_progress_log(path: &Path) -> io::Result<String> {
    Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned())
}

fn excel_pids_from_progress_log(progress_path: &Path) -> BTreeSet<u32> {
    let Ok(text) = read_progress_log(progress_path) else {
        return BTreeSet::new();
    };

    let mut pids = BTreeSet::new();
    for marker in ["excel pid=", "excel_pid="] {
        let mut rest = text.as_str();
        while let Some(index) = rest.find(marker) {
            let after_marker = &rest[index + marker.len()..];
            let digits: String = after_marker
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect();
            if let Ok(pid) = digits.parse::<u32>() {
                pids.insert(pid);
            }
            rest = &after_marker[digits.len()..];
        }
    }
    pids
}

fn terminate_process_by_pid(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .arg("/PID")
            .arg(pid.to_string())
            .arg("/F")
            .arg("/T")
            .status();
    }

    #[cfg(not(windows))]
    {
        let _ = Command::new("powershell.exe")
            .arg("-NoProfile")
            .arg("-Command")
            .arg(format!(
                "Stop-Process -Id {pid} -Force -ErrorAction SilentlyContinue"
            ))
            .status();
    }
}

fn find_power_shell() -> Option<&'static str> {
    for candidate in ["powershell.exe", "pwsh", "powershell"] {
        if Command::new(candidate)
            .arg("-NoProfile")
            .arg("-Command")
            .arg("$PSVersionTable.PSVersion.ToString()")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Some(candidate);
        }
    }
    None
}

pub(super) fn power_shell_path(path: &Path) -> anyhow::Result<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    #[cfg(windows)]
    {
        return Ok(absolute.to_string_lossy().to_string());
    }

    #[cfg(not(windows))]
    {
        let text = absolute.to_string_lossy();
        if let Some(converted) = wsl_mnt_path_to_windows_path(&text) {
            return Ok(converted);
        }

        if let Ok(output) = Command::new("wslpath").arg("-w").arg(&absolute).output() {
            if output.status.success() {
                let converted = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !converted.is_empty() {
                    return Ok(converted);
                }
            }
        }

        Ok(text.to_string())
    }
}

#[cfg(not(windows))]
fn wsl_mnt_path_to_windows_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/mnt/")?;
    let (drive, tail) = rest.split_once('/')?;
    if drive.len() != 1 || !drive.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }

    Some(format!(
        "{}:\\{}",
        drive.to_ascii_uppercase(),
        tail.replace('/', "\\")
    ))
}

const RECALCULATE_AND_SAVE_SCRIPT: &str = include_str!("excel_recalculate.ps1");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recalculation_script_blocks_for_a_full_rebuild() {
        assert!(RECALCULATE_AND_SAVE_SCRIPT.contains("$excel.CalculateFullRebuild()"));
        assert!(!RECALCULATE_AND_SAVE_SCRIPT.contains("$excel.CalculationState"));
    }

    #[test]
    fn shared_workbook_links_keep_the_visible_path() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "excel-shrink-shared-workbook-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_nanos(),
        ));
        let case_dir = root.join("regression").join("tmp").join("case");
        fs::create_dir_all(&case_dir)?;
        let workbook = case_dir.join("output.xlsx");
        fs::write(&workbook, b"cleaned workbook bytes")?;
        let hash = file_content_hash(&workbook)?;
        let linked = replace_workbook_with_shared_cache_link(&workbook, &case_dir)?;
        assert_eq!(linked, workbook);
        assert_eq!(fs::read(&workbook)?, b"cleaned workbook bytes");
        assert!(
            root.join(".tmp")
                .join("workbooks")
                .join("shared")
                .join(hash)
                .join("workbook.xlsx")
                .is_file()
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
