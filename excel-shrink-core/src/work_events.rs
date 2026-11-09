//! Optional coarse work observations. Disabled runs perform only an atomic check.
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

static ENABLED: AtomicBool = AtomicBool::new(false);
static OBSERVER: Mutex<Option<Arc<dyn Observer>>> = Mutex::new(None);
static NEXT_TASK: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
pub enum TaskKind {
    SheetInitial,
    SheetFinalize,
    Chunk {
        index: usize,
        count: usize,
        range: &'static str,
    },
}

pub struct Task<'a> {
    pub id: u64,
    pub workbook: &'a str,
    pub sheet: &'a str,
    pub worker: Option<usize>,
    pub kind: TaskKind,
}

pub trait Observer: Send + Sync {
    fn workbook_started(&self, input: &Path, output: &Path);
    fn workbook_finished(
        &self,
        input: &Path,
        output: &Path,
        written: bool,
        status: &str,
        reason: Option<&str>,
    );
    fn task_started(&self, task: &Task<'_>);
    fn task_finished(&self, id: u64);
    fn should_stop(&self) -> bool;
}

pub struct Registration;

/// One observed cleanup owns the process-wide hook until all its work has joined.
pub fn register(observer: Arc<dyn Observer>) -> Result<Registration, &'static str> {
    let mut current = OBSERVER.lock().map_err(|_| "Work observer lock poisoned")?;
    if current.is_some() {
        return Err("An observed cleanup is already active");
    }
    *current = Some(observer);
    ENABLED.store(true, Ordering::Release);
    Ok(Registration)
}

impl Drop for Registration {
    fn drop(&mut self) {
        ENABLED.store(false, Ordering::Release);
        *OBSERVER.lock().expect("Work observer lock poisoned") = None;
    }
}

pub fn with_observer(action: impl FnOnce(&dyn Observer)) {
    if let Some(observer) = observer() {
        action(observer.as_ref());
    }
}

fn observer() -> Option<Arc<dyn Observer>> {
    if !ENABLED.load(Ordering::Acquire) {
        return None;
    }
    OBSERVER
        .lock()
        .expect("Work observer lock poisoned")
        .clone()
}

pub fn should_stop() -> bool {
    observer().is_some_and(|observer| observer.should_stop())
}

pub struct TaskGuard(Option<(Arc<dyn Observer>, u64)>);

impl TaskGuard {
    pub fn start(workbook: &str, sheet: &str, kind: TaskKind) -> Self {
        let Some(observer) = observer() else {
            return Self(None);
        };
        let id = NEXT_TASK.fetch_add(1, Ordering::Relaxed);
        observer.task_started(&Task {
            id,
            workbook,
            sheet,
            worker: rayon::current_thread_index(),
            kind,
        });
        Self(Some((observer, id)))
    }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        if let Some((observer, id)) = &self.0 {
            observer.task_finished(*id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorded(Mutex<Vec<(u64, Option<usize>, bool)>>);

    impl Observer for Recorded {
        fn workbook_started(&self, _: &Path, _: &Path) {}
        fn workbook_finished(&self, _: &Path, _: &Path, _: bool, _: &str, _: Option<&str>) {}
        fn task_started(&self, task: &Task<'_>) {
            if task.workbook == "work-events-probe" {
                self.0.lock().unwrap().push((task.id, task.worker, false));
            }
        }
        fn task_finished(&self, id: u64) {
            if let Some(task) = self.0.lock().unwrap().iter_mut().find(|task| task.0 == id) {
                task.2 = true;
            }
        }
        fn should_stop(&self) -> bool {
            false
        }
    }

    #[test]
    fn scoped_observer_records_real_rayon_workers_and_balanced_task_lifetimes() {
        let observed = Arc::new(Recorded::default());
        let registration = register(observed.clone()).unwrap();
        assert!(register(observed.clone()).is_err());
        rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap()
            .install(|| {
                rayon::scope(|scope| {
                    for _ in 0..4 {
                        scope.spawn(|_| {
                            let _task = TaskGuard::start(
                                "work-events-probe",
                                "sheet1.xml",
                                TaskKind::SheetInitial,
                            );
                        });
                    }
                });
            });
        drop(registration);
        let _disabled = TaskGuard::start("work-events-probe", "sheet1.xml", TaskKind::SheetInitial);
        let tasks = observed.0.lock().unwrap();
        assert_eq!(tasks.len(), 4);
        assert!(tasks
            .iter()
            .all(|(_, worker, done)| worker.is_some_and(|worker| worker < 2) && *done));
        let ids: std::collections::HashSet<_> = tasks.iter().map(|task| task.0).collect();
        assert_eq!(ids.len(), 4);
    }
}
