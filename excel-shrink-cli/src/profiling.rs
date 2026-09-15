//! Chrome Trace Event export for Perfetto. Independent of console/file log levels.
//! Only coarse `excel_shrink::profile` spans/events are recorded. The bounded queue
//! never blocks workers; an overflow is explicitly reported in the trace and stderr.
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Instant;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

pub use excel_shrink_core::profiling::TARGET;

enum Message {
    Event(Value),
    Finish,
}

struct Shared {
    sender: mpsc::SyncSender<Message>,
    origin: Instant,
    threads: Mutex<HashMap<ThreadId, u64>>,
    dropped: AtomicU64,
}

impl Shared {
    fn timestamp(&self) -> f64 {
        self.origin.elapsed().as_secs_f64() * 1_000_000.0
    }

    fn thread_id(&self) -> u64 {
        let current = thread::current();
        let mut threads = self.threads.lock().unwrap();
        if let Some(id) = threads.get(&current.id()) {
            return *id;
        }
        let id = threads.len() as u64 + 1;
        threads.insert(current.id(), id);
        self.emit(
            json!({"ph":"M", "name":"thread_name", "pid":std::process::id(),
            "tid":id, "args":{"name":current.name().map(str::to_owned)
                .unwrap_or_else(|| format!("worker {id}"))}}),
        );
        id
    }

    fn emit(&self, value: Value) {
        if self.sender.try_send(Message::Event(value)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub struct TraceLayer(Arc<Shared>);

pub struct TraceGuard {
    shared: Arc<Shared>,
    writer: Option<JoinHandle<io::Result<()>>>,
}

impl TraceGuard {
    pub fn finish(mut self) -> io::Result<()> {
        self.close()
    }

    fn close(&mut self) -> io::Result<()> {
        let Some(writer) = self.writer.take() else {
            return Ok(());
        };
        let _ = self.shared.sender.send(Message::Finish);
        let result = writer
            .join()
            .map_err(|_| io::Error::other("trace writer panicked"))?;
        let dropped = self.shared.dropped.load(Ordering::Relaxed);
        if dropped > 0 {
            eprintln!("Profiling trace is incomplete: {dropped} events dropped. Reduce the workload or trace detail.");
        }
        result
    }
}

impl Drop for TraceGuard {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("Failed to finish profiling trace: {error}");
        }
    }
}

pub fn create(path: &Path) -> io::Result<(TraceLayer, TraceGuard)> {
    let expanded = path
        .to_str()
        .filter(|s| s.contains("{timestamp}"))
        .map(|s| {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            std::path::PathBuf::from(
                s.replace("{timestamp}", &format!("{stamp}-{}", std::process::id())),
            )
        });
    let path = expanded.as_deref().unwrap_or(path);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    // Refuse to overwrite an input workbook, a source file, or an earlier recording.
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let (sender, receiver) = mpsc::sync_channel(8192);
    let shared = Arc::new(Shared {
        sender,
        origin: Instant::now(),
        threads: Mutex::new(HashMap::new()),
        dropped: AtomicU64::new(0),
    });
    let state = shared.clone();
    let writer = thread::Builder::new()
        .name("profile-writer".into())
        .spawn(move || write_trace(file, receiver, &state))?;
    eprintln!("Profiling trace: {}", path.display());
    Ok((
        TraceLayer(shared.clone()),
        TraceGuard {
            shared,
            writer: Some(writer),
        },
    ))
}

fn write_trace(file: File, receiver: mpsc::Receiver<Message>, shared: &Shared) -> io::Result<()> {
    let mut writer = BufWriter::new(file);
    writer.write_all(b"{\"traceEvents\":[\n")?;
    while let Ok(Message::Event(event)) = receiver.recv() {
        serde_json::to_writer(&mut writer, &event)?;
        writer.write_all(b",\n")?;
    }
    serde_json::to_writer(
        &mut writer,
        &json!({"ph":"M", "name":"process_name",
        "pid":std::process::id(), "args":{"name":"excel-shrink"}}),
    )?;
    write!(writer, "\n],\"displayTimeUnit\":\"ms\",\"otherData\":{{\"dropped_events\":{},\"timing\":\"wall time, not CPU time\"}}}}",
        shared.dropped.load(Ordering::Relaxed))?;
    writer.flush()
}

#[derive(Default)]
struct Fields(Map<String, Value>);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().into(), json!(format!("{value:?}")));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), json!(value));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().into(), json!(value));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().into(), json!(value));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().into(), json!(value));
    }
}

struct SpanData {
    name: String,
    args: Map<String, Value>,
    // A span may be entered on multiple threads or re-entered on the same thread.
    starts: Vec<(u64, f64)>,
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for TraceLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).unwrap();
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        fields.0.insert("span_id".into(), json!(id.into_u64()));
        if let Some(parent) = span.parent() {
            fields
                .0
                .insert("parent_span_id".into(), json!(parent.id().into_u64()));
        }
        let name = fields
            .0
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or(attrs.metadata().name())
            .to_owned();
        span.extensions_mut().insert(SpanData {
            name,
            args: fields.0,
            starts: Vec::new(),
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        values.record(&mut fields);
        ctx.span(id)
            .unwrap()
            .extensions_mut()
            .get_mut::<SpanData>()
            .unwrap()
            .args
            .extend(fields.0);
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        let tid = self.0.thread_id();
        ctx.span(id)
            .unwrap()
            .extensions_mut()
            .get_mut::<SpanData>()
            .unwrap()
            .starts
            .push((tid, self.0.timestamp()));
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        let tid = self.0.thread_id();
        let end = self.0.timestamp();
        let span = ctx.span(id).unwrap();
        let mut extensions = span.extensions_mut();
        let data = extensions.get_mut::<SpanData>().unwrap();
        if let Some(index) = data.starts.iter().rposition(|(thread, _)| *thread == tid) {
            let (_, start) = data.starts.remove(index);
            self.0.emit(json!({"ph":"X", "cat":"pipeline", "name":data.name,
                "pid":std::process::id(), "tid":tid, "ts":start, "dur":end-start, "args":data.args}));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        if let Some(span) = ctx.event_span(event) {
            fields
                .0
                .insert("span_id".into(), json!(span.id().into_u64()));
        }
        let name = fields
            .0
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("event")
            .to_owned();
        let tid = self.0.thread_id();
        let ts = self.0.timestamp();
        if name == "counter" || name == "active_workbooks" {
            let counter_name = fields
                .0
                .get("counter_name")
                .and_then(Value::as_str)
                .unwrap_or(&name);
            self.0
                .emit(json!({"ph":"C", "cat":"resources", "name":counter_name,
                "pid":std::process::id(), "tid":0, "ts":ts, "args":{"value":fields.0["value"]}}));
        } else if name == "flow_start" || name == "flow_end" {
            // Anchor both endpoints to zero-duration slices on the emitting threads.
            self.0.emit(json!({"ph":"X", "cat":"handoff", "name":name,
                "pid":std::process::id(), "tid":tid, "ts":ts, "dur":0, "args":fields.0}));
            let mut flow = json!({"ph":if name == "flow_start" {"s"} else {"f"},
                "cat":"handoff", "name":"handoff", "id":fields.0["flow_id"],
                "pid":std::process::id(), "tid":tid, "ts":ts});
            if name == "flow_end" {
                flow["bp"] = json!("e");
            }
            self.0.emit(flow);
        } else {
            self.0
                .emit(json!({"ph":"i", "s":"t", "cat":"metrics", "name":name,
                "pid":std::process::id(), "tid":tid, "ts":ts, "args":fields.0}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use excel_shrink_core::profiling::{drop_optional, drop_value, phase, Handoff, ResourceGuard};
    use tracing_subscriber::prelude::*;

    struct DropProbe(Arc<AtomicU64>);
    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
            tracing::info!(target: "excel_shrink::profile", event = "probe_destructor");
        }
    }

    fn trace_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "excel-shrink-{label}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn trace_roundtrip_preserves_parallel_handoffs_nested_spans_and_resource_lifetimes() {
        // Exercise the same single global subscriber as the CLI. Isolate it from
        // parallel unit tests that register disabled tracing callsites on Rayon.
        if std::env::var_os("EXCEL_SHRINK_PROFILE_TEST_CHILD").is_none() {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "profiling::tests::trace_roundtrip_preserves_parallel_handoffs_nested_spans_and_resource_lifetimes", "--nocapture"])
                .env("EXCEL_SHRINK_PROFILE_TEST_CHILD", "1")
                .output().unwrap();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        let path = trace_path("trace");
        let (layer, guard) = create(&path).unwrap();
        let subscriber = tracing_subscriber::registry().with(layer.with_filter(
            tracing_subscriber::filter::filter_fn(|m| m.target() == TARGET),
        ));
        let dispatch = tracing::Dispatch::new(subscriber);
        tracing::dispatcher::set_global_default(dispatch.clone()).unwrap();
        let destructions = Arc::new(AtomicU64::new(0));
        tracing::dispatcher::with_default(&dispatch, || {
            let _outer = phase("parent", "quoted\"workbook.xlsx", "");
            let link = Handoff::start("test_handoff", "quoted\"workbook.xlsx", "sheet1");
            let dispatcher = dispatch.clone();
            let destructions = destructions.clone();
            std::thread::spawn(move || {
                tracing::dispatcher::with_default(&dispatcher, || {
                    let _child = phase("child", "quoted\"workbook.xlsx", "sheet1");
                    link.finish();
                    let _bytes = ResourceGuard::add("test_queued_bytes", 4096);
                    let _nested = phase("nested", "", "");
                    drop_value("test.probe", DropProbe(destructions));
                    let closing_span = phase("closing_span", "", "");
                    drop_value("test.closing_span", closing_span);
                    drop_optional::<String>("test.absent", None);
                    tracing::info!("ordinary logs must not appear in the trace");
                })
            })
            .join()
            .unwrap();
        });
        assert_eq!(destructions.load(Ordering::Relaxed), 1);
        guard.finish().unwrap();
        let trace: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let events = trace["traceEvents"].as_array().unwrap();
        assert_eq!(trace["otherData"]["dropped_events"], 0);
        let parent = events.iter().find(|e| e["name"] == "parent").unwrap();
        let child = events.iter().find(|e| e["name"] == "child").unwrap();
        let nested = events.iter().find(|e| e["name"] == "nested").unwrap();
        assert_ne!(parent["tid"], child["tid"]);
        assert_eq!(parent["args"]["xlsx"], "quoted\"workbook.xlsx");
        assert_eq!(nested["args"]["parent_span_id"], child["args"]["span_id"]);
        assert!(child["dur"].as_f64().unwrap() >= nested["dur"].as_f64().unwrap());
        let start = events.iter().find(|e| e["ph"] == "s").unwrap();
        let end = events.iter().find(|e| e["ph"] == "f").unwrap();
        assert_eq!(start["id"], end["id"]);
        assert_ne!(start["tid"], end["tid"]);
        let values: Vec<_> = events
            .iter()
            .filter(|e| e["name"] == "test_queued_bytes")
            .map(|e| e["args"]["value"].as_u64().unwrap())
            .collect();
        assert_eq!(values, [4096, 0]);
        for resource in ["test.probe", "test.closing_span"] {
            let before = events
                .iter()
                .position(|e| e["name"] == "before_drop" && e["args"]["resource"] == resource)
                .unwrap();
            let after = events
                .iter()
                .position(|e| e["name"] == "after_drop" && e["args"]["resource"] == resource)
                .unwrap();
            assert!(before < after);
            assert_eq!(
                events[before]["args"]["drop_id"],
                events[after]["args"]["drop_id"]
            );
            assert!(events[after]["args"]["elapsed_ns"].as_u64().is_some());
            if resource == "test.probe" {
                let body = events
                    .iter()
                    .position(|e| e["name"] == "probe_destructor")
                    .unwrap();
                assert!(before < body && body < after);
            } else {
                // Dropping an entered span changes scope; the ID must still pair.
                assert_ne!(
                    events[before]["args"]["span_id"],
                    events[after]["args"]["span_id"]
                );
            }
        }
        assert!(!events
            .iter()
            .any(|e| e["args"]["resource"] == "test.absent"));
        assert!(!events
            .iter()
            .any(|e| e.to_string().contains("ordinary logs")));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn existing_trace_or_input_file_is_never_overwritten() {
        let path = trace_path("existing");
        std::fs::write(&path, b"original workbook or trace").unwrap();
        assert!(create(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original workbook or trace");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn overflowing_queue_is_counted_without_blocking() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let shared = Shared {
            sender,
            origin: Instant::now(),
            threads: Mutex::new(HashMap::new()),
            dropped: AtomicU64::new(0),
        };
        shared.emit(json!({"ph":"i"}));
        shared.emit(json!({"ph":"i"}));
        assert_eq!(shared.dropped.load(Ordering::Relaxed), 1);
    }
}
