use super::*;
use std::{io::Write, time::Duration};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sheet-scope-process-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn source(&self, names: &[&str]) -> Result<MaterializedSource> {
        let root = self.0.join("input");
        fs::create_dir(&root)?;
        for name in names {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, name.as_bytes())?;
        }
        sources::discover(&root)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture_stage(input: InputKind, output: OutputKind, action: &str) -> Stage {
    Stage {
        name: "fixture".into(),
        program: std::env::current_exe().unwrap(),
        arguments: vec![
            "--ignored".into(), "--exact".into(),
            "validation_process::tests::processor_fixture".into(), "--nocapture".into(),
        ],
        input,
        output,
        output_extension: None,
        timeout_seconds: 30,
        working_directory: None,
        dependencies: Vec::new(),
        environment: BTreeMap::from([
            ("SHEET_SCOPE_TEST_ACTION".into(), action.into()),
            ("SHEET_SCOPE_TEST_INPUT".into(), "{input}".into()),
            ("SHEET_SCOPE_TEST_OUTPUT".into(), "{output}".into()),
        ]),
    }
}

/// Subprocess fixture, deliberately invoked through the current test executable.
/// This exercises argv, timeout, logs and process ownership without requiring an
/// installed converter, shell or Excel. `cargo test --ignored` alone is a no-op.
#[test]
#[ignore = "subprocess fixture for processing orchestration tests"]
fn processor_fixture() {
    let Ok(action) = std::env::var("SHEET_SCOPE_TEST_ACTION") else { return };
    let input = PathBuf::from(std::env::var_os("SHEET_SCOPE_TEST_INPUT").unwrap());
    let output = PathBuf::from(std::env::var_os("SHEET_SCOPE_TEST_OUTPUT").unwrap());
    println!("fixture standard output");
    eprintln!("fixture standard error");
    std::io::stdout().flush().unwrap();
    std::io::stderr().flush().unwrap();
    if action == "sleep" {
        std::thread::sleep(Duration::from_secs(30));
        return;
    }
    if action == "fail" {
        std::process::exit(7);
    }
    let _counter = ConcurrentFixture::enter();
    if action == "copy" {
        fs::copy(input, output).unwrap();
        return;
    }
    if action == "directory" || action == "partial" || action == "nonzero_partial" {
        let inputs = sources::discover(&input).unwrap();
        for (index, (name, path)) in inputs.files.iter().enumerate() {
            if index > 0 && action != "directory" {
                break;
            }
            let target = output.join(name);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(path, target).unwrap();
        }
        if action == "nonzero_partial" {
            std::process::exit(7);
        }
        return;
    }
    if action == "archive" {
        let temporary = TestDirectory::new();
        let unpacked = sources::extract_archive(&input, &temporary.0.join("unpacked")).unwrap();
        sources::archive_files(&unpacked.files, &output).unwrap();
        return;
    }
    panic!("Unknown processor fixture action: {action}");
}

struct ConcurrentFixture(Option<PathBuf>);

impl ConcurrentFixture {
    fn enter() -> Self {
        let path = std::env::var_os("SHEET_SCOPE_TEST_COUNTER").map(PathBuf::from);
        if let Some(path) = &path {
            Self::change(path, 1);
            std::thread::sleep(Duration::from_millis(150));
        }
        Self(path)
    }

    fn change(path: &Path, amount: i32) {
        let lock = fs::OpenOptions::new().write(true).create(true).truncate(false)
            .open(path.with_extension("lock")).unwrap();
        lock.lock().unwrap();
        let mut counts: (i32, i32) = fs::read(path).ok()
            .map(|bytes| serde_json::from_slice(&bytes).unwrap()).unwrap_or((0, 0));
        counts.0 += amount;
        counts.1 = counts.1.max(counts.0);
        fs::write(path, serde_json::to_vec(&counts).unwrap()).unwrap();
    }
}

impl Drop for ConcurrentFixture {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            Self::change(path, -1);
        }
    }
}

#[test]
fn file_processors_preserve_names_write_directly_and_respect_the_process_bound() -> Result<()> {
    let test = TestDirectory::new();
    let inputs = test.source(&["a.xlsx", "nested/b.xlsx", "other/c.xlsx", "d.xlsx"])?;
    let output = test.0.join("output");
    let logs = test.0.join("logs");
    let counter = test.0.join("concurrency.json");
    let mut stage = fixture_stage(InputKind::File, OutputKind::File, "copy");
    stage.environment.insert("SHEET_SCOPE_TEST_COUNTER".into(), counter.to_string_lossy().into_owned());
    let mut progress = crate::validation::ProcessingProgress::new("fixture".into(), 1, 4);
    let mut observe = |outcome: &ProcessOutcome, pid: Option<u32>| -> Result<()> {
        if outcome.status == ProcessStatus::Running { assert!(pid.is_some()); }
        progress.observe(outcome, pid);
        assert!(progress.running.len() <= 2);
        Ok(())
    };
    let result = run_stage(&stage, &inputs, &output, &logs, 2, false, Some(&mut observe))?;
    assert_eq!(progress.completed, 4);
    assert_eq!(progress.succeeded, 4);
    assert!(progress.running.is_empty());
    assert_eq!(progress.recent.len(), 4);
    assert_eq!(result.failed, 0);
    assert_eq!(result.source.files.len(), 4);
    for (name, path) in &result.source.files {
        assert_eq!(*path, fs::canonicalize(&output)?.join(name));
        assert_eq!(fs::read(path)?, name.as_bytes());
    }
    let (active, maximum): (i32, i32) = serde_json::from_slice(&fs::read(counter)?)?;
    assert_eq!(active, 0);
    assert!((1..=2).contains(&maximum));
    assert!(fs::read_to_string(logs.join(&result.outcomes[0].stdout_log))?.contains("fixture standard output"));
    assert!(fs::read_to_string(logs.join(&result.outcomes[0].stderr_log))?.contains("fixture standard error"));
    Ok(())
}

#[test]
fn failed_live_status_publication_stops_and_closes_the_owned_process() -> Result<()> {
    let test = TestDirectory::new();
    let inputs = test.source(&["a.xlsx"])?;
    let stage = fixture_stage(InputKind::File, OutputKind::File, "sleep");
    let logs = test.0.join("logs");
    let mut statuses = Vec::new();
    let mut observe = |outcome: &ProcessOutcome, _pid: Option<u32>| -> Result<()> {
        statuses.push(outcome.status);
        if outcome.status == ProcessStatus::Running { anyhow::bail!("Live status publication failed") }
        Ok(())
    };
    let error = run_stage(&stage, &inputs, &test.0.join("output"), &logs, 1, false, Some(&mut observe)).unwrap_err();
    assert!(format!("{error:#}").contains("Live status publication failed"));
    assert_eq!(statuses, vec![ProcessStatus::Running, ProcessStatus::Failed]);
    let receipt: ProcessOutcome = serde_json::from_slice(&fs::read(logs.join("invocations/000001/outcome.json"))?)?;
    assert_eq!(receipt.status, ProcessStatus::Failed);
    assert!(receipt.finished_at.is_some());
    assert!(receipt.error.unwrap().contains("owned process stopped"));
    Ok(())
}

#[test]
fn failing_processes_keep_diagnostics_and_stop_future_dispatch() -> Result<()> {
    let test = TestDirectory::new();
    let inputs = test.source(&["a.xlsx", "b.xlsx", "c.xlsx"])?;
    let logs = test.0.join("logs");
    let stage = fixture_stage(InputKind::File, OutputKind::File, "fail");
    assert!(run_stage(&stage, &inputs, &test.0.join("output"), &logs, 1, false, None).is_err());
    let record: serde_json::Value = serde_json::from_slice(&fs::read(logs.join("stage.json"))?)?;
    let outcomes: Vec<ProcessOutcome> = serde_json::from_value(record["outcomes"].clone())?;
    assert_eq!(outcomes.len(), 3);
    assert_eq!(outcomes[0].status, ProcessStatus::Failed);
    assert_eq!(outcomes[0].exit_code, Some(7));
    assert_eq!(outcomes[1].status, ProcessStatus::Skipped);
    assert_eq!(outcomes[2].status, ProcessStatus::Skipped);
    assert!(fs::read_to_string(logs.join(&outcomes[0].stderr_log))?.contains("fixture standard error"));
    assert!(inputs.files.values().all(|path| path.is_file()));
    Ok(())
}

#[test]
fn successful_batch_process_can_continue_with_only_present_outputs() -> Result<()> {
    let test = TestDirectory::new();
    let inputs = test.source(&["a.xlsx", "b.xlsx"])?;
    let stage = fixture_stage(InputKind::Directory, OutputKind::Directory, "partial");
    let result = run_stage(&stage, &inputs, &test.0.join("output"), &test.0.join("logs"), 4, true, None)?;
    assert_eq!(result.outcomes.len(), 1);
    assert_eq!(result.outcomes[0].status, ProcessStatus::Incomplete);
    assert_eq!(result.source.files.len(), 1);
    assert!(result.source.files.contains_key("a.xlsx"));
    assert_eq!(result.outcomes[0].missing_outputs, ["b.xlsx"]);
    Ok(())
}

#[test]
fn nonzero_batch_exit_does_not_certify_partially_written_files() -> Result<()> {
    let test = TestDirectory::new();
    let inputs = test.source(&["a.xlsx", "b.xlsx"])?;
    let stage = fixture_stage(InputKind::Directory, OutputKind::Directory, "nonzero_partial");
    let result = run_stage(&stage, &inputs, &test.0.join("output"), &test.0.join("logs"), 4, true, None)?;
    assert_eq!(result.failed, 1);
    assert_eq!(result.outcomes[0].produced_outputs, ["a.xlsx"]);
    assert!(result.source.files.is_empty());
    Ok(())
}

#[test]
fn archive_processors_receive_one_zip_and_publish_named_outputs() -> Result<()> {
    let test = TestDirectory::new();
    let inputs = test.source(&["a.xlsx", "nested/b.xlsx"])?;
    let stage = fixture_stage(InputKind::Archive, OutputKind::Archive, "archive");
    let result = run_stage(&stage, &inputs, &test.0.join("output"), &test.0.join("logs"), 4, false, None)?;
    assert_eq!(result.outcomes.len(), 1);
    assert_eq!(result.source.files.len(), 2);
    assert_eq!(fs::read(&result.source.files["nested/b.xlsx"])? , b"nested/b.xlsx");
    assert!(fs::read_dir(&test.0)?.all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".sheet-scope-stage-")));
    Ok(())
}

#[test]
fn timeout_is_recorded_without_publishing_an_output() -> Result<()> {
    let test = TestDirectory::new();
    let inputs = test.source(&["a.xlsx"])?;
    let mut stage = fixture_stage(InputKind::File, OutputKind::File, "sleep");
    stage.timeout_seconds = 1;
    let result = run_stage(&stage, &inputs, &test.0.join("output"), &test.0.join("logs"), 1, true, None)?;
    assert_eq!(result.outcomes[0].status, ProcessStatus::TimedOut);
    assert!(result.outcomes[0].elapsed_ms < 20_000);
    assert!(result.source.files.is_empty());
    Ok(())
}

#[test]
fn changed_extensions_reject_collisions_before_launching_a_processor() -> Result<()> {
    let test = TestDirectory::new();
    let inputs = test.source(&["a.xls", "a.xlsx"])?;
    let mut stage = fixture_stage(InputKind::File, OutputKind::File, "copy");
    stage.output_extension = Some("xlsx".into());
    assert!(run_stage(&stage, &inputs, &test.0.join("output"), &test.0.join("logs"), 2, true, None).is_err());
    assert!(!test.0.join("logs").exists());
    assert_eq!(output_name("nested/old.xls", Some(".xlsx"))?, "nested/old.xlsx");
    Ok(())
}

#[test]
fn stale_outputs_and_in_place_destinations_are_rejected() -> Result<()> {
    let test = TestDirectory::new();
    let inputs = test.source(&["a.xlsx"])?;
    let stage = fixture_stage(InputKind::File, OutputKind::File, "copy");
    assert!(run_stage(&stage, &inputs, &inputs.root, &test.0.join("logs"), 1, false, None).is_err());
    let output = test.0.join("output");
    fs::create_dir(&output)?;
    fs::write(output.join("a.xlsx"), b"keep this output")?;
    assert!(run_stage(&stage, &inputs, &output, &test.0.join("logs"), 1, false, None).is_err());
    assert_eq!(fs::read(output.join("a.xlsx"))?, b"keep this output");
    Ok(())
}

#[test]
fn directory_followup_sees_only_accepted_workbooks() -> Result<()> {
    let test = TestDirectory::new();
    let mut inputs = test.source(&["a.xlsx", "b.xlsx"])?;
    inputs.files.remove("b.xlsx");
    let stage = fixture_stage(InputKind::Directory, OutputKind::Directory, "directory");
    let result = run_stage(&stage, &inputs, &test.0.join("output"), &test.0.join("logs"), 1, false, None)?;
    assert_eq!(result.source.files.len(), 1);
    assert!(!result.source.root.join("b.xlsx").exists());
    assert!(inputs.root.join("b.xlsx").exists());
    Ok(())
}

#[test]
fn cleanup_uncertainty_survives_context_and_preserves_owned_scratch() -> Result<()> {
    let test = TestDirectory::new();
    let scratch = ScratchDirectory::create(&test.0)?;
    let retained = scratch.path.clone();
    fs::write(retained.join("interrupted.xlsx"), b"working evidence")?;
    let error: anyhow::Error = UnconfirmedTermination { detail: "fixture".into() }.into();
    let error = error.context("stage failed");
    assert!(error.is::<UnconfirmedTermination>());
    scratch.preserve();
    assert_eq!(fs::read(retained.join("interrupted.xlsx"))?, b"working evidence");
    Ok(())
}
