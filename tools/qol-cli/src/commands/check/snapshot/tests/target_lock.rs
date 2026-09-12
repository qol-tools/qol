use super::*;
use qol_process::CancellationToken;

#[test]
fn a_second_target_lock_reports_contention_and_releases_cleanly() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target");
    let cancellation = CancellationToken::new();
    let first = TargetLock::acquire(&target, &cancellation).unwrap();

    let error = TargetLock::acquire(&target, &cancellation)
        .err()
        .unwrap()
        .to_string();

    assert!(error.contains("already using"), "got: {error}");
    drop(first);
    assert!(TargetLock::acquire(&target, &cancellation).is_ok());
}

#[test]
fn a_cancelled_target_lock_request_fails_explicitly() {
    let directory = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = TargetLock::acquire(&directory.path().join("target"), &cancellation)
        .err()
        .unwrap()
        .to_string();

    assert!(error.contains("cancelled"), "got: {error}");
}
