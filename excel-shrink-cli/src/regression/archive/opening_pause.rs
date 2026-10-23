//! Credit only time spent waiting for another group's bounded Excel startup.
//!
//! The coordinator updates a cumulative counter for existing groups while a
//! new group opens. A frozen counter grants no further time, and neither this
//! clock nor its file changes parent liveness or memory limits.
use serde::Deserialize;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Deserialize)]
struct Measurement {
    elapsed_millis: u64,
}

pub(super) fn read_counter(path: &Path) -> Option<u64> {
    // A damaged or unexpectedly large telemetry file must not allocate an
    // unbounded buffer or disable the worker's deadline.
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(257)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 256 {
        return None;
    }
    serde_json::from_slice::<Measurement>(&bytes)
        .ok()
        .map(|measurement| measurement.elapsed_millis)
}

#[derive(Debug, Default)]
struct Credit {
    last_counter: Option<u64>,
    credited: Duration,
}

impl Credit {
    fn update(&mut self, wall_elapsed: Duration, counter: Option<u64>) -> Duration {
        if let Some(counter) = counter {
            if let Some(previous) = self.last_counter {
                if counter > previous {
                    self.credited = self
                        .credited
                        .saturating_add(Duration::from_millis(counter - previous))
                        .min(wall_elapsed);
                    self.last_counter = Some(counter);
                }
                // Ignore resets and out-of-order reads. Recounting an old
                // range could otherwise extend a deadline repeatedly.
            } else {
                // No valid baseline means no credit for earlier startup time.
                self.last_counter = Some(counter);
            }
        }
        wall_elapsed.saturating_sub(self.credited)
    }
}

#[derive(Debug)]
pub(in crate::regression) struct OpeningPause {
    started: Instant,
    elapsed_before: Duration,
    path: Option<PathBuf>,
    credit: Credit,
}

impl OpeningPause {
    pub fn new(path: Option<&Path>) -> Self {
        Self::from_elapsed(path, Duration::ZERO, path.and_then(read_counter))
    }

    // Cleanup observers can discover an existing ready marker after its wait
    // began. Retain that first marker's age and counter baseline exactly once.
    pub(super) fn from_elapsed(
        path: Option<&Path>,
        elapsed_before: Duration,
        baseline: Option<u64>,
    ) -> Self {
        Self {
            started: Instant::now(),
            elapsed_before,
            path: path.map(Path::to_path_buf),
            credit: Credit {
                last_counter: baseline,
                credited: Duration::ZERO,
            },
        }
    }

    pub fn active_elapsed(&mut self) -> Duration {
        let counter = self.path.as_deref().and_then(read_counter);
        self.credit.update(self.wall_elapsed(), counter)
    }

    pub fn wall_elapsed(&self) -> Duration {
        self.elapsed_before.saturating_add(self.started.elapsed())
    }

    pub fn credited(&self) -> Duration {
        self.credit.credited
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn credits_only_new_pause_time_and_stale_counter_cannot_hold_deadline() {
        let mut credit = Credit {
            last_counter: Some(1000),
            ..Credit::default()
        };
        assert_eq!(credit.update(millis(400), Some(1300)), millis(100));
        assert_eq!(credit.update(millis(900), Some(1500)), millis(400));
        assert_eq!(credit.update(millis(3000), Some(1500)), millis(2500));
        assert_eq!(credit.credited, millis(500));
    }

    #[test]
    fn missing_baseline_and_resets_do_not_grant_historical_or_repeated_credit() {
        let mut credit = Credit::default();
        assert_eq!(credit.update(millis(100), None), millis(100));
        assert_eq!(credit.update(millis(200), Some(5000)), millis(200));
        assert_eq!(credit.update(millis(400), Some(5200)), millis(200));
        assert_eq!(credit.update(millis(500), Some(100)), millis(300));
        assert_eq!(credit.update(millis(600), Some(5200)), millis(400));
        assert_eq!(credit.update(millis(700), None), millis(500));
        assert_eq!(credit.update(millis(800), Some(5300)), millis(500));
    }

    #[test]
    fn excessive_counter_credit_is_capped_and_never_carried_forward() {
        let mut credit = Credit {
            last_counter: Some(0),
            ..Credit::default()
        };
        assert_eq!(credit.update(millis(100), Some(u64::MAX)), Duration::ZERO);
        assert_eq!(credit.credited, millis(100));
        assert_eq!(credit.update(millis(900), Some(u64::MAX)), millis(800));
        assert_eq!(credit.update(millis(1500), Some(1)), millis(1400));
    }

    #[test]
    fn malformed_missing_oversized_or_overflowed_files_have_no_counter() -> anyhow::Result<()> {
        let root =
            super::super::artifact_cache::unique_path(&std::env::temp_dir(), "opening-pause")?;
        std::fs::create_dir(&root)?;
        let path = root.join("pause.json");
        assert_eq!(read_counter(&path), None);
        for invalid in [
            String::from("{"),
            String::from("{\"elapsed_millis\":-1}"),
            String::from("{\"elapsed_millis\":1.5}"),
            String::from("{\"elapsed_millis\":18446744073709551616}"),
            String::from("{\"elapsed_millis\":0}") + &" ".repeat(256),
        ] {
            std::fs::write(&path, invalid)?;
            assert_eq!(read_counter(&path), None);
        }
        std::fs::write(&path, "{\"elapsed_millis\":1250}")?;
        assert_eq!(read_counter(&path), Some(1250));
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
