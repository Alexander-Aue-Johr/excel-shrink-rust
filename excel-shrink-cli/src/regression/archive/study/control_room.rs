//! Browser controls and durable snapshots; comparison correctness never depends on a tab.
use super::*;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Controls {
    pub target_groups: usize,
    pub paused: bool,
    pub open_timeout_seconds: u64,
    pub job_timeout_seconds: u64,
    pub equal_preview_width: u32,
}

impl Controls {
    pub fn read(root: &Path) -> Result<Self> {
        let value: Self = serde_json::from_slice(&fs::read(root.join("control.json"))?)?;
        ensure!(
            value.target_groups > 0
                && value.open_timeout_seconds > 0
                && value.job_timeout_seconds > 0,
            "control room limits must be positive"
        );
        ensure!(
            [0, 128, 256, 512, 1024].contains(&value.equal_preview_width),
            "invalid equal tile preview width"
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

impl Room {
    pub fn start(output: &Path, args: &StudyArgs, run: &Value) -> Result<Self> {
        let root = output.join("control-room");
        fs::create_dir_all(root.join("groups"))?;
        let controls = Controls {
            target_groups: args.comparison_workers,
            paused: false,
            open_timeout_seconds: args.startup_timeout_seconds,
            job_timeout_seconds: args.workbook_timeout_seconds,
            equal_preview_width: Controls::read(&root)
                .map(|old| old.equal_preview_width)
                .unwrap_or(512),
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
            ensure!(health["host_version"] == version,
                "control room on port {} uses a different host build; stop that control room or choose another --control-room-port (the existing server was left running)", args.control_room_port);
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
        queue["run_id"] = run["run_id"].clone();
        queue["machine_id"] = run["machine_id"].clone();
        queue["machine"] = json!(args
            .machine_label
            .clone()
            .unwrap_or_else(|| "This machine".into()));
        queue["runner_pid"] = json!(std::process::id());
        queue["runner_started_unix_ms"] = json!(process_started_unix_ms());
        queue["running"] = json!(true);
        queue["phase"] = json!("preparing");
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
        if let Some(groups) = queue["groups"].as_array_mut() {
            for group in groups.iter_mut().filter(|group| group["active"] == true) {
                group["active"] = json!(false);
                group["status"] = json!("interrupted");
            }
        }
        queue["updated_unix_ms"] = json!(provenance::now_ms());
        write(&self.root.join("queue.json"), &queue)
    }
}

impl Drop for Room {
    fn drop(&mut self) {
        if !self.finished {
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
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetProcessTimes(
            process: *mut std::ffi::c_void,
            created: *mut u64,
            exited: *mut u64,
            kernel: *mut u64,
            user: *mut u64,
        ) -> i32;
    }
    let (mut created, mut exited, mut kernel, mut user) = (0, 0, 0, 0);
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    (ok != 0)
        .then_some(created)?
        .checked_sub(116_444_736_000_000_000)
        .map(|ticks| ticks / 10_000)
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

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(Controls::read(&root)?.target_groups, 128);
        value["target_groups"] = json!(0);
        write(&root.join("control.json"), &value)?;
        assert!(Controls::read(&root).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
