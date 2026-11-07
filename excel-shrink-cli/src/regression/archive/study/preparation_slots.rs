//! Process-owned stage permits; a crashed worker releases its file locks.
use super::control_room::{self, Controls};
use anyhow::{Context, Result};
use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::time::{Duration, Instant};

pub(super) enum Stage {
    Conversion,
    Shrink,
}

/// Wait for the live stage limit without retaining a permit from the prior stage.
pub(super) fn acquire(stage: Stage) -> Result<Option<File>> {
    let Some(root) = std::env::var_os("EXCEL_SHRINK_PREPARATION_CONTROL_DIR") else {
        return Ok(None);
    };
    let root = Path::new(&root);
    let counter = std::env::var_os("EXCEL_SHRINK_PREPARATION_WAIT").map(std::path::PathBuf::from);
    let prior = counter
        .as_deref()
        .and_then(|path| {
            fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        })
        .and_then(|value| value["elapsed_millis"].as_u64())
        .unwrap_or(0);
    let started = Instant::now();
    let mut last_update = Instant::now();
    let heartbeat = std::env::var_os("EXCEL_SHRINK_PARENT_HEARTBEAT");
    let mut waiting_announced = false;
    loop {
        if let Some(path) = &heartbeat {
            anyhow::ensure!(
                super::super::batch_cleanup::parent_alive(Path::new(path)),
                "preparation coordinator heartbeat expired"
            );
        }
        let controls = Controls::read(root)?;
        let (name, limit) = match stage {
            Stage::Conversion => ("conversion", controls.conversion_processes),
            Stage::Shrink => ("shrink", controls.prepare_processes),
        };
        let permit = try_acquire(root, name, limit)?;
        if permit.is_some() || last_update.elapsed() >= Duration::from_secs(1) {
            if let Some(path) = &counter {
                control_room::write(
                    path,
                    &serde_json::json!({"elapsed_millis":prior.saturating_add(started.elapsed().as_millis() as u64)}),
                )?;
            }
            last_update = Instant::now();
        }
        if let Some(permit) = permit {
            if waiting_announced {
                super::super::work_state::event(
                    name,
                    "stage_admitted",
                    "Preparation stage admitted",
                    false,
                    None,
                    serde_json::json!({"stage":name}),
                );
            }
            return Ok(Some(permit));
        }
        if !waiting_announced {
            super::super::work_state::event(
                name,
                "stage_capacity_wait",
                "Waiting for preparation stage capacity",
                false,
                Some("stage concurrency limit"),
                serde_json::json!({"stage":name,"limit":limit}),
            );
            waiting_announced = true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Serialize admission only, then retain one slot lock throughout the operation.
fn try_acquire(root: &Path, stage: &str, limit: usize) -> Result<Option<File>> {
    let directory = root.join("preparation-slots").join(stage);
    fs::create_dir_all(&directory)?;
    let admission = open_lock(&directory.join("admission.lock"))?;
    admission
        .lock()
        .context("lock preparation stage admission")?;
    let mut occupied = 0;
    let mut available = None;
    // Include old high-numbered slots when a live limit has been reduced.
    for entry in fs::read_dir(&directory)? {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "slot") {
            continue;
        }
        let file = open_lock(&path)?;
        match file.try_lock() {
            Ok(()) => {
                if available.is_none() {
                    available = Some(file);
                }
            }
            Err(std::fs::TryLockError::WouldBlock) => occupied += 1,
            Err(error) => return Err(error.into()),
        }
    }
    if occupied >= limit {
        return Ok(None);
    }
    if available.is_none() {
        let file = open_lock(&directory.join(format!("{}.slot", uuid::Uuid::new_v4())))?;
        file.lock()?;
        available = Some(file);
    }
    Ok(available)
}

fn open_lock(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_are_independent_and_reductions_drain_existing_slots() -> Result<()> {
        let root = std::env::temp_dir().join(format!("preparation-slots-{}", uuid::Uuid::new_v4()));
        let first = try_acquire(&root, "conversion", 2)?.unwrap();
        let second = try_acquire(&root, "conversion", 2)?.unwrap();
        assert!(try_acquire(&root, "conversion", 2)?.is_none());
        let shrink = try_acquire(&root, "shrink", 1)?.unwrap();
        drop(first);
        assert!(try_acquire(&root, "conversion", 1)?.is_none());
        drop(second);
        assert!(try_acquire(&root, "conversion", 1)?.is_some());
        drop(shrink);
        assert!(try_acquire(&root, "shrink", 1)?.is_some());
        Ok(())
    }
}
