//! Optional observations and explicit per-run resource configuration.
use anyhow::Result;
use corpus_runner::{
    memory::MemoryReserve,
    telemetry::{Events, Observer, Operation},
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

pub struct RunContext {
    pub events: Events,
    pub memory_reserve: Option<MemoryReserve>,
    observation_error: Mutex<Option<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use corpus_runner::telemetry::Event;

    struct BrokenObserver;

    impl Observer for BrokenObserver {
        fn publish(&self, _: &Event) -> Result<()> {
            anyhow::bail!("event storage unavailable")
        }
    }

    #[test]
    fn observer_error_is_retained_until_the_run_boundary() {
        let context = RunContext::observed(Operation::XlsxConvert, Arc::new(BrokenObserver));
        context.observe("job.completed", Some("workbook"), Some(0), Value::Null);
        assert!(context
            .observation_error()
            .unwrap()
            .contains("event storage unavailable"));
        assert!(context.ensure_observed().is_err());
    }
}

impl RunContext {
    pub fn quiet(operation: Operation) -> Self {
        Self {
            events: Events::quiet(operation),
            memory_reserve: None,
            observation_error: Mutex::new(None),
        }
    }

    pub fn observed(operation: Operation, observer: Arc<dyn Observer>) -> Self {
        Self {
            events: Events::new(operation, observer),
            memory_reserve: None,
            observation_error: Mutex::new(None),
        }
    }

    /// Preserve native outcomes when an optional observer fails, then fail the run explicitly.
    pub fn observe(
        &self,
        event_type: &str,
        workbook_id: Option<&str>,
        worker_index: Option<usize>,
        payload: Value,
    ) {
        if let Err(error) = self
            .events
            .emit(event_type, workbook_id, worker_index, payload)
        {
            let mut retained = self
                .observation_error
                .lock()
                .expect("Observation error lock poisoned");
            if retained.is_none() {
                let message = format!("{error:#}");
                eprintln!("Telemetry publication failed: {message}");
                *retained = Some(message);
            }
        }
    }

    pub fn observation_error(&self) -> Option<String> {
        self.observation_error
            .lock()
            .expect("Observation error lock poisoned")
            .clone()
    }

    pub fn ensure_observed(&self) -> Result<()> {
        if let Some(error) = self.observation_error() {
            anyhow::bail!("Telemetry publication failed; native artifacts retained: {error}");
        }
        Ok(())
    }
}
