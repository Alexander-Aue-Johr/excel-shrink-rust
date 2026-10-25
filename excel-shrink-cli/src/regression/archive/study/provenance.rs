//! Immutable run manifests and explicit attribution of separately measured phases.
use super::*;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const VERSION: u32 = 1;

#[derive(Clone, Copy)]
pub(super) enum Stage {
    Preparation,
    Comparison,
    WhitespaceDiagnosis,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Preparation => "preparation",
            Self::Comparison => "comparison",
            Self::WhitespaceDiagnosis => "whitespace_diagnosis",
        }
    }
}

/// Call once per invocation. Options must be explicitly selected public settings,
/// never raw argv, request contents, access URLs, cookies or credentials.
pub(super) fn start_run(
    output: &Path,
    label: Option<&str>,
    selection: Value,
    safe_options: Value,
) -> Result<Value> {
    let machine = collect_machine(label);
    let snapshot_hash = artifact_cache::sha256(&serde_json::to_vec(&machine)?);
    let machine_path = format!(
        "metadata/machines/{}-{snapshot_hash}.json",
        machine["machine_id"].as_str().unwrap()
    );
    immutable_json(&output.join(&machine_path), &machine)?;
    let executable = std::env::current_exe()?;
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root())
        .output()
        .ok()
        .filter(|r| r.status.success())
        .map(|r| String::from_utf8_lossy(&r.stdout).trim().to_owned());
    let checkout_dirty = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .current_dir(repo_root()).output().ok().filter(|r| r.status.success())
        .map(|r| !r.stdout.is_empty());
    let run = json!({
        "schema_version":VERSION,"run_id":new_uuid()?,"started_unix_ms":now_ms(),
        "checkout_git_commit":commit,"checkout_dirty":checkout_dirty,
        "executable_sha256":artifact_cache::hash_file(&executable)?,
        "executable_filename":executable.file_name().map(|n| n.to_string_lossy()),
        "machine_id":machine["machine_id"],"machine_manifest":machine_path,
        "selection":selection,"options":safe_options
    });
    immutable_json(
        &output.join(format!(
            "metadata/runs/{}.json",
            run["run_id"].as_str().unwrap()
        )),
        &run,
    )?;
    Ok(run)
}

/// Stamp only a phase actually executed by this run. Loading an old record must
/// never assign the current machine to its historical measurements.
pub(super) fn stamp(record: &mut Value, run: &Value, stage: Stage) {
    if !record["provenance"].is_object() {
        record["provenance"] = json!({"schema_version":VERSION});
    }
    let reference = json!({"run_id":run["run_id"],"machine_id":run["machine_id"],
        "machine_manifest":run["machine_manifest"],"measured_unix_ms":now_ms()});
    let old = record["provenance"][stage.name()].clone();
    if old.is_object() && old["run_id"] != reference["run_id"] {
        retire(record, stage);
    }
    record["provenance"][stage.name()] = reference.clone();
    if matches!(stage, Stage::Comparison) {
        for profile in record["results"].as_array_mut().into_iter().flatten() {
            // One reference covers this paired experiment, including the shared
            // original measurement. Never construct ratios across machines/runs.
            if profile["original_open_seconds"].is_number()
                && profile["shrunk_open_seconds"].is_number()
            {
                profile["open_measurement_provenance"] =
                    json!({"original":reference,"shrunk":reference});
            }
        }
    }
    if matches!(stage, Stage::WhitespaceDiagnosis) && record["whitespace_diagnosis"].is_object() {
        record["whitespace_diagnosis"]["provenance"] = reference;
    }
}

/// When the associated results are discarded, retain their historical origin
/// without attributing the replacement attempt to that previous machine.
pub(super) fn retire(record: &mut Value, stage: Stage) {
    let Some(provenance) = record.get_mut("provenance").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(reference) = provenance.remove(stage.name()).filter(Value::is_object) else {
        return;
    };
    if !provenance.get("history").is_some_and(Value::is_array) {
        provenance.insert("history".into(), json!([]));
    }
    provenance["history"].as_array_mut().unwrap().push(json!({
        "stage":stage.name(),"reference":reference,"retired_unix_ms":now_ms()
    }));
}

/// A ratio is eligible only with the same explicit paired-experiment reference.
pub(super) fn paired_speedup(profile: &Value) -> Option<f64> {
    let original = &profile["open_measurement_provenance"]["original"];
    let shrunk = &profile["open_measurement_provenance"]["shrunk"];
    if !original.is_object()
        || original != shrunk
        || original["run_id"].as_str().is_none()
        || original["machine_id"].as_str().is_none()
    {
        return None;
    }
    let before = profile["original_open_seconds"].as_f64()?;
    let after = profile["shrunk_open_seconds"].as_f64()?;
    (before.is_finite() && after.is_finite() && before > 0.0 && after > 0.0)
        .then_some(before / after)
}

fn native_ready_speedup(profile: &Value) -> Option<f64> {
    let native = &profile["native_startup"];
    if native["method"] != "launch-to-addin-ready-v1" || native["shared_client"] != false
        || native["provenance"]["run_id"].as_str().is_none()
        || native["provenance"]["machine_id"].as_str().is_none()
        || native["attempt_id"].as_str().is_none()
    { return None; }
    let before = native["original"]["seconds"].as_f64()?;
    let after = native["shrunk"]["seconds"].as_f64()?;
    (before.is_finite() && after.is_finite() && before > 0.0 && after > 0.0).then_some(before / after)
}

/// Include this object as `report["measurement_provenance"]` before HTML render.
pub(super) fn report_metadata(output: &Path, records: &[Value]) -> Result<Value> {
    let mut runs = BTreeMap::new();
    let mut machines = BTreeMap::new();
    let mut groups: BTreeMap<String, Value> = BTreeMap::new();
    for record in records {
        let comparison = &record["provenance"]["comparison"];
        let machine = comparison["machine_id"].as_str().unwrap_or("unknown");
        let group = groups.entry(machine.to_owned()).or_insert_with(|| {
            json!({
                "machine_id":machine,"workbooks":0,"preparations":0,"whitespace_diagnoses":0,
                "paired_speedups":[],"native_ready_speedups":[],"run_ids":[]
            })
        });
        group["workbooks"] = json!(group["workbooks"].as_u64().unwrap() + 1);
        for profile in record["results"].as_array().into_iter().flatten() {
            let native = &profile["native_startup"];
            if native["provenance"]["machine_id"] == machine {
                if let Some(ratio) = native_ready_speedup(profile) {
                    group["native_ready_speedups"].as_array_mut().unwrap().push(json!({
                        "identity":record["identity"],"profile":profile["profile"],"speedup":ratio,
                        "method":native["method"],"original_seconds":native["original"]["seconds"],
                        "shrunk_seconds":native["shrunk"]["seconds"],"run_id":native["provenance"]["run_id"],
                        "attempt_id":native["attempt_id"]
                    }));
                }
            }
            if let Some(ratio) = paired_speedup(profile) {
                if profile["open_measurement_provenance"]["original"]["machine_id"] == machine {
                    group["paired_speedups"].as_array_mut().unwrap().push(json!({
                        "identity":record["identity"],"profile":profile["profile"],"speedup":ratio,
                        "original_seconds":profile["original_open_seconds"],"shrunk_seconds":profile["shrunk_open_seconds"],
                        "run_id":profile["open_measurement_provenance"]["original"]["run_id"]
                    }));
                }
            }
        }
        let mut references = Vec::new();
        collect_references(&record["provenance"], &mut references);
        for reference in references {
            let Some(id) = reference["run_id"].as_str() else {
                continue;
            };
            ensure!(valid_uuid(id), "invalid provenance run ID");
            if !runs.contains_key(id) {
                let path = output.join(format!("metadata/runs/{id}.json"));
                if !path.is_file() {
                    continue;
                } // Legacy imports remain visibly unknown.
                let run: Value = serde_json::from_slice(&fs::read(path)?)?;
                ensure!(run["run_id"] == id, "run manifest ID mismatch");
                let manifest = run["machine_manifest"]
                    .as_str()
                    .context("missing machine manifest")?;
                ensure!(
                    manifest.starts_with("metadata/machines/")
                        && !manifest.contains("..")
                        && !manifest.contains('\\')
                        && !manifest.contains(':'),
                    "invalid machine manifest path"
                );
                let snapshot: Value = serde_json::from_slice(&fs::read(output.join(manifest))?)?;
                ensure!(
                    snapshot["machine_id"] == run["machine_id"],
                    "machine manifest ID mismatch"
                );
                machines.insert(manifest.to_owned(), snapshot);
                runs.insert(id.to_owned(), run);
            }
        }
    }
    // Counts for preparation/diagnosis have their own machine attribution.
    for record in records {
        for (phase, counter) in [
            ("preparation", "preparations"),
            ("whitespace_diagnosis", "whitespace_diagnoses"),
        ] {
            if let Some(id) = record["provenance"][phase]["machine_id"].as_str() {
                let group = groups.entry(id.to_owned()).or_insert_with(|| {
                    json!({
                        "machine_id":id,"workbooks":0,"preparations":0,"whitespace_diagnoses":0,
                        "paired_speedups":[],"native_ready_speedups":[],"run_ids":[]
                    })
                });
                group[counter] = json!(group[counter].as_u64().unwrap() + 1);
            }
        }
    }
    for group in groups.values_mut() {
        group["run_ids"] = json!(runs
            .values()
            .filter(|r| r["machine_id"] == group["machine_id"])
            .map(|r| r["run_id"].clone())
            .collect::<Vec<_>>());
    }
    Ok(
        json!({"schema_version":VERSION,"runs":runs.into_values().collect::<Vec<_>>(),
        "machines":machines.into_iter().map(|(path,machine)|json!({"path":path,"metadata":machine})).collect::<Vec<_>>(),
        "groups":groups.into_values().collect::<Vec<_>>() }),
    )
}

pub(super) fn collect_references<'a>(value: &'a Value, references: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            if map.contains_key("run_id") && map.contains_key("machine_id") {
                references.push(value);
            }
            for nested in map.values() {
                collect_references(nested, references);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_references(item, references);
            }
        }
        _ => {}
    }
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

pub(super) fn new_uuid() -> Result<String> {
    let mut bytes = [0u8; 16];
    #[cfg(windows)]
    {
        #[link(name = "bcrypt")]
        unsafe extern "system" {
            fn BCryptGenRandom(
                provider: *mut std::ffi::c_void,
                buffer: *mut u8,
                length: u32,
                flags: u32,
            ) -> i32;
        }
        ensure!(
            unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 16, 2) } == 0,
            "generate run UUID"
        );
    }
    #[cfg(not(windows))]
    {
        use std::io::Read;
        fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    }
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let hex = artifact_cache::hex(&bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

fn immutable_json(path: &Path, value: &Value) -> Result<()> {
    if path.exists() {
        ensure!(
            serde_json::from_slice::<Value>(&fs::read(path)?)? == *value,
            "conflicting immutable metadata: {}",
            path.display()
        );
    } else {
        artifact_cache::atomic_write(path, &serde_json::to_vec_pretty(value)?)?;
    }
    Ok(())
}

fn collect_machine(label: Option<&str>) -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_keep_distinct_machines_and_legacy_metrics_stay_unknown() {
        let mut record =
            json!({"results":[{"original_open_seconds":2.0,"shrunk_open_seconds":1.0}]});
        assert_eq!(paired_speedup(&record["results"][0]), None);
        let first = json!({"run_id":"prepare","machine_id":"laptop","machine_manifest":"a"});
        let second = json!({"run_id":"compare","machine_id":"tower","machine_manifest":"b"});
        stamp(&mut record, &first, Stage::Preparation);
        stamp(&mut record, &second, Stage::Comparison);
        assert_eq!(record["provenance"]["preparation"]["machine_id"], "laptop");
        assert_eq!(record["provenance"]["comparison"]["machine_id"], "tower");
        assert_eq!(paired_speedup(&record["results"][0]), Some(2.0));
        record["results"][0]["open_measurement_provenance"]["shrunk"]["run_id"] =
            json!("different run");
        assert_eq!(paired_speedup(&record["results"][0]), None);
    }

    #[test]
    fn reruns_keep_the_previous_phase_reference() {
        let mut record = json!({});
        stamp(
            &mut record,
            &json!({"run_id":"old","machine_id":"a"}),
            Stage::WhitespaceDiagnosis,
        );
        stamp(
            &mut record,
            &json!({"run_id":"new","machine_id":"b"}),
            Stage::WhitespaceDiagnosis,
        );
        assert_eq!(
            record["provenance"]["history"][0]["reference"]["run_id"],
            "old"
        );
        assert_eq!(record["whitespace_diagnosis"]["provenance"], Value::Null);
    }

    #[test]
    fn run_ids_are_random_valid_v4_uuids() -> Result<()> {
        let first = new_uuid()?;
        let second = new_uuid()?;
        assert!(valid_uuid(&first));
        assert_ne!(first, second);
        assert_eq!(&first[14..15], "4");
        Ok(())
    }

    #[test]
    fn report_keeps_unknown_legacy_and_separate_preparation_machine() -> Result<()> {
        let root = super::super::tests::test_directory("machine-summary")?;
        let mut record = json!({"identity":"new","results":[{"profile":"default","original_open_seconds":4.0,"shrunk_open_seconds":2.0}]});
        stamp(
            &mut record,
            &json!({"run_id":new_uuid()?,"machine_id":"laptop"}),
            Stage::Preparation,
        );
        stamp(
            &mut record,
            &json!({"run_id":new_uuid()?,"machine_id":"tower"}),
            Stage::Comparison,
        );
        let report = report_metadata(&root, &[record, json!({"identity":"legacy","results":[]})])?;
        let groups = report["groups"].as_array().unwrap();
        assert_eq!(
            groups
                .iter()
                .find(|g| g["machine_id"] == "unknown")
                .unwrap()["workbooks"],
            1
        );
        assert_eq!(
            groups.iter().find(|g| g["machine_id"] == "laptop").unwrap()["workbooks"],
            0
        );
        assert_eq!(
            groups.iter().find(|g| g["machine_id"] == "laptop").unwrap()["preparations"],
            1
        );
        assert_eq!(
            groups.iter().find(|g| g["machine_id"] == "tower").unwrap()["paired_speedups"][0]
                ["speedup"],
            2.0
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
