//! Optional workbook events for CLI logs, web projections and later replay.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Operation {
    XlsxConvert,
    CheckPerformance,
    SheetShrink,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Event {
    pub version: u32,
    pub sequence: u64,
    pub elapsed_millis: u64,
    pub operation: Operation,
    pub event_type: String,
    pub workbook_id: Option<String>,
    pub worker_index: Option<usize>,
    pub payload: Value,
}

/// Implementations must return promptly; rendering and network delivery belong to the consumer.
pub trait Observer: Send + Sync {
    fn publish(&self, event: &Event) -> Result<()>;
}

#[derive(Default)]
struct NoEvents;

impl Observer for NoEvents {
    fn publish(&self, _event: &Event) -> Result<()> {
        Ok(())
    }
}

pub struct JsonLines {
    file: Mutex<File>,
}

impl JsonLines {
    pub fn create(path: &Path) -> Result<Self> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
}

impl Observer for JsonLines {
    fn publish(&self, event: &Event) -> Result<()> {
        let mut bytes = serde_json::to_vec(event)?;
        bytes.push(b'\n');
        self.file
            .lock()
            .map_err(|_| anyhow::anyhow!("Event log lock poisoned"))?
            .write_all(&bytes)?;
        Ok(())
    }
}

/// Sequence assignment and publication share a lock, preserving order across workers.
pub struct Events {
    operation: Operation,
    started: Instant,
    sequence: Mutex<u64>,
    observer: Arc<dyn Observer>,
}

impl Events {
    pub fn new(operation: Operation, observer: Arc<dyn Observer>) -> Self {
        Self {
            operation,
            started: Instant::now(),
            sequence: Mutex::new(0),
            observer,
        }
    }

    pub fn quiet(operation: Operation) -> Self {
        Self::new(operation, Arc::new(NoEvents))
    }

    pub fn emit(
        &self,
        event_type: &str,
        workbook_id: Option<&str>,
        worker_index: Option<usize>,
        payload: Value,
    ) -> Result<()> {
        let mut sequence = self
            .sequence
            .lock()
            .map_err(|_| anyhow::anyhow!("Event sequence lock poisoned"))?;
        let event = Event {
            version: 1,
            sequence: *sequence,
            elapsed_millis: self
                .started
                .elapsed()
                .as_millis()
                .try_into()
                .context("Event clock overflow")?,
            operation: self.operation,
            event_type: event_type.to_owned(),
            workbook_id: workbook_id.map(str::to_owned),
            worker_index,
            payload,
        };
        self.observer.publish(&event)?;
        *sequence += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_events_retain_sequence_order() -> Result<()> {
        let path = std::env::temp_dir().join(format!("events-{}.jsonl", uuid::Uuid::new_v4()));
        let events = Events::new(Operation::XlsxConvert, Arc::new(JsonLines::create(&path)?));
        std::thread::scope(|scope| {
            for worker in 0..4 {
                let events = &events;
                scope.spawn(move || {
                    for _ in 0..10 {
                        events
                            .emit("workbook.started", Some("book"), Some(worker), Value::Null)
                            .unwrap();
                    }
                });
            }
        });
        drop(events);
        for (index, line) in std::fs::read_to_string(&path)?.lines().enumerate() {
            let event: Event = serde_json::from_str(line)?;
            assert_eq!(event.sequence, index as u64);
        }
        std::fs::remove_file(path)?;
        Ok(())
    }
}
