//! One reusable, owner-scoped inspection pair, isolated from corpus workers.
use super::*;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;

#[derive(Default)]
pub(super) struct Inspector {
    broker: Option<Broker>,
    directory: Option<PathBuf>,
    copies: Option<(String, PathBuf, PathBuf)>,
}

struct Broker {
    child: Child,
    input: BufWriter<ChildStdin>,
    responses: mpsc::Receiver<Result<Value>>,
    #[cfg(windows)]
    _job: crate::process_job::Job,
}

pub(super) async fn select(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
    payload: Result<Json<Value>, JsonRejection>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    let body = object_payload(payload)?;
    if !body
        .as_object()
        .unwrap()
        .keys()
        .all(|key| matches!(key.as_str(), "sheet" | "range"))
    {
        return Err(ApiError::bad_request("Unknown region inspection option."));
    }
    let sheet = required_string(&body, "sheet")?.to_owned();
    let range = required_string(&body, "range")?.to_owned();
    validate_region(&sheet, &range).map_err(ApiError::bad_request)?;
    if !known_workbook(&room, &id).await? {
        return Err(ApiError::not_found("Unknown comparison group."));
    }
    let response = tokio::task::spawn_blocking(move || {
        let mut inspector = room
            .inspection_pair
            .lock()
            .map_err(|_| anyhow::anyhow!("Inspection lock poisoned"))?;
        inspector.select(&room.root, &room.web, &id, &sheet, &range)
    })
    .await
    .context("Inspect selected range")??;
    Ok(Json(response))
}

/// Accept one rectangular A1 range; never pass expressions or external references to COM.
fn validate_region(sheet: &str, range: &str) -> Result<()> {
    ensure!(
        !sheet.is_empty() && sheet.chars().count() <= 31 && !sheet.chars().any(char::is_control),
        "Invalid worksheet name."
    );
    let mut coordinates = range.split(':');
    let parse = |address: &str| -> Result<(u32, u32)> {
        let bytes = address.as_bytes();
        let letters = bytes
            .iter()
            .take_while(|byte| byte.is_ascii_uppercase())
            .count();
        ensure!(
            (1..=3).contains(&letters) && letters < bytes.len(),
            "Expected an A1 cell coordinate."
        );
        ensure!(
            bytes[letters..].iter().all(u8::is_ascii_digit),
            "Invalid row coordinate."
        );
        let column = bytes[..letters].iter().fold(0u32, |column, byte| {
            column * 26 + u32::from(*byte - b'A' + 1)
        });
        let row = address[letters..].parse::<u32>()?;
        ensure!(
            column <= 16_384 && (1..=1_048_576).contains(&row),
            "Inspection range exceeds the Excel grid."
        );
        Ok((row, column))
    };
    let first = parse(coordinates.next().context("Missing range")?)?;
    if let Some(last) = coordinates.next() {
        let last = parse(last)?;
        ensure!(
            first.0 <= last.0 && first.1 <= last.1,
            "Reversed inspection range."
        );
    }
    ensure!(
        coordinates.next().is_none(),
        "Expected one rectangular range."
    );
    Ok(())
}

impl Inspector {
    fn select(
        &mut self,
        root: &Path,
        web: &Path,
        id: &str,
        sheet: &str,
        range: &str,
    ) -> Result<Value> {
        if self.directory.is_none() {
            let directory = std::env::temp_dir()
                .join("excel-shrink-inspection")
                .join(uuid::Uuid::new_v4().to_string());
            std::fs::create_dir_all(&directory)?;
            self.directory = Some(directory);
        }
        let directory = self.directory.as_ref().unwrap();
        if self
            .copies
            .as_ref()
            .is_none_or(|(current, _, _)| current != id)
        {
            let inputs = directory.join(uuid::Uuid::new_v4().to_string());
            let (original, default) = inspection::copy_comparison_pair(root, id, &inputs)?;
            self.copies = Some((id.to_owned(), original, default));
        }
        if self.broker.is_none() {
            self.broker = Some(Broker::start(directory, web)?);
        }
        let (_, original, default) = self.copies.as_ref().unwrap();
        let request = json!({"id":uuid::Uuid::new_v4().to_string(), "original":original, "default":default, "sheet":sheet, "range":range});
        match self.broker.as_mut().unwrap().request(&request) {
            Ok(mut response) => {
                response["source"] = json!("prepared_read_only_copies");
                Ok(response)
            }
            Err(error) => {
                // A stalled COM call cannot retain invisible inspection processes.
                self.broker.take();
                Err(error)
            }
        }
    }
}

impl Broker {
    #[cfg(windows)]
    fn start(directory: &Path, web: &Path) -> Result<Self> {
        use std::os::windows::process::CommandExt;
        let source = web
            .parent()
            .context("Inspection helper directory missing")?;
        for name in [
            "inspect-region-pair.ps1",
            "InspectionWindows.cs",
            "CaptureProcesses.cs",
        ] {
            std::fs::copy(source.join(name), directory.join(name))?;
        }
        let job = crate::process_job::Job::new()?;
        let mut child = Command::new("powershell.exe")
            .args(["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(directory.join("inspect-region-pair.ps1"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(std::fs::File::create(
                directory.join("helper.stderr.log"),
            )?))
            .creation_flags(0x08000000)
            .spawn()
            .context("Start visible Excel inspection helper")?;
        if let Err(error) = job.add(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let input = BufWriter::new(child.stdin.take().context("Inspection input missing")?);
        let output = child.stdout.take().context("Inspection output missing")?;
        let (send, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let response = line
                    .map_err(anyhow::Error::from)
                    .and_then(|line| Ok(serde_json::from_str(&line)?));
                if send.send(response).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            responses,
            _job: job,
        })
    }

    #[cfg(not(windows))]
    fn start(_directory: &Path, _web: &Path) -> Result<Self> {
        anyhow::bail!("Visible Excel region inspection requires Windows.")
    }

    fn request(&mut self, request: &Value) -> Result<Value> {
        serde_json::to_writer(&mut self.input, request)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let response = self
            .responses
            .recv_timeout(Duration::from_secs(180))
            .context("Excel inspection did not respond within 180 seconds")??;
        ensure!(
            response["id"] == request["id"],
            "Inspection response identity changed."
        );
        ensure!(
            response["ok"] == true,
            "Excel inspection: {}",
            response["error"]
        );
        Ok(response)
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Inspector {
    fn drop(&mut self) {
        self.broker.take();
        if let Some(directory) = &self.directory {
            let expected = std::env::temp_dir().join("excel-shrink-inspection");
            if directory.parent() == Some(expected.as_path()) {
                if let Err(error) = std::fs::remove_dir_all(directory) {
                    tracing::warn!(path=%directory.display(),%error,"Cannot remove closed inspection copies");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercise visible windows, selection reuse, peer survival and owner cleanup.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires desktop Excel and INSPECTION_PAIR_BUNDLE with Sheet1"]
    fn native_inspection_reuses_peers_and_reopens_only_the_closed_process() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("inspection-pair-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("prepared"))?;
        std::fs::create_dir(root.join("control-room"))?;
        let bundle = std::env::var("INSPECTION_PAIR_BUNDLE")?;
        for id in ["one", "two"] {
            std::fs::copy(&bundle, root.join(format!("prepared/{id}.zip")))?;
        }
        let web = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("office-capture/control-room");
        let mut inspector = Inspector::default();
        let first = inspector.select(&root.join("control-room"), &web, "one", "Sheet1", "A1:B3")?;
        let second =
            inspector.select(&root.join("control-room"), &web, "one", "Sheet1", "B2:C4")?;
        for side in ["original", "default"] {
            assert_eq!(first[side]["pid"], second[side]["pid"]);
            assert_eq!(second[side]["read_only"], true);
            assert_eq!(second[side]["range"], "$B$2:$C$4");
        }
        assert_ne!(first["original"]["pid"], first["default"]["pid"]);
        platform::terminate_verified_process(
            first["original"]["pid"].as_i64().unwrap(),
            first["original"]["process_started_unix_ms"]
                .as_i64()
                .unwrap(),
        )?;
        let reopened =
            inspector.select(&root.join("control-room"), &web, "one", "Sheet1", "C3:D5")?;
        assert_ne!(first["original"]["pid"], reopened["original"]["pid"]);
        assert_eq!(first["default"]["pid"], reopened["default"]["pid"]);
        let switched =
            inspector.select(&root.join("control-room"), &web, "two", "Sheet1", "A1:C6")?;
        assert_eq!(reopened["original"]["pid"], switched["original"]["pid"]);
        assert_eq!(reopened["default"]["pid"], switched["default"]["pid"]);
        drop(inspector);
        for side in ["original", "default"] {
            let started = std::time::Instant::now();
            while crate::native_process::process_alive(
                switched[side]["pid"].as_i64().unwrap(),
                switched[side]["process_started_unix_ms"].as_i64().unwrap(),
            ) != Some(false)
            {
                ensure!(
                    started.elapsed() < Duration::from_secs(5),
                    "Inspection Excel survived its owner"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        std::fs::write(
            root.join("result.json"),
            serde_json::to_vec_pretty(
                &json!({"first":first,"reopened":reopened,"switched":switched,"owned_processes_closed":true}),
            )?,
        )?;
        println!("{}", root.display());
        Ok(())
    }

    #[test]
    fn inspection_accepts_only_sheet_names_and_rectangular_grid_addresses() {
        for range in ["A1", "B2:XFD1048576"] {
            assert!(validate_region("Sheet 1", range).is_ok());
        }
        for range in [
            "A0", "XFE1", "A1048577", "B2:A1", "A1,B2", "[book]A1", "A1:B2:C3", "A1;cmd",
        ] {
            assert!(validate_region("Sheet 1", range).is_err(), "{range}");
        }
        assert!(validate_region("", "A1").is_err());
    }
}
