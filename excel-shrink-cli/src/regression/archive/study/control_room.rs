//! Browser controls and durable snapshots; comparison correctness never depends on a tab.
use super::*;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};

#[derive(Debug)]
pub(super) struct StopRequested;
impl std::fmt::Display for StopRequested {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("run stopped after finishing admitted workbooks")
    }
}
impl std::error::Error for StopRequested {}

// Scope requests to the exact process, including across the startup/run-ID change.
pub(super) fn stop_requested(root: &Path) -> bool {
    read_json(&root.join("stop-request.json")).is_some_and(|request| {
        request["runner_pid"] == std::process::id()
            && process_started_unix_ms().is_some_and(|started|
                request["runner_started_unix_ms"].as_u64() == Some(started))
    })
}

pub(super) fn check_stop(root: &Path) -> Result<()> {
    if stop_requested(root) {
        return Err(StopRequested.into());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Controls {
    pub target_groups: usize,
    pub paused: bool,
    #[serde(default = "default_system_reserve_mib")]
    pub system_reserve_mib: u64,
    pub open_timeout_seconds: u64,
    pub job_timeout_seconds: u64,
    pub equal_preview_width: u32,
    #[serde(default = "default_capture_megapixels")]
    pub capture_tile_megapixels: u32,
    #[serde(default = "default_capture_batch")]
    pub capture_batch_tiles: u32,
    #[serde(default = "default_tall_sheet_threshold")]
    pub tall_sheet_threshold: u32,
    #[serde(default = "default_tall_sheet_edges")]
    pub tall_sheet_edge_tiles: u32,
    #[serde(default)]
    pub hide_excel: bool,
}

pub(super) const DEFAULT_SYSTEM_RESERVE_MIB: u64 = 1024;

fn default_system_reserve_mib() -> u64 {
    DEFAULT_SYSTEM_RESERVE_MIB
}

fn validate_system_reserve_mib(value: u64) -> Result<()> {
    ensure!(
        (1..=65536).contains(&value),
        "system reserve must be between 1 and 65536 MiB"
    );
    Ok(())
}

fn initial_system_reserve_mib(
    previous: Option<&Value>,
    environment: std::result::Result<String, std::env::VarError>,
) -> Result<u64> {
    let value = if let Some(value) = previous.and_then(|value| value.get("system_reserve_mib")) {
        // Keep a previously chosen live value, including the older 4096 MiB setting.
        value
            .as_u64()
            .context("system reserve must be an integer")?
    } else {
        match environment {
            Ok(text) => text
                .parse::<u64>()
                .context("invalid EXCEL_SHRINK_SYSTEM_RESERVE_MIB")?,
            Err(std::env::VarError::NotPresent) => DEFAULT_SYSTEM_RESERVE_MIB,
            Err(error) => return Err(error).context("invalid EXCEL_SHRINK_SYSTEM_RESERVE_MIB"),
        }
    };
    validate_system_reserve_mib(value)?;
    Ok(value)
}

pub(super) fn configured_system_reserve_mib() -> Result<u64> {
    initial_system_reserve_mib(None, std::env::var("EXCEL_SHRINK_SYSTEM_RESERVE_MIB"))
}

fn default_capture_megapixels() -> u32 {
    32
}
fn default_capture_batch() -> u32 {
    1
}
fn default_tall_sheet_threshold() -> u32 {
    40
}
fn default_tall_sheet_edges() -> u32 {
    2
}

fn tall_sheet_setting(name: &str, fallback: u32) -> Result<u32> {
    let value = match std::env::var(name) {
        Ok(text) => text
            .parse::<u32>()
            .with_context(|| format!("invalid {name}"))?,
        Err(std::env::VarError::NotPresent) => fallback,
        Err(error) => return Err(error).with_context(|| format!("invalid {name}")),
    };
    ensure!(
        (1..=i32::MAX as u32).contains(&value),
        "{name} must be a positive 32-bit integer"
    );
    Ok(value)
}

impl Controls {
    pub fn read(root: &Path) -> Result<Self> {
        let value: Self = serde_json::from_slice(&fs::read(root.join("control.json"))?)?;
        validate_system_reserve_mib(value.system_reserve_mib)?;
        ensure!(
            value.open_timeout_seconds > 0 && value.job_timeout_seconds > 0,
            "control room limits must be positive"
        );
        ensure!(
            [0, 128, 256, 512, 1024].contains(&value.equal_preview_width),
            "invalid equal tile preview width"
        );
        ensure!(
            (1..=32).contains(&value.capture_tile_megapixels),
            "capture size must be 1..32 MP"
        );
        ensure!(
            (1..=16).contains(&value.capture_batch_tiles),
            "capture batch must be 1..16 tiles"
        );
        ensure!(
            (1..=i32::MAX as u32).contains(&value.tall_sheet_threshold)
                && (1..=i32::MAX as u32).contains(&value.tall_sheet_edge_tiles),
            "tall sheet settings must be positive 32-bit integers"
        );
        Ok(value)
    }
}

pub(super) struct Room {
    root: PathBuf,
    finished: bool,
    // A completed run leaves the read-only history available in the browser.
    // Dropping Child releases our handle, not the independent loopback server.
    _server: Option<Child>,
}

pub(super) fn archive_record(root: &Path, id: &str, record: &Value) -> Result<()> {
    let execution_run = record["last_execution"]["run_id"].as_str();
    let control_run = record["control_room_attempt"]["run_id"].as_str();
    let attempt = match execution_run {
        // A later CLI run may still carry an older control-room attempt field.
        Some(run) if Some(run) != control_run => run,
        _ => record["control_room_attempt"]["id"]
            .as_str()
            .or(execution_run)
            .unwrap_or("legacy"),
    };
    let directory = root.join("groups").join(group_id(id)).join("history");
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!(
        "record-{}.json",
        &artifact_cache::sha256(attempt.as_bytes())[..20]
    ));
    // Archive before queueing or preparation changes the live record. Repeated
    // admission checkpoints must not replace the original failed attempt.
    if !path.is_file() {
        write(&path, record)?;
    }
    Ok(())
}

// Serial preparation, classic retries and supplemental diagnosis are work too. Publish their
// actual coordinator, with a fresh attempt identity so stale Excel clients are
// never mistaken for workers belonging to this invocation.
pub(super) fn begin_serial(
    output: &Path,
    source: &Source,
    id: &str,
    record: &mut Value,
    mode: JobMode,
    run: &Value,
) -> Result<()> {
    let root = output.join("control-room");
    archive_record(&root, id, record)?;
    let attempt = provenance::new_uuid()?;
    record["control_room_attempt"] = json!({"id":attempt,"run_id":run["run_id"],
        "started_unix_ms":provenance::now_ms(),"settings":Controls::read(&root)?});
    write_record(&record_path(output, id), record)?;
    let mut queue = read_json(&root.join("queue.json")).context("missing control room queue")?;
    let groups = queue["groups"]
        .as_array_mut()
        .context("missing control room groups")?;
    let status = match mode {
        JobMode::Prepare => "preparing",
        JobMode::Compare => "retrying_classic",
        JobMode::Diagnose => "diagnosing",
        JobMode::Legacy => "comparing",
    };
    let mut group = json!({"id":group_id(id),"workbook":record["name"],"corpus":source.name,
        "attempt_id":attempt,"run_id":run["run_id"],"active":true,"restart_eligible":false,
        "status":status,"sequence":groups.len()+1,
        "started_unix_ms":record["control_room_attempt"]["started_unix_ms"]});
    if mode == JobMode::Compare {
        group["capture_mode"] = json!("classic");
    }
    if let Some(old) = groups.iter_mut().find(|g| g["id"] == group["id"]) {
        group["sequence"] = old["sequence"].clone();
        *old = group;
    } else {
        groups.push(group);
    }
    queue["phase"] = json!(if mode == JobMode::Prepare {
        "preparing"
    } else {
        "comparing"
    });
    queue["open_groups"] = json!(1);
    queue["working_groups"] = json!(1);
    queue["cleanup_groups"] = json!(0);
    queue["updated_unix_ms"] = json!(provenance::now_ms());
    write(&root.join("queue.json"), &queue)
}

pub(super) fn finish_serial(output: &Path, id: &str, record: &Value) -> Result<()> {
    let path = output.join("control-room/queue.json");
    let mut queue = read_json(&path).context("missing control room queue")?;
    if let Some(group) = queue["groups"]
        .as_array_mut()
        .and_then(|groups| groups.iter_mut().find(|g| g["id"] == group_id(id)))
    {
        group["active"] = json!(false);
        group["status"] = record["status"].clone();
    }
    queue["open_groups"] = json!(0);
    queue["working_groups"] = json!(0);
    queue["cleanup_groups"] = json!(0);
    queue["updated_unix_ms"] = json!(provenance::now_ms());
    write(&path, &queue)
}

impl Room {
    pub fn start(output: &Path, args: &StudyArgs, run: &Value) -> Result<Self> {
        let root = output.join("control-room");
        fs::create_dir_all(root.join("groups"))?;
        let previous_controls = Controls::read(&root).ok();
        let controls = Controls {
            target_groups: args.comparison_workers,
            paused: false,
            system_reserve_mib: initial_system_reserve_mib(
                read_json(&root.join("control.json")).as_ref(),
                std::env::var("EXCEL_SHRINK_SYSTEM_RESERVE_MIB"),
            )?,
            open_timeout_seconds: args.startup_timeout_seconds,
            job_timeout_seconds: args.workbook_timeout_seconds,
            equal_preview_width: previous_controls
                .as_ref()
                .map(|old| old.equal_preview_width)
                .unwrap_or(512),
            capture_tile_megapixels: previous_controls
                .as_ref()
                .map(|old| old.capture_tile_megapixels)
                .unwrap_or(32),
            capture_batch_tiles: previous_controls
                .as_ref()
                .map(|old| old.capture_batch_tiles)
                .unwrap_or(1),
            // Explicit launcher settings override the previous run once;
            // subsequent group snapshots use the Observatory's control file.
            tall_sheet_threshold: tall_sheet_setting(
                "EXCEL_SHRINK_TALL_SHEET_THRESHOLD",
                previous_controls
                    .as_ref()
                    .map(|old| old.tall_sheet_threshold)
                    .unwrap_or_else(default_tall_sheet_threshold),
            )?,
            tall_sheet_edge_tiles: tall_sheet_setting(
                "EXCEL_SHRINK_TALL_SHEET_EDGE_TILES",
                previous_controls
                    .as_ref()
                    .map(|old| old.tall_sheet_edge_tiles)
                    .unwrap_or_else(default_tall_sheet_edges),
            )?,
            // The launcher supplies the saved visibility choice explicitly.
            // A previous hidden run must not override today's unchecked box.
            hide_excel: args.hide_excel,
        };
        let source = repo_root().join("excel-shrink-cli/office-capture/resident-host");
        let version = host_version(&source)?;
        let existing_health = health(args.control_room_port);
        let server = if let Some(health) = existing_health {
            ensure!(
                health["kind"] == "excel-shrink-control-room" && same_path(&health["root"], &root),
                "port {} belongs to another service or study",
                args.control_room_port
            );
            ensure!(
                health["host_version"] == version,
                "control room on port {} uses a different host build; stop that control room or choose another --control-room-port (the existing server was left running)",
                args.control_room_port
            );
            None
        } else {
            ensure!(
                std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, args.control_room_port)).is_ok(),
                "control room port {} is already in use",
                args.control_room_port
            );
            // A previous build can remain open for historical inspection. Its
            // locked DLL must not prevent starting a new version on another port.
            let host = root.join("host").join(&version[..20]);
            fs::create_dir_all(&host)?;
            let mut build = Command::new("dotnet");
            build
                .args([
                    "build",
                    "--configuration",
                    "Release",
                    "--no-restore",
                    "--output",
                ])
                .arg(&host)
                .arg(source.join("ResidentCaptureHost.csproj"))
                .stdout(Stdio::from(fs::File::create(root.join("host-build.log"))?))
                .stderr(Stdio::from(fs::File::create(
                    root.join("host-build.stderr.log"),
                )?));
            hidden(&mut build);
            ensure!(
                build.status()?.success(),
                "could not build control room host; see {}",
                root.join("host-build.log").display()
            );
            let settings = root.join("host-settings.json");
            write(
                &settings,
                &json!({"root":root,"web_root":repo_root().join("excel-shrink-cli/office-capture/control-room"),
                "port":args.control_room_port,"host_version":version,"initial_controls":controls}),
            )?;
            let mut command = Command::new("dotnet");
            command
                .arg(host.join("ResidentCaptureHost.dll"))
                .arg("--control-room")
                .arg(&settings)
                .stdin(Stdio::null())
                .stdout(Stdio::from(fs::File::create(root.join("host.stdout.log"))?))
                .stderr(Stdio::from(fs::File::create(root.join("host.stderr.log"))?));
            hidden(&mut command);
            let mut child = command.spawn().context("start control room server")?;
            let started = Instant::now();
            loop {
                if let Some(value) = health(args.control_room_port) {
                    if value["kind"] == "excel-shrink-control-room"
                        && same_path(&value["root"], &root)
                        && value["host_version"] == version
                    {
                        break;
                    }
                }
                if child.try_wait()?.is_some() || started.elapsed() > Duration::from_secs(20) {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!(
                        "control room did not start; see {}",
                        root.join("host.stderr.log").display()
                    );
                }
                std::thread::sleep(Duration::from_millis(150));
            }
            Some(child)
        };
        // Publish ownership only after the matching host is healthy. If either
        // snapshot write fails, Drop records stopped instead of a phantom run.
        let room = Self {
            root,
            finished: false,
            _server: server,
        };
        write(
            &room.root.join("control.json"),
            &serde_json::to_value(controls)?,
        )?;
        let mut queue =
            read_json(&room.root.join("queue.json")).unwrap_or_else(|| json!({"groups":[]}));
        queue.as_object_mut().context("invalid control room queue")?.remove("startup");
        queue["run_id"] = run["run_id"].clone();
        queue["machine_id"] = run["machine_id"].clone();
        queue["machine"] = json!(args
            .machine_label
            .clone()
            .unwrap_or_else(|| "This machine".into()));
        queue["runner_pid"] = json!(std::process::id());
        queue["stop_supported"] = json!(true);
        queue["runner_started_unix_ms"] = json!(process_started_unix_ms());
        queue["running"] = json!(true);
        queue["phase"] = json!("preparing");
        queue["open_groups"] = json!(0);
        queue["working_groups"] = json!(0);
        queue["cleanup_groups"] = json!(0);
        queue["waiting_groups"] = json!(0);
        queue["updated_unix_ms"] = json!(provenance::now_ms());
        // Historical entries remain browsable, but a new invocation owns no old processes.
        if let Some(groups) = queue["groups"].as_array_mut() {
            for group in groups {
                if group["active"] == true {
                    group["active"] = json!(false);
                    group["status"] = json!("interrupted");
                }
            }
        }
        write(&room.root.join("queue.json"), &queue)?;
        eprintln!(
            "[study] Live control room: http://127.0.0.1:{}/",
            args.control_room_port
        );
        Ok(room)
    }

    pub fn set_run(&self, run: &Value, phase: &str) -> Result<()> {
        let path = self.root.join("queue.json");
        let mut queue = read_json(&path).context("missing control room queue")?;
        queue["run_id"] = run["run_id"].clone();
        queue["machine_id"] = run["machine_id"].clone();
        queue["phase"] = json!(phase);
        queue["updated_unix_ms"] = json!(provenance::now_ms());
        write(&path, &queue)
    }

    pub fn finish(&mut self) -> Result<()> {
        self.finish_with("finished")?;
        self.finished = true;
        Ok(())
    }

    fn finish_with(&self, phase: &str) -> Result<()> {
        let mut queue =
            read_json(&self.root.join("queue.json")).unwrap_or_else(|| json!({"groups":[]}));
        queue["running"] = json!(false);
        queue["phase"] = json!(phase);
        queue["open_groups"] = json!(0);
        queue["working_groups"] = json!(0);
        queue["cleanup_groups"] = json!(0);
        if let Some(groups) = queue["groups"].as_array_mut() {
            for group in groups.iter_mut().filter(|group| group["active"] == true) {
                group["active"] = json!(false);
                group["status"] = json!("interrupted");
            }
        }
        queue["updated_unix_ms"] = json!(provenance::now_ms());
        write(&self.root.join("queue.json"), &queue)?;
        reject_unfinished_commands(&self.root, &queue["run_id"])
    }
}

fn reject_unfinished_commands(root: &Path, run_id: &Value) -> Result<()> {
    let directory = root.join("commands");
    if !directory.is_dir() {
        return Ok(());
    }
    // A click can race the last comparison's completion. It must not remain
    // forever "queued" after its coordinator has stopped accepting work.
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let Some(mut command) = read_json(&path) else {
            continue;
        };
        if &command["run_id"] != run_id
            || !matches!(
                command["status"].as_str(),
                Some("pending" | "queued" | "restarting")
            )
        {
            continue;
        }
        command["status"] = json!("rejected");
        command["message"] = json!(
            "The owning study run ended before this request completed. Resume the study before requesting another restart."
        );
        command["updated_unix_ms"] = json!(provenance::now_ms());
        write(&path, &command)?;
    }
    Ok(())
}

impl Drop for Room {
    fn drop(&mut self) {
        if !self.finished {
            if let Some(output) = self.root.parent() {
                progress::stop(output);
            }
            let _ = self.finish_with("stopped");
        }
    }
}

fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

fn same_path(value: &Value, expected: &Path) -> bool {
    value.as_str().and_then(|path| fs::canonicalize(path).ok()) == fs::canonicalize(expected).ok()
}

#[cfg(windows)]
fn process_started_unix_ms() -> Option<u64> {
    work_state::process_started_unix_ms()
}

#[cfg(not(windows))]
fn process_started_unix_ms() -> Option<u64> {
    None
}

fn host_version(source: &Path) -> Result<String> {
    let mut files = fs::read_dir(source)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|path| {
        path.is_file()
            && matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("cs" | "csproj")
            )
    });
    files.sort();
    ensure!(!files.is_empty(), "control room host sources are missing");
    let mut bytes = Vec::new();
    for file in files {
        let name = file
            .file_name()
            .context("host source has no filename")?
            .to_string_lossy();
        let contents = fs::read(&file)?;
        bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&(contents.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&contents);
    }
    Ok(artifact_cache::sha256(&bytes))
}

fn health(port: u16) -> Option<Value> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(
        stream,
        "GET /health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.take(64 * 1024).read_to_string(&mut response).ok()?;
    let (headers, body) = response.split_once("\r\n\r\n")?;
    if !headers.lines().next()?.contains(" 200 ") {
        return None;
    }
    serde_json::from_str(body).ok()
}

pub(super) fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

pub(super) fn write(path: &Path, value: &Value) -> Result<()> {
    artifact_cache::atomic_write(path, &serde_json::to_vec(value)?)
}

pub(super) fn group_id(identity: &str) -> String {
    artifact_cache::sha256(identity.as_bytes())[..20].to_owned()
}

// Called only before admitting a new workbook, never inside its preparation,
// comparison, whitespace controls or cleanup. Zero is a reversible idle state.
pub(super) fn wait_for_admission(output: &Path, phase: &str) -> Result<()> {
    let root = output.join("control-room");
    let mut waiting = false;
    loop {
        check_stop(&root)?;
        if Controls::read(&root).is_ok_and(|controls| controls.target_groups > 0) {
            if waiting {
                if let Some(mut queue) = read_json(&root.join("queue.json")) {
                    queue["phase"] = json!(phase);
                    write(&root.join("queue.json"), &queue)?;
                }
            }
            return Ok(());
        }
        if let Some(mut queue) = read_json(&root.join("queue.json")) {
            queue["phase"] = json!("paused");
            queue["updated_unix_ms"] = json!(provenance::now_ms());
            write(&root.join("queue.json"), &queue)?;
        }
        progress::tick(output);
        waiting = true;
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_retry_has_its_own_attempt_and_finishes_before_report_publication() -> Result<()> {
        let output = super::super::tests::test_directory("classic-attempt")?;
        let room = output.join("control-room");
        fs::create_dir_all(&room)?;
        fs::create_dir_all(output.join("records"))?;
        write(&room.join("control.json"), &json!({"target_groups":1,"paused":false,
            "open_timeout_seconds":60,"job_timeout_seconds":120,"equal_preview_width":512}))?;
        let id = "source:0:example.xlsx";
        let mut record = json!({"identity":id,"name":"example.xlsx","capture_mode":"classic",
            "status":"skipped","complete":true,"control_room_attempt":{"id":"resident-old",
            "run_id":"run","started_unix_ms":1},"last_execution":{"run_id":"run"}});
        write(&room.join("queue.json"), &json!({"groups":[{"id":group_id(id),
            "attempt_id":"resident-old","sequence":7,"status":"waiting_cleanup","active":true}],
            "waiting_groups":4,"cleanup_groups":1}))?;
        let source = Source { id:"source".into(), name:"Example".into(), path:PathBuf::new(),
            members:vec![], fingerprint:"source".into(), selection:selection::Plan::new(vec![], None)? };
        begin_serial(&output, &source, id, &mut record, JobMode::Compare, &json!({"run_id":"run"}))?;
        let queue = read_json(&room.join("queue.json")).unwrap();
        let group = &queue["groups"][0];
        assert_ne!(group["attempt_id"], "resident-old");
        assert_eq!(group["attempt_id"], record["control_room_attempt"]["id"]);
        assert!(group["started_unix_ms"].as_u64().unwrap() > 1);
        assert_eq!(group["sequence"], 7);
        assert_eq!(group["status"], "retrying_classic");
        assert_eq!(group["capture_mode"], "classic");
        assert_eq!(queue["cleanup_groups"], 0);
        assert_eq!(queue["waiting_groups"], 4);
        // A blocked derived report cannot prevent the small queue update.
        fs::create_dir(output.join("diagnosis.json"))?;
        record["status"] = json!("unchanged");
        finish_serial(&output, id, &record)?;
        let queue = read_json(&room.join("queue.json")).unwrap();
        assert_eq!(queue["groups"][0]["active"], false);
        assert_eq!(queue["groups"][0]["status"], "unchanged");
        assert_eq!(queue["open_groups"], 0);
        fs::remove_dir_all(output)?;
        Ok(())
    }

    #[test]
    fn retry_history_preserves_each_failed_run_without_overwriting_an_attempt() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "retry-record-history")?;
        let id = "workbook";
        let mut record = json!({"complete":true,"status":"skipped","prepared":true,
            "error":"first failure","control_room_attempt":{"id":"attempt-old","run_id":"run-old"},
            "last_execution":{"run_id":"run-old"}});
        archive_record(&root, id, &record)?;
        record["error"] = json!("changed after queueing");
        archive_record(&root, id, &record)?;
        let history = root.join("groups").join(group_id(id)).join("history");
        let old = history.join(format!("record-{}.json", &artifact_cache::sha256(b"attempt-old")[..20]));
        assert_eq!(read_json(&old).unwrap()["error"], "first failure");
        record["last_execution"]["run_id"] = json!("run-new");
        record["error"] = json!("retry failed differently");
        archive_record(&root, id, &record)?;
        let new = history.join(format!("record-{}.json", &artifact_cache::sha256(b"run-new")[..20]));
        assert_eq!(read_json(&new).unwrap(), record);
        assert_eq!(fs::read_dir(&history)?.count(), 2);
        assert_eq!(read_json(&old).unwrap()["prepared"], true);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn system_reserve_starts_from_saved_controls_then_environment_then_1024_mib() -> Result<()> {
        assert_eq!(
            initial_system_reserve_mib(None, Err(std::env::VarError::NotPresent))?,
            1024
        );
        assert_eq!(initial_system_reserve_mib(None, Ok("2048".into()))?, 2048);
        let legacy = json!({"target_groups": 2});
        assert_eq!(
            initial_system_reserve_mib(Some(&legacy), Ok("2048".into()))?,
            2048
        );
        let saved = json!({"system_reserve_mib": 4096});
        assert_eq!(
            initial_system_reserve_mib(Some(&saved), Ok("512".into()))?,
            4096
        );
        assert_eq!(
            initial_system_reserve_mib(Some(&saved), Ok("invalid".into()))?,
            4096
        );
        for valid in ["1", "65536"] {
            assert_eq!(
                initial_system_reserve_mib(None, Ok(valid.into()))?,
                valid.parse::<u64>()?
            );
        }
        for invalid in ["0", "65537", "-1", "1.5", "invalid"] {
            assert!(initial_system_reserve_mib(None, Ok(invalid.into())).is_err());
        }
        Ok(())
    }

    #[test]
    fn live_system_reserve_rejects_invalid_updates_and_preserves_saved_values() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "system-reserve-controls")?;
        fs::create_dir(&root)?;
        let mut value = json!({"target_groups":2,"paused":false,
            "open_timeout_seconds":60,"job_timeout_seconds":60,"equal_preview_width":512});
        write(&root.join("control.json"), &value)?;
        assert_eq!(Controls::read(&root)?.system_reserve_mib, 1024);
        for valid in [1, 512, 1024, 4096, 65536] {
            value["system_reserve_mib"] = json!(valid);
            write(&root.join("control.json"), &value)?;
            let controls = Controls::read(&root)?;
            assert_eq!(controls.system_reserve_mib, valid);
            assert_eq!(
                serde_json::to_value(&controls)?["system_reserve_mib"],
                valid
            );
        }
        for invalid in [
            json!(0),
            json!(65537),
            json!(-1),
            json!(1.5),
            json!("512"),
            Value::Null,
        ] {
            value["system_reserve_mib"] = invalid;
            write(&root.join("control.json"), &value)?;
            assert!(Controls::read(&root).is_err());
            assert!(initial_system_reserve_mib(Some(&value), Ok("512".into())).is_err());
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn stop_ends_paused_admission_and_ignores_another_process_incarnation() -> Result<()> {
        let output = artifact_cache::unique_path(&std::env::temp_dir(), "stop-admission")?;
        let root = output.join("control-room");
        fs::create_dir_all(&root)?;
        let started = process_started_unix_ms().unwrap();
        let mut request = json!({"runner_pid":std::process::id(),"runner_started_unix_ms":started - 1});
        write(&root.join("stop-request.json"), &request)?;
        assert!(!stop_requested(&root));
        request["runner_started_unix_ms"] = json!(started);
        write(&root.join("stop-request.json"), &request)?;
        assert!(wait_for_admission(&output, "preparing").unwrap_err().is::<StopRequested>());
        request["runner_pid"] = json!(0);
        write(&root.join("stop-request.json"), &request)?;
        assert!(!stop_requested(&root));
        fs::remove_dir_all(output)?;
        Ok(())
    }
    #[test]
    fn zero_target_waits_without_starting_work_and_resumes_in_place() -> Result<()> {
        let output = artifact_cache::unique_path(&std::env::temp_dir(), "zero-target")?;
        let root = output.join("control-room");
        fs::create_dir_all(&root)?;
        let mut controls = json!({"target_groups":0,"paused":false,"open_timeout_seconds":60,"job_timeout_seconds":60,"equal_preview_width":512});
        write(&root.join("control.json"), &controls)?;
        write(
            &root.join("queue.json"),
            &json!({"groups":[],"phase":"preparing","run_id":"same-run"}),
        )?;
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::scope(|scope| -> Result<()> {
            let output = &output;
            scope.spawn(move || {
                sender
                    .send(
                        wait_for_admission(output, "preparing").map_err(|error| error.to_string()),
                    )
                    .unwrap();
            });
            // The gate must stay closed long enough for at least one UI refresh.
            let waiting = receiver.recv_timeout(Duration::from_millis(650));
            let snapshot = read_json(&root.join("queue.json")).unwrap();
            controls["target_groups"] = json!(1);
            write(&root.join("control.json"), &controls)?;
            let resumed = receiver.recv_timeout(Duration::from_secs(3));
            assert!(matches!(
                waiting,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ));
            assert_eq!(snapshot["phase"], "paused");
            assert!(resumed.unwrap().is_ok());
            let snapshot = read_json(&root.join("queue.json")).unwrap();
            assert_eq!(snapshot["phase"], "preparing");
            assert_eq!(snapshot["run_id"], "same-run");
            Ok(())
        })?;
        fs::remove_dir_all(output)?;
        Ok(())
    }
    #[test]
    fn shutdown_resolves_late_requests_without_rewriting_completed_commands() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "late-control-command")?;
        fs::create_dir_all(root.join("commands"))?;
        for status in ["pending", "queued", "restarting", "completed"] {
            write(
                &root.join("commands").join(format!("{status}.json")),
                &json!({"run_id":"this","status":status}),
            )?;
        }
        reject_unfinished_commands(&root, &json!("this"))?;
        for status in ["pending", "queued", "restarting"] {
            assert_eq!(
                read_json(&root.join("commands").join(format!("{status}.json"))).unwrap()["status"],
                "rejected"
            );
        }
        assert_eq!(
            read_json(&root.join("commands/completed.json")).unwrap()["status"],
            "completed"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[cfg(windows)]
    #[test]
    fn runner_identity_uses_actual_process_creation_time() {
        let created = process_started_unix_ms().expect("read current process creation time");
        assert!(created > 0 && created <= provenance::now_ms());
        assert_eq!(process_started_unix_ms(), Some(created));
    }
    #[test]
    fn host_version_tracks_compiled_sources_and_ignores_logs() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "control-host-version")?;
        fs::create_dir(&root)?;
        fs::write(root.join("Program.cs"), b"first host")?;
        fs::write(root.join("ResidentCaptureHost.csproj"), b"project")?;
        let first = host_version(&root)?;
        fs::write(root.join("host.log"), b"runtime messages")?;
        assert_eq!(host_version(&root)?, first);
        fs::write(root.join("ControlRoom.cs"), b"new control service")?;
        let second = host_version(&root)?;
        assert_ne!(second, first);
        fs::write(root.join("Program.cs"), b"changed host")?;
        assert_ne!(host_version(&root)?, second);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn manual_controls_allow_large_targets_but_reject_invalid_state() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "manual-controls")?;
        fs::create_dir_all(&root)?;
        let mut value = json!({"target_groups":128,"paused":true,"open_timeout_seconds":7200,"job_timeout_seconds":600,"equal_preview_width":256});
        write(&root.join("control.json"), &value)?;
        let legacy = Controls::read(&root)?;
        assert_eq!(legacy.target_groups, 128);
        assert_eq!(legacy.system_reserve_mib, 1024);
        assert_eq!(legacy.capture_tile_megapixels, 32);
        assert_eq!(legacy.capture_batch_tiles, 1);
        assert_eq!(legacy.tall_sheet_threshold, 40);
        assert_eq!(legacy.tall_sheet_edge_tiles, 2);
        assert!(!legacy.hide_excel);
        for (key, bad) in [
            ("capture_tile_megapixels", 0),
            ("capture_tile_megapixels", 33),
            ("capture_batch_tiles", 0),
            ("capture_batch_tiles", 17),
            ("tall_sheet_threshold", 0),
            ("tall_sheet_threshold", i32::MAX as u32 + 1),
            ("tall_sheet_edge_tiles", 0),
            ("tall_sheet_edge_tiles", i32::MAX as u32 + 1),
        ] {
            let mut invalid = value.clone();
            invalid[key] = json!(bad);
            write(&root.join("control.json"), &invalid)?;
            assert!(
                Controls::read(&root).is_err(),
                "accepted invalid {key}={bad}"
            );
        }
        value["target_groups"] = json!(0);
        value["tall_sheet_threshold"] = json!(75);
        value["tall_sheet_edge_tiles"] = json!(3);
        write(&root.join("control.json"), &value)?;
        let updated = Controls::read(&root)?;
        assert_eq!(updated.target_groups, 0);
        assert_eq!(updated.tall_sheet_threshold, 75);
        assert_eq!(updated.tall_sheet_edge_tiles, 3);
        value["job_timeout_seconds"] = json!(0);
        write(&root.join("control.json"), &value)?;
        assert!(Controls::read(&root).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
