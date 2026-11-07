use super::*;

#[test]
fn ram_reserve_has_a_512_mib_floor() {
    assert!(!has_startup_reserve(512 * 1_048_576 - 1, 0));
    assert!(has_startup_reserve(512 * 1_048_576, 0));
    assert!(!has_startup_reserve(768 * 1_048_576, 1024));
}

#[test]
fn a_session_lease_is_exclusive_and_released_on_drop() -> Result<()> {
    let root = std::env::temp_dir().join(format!("conversion-lease-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root)?;
    let first = open_lease(&root)?;
    first.try_lock()?;
    let second = open_lease(&root)?;
    assert!(second.try_lock().is_err());
    drop(first);
    second.try_lock()?;
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
        pool.workers.len() == 2 && pool.workers.iter().all(worker::Worker::ready)
    })?;
    let original: Vec<_> = pool
        .workers
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
                let pool_root = pool.root.clone();
                handles
                    .push(scope.spawn(move || client::convert(&pool_root, &input, &output, &work)));
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
        let pool_root = pool.root.clone();
        let input = fixtures.join("password.xls");
        let output = root.join("password.xlsx");
        let work = root.join("password");
        let handle = scope.spawn(move || client::convert(&pool_root, &input, &output, &work));
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
        pool.workers.len() == 2 && pool.workers.iter().all(worker::Worker::ready)
    })?;
    let survivors = pool
        .workers
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
        pool.workers
            .iter()
            .filter_map(|worker| read_json(&worker.directory.join("identity.json"))),
    );
    pump_until(&mut pool, 1, |pool| {
        pool.workers.len() == 1 && !pool.workers[0].is_retiring()
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

fn pump_until(pool: &mut Pool, target: usize, ready: impl Fn(&Pool) -> bool) -> Result<()> {
    let started = Instant::now();
    while !ready(pool) {
        if let Err(error) = pool.reconcile(target, 512) {
            pool.abort(&error)?;
            return Err(error);
        }
        if started.elapsed() >= Duration::from_secs(150) {
            let error = anyhow::anyhow!("native pool test timed out: {}", pool.root.display());
            pool.abort(&error)?;
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
