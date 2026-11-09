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

#[test]
fn idle_pool_polls_coalesce_heartbeat_writes() -> Result<()> {
    let root = std::env::temp_dir().join(format!("pool-heartbeat-{}", uuid::Uuid::new_v4()));
    let mut pool = Pool::new(root.clone())?;
    let heartbeat = root.join("heartbeat");
    // Keep the publication deadline fixed while polling an unchanged pool.
    pool.last_publish = Instant::now() + Duration::from_secs(60);
    fs::write(&heartbeat, b"unchanged-probe")?;
    pool.reconcile(0, 512)?;
    assert_eq!(fs::read(&heartbeat)?, b"unchanged-probe");
    pool.last_publish = Instant::now() - Duration::from_secs(2);
    pool.reconcile(0, 512)?;
    assert_eq!(fs::read(&heartbeat)?, b"running");
    Ok(())
}
