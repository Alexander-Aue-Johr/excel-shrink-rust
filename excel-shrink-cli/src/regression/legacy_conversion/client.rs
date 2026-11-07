//! Lease one ready session and retain the lease until its result is durable.
use super::super::excel_automation::ExcelOpenMetrics;
use super::*;

pub(in crate::regression) fn convert(
    root: &Path,
    input: &Path,
    output: &Path,
    work: &Path,
) -> Result<ExcelOpenMetrics> {
    fs::create_dir_all(work)?;
    // A vanished helper is infrastructure failure. Retry once using a new session.
    for attempt in 0..2 {
        let (session, lease) = lease_session(root)?;
        let directory = work.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&directory)?;
        let request = Request {
            id: uuid::Uuid::new_v4().to_string(),
            input: PathBuf::from(super::super::excel_automation::power_shell_path(input)?),
            output: PathBuf::from(super::super::excel_automation::power_shell_path(output)?),
            directory: PathBuf::from(super::super::excel_automation::power_shell_path(
                &directory,
            )?),
        };
        let result = run_request(root, &session, &request);
        drop(lease);
        match result {
            Ok(Some(metrics)) => {
                fs::write(
                    work.join("excel-open-metrics.json"),
                    serde_json::to_vec(&metrics)?,
                )?;
                return Ok(metrics);
            }
            Ok(None) if attempt == 0 => continue,
            Ok(None) => anyhow::bail!(
                "persistent conversion helper exited twice; inspect {}",
                directory.display()
            ),
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded conversion retry")
}

fn lease_session(root: &Path) -> Result<(PathBuf, File)> {
    let started = Instant::now();
    let wait_path = std::env::var_os("EXCEL_SHRINK_PREPARATION_WAIT").map(PathBuf::from);
    let previous = wait_path
        .as_deref()
        .and_then(read_json)
        .and_then(|value| value["elapsed_millis"].as_u64())
        .unwrap_or(0);
    let mut last_counter = Instant::now();
    loop {
        check_pool(root)?;
        if let Ok(bytes) = fs::read(root.join("manifest.json")) {
            let manifest: Manifest = serde_json::from_slice(&bytes)?;
            for member in manifest
                .workers
                .iter()
                .filter(|member| member.ready && !member.retiring)
            {
                let lease = open_lease(&member.directory)?;
                if lease.try_lock().is_err() {
                    continue;
                }
                if member.directory.join("retire").exists()
                    || member.directory.join("stopped").exists()
                    || member.directory.join("cancel").exists()
                {
                    continue;
                }
                publish_wait(&wait_path, previous, started)?;
                return Ok((member.directory.clone(), lease));
            }
        }
        if last_counter.elapsed() >= Duration::from_secs(1) {
            publish_wait(&wait_path, previous, started)?;
            last_counter = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn publish_wait(path: &Option<PathBuf>, previous: u64, started: Instant) -> Result<()> {
    if let Some(path) = path {
        write_json(
            path,
            &json!({"elapsed_millis":previous.saturating_add(started.elapsed().as_millis() as u64)}),
        )?;
    }
    Ok(())
}

/// Timeout is measured per observable COM phase, excluding the capacity wait above.
fn run_request(root: &Path, session: &Path, request: &Request) -> Result<Option<ExcelOpenMetrics>> {
    write_json(&session.join("request.json"), request)?;
    let timeout = super::super::visual_diagnostics::seconds_from_env(
        "EXCEL_SHRINK_AUTOMATION_STALL_SECONDS",
        120.0,
    );
    let mut last_phase = Value::Null;
    let mut last_progress = Instant::now();
    loop {
        if let Some(response) = read_json(&request.directory.join("response.json")) {
            if response["id"] != request.id {
                retire_and_wait(session)?;
                anyhow::bail!("conversion response belongs to another request");
            }
            if response["success"] == true {
                anyhow::ensure!(
                    request.output.is_file(),
                    "conversion response has no output workbook"
                );
                let metrics = serde_json::from_value(response["metrics"].clone())?;
                return Ok(Some(metrics));
            }
            retire_and_wait(session)?;
            let work = request
                .directory
                .parent()
                .context("conversion request has no parent")?;
            fs::write(
                work.join("excel-open-failure.json"),
                serde_json::to_vec(&response)?,
            )?;
            anyhow::bail!(
                "Excel conversion {} during {}: {}",
                response["kind"].as_str().unwrap_or("failed"),
                response["phase"].as_str().unwrap_or("unknown phase"),
                response["error"]
                    .as_str()
                    .unwrap_or("native error unavailable")
            );
        }
        if session.join("stopped").exists() {
            return Ok(None);
        }
        if let Some(progress) = read_json(&request.directory.join("progress.json")) {
            if progress != last_phase {
                last_phase = progress;
                last_progress = Instant::now();
            }
        }
        if last_progress.elapsed() > timeout {
            retire_and_wait(session)?;
            let failure = json!({"kind":"conversion_timeout","phase":last_phase["phase"],
                "error":format!("No conversion phase progress for {:.1} seconds", timeout.as_secs_f64()),"session":session});
            fs::write(
                request.directory.join("failure.json"),
                serde_json::to_vec(&failure)?,
            )?;
            anyhow::bail!(
                "Excel conversion timed out during {}; inspect {}",
                last_phase["phase"],
                request.directory.display()
            );
        }
        check_pool(root)?;
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn retire_and_wait(session: &Path) -> Result<()> {
    fs::write(session.join("cancel"), b"")?;
    let started = Instant::now();
    while !session.join("stopped").exists() {
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(15),
            "conversion cleanup remains unconfirmed: {}",
            session.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn check_pool(root: &Path) -> Result<()> {
    if let Some(error) = read_json(&root.join("error.json")) {
        anyhow::bail!("conversion pool failed: {}", error["error"]);
    }
    if let Ok(metadata) = fs::metadata(root.join("heartbeat")) {
        anyhow::ensure!(
            metadata.modified()?.elapsed()? < Duration::from_secs(30),
            "conversion pool manager stopped responding"
        );
    }
    if let Some(heartbeat) = std::env::var_os("EXCEL_SHRINK_PARENT_HEARTBEAT") {
        let age = fs::metadata(heartbeat)?.modified()?.elapsed()?;
        anyhow::ensure!(
            age < Duration::from_secs(60),
            "conversion coordinator heartbeat expired"
        );
    }
    Ok(())
}
