//! Shared SheetShrink execution with coarse workbook, sheet and chunk observations.
use crate::{pipeline, Cli};
use anyhow::{ensure, Context, Result};
use clap::Parser;
use corpus_runner::{
    memory::{self, MemoryReserve},
    package_sizes,
};
use excel_automation::context::RunContext;
use excel_shrink_core::work_events::{self, Observer, Task, TaskKind};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub struct Options {
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub recursive: bool,
    pub threads: usize,
    pub max_active_workbooks: usize,
    pub parallel_range_chunk_size: usize,
}

struct Bridge {
    context: Arc<RunContext>,
    inputs: HashMap<String, (String, String)>,
    active_tasks: Mutex<HashMap<u64, (String, Option<usize>)>>,
    reserve: MemoryReserve,
    memory: Mutex<(Instant, Option<String>)>,
    failed: AtomicUsize,
}

impl Bridge {
    fn input(&self, path: &str) -> Option<&(String, String)> {
        self.inputs.get(path)
    }
}

impl Observer for Bridge {
    fn workbook_started(&self, input: &Path, _: &Path) {
        let key = input.display().to_string();
        let Some((id, name)) = self.input(&key) else {
            return;
        };
        self.context
            .observe("job.started", Some(id), None, json!({"name": name}));
        let metadata = package_sizes::inspect(input);
        let payload = match metadata {
            Ok(package) => {
                json!({"package": package, "sheet_names": sheet_names(input).unwrap_or_default()})
            }
            Err(error) => json!({"error": format!("{error:#}")}),
        };
        self.context
            .observe("workbook.metadata", Some(id), None, payload);
    }

    fn workbook_finished(
        &self,
        input: &Path,
        output: &Path,
        written: bool,
        status: &str,
        reason: Option<&str>,
    ) {
        let key = input.display().to_string();
        let Some((id, name)) = self.input(&key) else {
            return;
        };
        let sizes = if written {
            package_sizes::inspect(input).and_then(|before| {
            Ok(json!({"before_optimization": before, "after_optimization": package_sizes::inspect(output)?}))
        })
        } else {
            Err(anyhow::anyhow!(
                "{status}: {}",
                reason.unwrap_or("No output workbook was published")
            ))
        };
        let (kind, payload) = match sizes {
            Ok(sizes) => (
                "job.completed",
                json!({"name": name, "archive_sizes": sizes, "output": output}),
            ),
            Err(error) => {
                self.failed.fetch_add(1, Ordering::Relaxed);
                (
                    "job.failed",
                    json!({"name": name, "error": format!("{error:#}")}),
                )
            }
        };
        self.context.observe(kind, Some(id), None, payload);
    }

    fn task_started(&self, task: &Task<'_>) {
        let Some((id, _)) = self.input(task.workbook) else {
            return;
        };
        let (phase, chunk_index, chunk_count, range) = match task.kind {
            TaskKind::SheetInitial => ("initial", None, 1, None),
            TaskKind::SheetFinalize => ("finalize", None, 1, None),
            TaskKind::Chunk {
                index,
                count,
                range,
            } => ("chunk", Some(index), count, Some(range)),
        };
        self.active_tasks
            .lock()
            .unwrap()
            .insert(task.id, (id.clone(), task.worker));
        self.context.observe(
            "work.started",
            Some(id),
            task.worker,
            json!({
                "task_id": task.id, "sheet": task.sheet, "phase": phase,
                "chunk_index": chunk_index, "chunk_count": chunk_count, "range": range,
            }),
        );
    }

    fn task_finished(&self, id: u64) {
        if let Some((workbook, worker)) = self.active_tasks.lock().unwrap().remove(&id) {
            self.context.observe(
                "work.finished",
                Some(&workbook),
                worker,
                json!({"task_id": id}),
            );
        }
    }

    fn should_stop(&self) -> bool {
        if self.context.should_stop() {
            return true;
        }
        let mut memory = self.memory.lock().unwrap();
        if memory.0.elapsed() < Duration::from_millis(100) {
            return memory.1.is_some();
        }
        memory.0 = Instant::now();
        if let Err(error) = memory::snapshot()
            .map_err(anyhow::Error::from)
            .and_then(|snapshot| self.reserve.ensure_available(snapshot))
        {
            memory.1 = Some(format!("{error:#}"));
            self.context.stop_handle().store(true, Ordering::Release);
            return true;
        }
        false
    }
}

fn sheet_names(path: &Path) -> Result<HashMap<String, String>> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let mut parts = Vec::new();
    for name in ["xl/workbook.xml", "xl/_rels/workbook.xml.rels"] {
        let mut entry = zip.by_name(name)?;
        ensure!(
            entry.size() <= 8 * 1024 * 1024,
            "Workbook name metadata exceeds the display limit"
        );
        let mut xml = Vec::new();
        entry.read_to_end(&mut xml)?;
        parts.push(xml);
    }
    let targets = pipeline::extract_rid_to_target(&parts[1]);
    Ok(pipeline::extract_sheet_id_name_and_rid_in_order(&parts[0])
        .into_iter()
        .filter_map(|(_, name, rid)| targets.get(&rid).map(|part| (part.clone(), name)))
        .collect())
}

pub fn run(options: Options, context: RunContext) -> Result<()> {
    ensure!(
        !options.output.exists(),
        "Choose a new SheetShrink output directory"
    );
    ensure!(
        options.max_active_workbooks > 0,
        "Active workbook limit must be positive"
    );
    let inputs =
        corpus_runner::sources::expand_paths(&options.inputs, &["xlsx"], options.recursive)?;
    let mut seen = HashSet::new();
    let inputs = inputs
        .into_iter()
        .map(|path| {
            ensure!(
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx")),
                "SheetShrink inputs must use XLSX"
            );
            let path = std::fs::canonicalize(path)?;
            ensure!(
                seen.insert(path.clone()),
                "The same input path was selected more than once"
            );
            Ok(path)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut arguments = vec![
        "sheetshrink".into(),
        inputs[0].as_os_str().to_owned(),
        options.output.as_os_str().to_owned(),
    ];
    for (flag, value) in [
        ("--rayon-threads", options.threads),
        ("--writer-workers", options.threads),
        ("--max-active-workbooks", options.max_active_workbooks),
        (
            "--parallel-range-chunk-size",
            options.parallel_range_chunk_size,
        ),
    ] {
        arguments.extend([flag.into(), value.to_string().into()]);
    }
    let mut cli = Cli::try_parse_from(arguments)?;
    cli.batch_report = Some(options.output.join("preparation.json"));
    let pairs: Vec<_> = inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            Ok((
                input.clone(),
                options
                    .output
                    .join(format!("input-{index:06}"))
                    .join(input.file_name().context("Input filename missing")?),
            ))
        })
        .collect::<Result<_>>()?;
    let identities = inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            (
                input.display().to_string(),
                (
                    format!("workbook-{index}"),
                    input.file_name().unwrap().to_string_lossy().into_owned(),
                ),
            )
        })
        .collect();
    let available = memory::snapshot()?;
    let reserve = match context.memory_reserve {
        Some(reserve) => reserve,
        None => MemoryReserve::configured(available)?,
    };
    reserve.ensure_available(available)?;
    let context = Arc::new(context);
    let bridge = Arc::new(Bridge {
        context: Arc::clone(&context),
        inputs: identities,
        active_tasks: Mutex::new(HashMap::new()),
        reserve,
        memory: Mutex::new((Instant::now(), None)),
        failed: AtomicUsize::new(0),
    });
    let registration = work_events::register(bridge.clone()).map_err(anyhow::Error::msg)?;
    context.observe("run.started", None, None, json!({"total": inputs.len(), "work_unit": "sheet", "sheet_jobs": true, "size_reduction": true}));
    let result = pipeline::process_file_pairs(pairs, &cli);
    drop(registration);
    let failed = bridge.failed.load(Ordering::Relaxed);
    let control_error = bridge.memory.lock().unwrap().1.clone();
    context.observe(
        if context.should_stop() {
            "run.stopped"
        } else if result.is_err() || failed > 0 {
            "run.failed"
        } else {
            "run.completed"
        },
        None,
        None,
        Value::Null,
    );
    if let Some(error) = control_error {
        anyhow::bail!("{error}");
    }
    ensure!(
        !context.should_stop(),
        "SheetShrink stopped at a work boundary"
    );
    result?;
    context.ensure_observed()?;
    ensure!(
        failed == 0,
        "{failed} SheetShrink outputs were not published"
    );
    Ok(())
}
