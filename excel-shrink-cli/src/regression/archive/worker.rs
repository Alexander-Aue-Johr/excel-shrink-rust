use super::*;

mod memory;

pub(super) fn memory_limit_bytes() -> std::io::Result<u64> {
    memory::limit_bytes()
}

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
        .env(
            "EXCEL_SHRINK_RESIDENT_CAPTURE",
            if job.resident_capture { "1" } else { "0" },
        )
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
    command.env(
        "EXCEL_SHRINK_HIDE_EXCEL",
        if job.hide_excel { "1" } else { "0" },
    );
    if let Some(root) = job.control_room.as_ref().filter(|_| job.resident_capture) {
        let record: Value = serde_json::from_slice(&fs::read(&job.record)?)?;
        let attempt = &record["control_room_attempt"];
        command
            .env("EXCEL_SHRINK_CONTROL_ROOM_DIR", root)
            .env("EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID", &key[..20])
            .env(
                "EXCEL_SHRINK_CONTROL_ROOM_OWNER_PID",
                std::process::id().to_string(),
            );
        for (name, value) in [
            (
                "EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID",
                attempt["id"].as_str().map(str::to_owned),
            ),
            (
                "EXCEL_SHRINK_CAPTURE_TILE_MEGAPIXELS",
                attempt["settings"]["capture_tile_megapixels"]
                    .as_u64()
                    .map(|n| n.to_string()),
            ),
            (
                "EXCEL_SHRINK_CAPTURE_BATCH_TILES",
                attempt["settings"]["capture_batch_tiles"]
                    .as_u64()
                    .map(|n| n.to_string()),
            ),
        ] {
            if let Some(value) = value {
                command.env(name, value);
            } else {
                command.env_remove(name);
            }
        }
    } else {
        command
            .env_remove("EXCEL_SHRINK_CONTROL_ROOM_DIR")
            .env_remove("EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID")
            .env_remove("EXCEL_SHRINK_CONTROL_ROOM_OWNER_PID");
        command
            .env_remove("EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID")
            .env_remove("EXCEL_SHRINK_CAPTURE_TILE_MEGAPIXELS")
            .env_remove("EXCEL_SHRINK_CAPTURE_BATCH_TILES");
    }
    if job.resident_capture {
        let record: Value = serde_json::from_slice(&fs::read(&job.record)?)?;
        if let Some(mib) = resident_memory_limit_mib(&record)? {
            // Each adaptive wave chooses its own Excel budget. Keep this local
            // to the child so concurrent workers and classic retries retain
            // their independently configured limits.
            command.env("EXCEL_SHRINK_RESIDENT_MEMORY_MIB", mib.to_string());
        }
    }
    if let Some(barrier) = &job.cleanup_barrier {
        let config = job.work.join("batch-cleanup.json");
        fs::write(&config, serde_json::to_vec(barrier)?)?;
        command.env(batch_cleanup::ENV, &config);
    } else {
        command.env_remove(batch_cleanup::ENV);
    }
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
        cleanup_barrier: job.cleanup_barrier.clone(),
    };
    let outcome = wait_for_worker(
        &mut running.child,
        Duration::from_secs(args.workbook_timeout_seconds),
        job.cleanup_barrier.as_ref(),
        || {
            fs::write(&heartbeat, b"running")?;
            on_progress()
        },
    );
    running.stop()?;
    outcome
}

fn resident_memory_limit_mib(record: &Value) -> Result<Option<u64>> {
    let value = &record["comparison_batch"]["resident_memory_limit_mib"];
    if value.is_null() {
        return Ok(None);
    }
    let mib = value.as_u64().filter(|mib| (1024..=65536).contains(mib));
    ensure!(
        mib.is_some(),
        "comparison_batch.resident_memory_limit_mib must be an integer between 1024 and 65536"
    );
    Ok(mib)
}

fn wait_for_worker(
    child: &mut Child,
    timeout: Duration,
    cleanup_barrier: Option<&batch_cleanup::Participant>,
    mut on_progress: impl FnMut() -> Result<()>,
) -> Result<()> {
    let mut clock = opening_pause::OpeningPause::new(
        cleanup_barrier.and_then(|barrier| barrier.pause_path.as_deref()),
    );
    let mut last_notice = Instant::now();
    loop {
        ensure!(
            !cleanup_barrier.is_some_and(|barrier| barrier.cancelled()),
            "resident comparison cancelled by coordinator"
        );
        ensure!(
            !cleanup_barrier.is_some_and(|barrier| barrier.restart_requested()),
            "user requested a fresh comparison attempt"
        );
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
        if cleanup_barrier.is_some_and(|barrier| barrier.waiting_expired()) {
            anyhow::bail!("resident batch cleanup barrier timed out");
        }
        if !cleanup_barrier.is_some_and(|barrier| barrier.manual_control)
            && clock.active_elapsed() >= timeout
            && !cleanup_barrier.is_some_and(|barrier| barrier.deadline_paused())
        {
            return Err(anyhow::anyhow!(
                "workbook timed out after {} active seconds ({:.1}s wall time, {:.1}s opening pause)",
                timeout.as_secs_f64(),
                clock.wall_elapsed().as_secs_f64(),
                clock.credited().as_secs_f64()
            ));
        }
        if last_notice.elapsed() >= Duration::from_secs(30) {
            eprintln!(
                "[archive] Workbook running · {:.0}s elapsed; {:.0}s opening pause",
                clock.wall_elapsed().as_secs_f64(),
                clock.credited().as_secs_f64()
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
    cleanup_barrier: Option<batch_cleanup::Participant>,
}
impl RunningWorker {
    fn stop(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        if let Some(barrier) = self
            .cleanup_barrier
            .as_ref()
            .filter(|barrier| !barrier.cancelled())
        {
            // A dead or timed-out Rust worker may still own a live launcher.
            // Mark its slot terminal before cleanup can affect another session.
            if let Err(error) = barrier.wait(Some(&self.work.join("parent-heartbeat")), true) {
                eprintln!("[archive] Bounded batch cleanup wait ended: {error:#}");
            }
        }
        // Close Windows launchers first: their jobs own only this worker's Excel instances.
        let cleanup = cleanup_windows(&self.work);
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
        cleanup
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
    fn parent_cancellation_stops_owned_worker_without_waiting_for_manual_peers() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "cancel-worker")?;
        fs::create_dir(&root)?;
        fs::write(root.join("parent-heartbeat"), b"running")?;
        let mut batch = batch_cleanup::Batch::growing(&root, 3600)?;
        let participant = batch.add_manual_participant()?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "--exact",
                "regression::archive::worker::tests::cancellation_test_child",
            ])
            .env("EXCEL_SHRINK_CANCELLATION_TEST_CHILD", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut running = RunningWorker {
            child: command.spawn()?,
            work: root.clone(),
            stopped: false,
            cleanup_barrier: Some(participant.clone()),
        };
        let started = Instant::now();
        let outcome = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                participant.cancel();
            });
            wait_for_worker(
                &mut running.child,
                Duration::from_secs(3600),
                Some(&participant),
                || Ok(()),
            )
        });
        assert!(format!("{:#}", outcome.unwrap_err()).contains("cancelled"));
        running.stop()?;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(running.child.try_wait()?.is_some());
        assert!(!participant.released());
        drop(batch);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn cancellation_test_child() {
        if std::env::var("EXCEL_SHRINK_CANCELLATION_TEST_CHILD").as_deref() == Ok("1") {
            std::thread::sleep(Duration::from_secs(30));
        }
    }

    #[test]
    fn per_wave_resident_memory_limit_is_optional_and_bounded() -> Result<()> {
        assert_eq!(resident_memory_limit_mib(&json!({}))?, None);
        for mib in [1024, 4096, 16384, 65536] {
            assert_eq!(
                resident_memory_limit_mib(
                    &json!({"comparison_batch":{"resident_memory_limit_mib":mib}})
                )?,
                Some(mib)
            );
        }
        for value in [
            json!(0),
            json!(1023),
            json!(65537),
            json!(-1),
            json!(1.5),
            json!("4096"),
        ] {
            assert!(
                resident_memory_limit_mib(
                    &json!({"comparison_batch":{"resident_memory_limit_mib":value}})
                )
                .is_err()
            );
        }
        Ok(())
    }

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
            resident_capture: false,
            hide_excel: false,
            control_room: None,
            cleanup_barrier: None,
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
                cleanup_barrier: None,
            };
            let result =
                wait_for_worker(&mut running.child, Duration::from_secs(10), None, || Ok(()));
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
            let measured: Value =
                serde_json::from_slice(&fs::read(root.join("worker-memory.json"))?)?;
            assert_eq!(measured["pid"], running.child.id());
            assert_eq!(measured["metric"], memory::METRIC);
            assert_eq!(measured["final_sample"], true);
            assert!(measured["current_bytes"].as_u64().unwrap() > 0);
            assert!(
                measured["peak_bytes"].as_u64().unwrap()
                    >= measured["current_bytes"].as_u64().unwrap()
            );
            assert!(measured["sampled_unix_ms"].as_u64().unwrap() > 0);
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
            None,
        );
        if scenario != "complete" {
            std::thread::sleep(Duration::from_secs(20));
            panic!("watchdog did not stop the test worker");
        }
        drop(watchdog);
        Ok(())
    }

    #[test]
    fn ready_batch_wait_is_excluded_from_both_worker_deadlines() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "batch-watchdog")?;
        fs::create_dir(&root)?;
        fs::write(root.join("parent-heartbeat"), b"running")?;
        let batch = batch_cleanup::Batch::new(&root, 2, 5)?;
        let participant = batch.participant(0);
        participant.mark_ready()?;
        let config = root.join("barrier.json");
        fs::write(&config, serde_json::to_vec(&participant)?)?;
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "regression::archive::worker::tests::batch_wait_test_child",
                "--nocapture",
            ])
            .env("EXCEL_SHRINK_BATCH_WAIT_TEST_ROOT", &root)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        // The child lives beyond both deadlines. Only its completed readiness
        // suspends them; the peer timeout and parent checks remain enabled.
        let outcome = wait_for_worker(&mut child, Duration::ZERO, Some(&participant), || Ok(()));
        if outcome.is_err() {
            let _ = child.kill();
        }
        let output = child.wait_with_output()?;
        assert!(
            outcome.is_ok(),
            "{outcome:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success());
        drop(batch);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn batch_wait_test_child() -> Result<()> {
        let Some(root) = std::env::var_os("EXCEL_SHRINK_BATCH_WAIT_TEST_ROOT") else {
            return Ok(());
        };
        let root = PathBuf::from(root);
        let participant = serde_json::from_slice(&fs::read(root.join("barrier.json"))?)?;
        let watchdog = WorkerWatchdog::start(
            root.clone(),
            root.join("parent-heartbeat"),
            Duration::ZERO,
            u64::MAX,
            Some(participant),
        );
        std::thread::sleep(Duration::from_millis(2300));
        drop(watchdog);
        Ok(())
    }

    #[test]
    fn peer_opening_time_is_excluded_from_both_worker_deadlines() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "opening-watchdog")?;
        fs::create_dir(&root)?;
        fs::write(root.join("parent-heartbeat"), b"running")?;
        let batch = batch_cleanup::Batch::new(&root, 2, 5)?;
        let mut participant = batch.participant(0);
        let pause = root.join("opening-pause.json");
        participant.pause_path = Some(pause.clone());
        fs::write(&pause, br#"{"elapsed_millis":0}"#)?;
        fs::write(root.join("barrier.json"), serde_json::to_vec(&participant)?)?;
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "regression::archive::worker::tests::opening_pause_test_child",
                "--nocapture",
            ])
            .env("EXCEL_SHRINK_OPENING_PAUSE_TEST_ROOT", &root)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let started = Instant::now();
        let outcome = wait_for_worker(
            &mut child,
            Duration::from_secs(1),
            Some(&participant),
            || {
                artifact_cache::atomic_write(
                    &pause,
                    &serde_json::to_vec(&json!({"elapsed_millis":started.elapsed().as_millis()}))?,
                )
            },
        );
        if outcome.is_err() {
            let _ = child.kill();
        }
        let output = child.wait_with_output()?;
        assert!(
            outcome.is_ok(),
            "{outcome:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success());
        // This was an active comparison, not a completed cleanup-barrier wait.
        assert!(!participant.ready());
        drop(batch);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn opening_pause_test_child() -> Result<()> {
        let Some(root) = std::env::var_os("EXCEL_SHRINK_OPENING_PAUSE_TEST_ROOT") else {
            return Ok(());
        };
        let root = PathBuf::from(root);
        let participant = serde_json::from_slice(&fs::read(root.join("barrier.json"))?)?;
        let watchdog = WorkerWatchdog::start(
            root.clone(),
            root.join("parent-heartbeat"),
            Duration::from_secs(1),
            u64::MAX,
            Some(participant),
        );
        std::thread::sleep(Duration::from_millis(2300));
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
            cleanup_barrier: None,
        };
        let failure = wait_for_worker(&mut stuck.child, Duration::from_millis(20), None, || Ok(()))
            .unwrap_err();
        assert!(failure.to_string().contains("timed out"));
        stuck.stop()?;
        assert!(stuck.child.try_wait()?.is_some());
        let mut next = Command::new("true").spawn()?;
        wait_for_worker(&mut next, Duration::from_secs(2), None, || Ok(()))?;
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
        job.cleanup_barrier.clone(),
    ))
}

struct WorkerWatchdog {
    stop: std::sync::mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WorkerWatchdog {
    fn start(
        work: PathBuf,
        heartbeat: PathBuf,
        timeout: Duration,
        memory_limit: u64,
        cleanup_barrier: Option<batch_cleanup::Participant>,
    ) -> Self {
        let (stop, stopped) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut clock = opening_pause::OpeningPause::new(
                cleanup_barrier
                    .as_ref()
                    .and_then(|barrier| barrier.pause_path.as_deref()),
            );
            let mut memory_warning_logged = false;
            let mut telemetry_warning_logged = false;
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
                record_memory_usage(
                    &work,
                    memory,
                    memory_limit,
                    false,
                    &mut telemetry_warning_logged,
                );
                let parent_alive = batch_cleanup::parent_alive(&heartbeat);
                let reason = if !parent_alive {
                    Some("parent runner stopped responding")
                } else if cleanup_barrier
                    .as_ref()
                    .is_some_and(|barrier| barrier.waiting_expired())
                {
                    Some("resident batch cleanup barrier timed out")
                } else if !cleanup_barrier
                    .as_ref()
                    .is_some_and(|barrier| barrier.manual_control)
                    && clock.active_elapsed() > timeout
                    && !cleanup_barrier
                        .as_ref()
                        .is_some_and(|barrier| barrier.deadline_paused())
                {
                    Some("workbook deadline reached")
                } else if memory.is_some_and(|usage| usage.current_bytes > memory_limit) {
                    Some("workbook worker exceeded memory limit")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    log_memory_usage(memory, memory_limit);
                    eprintln!(
                        "[archive] {reason}; {:.1}s wall time, {:.1}s opening pause; closing owned Excel processes",
                        clock.wall_elapsed().as_secs_f64(),
                        clock.credited().as_secs_f64()
                    );
                    if parent_alive {
                        if let Some(barrier) = &cleanup_barrier {
                            // Keep peer sessions alive while this failed worker
                            // waits for their checkpoints or bounded deadlines.
                            let _ = barrier.wait(Some(&heartbeat), false);
                        }
                    }
                    let _ = cleanup_windows(&work);
                    record_memory_usage(
                        &work,
                        memory::usage().ok().or(memory),
                        memory_limit,
                        true,
                        &mut telemetry_warning_logged,
                    );
                    std::process::exit(124);
                }
            }
            let memory = memory::usage().ok();
            record_memory_usage(
                &work,
                memory,
                memory_limit,
                true,
                &mut telemetry_warning_logged,
            );
            log_memory_usage(memory, memory_limit);
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

fn record_memory_usage(
    work: &Path,
    usage: Option<memory::Usage>,
    limit: u64,
    final_sample: bool,
    warning_logged: &mut bool,
) {
    let Some(usage) = usage else { return };
    let sampled_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let measured = json!({
        "version":1,"pid":std::process::id(),"metric":memory::METRIC,
        "current_bytes":usage.current_bytes,"peak_bytes":usage.peak_bytes,
        "limit_bytes":limit,"sampled_unix_ms":sampled_unix_ms,
        "final_sample":final_sample,
    });
    let saved = serde_json::to_vec(&measured)
        .context("serialize worker memory measurement")
        .and_then(|bytes| artifact_cache::atomic_write(&work.join("worker-memory.json"), &bytes));
    if let Err(error) = saved {
        if !*warning_logged {
            eprintln!("[archive] Cannot save worker memory measurement: {error:#}");
            *warning_logged = true;
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
