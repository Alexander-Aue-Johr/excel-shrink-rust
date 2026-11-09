//! Shared PowerShell discovery, argument handling and Windows path normalization.
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub fn power_shell_script_command(
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
    let script_arg = if cfg!(windows) || program.ends_with(".exe") {
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

pub fn find_power_shell() -> Option<&'static str> {
    #[cfg(windows)]
    {
        // Native Excel automation uses inbox PowerShell. Probing its version in
        // another process for every workbook adds an unnecessary CLR startup.
        static SHELL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        return SHELL
            .get_or_init(|| {
                let root = std::env::var_os("SystemRoot")?;
                let path =
                    PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
                path.is_file().then(|| path.to_string_lossy().into_owned())
            })
            .as_deref();
    }
    #[cfg(not(windows))]
    {
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
}

pub fn power_shell_path(path: &Path) -> anyhow::Result<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    #[cfg(windows)]
    {
        // canonicalize() produces verbatim paths on Windows. PowerShell 5's
        // filesystem provider cannot resolve those (including $PSScriptRoot).
        let text = absolute.to_string_lossy();
        if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            return Ok(format!(r"\\{unc}"));
        }
        if let Some(disk) = text.strip_prefix(r"\\?\") {
            if disk.as_bytes().get(1) == Some(&b':') {
                return Ok(disk.to_owned());
            }
        }
        return Ok(text.into_owned());
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
pub fn wsl_mnt_path_to_windows_path(path: &str) -> Option<String> {
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

pub(crate) fn seconds_from_env(name: &str, fallback: f64) -> Duration {
    let seconds = std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.1 && *value <= 86400.0)
        .unwrap_or(fallback);
    Duration::from_secs_f64(seconds)
}
