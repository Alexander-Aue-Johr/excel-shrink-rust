use super::*;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{fs, path::{Path, PathBuf}, time::{Duration, Instant}};
fn read_json(path: &Path) -> Option<Value> {
    fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok())
}
fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    corpus_runner::artifacts::atomic_write(path, &serde_json::to_vec(value)?)
}


/// Measure workbook switching separately from process and Office.js readiness.
#[test]
#[ignore = "requires desktop Excel and NATIVE_REUSE_ORIGINAL/NATIVE_REUSE_SHRUNK; run serially"]
fn native_xlsx_pair_opens_in_the_same_owned_excel_process() -> Result<()> {
    let inputs = [
        PathBuf::from(std::env::var("NATIVE_REUSE_ORIGINAL")?),
        PathBuf::from(std::env::var("NATIVE_REUSE_SHRUNK")?),
    ];
    let source_hashes = inputs
        .iter()
        .map(|path| deltasieve_engine::reuse::file_sha256(path))
        .collect::<Result<Vec<_>>>()?;
    let root = std::env::temp_dir().join(format!("native-workbook-switch-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root)?;
    let startup = Instant::now();
    let mut pool = Pool::new(root.join("pool"))?;
    pump_until(&mut pool, 1, |pool| {
        pool.sessions().len() == 1 && pool.sessions()[0].ready
    })?;
    let startup_seconds = startup.elapsed().as_secs_f64();
    let identity = read_json(&pool.sessions()[0].directory.join("identity.json"))
        .context("Missing owned Excel identity")?;
    let mut samples = Vec::new();
    for round in 0..2 {
        for (index, input) in inputs.iter().enumerate() {
            let output = root.join(format!("round-{round}-{index}.xlsx"));
            let work = root.join(format!("request-{round}-{index}"));
            let metrics = std::thread::scope(|scope| -> Result<_> {
                let pool_root = pool.root().to_path_buf();
                let request = scope.spawn(move || convert(&pool_root, input, &output, &work));
                pump_until(&mut pool, 1, |_| request.is_finished())?;
                request.join().unwrap()
            })?;
            assert_eq!(metrics.excel_pid.map(u64::from), identity["pid"].as_u64());
            samples.push(serde_json::to_value(metrics)?);
        }
    }
    drop(pool);
    assert_eq!(
        crate::native_process::process_alive(
            identity["pid"].as_i64().context("Excel PID")?,
            identity["started_ms"].as_i64().context("Excel birth time")?
        ),
        Some(false)
    );
    for (input, hash) in inputs.iter().zip(source_hashes) {
        assert_eq!(deltasieve_engine::reuse::file_sha256(input)?, hash);
    }
    write_json(
        &root.join("test-results.json"),
        &json!({"scope":"COM workbook open in reused conversion worker; excludes Office.js",
            "worker_startup_seconds":startup_seconds,"identity":identity,"samples":samples,
            "source_hashes_unchanged":true,"owned_excel_exit_confirmed":true}),
    )?;
    println!("Workbook switch measurements: {}", root.display());
    Ok(())
}







/// Exercise reuse, independent sessions, error recycling, scale-down and cleanup.
#[test]
#[ignore = "requires desktop Excel and LEGACY_CONVERSION_FIXTURES; run serially"]
fn native_pool_reuses_excel_and_recovers_failed_sessions() -> Result<()> {
    let fixtures = PathBuf::from(std::env::var("LEGACY_CONVERSION_FIXTURES")?);
    let root = std::env::temp_dir().join(format!(
        "persistent-conversion-test-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir(&root)?;
    let baseline = root.join("baseline.xlsx");
    super::super::excel_automation::convert_legacy_workbook(
        &fixtures.join("normal.xls"),
        &baseline,
        &root.join("baseline"),
    )?;
    let mut pool = Pool::new(root.join("pool"))?;
    pump_until(&mut pool, 2, |pool| {
        pool.sessions().len() == 2 && pool.sessions().iter().all(|worker| worker.ready)
    })?;
    let original: Vec<_> = pool
        .sessions()
        .iter()
        .map(|worker| read_json(&worker.directory.join("identity.json")).unwrap())
        .collect();
    assert_ne!(original[0]["pid"], original[1]["pid"]);
    let mut metrics = Vec::new();
    for round in 0..2 {
        let batch = std::thread::scope(|scope| -> Result<Vec<_>> {
            let mut handles = Vec::new();
            for index in 0..2 {
                let input = fixtures.join("normal.xls");
                let output = root.join(format!("round-{round}-{index}.xlsx"));
                let work = root.join(format!("round-{round}-{index}"));
                let pool_root = pool.root().to_path_buf();
                handles
                    .push(scope.spawn(move || convert(&pool_root, &input, &output, &work)));
            }
            pump_until(&mut pool, 2, |_| {
                handles.iter().all(|handle| handle.is_finished())
            })?;
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect()
        })?;
        metrics.extend(batch);
        for index in 0..2 {
            let comparison = super::super::xlsx_value_compare::compare_workbook_preservation(
                &baseline,
                &root.join(format!("round-{round}-{index}.xlsx")),
                super::super::xlsx_value_compare::MissingCellPolicy::strict(),
                None,
            )?;
            assert_eq!(comparison.difference_count, 0);
        }
    }
    assert!(metrics
        .iter()
        .any(|metrics| metrics.workbooks_converted.is_some_and(|count| count > 1)));
    assert!(metrics.iter().all(|metrics| original
        .iter()
        .any(|identity| identity["pid"].as_u64() == metrics.excel_pid.map(u64::from))));
    std::thread::scope(|scope| -> Result<()> {
        let pool_root = pool.root().to_path_buf();
        let input = fixtures.join("password.xls");
        let output = root.join("password.xlsx");
        let work = root.join("password");
        let handle = scope.spawn(move || convert(&pool_root, &input, &output, &work));
        pump_until(&mut pool, 2, |_| handle.is_finished())?;
        assert!(handle
            .join()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("password_required"));
        assert!(!root.join("password.xlsx").exists());
        Ok(())
    })?;
    pump_until(&mut pool, 2, |pool| {
        pool.sessions().len() == 2 && pool.sessions().iter().all(|worker| worker.ready)
    })?;
    let survivors = pool
        .sessions()
        .iter()
        .filter(|worker| {
            let current = read_json(&worker.directory.join("identity.json")).unwrap();
            original.iter().any(|identity| identity == &current)
        })
        .count();
    assert_eq!(
        survivors, 1,
        "one failed session should be replaced; its peer must survive"
    );
    let mut identities = original;
    identities.extend(
        pool.sessions()
            .iter()
            .filter_map(|worker| read_json(&worker.directory.join("identity.json"))),
    );
    pump_until(&mut pool, 1, |pool| {
        pool.sessions().len() == 1 && !pool.sessions()[0].retiring
    })?;
    drop(pool);
    for identity in identities {
        assert_eq!(
            crate::native_process::process_alive(
                identity["pid"].as_i64().unwrap(),
                identity["started_ms"].as_i64().unwrap()
            ),
            Some(false)
        );
    }
    write_json(
        &root.join("test-results.json"),
        &json!({"metrics":metrics,"value_differences":0,"password_rejected":true,"owned_processes_closed":true}),
    )?;
    println!("{}", root.display());
    Ok(())
}

/// Replay a shared XLS with independently checked XLSX values and formulas.
#[test]
#[ignore = "requires LEGACY_SHARED_FIXTURE containing shared.xls and expected.xlsx"]
fn native_shared_revision_history_converts_without_changing_inputs() -> Result<()> {
    let fixtures = PathBuf::from(std::env::var("LEGACY_SHARED_FIXTURE")?);
    let input = fixtures.join("shared.xls");
    let original = fs::read(&input)?;
    let root =
        std::env::temp_dir().join(format!("shared-conversion-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root)?;
    let mut pool = Pool::new(root.join("pool"))?;
    let output = root.join("converted.xlsx");
    let metrics = std::thread::scope(|scope| -> Result<_> {
        let pool_root = pool.root().to_path_buf();
        let work = root.join("conversion");
        let result_path = output.clone();
        let source_path = input.clone();
        let handle =
            scope.spawn(move || convert(&pool_root, &source_path, &result_path, &work));
        pump_until(&mut pool, 1, |_| handle.is_finished())?;
        handle.join().unwrap()
    })?;
    assert_eq!(metrics.shared_editing_removed, Some(true));
    assert_eq!(
        original,
        fs::read(&input)?,
        "conversion must not modify its input"
    );
    let comparison = super::super::xlsx_value_compare::compare_workbook_preservation(
        &fixtures.join("expected.xlsx"),
        &output,
        super::super::xlsx_value_compare::MissingCellPolicy::strict(),
        None,
    )?;
    assert_eq!(comparison.difference_count, 0);
    drop(pool);
    write_json(
        &root.join("test-results.json"),
        &json!({"metrics":metrics,"value_differences":0,"source_unchanged":true}),
    )?;
    println!("{}", root.display());
    Ok(())
}

fn pump_until(pool: &mut Pool, target: usize, ready: impl Fn(&Pool) -> bool) -> Result<()> {
    let started = Instant::now();
    while !ready(pool) {
        if let Err(error) = pool.reconcile(target, 512) {
            pool.abort(&error)?;
            return Err(error);
        }
        if started.elapsed() >= Duration::from_secs(150) {
            let error = anyhow::anyhow!("native pool test timed out: {}", pool.root().display());
            pool.abort(&error)?;
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
