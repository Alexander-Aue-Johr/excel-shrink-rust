//! Keep resident Excel sessions alive until every workbook in a batch is done.
use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub(in crate::regression) const ENV: &str = "EXCEL_SHRINK_BATCH_CLEANUP";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::regression) struct Participant {
    pub ready_path: PathBuf,
    pub release_path: PathBuf,
    pub owner_pid: u32,
    pub timeout_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_path: Option<PathBuf>,
    #[serde(default)]
    pub manual_control: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_path: Option<PathBuf>,
    #[serde(skip)]
    wait_clock: Arc<Mutex<Option<opening_pause::OpeningPause>>>,
    // Parent thread clones share cancellation. Serialized worker descriptions
    // get a fresh flag; their parent terminates the owned child on cancellation.
    #[serde(skip)]
    cancelled: Arc<AtomicBool>,
}

impl Participant {
    // A user restart stops work at the next boundary, but still waits for peer
    // cleanup. Fatal coordinator cancellation deliberately bypasses that wait.
    pub fn request_restart(&self) -> Result<()> {
        create_marker(
            self.restart_path
                .as_deref()
                .context("restart marker unavailable")?,
        )
    }

    pub fn restart_requested(&self) -> bool {
        self.restart_path.as_deref().is_some_and(Path::is_file)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

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
        let counter = self
            .pause_path
            .as_deref()
            .and_then(opening_pause::read_counter);
        create_marker_with_contents(
            &self.ready_path,
            &serde_json::to_vec(&json!({"opening_pause_millis":counter}))?,
        )
    }

    pub fn waiting_expired(&self) -> bool {
        // Manual sessions may be paused by the user. Parent liveness and per-job
        // deadlines remain enforced; a completed peer must survive that pause.
        if self.manual_control {
            return false;
        }
        let Ok(mut state) = self.wait_clock.lock() else {
            return true;
        };
        if state.is_none() {
            let Some(age) = self
                .ready_path
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|time| time.elapsed().ok())
            else {
                return false;
            };
            // Legacy or malformed markers establish no historical credit.
            // New Rust and launcher markers share this counter baseline.
            let baseline = fs::File::open(&self.ready_path).ok().and_then(|file| {
                let mut bytes = Vec::new();
                file.take(4097).read_to_end(&mut bytes).ok()?;
                if bytes.len() > 4096 {
                    return None;
                }
                serde_json::from_slice::<Value>(&bytes).ok()?["opening_pause_millis"].as_u64()
            });
            *state = Some(opening_pause::OpeningPause::from_elapsed(
                self.pause_path.as_deref(),
                age,
                baseline,
            ));
        }
        state.as_mut().unwrap().active_elapsed() > Duration::from_secs(self.timeout_seconds)
    }

    pub fn deadline_paused(&self) -> bool {
        self.ready() && !self.waiting_expired()
    }

    pub fn wait(&self, heartbeat: Option<&Path>, refresh_heartbeat: bool) -> Result<()> {
        ensure!(
            !self.cancelled(),
            "resident comparison cancelled by coordinator"
        );
        self.mark_ready()?;
        while !self.released() {
            ensure!(
                !self.cancelled(),
                "resident comparison cancelled by coordinator"
            );
            ensure!(
                !self.waiting_expired(),
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
    create_marker_with_contents(path, b"ready\n")
}

fn create_marker_with_contents(path: &Path, contents: &[u8]) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }
    let pending =
        artifact_cache::unique_path(path.parent().context("marker has no parent")?, "ready")?;
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        // Publish the baseline together with readiness. Replacing an existing
        // marker would incorrectly reset every observer's cleanup deadline.
        let published = fs::hard_link(&pending, path);
        match published {
            Ok(()) => Ok(()),
            Err(_) if path.is_file() => Ok(()),
            Err(error) => Err(error.into()),
        }
    })();
    let _ = fs::remove_file(&pending);
    result
}

pub(super) struct Batch {
    directory: PathBuf,
    participants: Vec<Participant>,
    admission_open: bool,
    timeout_seconds: u64,
}

impl Batch {
    pub fn new(output: &Path, count: usize, workbook_timeout_seconds: u64) -> Result<Self> {
        let mut batch = Self::growing(output, workbook_timeout_seconds)?;
        for _ in 0..count {
            batch.add_participant()?;
        }
        batch.close_admission();
        Ok(batch)
    }

    pub fn growing(output: &Path, workbook_timeout_seconds: u64) -> Result<Self> {
        let root = output.join(".work").join("batches");
        fs::create_dir_all(&root)?;
        let directory = artifact_cache::unique_path(&root, "cleanup")?;
        fs::create_dir(&directory)?;
        Ok(Self {
            directory,
            participants: Vec::new(),
            admission_open: true,
            timeout_seconds: workbook_timeout_seconds.saturating_add(60),
        })
    }

    pub fn add_participant(&mut self) -> Result<Participant> {
        ensure!(self.admission_open, "comparison wave is already draining");
        let index = self.participants.len();
        let participant = Participant {
            ready_path: self.directory.join(format!("{index}.ready")),
            release_path: self.directory.join("release"),
            owner_pid: std::process::id(),
            timeout_seconds: self.timeout_seconds,
            pause_path: Some(self.directory.join(format!("{index}.pause.json"))),
            manual_control: false,
            restart_path: Some(self.directory.join(format!("{index}.restart"))),
            wait_clock: Arc::default(),
            cancelled: Arc::default(),
        };
        artifact_cache::atomic_write(
            participant.pause_path.as_ref().unwrap(),
            br#"{"elapsed_millis":0}"#,
        )?;
        self.participants.push(participant.clone());
        Ok(participant)
    }

    pub fn add_manual_participant(&mut self) -> Result<Participant> {
        let mut participant = self.add_participant()?;
        participant.manual_control = true;
        *self.participants.last_mut().unwrap() = participant.clone();
        Ok(participant)
    }

    pub fn close_admission(&mut self) {
        self.admission_open = false;
    }

    pub fn admission_open(&self) -> bool {
        self.admission_open
    }

    pub fn participant(&self, index: usize) -> Participant {
        self.participants[index].clone()
    }

    pub fn release_if_ready(&self) -> Result<()> {
        if !self.admission_open && self.participants.iter().all(Participant::ready) {
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
    fn user_restart_is_visible_in_worker_but_waits_for_peer_cleanup() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "restart-cleanup")?;
        fs::create_dir(&root)?;
        let mut batch = Batch::growing(&root, 60)?;
        let first = batch.add_manual_participant()?;
        let second = batch.add_manual_participant()?;
        let worker: Participant = serde_json::from_slice(&serde_json::to_vec(&first)?)?;
        batch.close_admission();
        first.request_restart()?;
        assert!(worker.restart_requested());
        assert!(!worker.cancelled());
        first.mark_ready()?;
        batch.release_if_ready()?;
        assert!(
            !first.released(),
            "restart must not close Excel under an active peer"
        );
        second.mark_ready()?;
        batch.release_if_ready()?;
        worker.wait(None, false)?;
        assert!(first.released());
        drop(batch);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn parent_cancellation_wakes_manual_waiters_without_releasing_the_batch() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "cancel-cleanup")?;
        fs::create_dir(&root)?;
        let mut batch = Batch::growing(&root, 3600)?;
        let participant = batch.add_manual_participant()?;
        let observer = participant.clone();
        let worker_copy: Participant = serde_json::from_slice(&serde_json::to_vec(&participant)?)?;
        let started = Instant::now();
        let outcome = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                observer.cancel();
            });
            participant.wait(None, false)
        });
        assert!(format!("{:#}", outcome.unwrap_err()).contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!participant.released());
        assert!(participant.cancelled() && observer.cancelled());
        assert!(
            !worker_copy.cancelled(),
            "parent-only cancellation is not persisted into worker JSON"
        );
        drop(batch);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn concurrent_readiness_preserves_the_first_complete_marker() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "cleanup-marker-race")?;
        fs::create_dir(&root)?;
        let marker = root.join("ready.json");
        let gate = std::sync::Barrier::new(16);
        let observations = std::thread::scope(|scope| -> Result<Vec<_>> {
            let handles: Vec<_> = (0..16)
                .map(|index| {
                    let marker = &marker;
                    let gate = &gate;
                    scope.spawn(move || -> Result<_> {
                        gate.wait();
                        create_marker_with_contents(
                            marker,
                            &serde_json::to_vec(&json!({"opening_pause_millis":index}))?,
                        )?;
                        Ok((fs::read(marker)?, marker.metadata()?.modified()?))
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect()
        })?;
        let expected = (fs::read(&marker)?, marker.metadata()?.modified()?);
        let parsed: Value = serde_json::from_slice(&expected.0)?;
        assert!(parsed["opening_pause_millis"].as_u64().unwrap() < 16);
        for observed in observations {
            assert_eq!(observed, expected);
        }
        assert_eq!(fs::read_dir(&root)?.count(), 1, "temporary markers leaked");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn first_ready_marker_credits_peer_startup_then_expires_with_a_frozen_counter() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "cleanup-pause")?;
        fs::create_dir(&root)?;
        let batch = Batch::new(&root, 2, 1)?;
        let mut first = batch.participant(0);
        first.timeout_seconds = 1;
        let pause = first.pause_path.as_ref().unwrap();
        artifact_cache::atomic_write(pause, br#"{"elapsed_millis":1000}"#)?;
        first.mark_ready()?;
        let original = fs::read(&first.ready_path)?;
        fs::File::options()
            .write(true)
            .open(&first.ready_path)?
            .set_times(
                fs::FileTimes::new()
                    .set_modified(std::time::SystemTime::now() - Duration::from_secs(3)),
            )?;
        let modified = first.ready_path.metadata()?.modified()?;
        artifact_cache::atomic_write(pause, br#"{"elapsed_millis":3500}"#)?;
        first.mark_ready()?;
        assert_eq!(fs::read(&first.ready_path)?, original);
        assert_eq!(first.ready_path.metadata()?.modified()?, modified);
        // A separately deserialized observer must also retain the earlier credit.
        let observer: Participant = serde_json::from_slice(&serde_json::to_vec(&first)?)?;
        assert!(!first.waiting_expired());
        assert!(!observer.waiting_expired());
        assert!(first.deadline_paused());
        std::thread::sleep(Duration::from_millis(650));
        assert!(first.waiting_expired());
        assert!(observer.waiting_expired());
        assert!(!first.deadline_paused());
        drop(batch);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn cleanup_wait_can_exceed_wall_deadline_during_bounded_peer_startup() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "cleanup-wait-pause")?;
        fs::create_dir(&root)?;
        let batch = Batch::new(&root, 2, 1)?;
        let mut first = batch.participant(0);
        first.timeout_seconds = 1;
        first.mark_ready()?;
        let outcome = std::thread::scope(|scope| {
            scope.spawn(|| -> Result<()> {
                let started = Instant::now();
                while started.elapsed() < Duration::from_millis(1300) {
                    artifact_cache::atomic_write(
                        first.pause_path.as_ref().unwrap(),
                        &serde_json::to_vec(
                            &json!({"elapsed_millis":started.elapsed().as_millis()}),
                        )?,
                    )?;
                    std::thread::sleep(Duration::from_millis(50));
                }
                create_marker(&first.release_path)
            });
            first.wait(None, false)
        });
        outcome?;
        assert!(first.released());
        drop(batch);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn old_ready_marker_cannot_claim_historical_pause_credit() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "cleanup-old-marker")?;
        fs::create_dir(&root)?;
        let batch = Batch::new(&root, 2, 1)?;
        let mut first = batch.participant(0);
        first.timeout_seconds = 1;
        create_marker(&first.ready_path)?;
        fs::File::options()
            .write(true)
            .open(&first.ready_path)?
            .set_times(
                fs::FileTimes::new()
                    .set_modified(std::time::SystemTime::now() - Duration::from_secs(3)),
            )?;
        artifact_cache::atomic_write(
            first.pause_path.as_ref().unwrap(),
            br#"{"elapsed_millis":999999999}"#,
        )?;
        assert!(first.waiting_expired());
        drop(batch);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn growing_wave_cannot_release_before_admission_closes_or_add_after_cleanup() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "growing-cleanup")?;
        fs::create_dir(&root)?;
        let mut batch = Batch::growing(&root, 1)?;
        batch.release_if_ready()?;
        assert!(!batch.directory.join("release").exists());
        let first = batch.add_participant()?;
        let pause: Value = serde_json::from_slice(&fs::read(first.pause_path.as_ref().unwrap())?)?;
        assert_eq!(pause["elapsed_millis"], 0);
        first.mark_ready()?;
        batch.release_if_ready()?;
        assert!(!first.released());
        let second = batch.add_participant()?;
        batch.close_admission();
        assert!(batch.add_participant().is_err());
        batch.release_if_ready()?;
        assert!(!first.released());
        second.mark_ready()?;
        batch.release_if_ready()?;
        assert!(first.released() && second.released());
        drop(batch);
        fs::remove_dir_all(root)?;
        Ok(())
    }

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
