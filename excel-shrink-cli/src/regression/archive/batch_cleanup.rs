//! Keep resident Excel sessions alive until every workbook in a batch is done.
use super::*;
use std::io::Write;

pub(in crate::regression) const ENV: &str = "EXCEL_SHRINK_BATCH_CLEANUP";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::regression) struct Participant {
    pub ready_path: PathBuf,
    pub release_path: PathBuf,
    pub owner_pid: u32,
    pub timeout_seconds: u64,
}

impl Participant {
    pub fn from_environment() -> Result<Option<Self>> {
        std::env::var_os(ENV)
            .map(|path| Ok(serde_json::from_slice(&fs::read(path)?)?))
            .transpose()
    }

    pub fn ready(&self) -> bool {
        self.ready_path.is_file()
    }

    pub fn released(&self) -> bool {
        self.release_path.is_file()
    }

    // Several owners can observe the same failure. Preserve the first timestamp
    // so repeated notifications never extend the bounded cleanup wait.
    pub fn mark_ready(&self) -> Result<()> {
        create_marker(&self.ready_path)
    }

    pub fn waiting_expired(&self) -> bool {
        self.ready_path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(self.timeout_seconds))
    }

    pub fn deadline_paused(&self) -> bool {
        self.ready() && !self.waiting_expired()
    }

    pub fn wait(&self, heartbeat: Option<&Path>, refresh_heartbeat: bool) -> Result<()> {
        self.mark_ready()?;
        let started = Instant::now();
        while !self.released() {
            ensure!(
                !self.waiting_expired()
                    && started.elapsed() < Duration::from_secs(self.timeout_seconds),
                "resident batch cleanup barrier timed out"
            );
            if let Some(heartbeat) = heartbeat {
                if refresh_heartbeat {
                    fs::write(heartbeat, b"waiting for batch cleanup")?;
                } else {
                    ensure!(
                        parent_alive(heartbeat),
                        "batch coordinator stopped responding"
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }

    pub fn wait_in_worker(&self) -> Result<()> {
        let heartbeat = std::env::var_os("EXCEL_SHRINK_PARENT_HEARTBEAT").map(PathBuf::from);
        self.wait(heartbeat.as_deref(), false)
    }
}

pub(super) fn parent_alive(heartbeat: &Path) -> bool {
    fs::metadata(heartbeat)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age < Duration::from_secs(60))
}

fn create_marker(path: &Path) -> Result<()> {
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(b"ready\n")?;
            file.sync_all()?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) struct Batch {
    directory: PathBuf,
    participants: Vec<Participant>,
}

impl Batch {
    pub fn new(output: &Path, count: usize, workbook_timeout_seconds: u64) -> Result<Self> {
        let root = output.join(".work").join("batches");
        fs::create_dir_all(&root)?;
        let directory = artifact_cache::unique_path(&root, "cleanup")?;
        fs::create_dir(&directory)?;
        let participants = (0..count)
            .map(|index| Participant {
                ready_path: directory.join(format!("{index}.ready")),
                release_path: directory.join("release"),
                owner_pid: std::process::id(),
                timeout_seconds: workbook_timeout_seconds.saturating_add(60),
            })
            .collect();
        Ok(Self {
            directory,
            participants,
        })
    }

    pub fn participant(&self, index: usize) -> Participant {
        self.participants[index].clone()
    }

    pub fn release_if_ready(&self) -> Result<()> {
        if self.participants.iter().all(Participant::ready) {
            create_marker(&self.directory.join("release"))?;
        }
        Ok(())
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        // Retain markers after a coordinator error: waiting children may still
        // need the release, and the next run uses a fresh directory.
        let _ = create_marker(&self.directory.join("release"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_peer_waits_for_failed_peer_and_marker_is_idempotent() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "cleanup-barrier")?;
        fs::create_dir(&root)?;
        let batch = Batch::new(&root, 2, 1)?;
        let first = batch.participant(0);
        first.mark_ready()?;
        let modified = first.ready_path.metadata()?.modified()?;
        first.mark_ready()?;
        assert_eq!(first.ready_path.metadata()?.modified()?, modified);
        batch.release_if_ready()?;
        assert!(!first.released());
        assert!(first.deadline_paused());
        // A launch failure or dead worker marks its slot before owned cleanup.
        batch.participant(1).mark_ready()?;
        batch.release_if_ready()?;
        first.wait(None, false)?;
        assert!(first.released());
        drop(batch);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn missing_parent_does_not_leave_a_ready_worker_waiting() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "cleanup-parent")?;
        fs::create_dir(&root)?;
        let batch = Batch::new(&root, 2, 1)?;
        let result = batch
            .participant(0)
            .wait(Some(&root.join("missing-heartbeat")), false);
        assert!(format!("{:#}", result.unwrap_err()).contains("coordinator stopped"));
        drop(batch);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
