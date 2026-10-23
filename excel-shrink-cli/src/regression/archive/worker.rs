use super::*;

mod memory;

#[derive(Parser, Debug)]
pub(in crate::regression) struct WorkerArgs {
    #[arg(long)]
    job: PathBuf,
}

pub(in crate::regression) fn run_worker(args: WorkerArgs, log_level: LogLevel) -> Result<()> {
    let memory_limit = memory::limit_bytes().context("configure workbook worker memory limit")?;
    #[cfg(windows)]
    let _memory_limit = memory::enforce_limit(memory_limit)
        .context("enforce Windows workbook worker memory limit")?;
    let job: MemberJob = serde_json::from_slice(&fs::read(args.job)?)?;
    let _watchdog = start_watchdog(&job, memory_limit);
    let mut record: Value = serde_json::from_slice(&fs::read(&job.record)?)?;
    if job.mode != JobMode::Legacy {
        if let Err(error) = study::run_job(&job, &mut record, log_level) {
            record["status"] = json!("skipped");
            record["error"] = json!(format!("{error:#}"));
        }
        return write_record(&job.record, &record);
    }
    let checked = (|| -> Result<()> {
        let input = extract(&job)?;
        // Reject invalid grid coordinates before Excel can display a repair dialog.
        let plan = office_plan::build_workbook_visual_plan(&input)
            .context("validate workbook capture plan")?;
        ensure!(
            !plan.sheets.is_empty(),
            "workbook has no visible worksheets to compare"
        );
        let case: RegressionCase =
            serde_json::from_value(json!({"id":"workbook","input_path":input}))?;
        visual_diagnosis::diagnose_case(
            "archive",
            &repo_root(),
            &input,
            &job.report_root,
            &job.work.join("profiles"),
            None,
            &[],
            false,
            log_level,
            &case,
            |profile| {
                record["results"].as_array_mut().unwrap().push(profile);
                write_record(&job.record, &record)
            },
        )?;
        Ok(())
    })();
    match checked {
        Err(error) => {
            record["status"] = json!("skipped");
            record["error"] = json!(format!("{error:#}"));
        }
        Ok(()) => {
            let profiles = record["results"]
                .as_array()
                .context("missing profile results")?;
            let complete =
                !profiles.is_empty() && profiles.iter().all(|p| p["checks_complete"] == true);
            record["status"] = json!(if !complete {
                "skipped"
            } else if profiles
                .iter()
                .any(|p| p["failures"].as_array().is_some_and(|f| !f.is_empty()))
            {
                "differences"
            } else {
                "unchanged"
            });
            if !complete {
                record["error"] = json!(
                    "One or more profile checks could not be completed; see profile failures."
                );
            }
        }
    }
    // Parent owns completion: evidence must be preserved and workbooks deleted first.
    write_record(&job.record, &record)
}

pub(super) fn extract(job: &MemberJob) -> Result<PathBuf> {
    let mut zip = ZipArchive::new(fs::File::open(&job.archive)?)?;
    let mut entry = zip.by_index(job.index)?;
    ensure!(
        entry.enclosed_name().is_some() && !entry.name().contains('\\'),
        "unsafe ZIP entry path"
    );
    ensure!(
        entry
            .unix_mode()
            .is_none_or(|mode| mode & 0o170000 != 0o120000),
        "XLSX entry is a symbolic link"
    );
    ensure!(
        entry.size() <= job.max_workbook_bytes,
        "XLSX entry exceeds --max-workbook-bytes ({})",
        job.max_workbook_bytes
    );
    let extension = if entry.name().to_ascii_lowercase().ends_with(".xls") {
        "xls"
    } else {
        "xlsx"
    };
    let input = job.work.join(format!("source.{extension}"));
    let mut file = fs::File::create(&input)?;
    let bytes = std::io::copy(
        &mut (&mut entry).take(job.max_workbook_bytes + 1),
        &mut file,
    )?;
    ensure!(
        bytes <= job.max_workbook_bytes,
        "extracted XLSX exceeds size limit"
    );
    Ok(input)
}

pub(super) fn launch(
    job: &MemberJob,
    path: &Path,
    args: &ArchiveArgs,
    log_level: LogLevel,
    mut on_progress: impl FnMut() -> Result<()>,
) -> Result<()> {
    let log_path = job.report_root.join("logs");
    fs::create_dir_all(&log_path)?;
    let key = artifact_cache::sha256(job.identity.as_bytes());
    let log = fs::File::create(log_path.join(format!("{}.log", &key[..20])))?;
    let worker_log_level = match log_level {
        LogLevel::Info => LogLevel::Warn,
        level => level,
    };
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["regression", "archive-worker", "--job"])
        .arg(path)
        .args(["--console-log-level", worker_log_level.as_filter()])
        .env("EXCEL_SHRINK_REGRESSION_CACHE_DIR", job.work.join("cache"))
        .env("EXCEL_SHRINK_SHARED_WORKBOOK_DIR", job.work.join("shared"))
        .env(
            "EXCEL_SHRINK_AUTOMATION_STALL_SECONDS",
            args.startup_timeout_seconds.to_string(),
        )
        .env(
            "EXCEL_SHRINK_RECALC_TIMEOUT_SECONDS",
            args.workbook_timeout_seconds.to_string(),
        )
        .env(
            "EXCEL_SHRINK_CAPTURE_STARTUP_SECONDS",
            args.startup_timeout_seconds.to_string(),
        )
        .env(
            "EXCEL_SHRINK_CAPTURE_TIMEOUT_SECONDS",
            args.workbook_timeout_seconds.clamp(10, 7200).to_string(),
        )
        .env(
            "EXCEL_SHRINK_ARCHIVE_OWNERS",
            job.work.join("windows-owners.jsonl"),
        )
        .env(
            "WSLENV",
            format!(
                "{}:EXCEL_SHRINK_ARCHIVE_OWNERS/p",
                std::env::var("WSLENV").unwrap_or_default()
            ),
        )
        .env(
            "EXCEL_SHRINK_PARENT_HEARTBEAT",
            job.work.join("parent-heartbeat"),
        )
        .env(
            "EXCEL_SHRINK_WORKER_DEADLINE",
            args.workbook_timeout_seconds.to_string(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    if std::env::var_os("RUST_LOG").is_none() {
        command.env("RUST_LOG", worker_log_level.as_filter());
    }
    let transfers = job.work.join("transfers");
    fs::create_dir_all(&transfers)?;
    command.env("TMPDIR", &transfers);
    command.env("EXCEL_SHRINK_TRANSFER_ROOT", &transfers);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let heartbeat = job.work.join("parent-heartbeat");
    fs::write(&heartbeat, b"running")?;
    let mut running = RunningWorker {
        child: command.spawn().context("start isolated workbook worker")?,
        work: job.work.clone(),
        stopped: false,
    };
    let outcome = wait_for_worker(
        &mut running.child,
        Duration::from_secs(args.workbook_timeout_seconds),
        || {
            fs::write(&heartbeat, b"running")?;
            on_progress()
        },
    );
    running.stop()?;
    outcome
}

fn wait_for_worker(
    child: &mut Child,
    timeout: Duration,
    mut on_progress: impl FnMut() -> Result<()>,
) -> Result<()> {
    let started = Instant::now();
    let mut last_notice = Instant::now();
    loop {
        on_progress()?;
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "workbook worker exited with {status}; see the workbook log"
                ))
            };
        }
        if started.elapsed() >= timeout {
            return Err(anyhow::anyhow!(
                "workbook timed out after {} seconds",
                timeout.as_secs_f64()
            ));
        }
        if last_notice.elapsed() >= Duration::from_secs(30) {
            eprintln!(
                "[archive] Workbook running · {:.0}s elapsed",
                started.elapsed().as_secs_f64()
            );
            last_notice = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

struct RunningWorker {
    child: Child,
    work: PathBuf,
    stopped: bool,
}
impl RunningWorker {
    fn stop(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        // Close Windows launchers first: their jobs own only this worker's Excel instances.
        cleanup_windows(&self.work)?;
        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .output();
        }
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/F", "/T", "/PID", &self.child.id().to_string()])
                .output();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stopped = true;
        Ok(())
    }
}
impl Drop for RunningWorker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub(super) fn cleanup_windows(work: &Path) -> Result<()> {
    let registry = work.join("windows-owners.jsonl");
    if !registry.is_file() {
        return Ok(());
    }
    let script = repo_root().join("excel-shrink-cli/src/regression/archive_processes.ps1");
    let status = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(excel_automation::power_shell_path(&script)?)
        .arg("-RegistryPath")
        .arg(excel_automation::power_shell_path(&registry)?)
        .arg("-Stop")
        .status()
        .context("stop owned Excel launchers")?;
    ensure!(
        status.success(),
        "could not stop owned Windows processes; keeping work directory"
    );
    Ok(())
}

pub(super) fn retain_evidence(value: &mut Value, work: &Path, evidence: &Path) -> Result<()> {
    fn visit(
        value: &mut Value,
        work: &Path,
        evidence: &Path,
        copied: &mut HashMap<PathBuf, PathBuf>,
    ) -> Result<()> {
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    if key.ends_with("_image") {
                        if let Some(path) = value.as_str().map(PathBuf::from) {
                            let relative = path
                                .strip_prefix(work)
                                .context("evidence outside workbook work directory")?;
                            ensure!(
                                path.extension().is_some_and(|e| e == "png"),
                                "non-PNG report evidence"
                            );
                            let digest =
                                artifact_cache::sha256(relative.to_string_lossy().as_bytes());
                            let target = evidence.join(format!("{}.png", &digest[..20]));
                            if !copied.contains_key(&path) {
                                fs::create_dir_all(target.parent().unwrap())?;
                                fs::copy(&path, &target)
                                    .with_context(|| format!("retain {}", path.display()))?;
                                copied.insert(path, target.clone());
                            }
                            *value = json!(target);
                        }
                    } else {
                        visit(value, work, evidence, copied)?;
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    visit(item, work, evidence, copied)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(value, work, evidence, &mut HashMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extracts_one_member_and_rejects_parent_paths_and_size_overruns() -> Result<()> {
        let root = std::env::temp_dir().join(format!("archive-worker-test-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let archive = root.join("input.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&archive)?);
        for name in ["a.xlsx", "../bad.xlsx", "ignored.txt", "b.XLSX"] {
            writer.start_file(name, zip::write::FileOptions::default())?;
            writer.write_all(b"1234")?;
        }
        writer.finish()?;
        let mut zip = ZipArchive::new(fs::File::open(&archive)?)?;
        assert_eq!(workbook_members(&mut zip)?.len(), 3);
        let mut job = MemberJob {
            archive,
            index: 0,
            identity: "test".into(),
            name: "a".into(),
            work: root.clone(),
            record: root.join("record.json"),
            report_root: root.clone(),
            max_workbook_bytes: 4,
            mode: JobMode::Legacy,
            bundle: None,
            run_manifest: None,
        };
        assert_eq!(fs::read(extract(&job)?)?, b"1234");
        job.index = 1;
        assert!(extract(&job).unwrap_err().to_string().contains("unsafe"));
        job.index = 0;
        job.max_workbook_bytes = 3;
        assert!(extract(&job).unwrap_err().to_string().contains("exceeds"));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn keeps_only_referenced_images_and_rewrites_the_report_paths() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("archive-evidence-test-{}", std::process::id()));
        let work = root.join("work");
        fs::create_dir_all(&work)?;
        let image = work.join("changed.png");
        fs::write(&image, b"evidence")?;
        fs::write(work.join("source.xlsx"), b"workbook")?;
        fs::write(work.join("unchanged.png"), b"discard")?;
        let mut result = json!({"sheets":[{"original_image":image,"shrunk_image":image}]});
        retain_evidence(&mut result, &work, &root.join("evidence"))?;
        fs::remove_dir_all(&work)?;
        let retained = PathBuf::from(result["sheets"][0]["original_image"].as_str().unwrap());
        assert_eq!(fs::read(retained)?, b"evidence");
        assert_eq!(fs::read_dir(root.join("evidence"))?.count(), 1);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn a_corrupt_member_header_does_not_hide_the_next_workbook() -> Result<()> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for name in ["broken.xlsx", "valid.xlsx"] {
            writer.start_file(name, zip::write::FileOptions::default())?;
            writer.write_all(b"data")?;
        }
        let mut bytes = writer.finish()?.into_inner();
        bytes[0] = 0;
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes))?;
        let members = workbook_members(&mut archive)?;
        assert_eq!(members[0], (0, "unreadable ZIP entry #0".into()));
        assert_eq!(members[1], (1, "valid.xlsx".into()));
        Ok(())
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn watchdog_limits_isolated_workers_and_reports_memory_peaks() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("archive-watchdog-test-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let heartbeat = root.join("parent-heartbeat");
        for scenario in ["memory", "deadline", "complete"] {
            fs::write(&heartbeat, b"running")?;
            let mut command = Command::new(std::env::current_exe()?);
            command
                .args([
                    "--exact",
                    "regression::archive::worker::tests::watchdog_test_child",
                    "--nocapture",
                ])
                .env("EXCEL_SHRINK_WATCHDOG_TEST_SCENARIO", scenario)
                .env("EXCEL_SHRINK_WATCHDOG_TEST_ROOT", &root)
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            let mut running = RunningWorker {
                child: command.spawn()?,
                work: root.clone(),
                stopped: false,
            };
            let result = wait_for_worker(&mut running.child, Duration::from_secs(10), || Ok(()));
            running.stop()?;
            let status = running
                .child
                .try_wait()?
                .context("test child did not exit")?;
            let mut log = String::new();
            running
                .child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut log)?;
            if scenario == "complete" {
                assert!(result.is_ok(), "{result:?}\n{log}");
                assert!(status.success(), "{log}");
            } else {
                assert!(result.is_err());
                assert_eq!(status.code(), Some(124), "{scenario}: {log}");
                assert!(
                    log.contains(if scenario == "memory" {
                        "exceeded memory limit"
                    } else {
                        "deadline reached"
                    }),
                    "{log}"
                );
            }
            assert_eq!(log.matches("Worker memory (").count(), 1, "{log}");
            assert!(log.contains(memory::METRIC), "{log}");
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    // Invoked in a separate process so exiting on a watchdog limit cannot kill
    // the test runner. The small test limit exercises the real memory API.
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn watchdog_test_child() -> Result<()> {
        let Ok(scenario) = std::env::var("EXCEL_SHRINK_WATCHDOG_TEST_SCENARIO") else {
            return Ok(());
        };
        let root = PathBuf::from(std::env::var_os("EXCEL_SHRINK_WATCHDOG_TEST_ROOT").unwrap());
        let before = memory::usage()?;
        let allocation = vec![0xa5_u8; 16 * 1024 * 1024];
        std::hint::black_box(&allocation);
        let during = memory::usage()?;
        assert!(during.current_bytes >= before.current_bytes + 8 * 1024 * 1024);
        assert!(during.peak_bytes >= during.current_bytes);
        drop(allocation);
        assert!(memory::usage()?.peak_bytes >= during.peak_bytes);

        let watchdog = WorkerWatchdog::start(
            root.clone(),
            root.join("parent-heartbeat"),
            if scenario == "deadline" {
                Duration::ZERO
            } else {
                Duration::from_secs(30)
            },
            if scenario == "memory" { 1 } else { u64::MAX },
        );
        if scenario != "complete" {
            std::thread::sleep(Duration::from_secs(20));
            panic!("watchdog did not stop the test worker");
        }
        drop(watchdog);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn deadline_kills_a_stuck_worker_and_allows_the_next_one_to_finish() -> Result<()> {
        use std::os::unix::process::CommandExt;
        let root =
            std::env::temp_dir().join(format!("archive-timeout-test-{}", std::process::id()));
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60"]).process_group(0);
        let mut stuck = RunningWorker {
            child: command.spawn()?,
            work: root.clone(),
            stopped: false,
        };
        let failure =
            wait_for_worker(&mut stuck.child, Duration::from_millis(20), || Ok(())).unwrap_err();
        assert!(failure.to_string().contains("timed out"));
        stuck.stop()?;
        assert!(stuck.child.try_wait()?.is_some());
        let mut next = Command::new("true").spawn()?;
        wait_for_worker(&mut next, Duration::from_secs(2), || Ok(()))?;
        Ok(())
    }
}

// The worker also enforces its deadline: the terminal or parent runner can disappear.
fn start_watchdog(job: &MemberJob, memory_limit: u64) -> Option<WorkerWatchdog> {
    let heartbeat = std::env::var_os("EXCEL_SHRINK_PARENT_HEARTBEAT").map(PathBuf::from)?;
    let timeout = std::env::var("EXCEL_SHRINK_WORKER_DEADLINE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1800);
    Some(WorkerWatchdog::start(
        job.work.clone(),
        heartbeat,
        Duration::from_secs(timeout),
        memory_limit,
    ))
}

struct WorkerWatchdog {
    stop: std::sync::mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WorkerWatchdog {
    fn start(work: PathBuf, heartbeat: PathBuf, timeout: Duration, memory_limit: u64) -> Self {
        let (stop, stopped) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let started = Instant::now();
            let mut memory_warning_logged = false;
            while stopped.recv_timeout(Duration::from_secs(2))
                == Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            {
                let memory = match memory::usage() {
                    Ok(usage) => Some(usage),
                    Err(error) => {
                        if !memory_warning_logged {
                            eprintln!("[archive] Cannot measure worker memory: {error}");
                            memory_warning_logged = true;
                        }
                        None
                    }
                };
                let parent_alive = fs::metadata(&heartbeat)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|time| time.elapsed().ok())
                    .is_some_and(|age| age < Duration::from_secs(60));
                let reason = if !parent_alive {
                    Some("parent runner stopped responding")
                } else if started.elapsed() > timeout {
                    Some("workbook deadline reached")
                } else if memory.is_some_and(|usage| usage.current_bytes > memory_limit) {
                    Some("workbook worker exceeded memory limit")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    log_memory_usage(memory, memory_limit);
                    eprintln!("[archive] {reason}; closing owned Excel processes");
                    let _ = cleanup_windows(&work);
                    std::process::exit(124);
                }
            }
            log_memory_usage(memory::usage().ok(), memory_limit);
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for WorkerWatchdog {
    fn drop(&mut self) {
        // Waking the thread also records the OS peak for jobs shorter than one sample.
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn log_memory_usage(usage: Option<memory::Usage>, limit: u64) {
    if let Some(usage) = usage {
        eprintln!(
            "[archive] Worker memory ({}): current {:.1} MiB; peak {:.1} MiB; limit {:.0} MiB",
            memory::METRIC,
            usage.current_bytes as f64 / 1_048_576.0,
            usage.peak_bytes as f64 / 1_048_576.0,
            limit as f64 / 1_048_576.0,
        );
    }
}
