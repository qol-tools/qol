#![cfg(unix)]

use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use qol_tray::features::plugin_store::installer::testing::{
    acquire_operation_lock, lockfile_path, OperationLockHandle,
};
use tempfile::TempDir;

const PLUGIN_ID: &str = "plugin-test";

fn plugins_root() -> TempDir {
    TempDir::new().expect("tempdir")
}

#[test]
fn acquire_then_drop_releases_lock_for_next_caller() {
    let root = plugins_root();
    let lock_path = lockfile_path(root.path(), PLUGIN_ID);

    let first = acquire_operation_lock(root.path(), PLUGIN_ID).expect("first acquire");
    assert!(lock_path.exists(), "lockfile must be on disk while held");

    drop(first);
    assert!(
        !lock_path.exists(),
        "lockfile must be removed by Drop: {}",
        lock_path.display()
    );

    let _second = acquire_operation_lock(root.path(), PLUGIN_ID).expect("second acquire");
    assert!(lock_path.exists(), "second acquisition recreates lockfile");
}

#[test]
fn concurrent_acquire_on_same_plugin_id_serializes_through_one_winner() {
    const THREADS: usize = 8;
    let root = Arc::new(plugins_root());
    let barrier = Arc::new(Barrier::new(THREADS));

    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let root = Arc::clone(&root);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || -> Result<OperationLockHandle, String> {
                barrier.wait();
                acquire_operation_lock(root.path(), PLUGIN_ID).map_err(|e| e.to_string())
            })
        })
        .collect();

    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let winners = results.iter().filter(|r| r.is_ok()).count();
    let losers = results.iter().filter(|r| r.is_err()).count();
    assert_eq!(
        winners, 1,
        "exactly one thread must win; got {winners} winners / {losers} losers"
    );
    let loser_msg = results
        .iter()
        .find_map(|r| r.as_ref().err())
        .expect("at least one loser");
    assert!(
        loser_msg.contains("already in progress"),
        "loser must report contention, got: {loser_msg}"
    );
}

#[test]
fn distinct_plugin_ids_can_be_locked_concurrently() {
    let root = plugins_root();

    let lock_a = acquire_operation_lock(root.path(), "plugin-a").expect("acquire a");
    let lock_b = acquire_operation_lock(root.path(), "plugin-b").expect("acquire b");

    assert!(lockfile_path(root.path(), "plugin-a").exists());
    assert!(lockfile_path(root.path(), "plugin-b").exists());

    drop(lock_a);
    drop(lock_b);
}

#[test]
fn lockfile_with_dead_pid_is_reacquired() {
    let root = plugins_root();
    let lock_path = lockfile_path(root.path(), PLUGIN_ID);

    let dead_pid: u32 = 99_999_999;
    fs::write(&lock_path, format!("{dead_pid} {PLUGIN_ID}\n")).expect("plant lockfile");

    let _lock = acquire_operation_lock(root.path(), PLUGIN_ID)
        .expect("acquire must reap stale-pid lockfile");
    let owner = fs::read_to_string(&lock_path).expect("read recreated lockfile");
    let new_pid = owner
        .split_whitespace()
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .expect("recreated lockfile must contain numeric PID");
    assert_eq!(
        new_pid,
        std::process::id(),
        "stale lock takeover must record this process's PID, got {owner:?}"
    );
}

#[test]
fn lockfile_with_live_pid_blocks_acquisition() {
    let root = plugins_root();
    let lock_path = lockfile_path(root.path(), PLUGIN_ID);

    let live_pid = std::os::unix::process::parent_id();
    fs::write(&lock_path, format!("{live_pid} {PLUGIN_ID}\n")).expect("plant lockfile");

    let result = acquire_operation_lock(root.path(), PLUGIN_ID);

    let err = result.expect_err("acquire must fail while live PID owns lock");
    assert!(
        err.to_string().contains("already in progress"),
        "error must indicate contention, got: {err}"
    );
    let owner = fs::read_to_string(&lock_path).expect("lockfile must survive blocked acquire");
    assert!(
        owner.starts_with(&format!("{live_pid} ")),
        "blocked acquire must not rewrite live-owner lockfile, got: {owner:?}"
    );
}

#[test]
fn lockfile_owner_identity_decides_whether_a_live_pid_still_owns_it() {
    let live_pid = std::os::unix::process::parent_id();
    let identity = qol_process::process_identity(live_pid).expect("parent process identity");
    for (recorded, blocks) in [(identity.as_str(), true), ("replaced:0", false)] {
        let root = plugins_root();
        let lock_path = lockfile_path(root.path(), PLUGIN_ID);
        fs::write(
            &lock_path,
            format!("{live_pid} {PLUGIN_ID} earlier {recorded}\n"),
        )
        .expect("plant lockfile");

        let result = acquire_operation_lock(root.path(), PLUGIN_ID);

        assert_eq!(result.is_err(), blocks, "recorded identity {recorded:?}");
    }
}

#[test]
fn lockfile_left_by_this_pid_before_a_restart_is_reacquired() {
    let root = plugins_root();
    let lock_path = lockfile_path(root.path(), PLUGIN_ID);
    let self_pid = std::process::id();

    for planted in [
        format!("{self_pid} {PLUGIN_ID}\n"),
        format!("{self_pid} {PLUGIN_ID} earlier\n"),
    ] {
        fs::write(&lock_path, &planted).expect("plant lockfile");
        let held = acquire_operation_lock(root.path(), PLUGIN_ID)
            .unwrap_or_else(|error| panic!("{planted:?} must count as stale: {error}"));
        let err = acquire_operation_lock(root.path(), PLUGIN_ID)
            .expect_err("the lock this image holds must still block");
        assert!(err.to_string().contains("already in progress"), "{err}");
        drop(held);
    }
}

#[test]
fn panicking_acquire_holder_releases_lock_via_drop() {
    let root = Arc::new(plugins_root());
    let lock_path = lockfile_path(root.path(), PLUGIN_ID);

    let root_clone = Arc::clone(&root);
    let crashed = thread::spawn(move || {
        let _lock = acquire_operation_lock(root_clone.path(), PLUGIN_ID).expect("acquire");
        panic!("simulated install crash");
    });

    let join = crashed.join();
    assert!(join.is_err(), "thread must panic");

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while lock_path.exists() && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !lock_path.exists(),
        "Drop must clear lockfile during unwind: {}",
        lock_path.display()
    );

    let _next = acquire_operation_lock(root.path(), PLUGIN_ID)
        .expect("acquire after panicked holder must succeed");
}
