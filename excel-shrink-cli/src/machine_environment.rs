//! Hardware and installed Excel metadata collected without launching Excel.
use crate::regression::artifact_cache;
use serde_json::{json, Value};
#[cfg(windows)]
use anyhow::{ensure, Result};
#[cfg(not(windows))]
use std::fs;
#[cfg(windows)]
use std::time::{Duration, Instant};

const VERSION: u32 = 1;

pub(crate) fn collect(label: Option<&str>) -> Value {
    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into());
    let mut machine = platform_machine();
    let fingerprint = artifact_cache::sha256(
        &serde_json::to_vec(&json!({
            "cpu":machine["cpu"],"hardware":machine["hardware_identity"]
        }))
        .unwrap(),
    );
    machine.as_object_mut().unwrap().remove("hardware_identity");
    machine["schema_version"] = json!(VERSION);
    machine["machine_id"] = json!(artifact_cache::sha256(
        format!("{}:{fingerprint}", hostname.to_lowercase()).as_bytes()
    ));
    machine["hostname"] = json!(hostname);
    machine["label"] = json!(label.unwrap_or(&hostname));
    machine["hardware_fingerprint"] = json!(fingerprint);
    machine["architecture"] = json!(std::env::consts::ARCH);
    machine
}

#[cfg(windows)]
fn platform_machine() -> Value {
    // CIM and registry only: do not start Excel or change its registration.
    // Raw hardware UUID is used only inside the hash and never persisted.
    const SCRIPT: &str = r#"
$ErrorActionPreference='Stop'
$OutputEncoding=[Text.UTF8Encoding]::new($false); [Console]::OutputEncoding=$OutputEncoding
$r=[ordered]@{}; $warnings=@()
try { $cpu=@(Get-CimInstance Win32_Processor); $r.cpu=@($cpu | ForEach-Object { [ordered]@{model=$_.Name; physical_cores=$_.NumberOfCores; logical_cores=$_.NumberOfLogicalProcessors} }) } catch { $warnings+='CPU metadata unavailable' }
try { $s=Get-CimInstance Win32_ComputerSystem; $r.physical_memory_bytes=[uint64]$s.TotalPhysicalMemory; $r.manufacturer=$s.Manufacturer; $r.model=$s.Model } catch { $warnings+='System metadata unavailable' }
try { $r.hardware_identity=(Get-CimInstance Win32_ComputerSystemProduct).UUID } catch { $warnings+='Hardware identity unavailable' }
try { $o=Get-CimInstance Win32_OperatingSystem; $r.os=[ordered]@{name=$o.Caption; version=$o.Version; build=$o.BuildNumber; architecture=$o.OSArchitecture} } catch { $warnings+='OS metadata unavailable' }
try { $e=Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Office\ClickToRun\Configuration'; $r.excel=[ordered]@{installation='ClickToRun'; version=$e.VersionToReport; bitness=$e.Platform; products=$e.ProductReleaseIds} } catch { $r.excel=[ordered]@{version=$null; bitness=$null}; $warnings+='Excel ClickToRun metadata unavailable; Excel was not started' }
$r.collection_warnings=$warnings
$r | ConvertTo-Json -Depth 8 -Compress
"#;
    use std::os::windows::process::CommandExt;
    let collected = (|| -> Result<Value> {
        let mut child = std::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                SCRIPT,
            ])
            .creation_flags(0x08000000)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("hardware metadata collection timed out");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let result = child.wait_with_output()?;
        ensure!(
            result.status.success(),
            "hardware metadata collection failed"
        );
        Ok(serde_json::from_slice(&result.stdout)?)
    })();
    collected.unwrap_or_else(|e| {
        json!({"cpu":null,"physical_memory_bytes":null,
        "os":{"name":"windows"},"excel":{"version":null,"bitness":null},
        "collection_warnings":[e.to_string()]})
    })
}

#[cfg(not(windows))]
fn platform_machine() -> Value {
    let cpu = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let model = cpu.lines().find_map(|line| {
        line.strip_prefix("model name")
            .and_then(|s| s.split_once(':'))
            .map(|(_, v)| v.trim())
    });
    let ram = fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
        s.lines()
            .find(|l| l.starts_with("MemTotal:"))
            .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            .map(|n| n * 1024)
    });
    json!({"cpu":[{"model":model,"logical_cores":std::thread::available_parallelism().ok().map(|v|v.get())}],
        "physical_memory_bytes":ram,"os":{"name":std::env::consts::OS},
        "excel":{"version":null,"bitness":null},"collection_warnings":[]})
}

