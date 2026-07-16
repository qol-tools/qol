#[cfg(any(target_os = "linux", target_os = "windows"))]
use super::lifecycle::{CleanupFn, LifecycleHandle, LifecycleRegistration, TerminationAttempt};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use super::run::WorkerState;
#[cfg(target_os = "linux")]
use super::start::attach_worker;
use super::start::spawn_cleanup_evidence;
use super::*;
use crate::{FlowStart, FlowWorkerRequest, ImageImportStart, ImageImportWorkerRequest};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use anyhow::anyhow;
use qol_dev_env::ReportKind;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(any(unix, target_os = "windows"))]
use std::process::Command;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::process::Stdio;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::sync::{Arc, Mutex};
use std::thread;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::time::Instant;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn start(worktree: PathBuf, run_id: &str) -> FlowStart {
    FlowStart {
        workflow: "qol-shot-capture-region".to_string(),
        environment_id: "linux/mint-cinnamon".to_string(),
        worktree,
        run_id: run_id.to_string(),
        repeat: 1,
        jobs: 1,
        memory_mb: Some(4096),
        cpus: Some(4),
        force: false,
    }
}

fn guardian_command() -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "handle::tests::process_tree_guardian_helper",
        "--nocapture",
    ]);
    command
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn write_report(path: &Path, run_id: &str, status: &str, cleanup: bool) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        serde_json::to_vec(&json!({
            "run_id": run_id,
            "kind": "flow-fanout",
            "status": status,
            "workflow": { "repeat": 1 },
            "lanes": [{ "run_id": "lane-1", "cleanup": { "complete": cleanup } }],
            "payload": { "cleanup": { "complete": cleanup } }
        }))
        .unwrap(),
    )
    .unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn write_image_import_report(path: &Path, run_id: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        serde_json::to_vec(&json!({
            "run_id": run_id,
            "kind": "image-import",
            "status": "pass",
            "teardown": {
                "status": "complete",
                "qemu_exit_verified": true,
                "tree_exit_verified": true,
                "staging_removed": true
            }
        }))
        .unwrap(),
    )
    .unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn child(mode: &str, marker: Option<&Path>) -> LifecycleHandle {
    let registration = LifecycleRegistration::new(mode).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "handle::tests::subprocess_helper", "--nocapture"])
        .env("QOL_ORCHESTRATOR_TEST_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(marker) = marker {
        command.env("QOL_ORCHESTRATOR_TEST_MARKER", marker);
    }
    let process_tree =
        qol_process::own_current_process_tree_with_guardian(guardian_command()).unwrap();
    qol_process::isolate_owned_session(&mut command).unwrap();
    let child = process_tree
        .prepare_command(command)
        .unwrap()
        .spawn()
        .unwrap();
    registration.attach(child, process_tree)
}

fn ticket(root: &Path, run_id: &str) -> RunTicket {
    RunTicket::new(
        run_id.to_string(),
        ReportKind::FlowFanout,
        root.join("flows").join(run_id).join("report.json"),
    )
    .unwrap()
}

fn flow_request(root: &Path, run_id: &str) -> FlowWorkerRequest {
    FlowWorkerRequest {
        start: start(root.join("worktree"), run_id),
        run_root: root.join("runs"),
        plan_fingerprint: "a".repeat(64),
        verbose: false,
    }
}

fn image_import_start(root: &Path, run_id: &str) -> ImageImportStart {
    ImageImportStart {
        environment_id: "linux/mint-cinnamon".to_string(),
        source: root.join("source.qcow2"),
        worktree: root.join("worktree"),
        run_id: run_id.to_string(),
    }
}

fn image_import_ticket(root: &Path, run_id: &str) -> RunTicket {
    image_import_start(root, run_id)
        .ticket(&root.join("images"))
        .unwrap()
}

fn image_import_request(root: &Path, run_id: &str) -> ImageImportWorkerRequest {
    ImageImportWorkerRequest {
        start: image_import_start(root, run_id),
        image_root: root.join("images"),
        plan_fingerprint: "a".repeat(64),
        verbose: false,
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn handle(ticket: RunTicket, worker: LifecycleHandle) -> RunHandle {
    RunHandle {
        ticket,
        worker: Some(WorkerState::Running(worker)),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn worker_pid(handle: &RunHandle) -> u32 {
    let Some(WorkerState::Running(worker)) = handle.worker.as_ref() else {
        panic!("worker is not running");
    };
    worker.pid()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn wait_for_exit(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while qol_process::is_pid_alive(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!qol_process::is_pid_alive(pid));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn wait_for_terminal_worker_failure(handle: &mut RunHandle, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !matches!(
        handle.worker,
        Some(WorkerState::Failed {
            status: Some(_),
            ..
        })
    ) && Instant::now() < deadline
    {
        let _ = handle.poll_worker();
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "linux")]
fn wait_for_marker_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    fs::read_to_string(path).unwrap().parse::<u32>().unwrap()
}

#[test]
fn ticket_rejects_wrong_run_kind_and_layout() {
    let temp = tempfile::tempdir().unwrap();
    let report_path = temp.path().join("report.json");
    fs::write(
        &report_path,
        serde_json::to_vec(&json!({
            "run_id": "actual",
            "kind": "environment-batch",
            "status": "running"
        }))
        .unwrap(),
    )
    .unwrap();
    let wrong_run = RunTicket::new(
        "expected".to_string(),
        ReportKind::EnvironmentBatch,
        report_path.clone(),
    )
    .unwrap();
    let wrong_kind =
        RunTicket::new("actual".to_string(), ReportKind::FlowFanout, report_path).unwrap();
    assert!(wrong_run.read().is_err());
    assert!(wrong_kind.read().is_err());
    assert!(wrong_kind.worker_log_path().is_err());
}

#[test]
fn worker_layouts_keep_logs_outside_authoritative_run_directories() {
    let temp = tempfile::tempdir().unwrap();
    let flow = ticket(temp.path(), "flow-layout");
    assert_eq!(
        flow.worker_log_path().unwrap(),
        temp.path().join("flows/.workers/flow-layout.log")
    );
    let image = image_import_ticket(temp.path(), "image-layout");
    let image_run_dir = image.report_path.parent().unwrap();
    assert_eq!(
        image.worker_log_path().unwrap(),
        temp.path()
            .join("images/verified/imports/.workers/image-layout.log")
    );
    assert!(!image_run_dir.exists());
    assert_invalid_worker_layouts(temp.path());
}

fn assert_invalid_worker_layouts(root: &Path) {
    for invalid in [
        RunTicket::new(
            "image-layout".to_string(),
            ReportKind::ImageImport,
            root.join("images/imports/image-layout/report.json"),
        )
        .unwrap(),
        RunTicket::new(
            "image-layout".to_string(),
            ReportKind::ImageImport,
            root.join("images/verified/wrong/image-layout/report.json"),
        )
        .unwrap(),
        RunTicket::new(
            "environment-layout".to_string(),
            ReportKind::Environment,
            root.join("environments/environment-layout/report.json"),
        )
        .unwrap(),
    ] {
        assert!(invalid.worker_log_path().is_err(), "{invalid:?}");
    }
}

#[test]
fn worker_executable_must_be_absolute() {
    let temp = tempfile::tempdir().unwrap();
    let request = flow_request(temp.path(), "flow-relative-executable");
    let ticket = request.start.ticket(&request.run_root).unwrap();
    assert!(start_flow_worker(Path::new("qol"), guardian_command(), request, ticket).is_err());
}

#[test]
fn spawn_failure_evidence_distinguishes_pending_recovery() {
    use qol_process::PreparedSpawnCleanup::{NotStarted, RecoveryPending, Verified};

    assert_eq!(
        spawn_cleanup_evidence(NotStarted),
        "process creation did not start"
    );
    assert!(spawn_cleanup_evidence(Verified).contains("verified"));
    let pending = spawn_cleanup_evidence(RecoveryPending);
    assert!(pending.contains("unresolved"));
    assert!(pending.contains("may still be running"));
}

#[test]
fn flow_worker_rejects_a_different_run_root_ticket() {
    let temp = tempfile::tempdir().unwrap();
    let run_id = "flow-wrong-ticket";
    let request = flow_request(temp.path(), run_id);
    let wrong_ticket = ticket(temp.path(), run_id);
    assert!(start_flow_worker(
        &std::env::current_exe().unwrap(),
        guardian_command(),
        request,
        wrong_ticket,
    )
    .is_err());
}

#[test]
fn image_import_worker_rejects_a_different_ticket_kind() {
    let temp = tempfile::tempdir().unwrap();
    let run_id = "image-wrong-ticket";
    let request = image_import_request(temp.path(), run_id);
    let wrong_ticket = ticket(temp.path(), run_id);
    assert!(start_image_import_worker(
        &std::env::current_exe().unwrap(),
        guardian_command(),
        request,
        wrong_ticket,
    )
    .is_err());
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn image_import_worker_startup_does_not_claim_the_run_directory() {
    let temp = tempfile::tempdir().unwrap();
    let run_id = "image-missing-worker";
    let request = image_import_request(temp.path(), run_id);
    let ticket = image_import_ticket(temp.path(), run_id);
    let run_dir = ticket.report_path.parent().unwrap().to_path_buf();
    let log_path = ticket.worker_log_path().unwrap();
    assert!(start_image_import_worker(
        &temp.path().join("missing-qol-worker"),
        guardian_command(),
        request,
        ticket,
    )
    .is_err());
    assert!(log_path.is_file());
    assert!(!run_dir.exists());
}

#[cfg(target_os = "macos")]
#[test]
fn worker_start_rejects_unsupported_containment_before_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let request = flow_request(temp.path(), "flow-unsupported-containment");
    let ticket = request.start.ticket(&request.run_root).unwrap();
    let log_path = ticket.worker_log_path().unwrap();
    let error = match start_flow_worker(
        &std::env::current_exe().unwrap(),
        guardian_command(),
        request,
        ticket,
    ) {
        Ok(_) => panic!("unsupported containment started a worker"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("unsupported"));
    assert!(!log_path.exists());
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn worker_exit_without_terminal_cleanup_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = ticket(temp.path(), "flow-1");
    write_report(&ticket.report_path, "flow-1", "running", false);
    let mut handle = handle(ticket, child("exit", None));
    assert!(matches!(
        handle.wait().unwrap(),
        WaitState::Failed {
            report: Some(_),
            ..
        }
    ));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn exited_worker_is_reaped_even_when_its_report_is_malformed() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = ticket(temp.path(), "flow-malformed");
    fs::create_dir_all(ticket.report_path.parent().unwrap()).unwrap();
    fs::write(&ticket.report_path, b"not-json").unwrap();
    let mut handle = handle(ticket, child("exit", None));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match handle.poll() {
            Ok(WaitState::Failed {
                report: None,
                worker_exit,
            }) => {
                assert!(worker_exit.contains("authoritative report is unreadable"));
                break;
            }
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            state => panic!("unexpected worker state: {state:?}"),
        }
    }
    assert!(matches!(handle.worker, Some(WorkerState::Exited(_))));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn terminal_report_with_cleanup_proof_completes() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = ticket(temp.path(), "flow-1");
    write_report(&ticket.report_path, "flow-1", "pass", true);
    let mut handle = handle(ticket, child("exit", None));
    assert!(matches!(
        handle.wait().unwrap(),
        WaitState::Terminal {
            worker_success: true,
            ..
        }
    ));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn image_import_uses_the_shared_terminal_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = image_import_ticket(temp.path(), "image-import-1");
    write_image_import_report(&ticket.report_path, "image-import-1");
    let mut handle = handle(ticket, child("exit", None));
    assert!(matches!(
        handle.wait().unwrap(),
        WaitState::Terminal {
            worker_success: true,
            ..
        }
    ));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn wait_timeout_keeps_the_worker_owned() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = ticket(temp.path(), "flow-1");
    let mut handle = handle(ticket, child("slow-exit", None));
    assert_eq!(handle.wait_timeout(Duration::ZERO).unwrap(), None);
    assert!(matches!(handle.wait().unwrap(), WaitState::Failed { .. }));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn typed_worker_escalation_returns_process_tree_exit_proof() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = ticket(temp.path(), "flow-escalate");
    let mut handle = handle(ticket, child("slow-exit", None));
    let pid = worker_pid(&handle);
    let proof = handle.terminate_worker(Duration::from_secs(2)).unwrap();
    assert!(proof.is_some());
    assert!(!qol_process::is_pid_alive(pid));
    assert!(matches!(handle.poll().unwrap(), WaitState::Failed { .. }));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn invalid_termination_timeout_is_rejected_before_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = ticket(temp.path(), "flow-invalid-timeout");
    let mut handle = handle(ticket, child("bounded-exit", None));
    let pid = worker_pid(&handle);
    let error = handle.terminate_worker(Duration::MAX).unwrap_err();
    assert!(format!("{error:#}").contains("invalid typed worker termination timeout"));
    assert!(matches!(handle.worker, Some(WorkerState::Running(_))));
    assert!(qol_process::is_pid_alive(pid));
    assert!(handle
        .terminate_worker(Duration::from_secs(2))
        .unwrap()
        .is_some());
    assert!(!qol_process::is_pid_alive(pid));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn termination_timeout_becomes_a_sticky_terminal_failure_after_proof() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = ticket(temp.path(), "flow-bounded-escalation");
    let mut handle = handle(ticket, child("bounded-exit", None));
    let pid = worker_pid(&handle);
    let started = Instant::now();
    let error = delayed_termination_failure(&mut handle);
    assert!(started.elapsed() < Duration::from_millis(400));
    assert!(format!("{error:#}").contains("ownership remains with the lifecycle owner"));
    assert!(qol_process::is_pid_alive(pid));
    assert!(matches!(
        handle.worker,
        Some(WorkerState::Escalating { .. })
    ));
    wait_for_terminal_worker_failure(&mut handle, Duration::from_secs(3));
    assert!(!qol_process::is_pid_alive(pid));
    assert_sticky_terminal_failure(&mut handle);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn pending_cleanup_moves_from_the_owner_to_the_recovery_coordinator() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = ticket(temp.path(), "flow-background-recovery");
    let mut handle = handle(ticket, child("bounded-exit", None));
    let pid = worker_pid(&handle);
    let cleanup_threads = Arc::new(Mutex::new(Vec::new()));
    let observed_threads = Arc::clone(&cleanup_threads);
    let cleanup: CleanupFn = Arc::new(move |process_tree, timeout| {
        let thread = thread::current();
        let name = thread.name().unwrap_or("unnamed").to_string();
        let mut threads = observed_threads.lock().unwrap();
        threads.push(name);
        let first = threads.len() == 1;
        drop(threads);
        if first {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "injected owner cleanup timeout",
            ));
        }
        process_tree.terminate_and_wait(timeout)
    });
    let error = handle
        .terminate_worker_with_cleanup(
            Duration::from_millis(10),
            Box::new(|_, _| TerminationAttempt {
                proof: Err(anyhow!("injected process-tree timeout")),
                root_stop: Err(anyhow!("injected exact-root timeout")),
            }),
            cleanup,
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("injected process-tree timeout"));
    wait_for_terminal_worker_failure(&mut handle, Duration::from_secs(3));
    assert!(!qol_process::is_pid_alive(pid));
    let threads = cleanup_threads.lock().unwrap();
    assert_eq!(threads[0], "qol-worker-owner-bounded-exit");
    assert_eq!(threads[1], "qol-worker-recovery");
    let WaitState::Failed { worker_exit, .. } = handle.poll().unwrap() else {
        panic!("background recovery did not reach a proof-complete failure");
    };
    assert!(worker_exit.contains("background recovery coordinator"));
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn delayed_termination_failure(handle: &mut RunHandle) -> anyhow::Error {
    handle
        .terminate_worker_with(
            Duration::from_millis(10),
            Box::new(|_, _| {
                thread::sleep(Duration::from_millis(600));
                TerminationAttempt {
                    proof: Err(anyhow!("injected process-tree timeout")),
                    root_stop: Err(anyhow!("injected exact-root timeout")),
                }
            }),
        )
        .unwrap_err()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn assert_sticky_terminal_failure(handle: &mut RunHandle) {
    for _ in 0..2 {
        let WaitState::Failed { worker_exit, .. } = handle.poll().unwrap() else {
            panic!("proof-complete cleanup failure was not terminal");
        };
        assert!(worker_exit.contains("injected process-tree timeout"));
        assert!(worker_exit.contains("injected exact-root timeout"));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn failed_termination_retries_until_a_stubborn_descendant_is_proven_gone() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("stubborn-descendant");
    let ticket = ticket(temp.path(), "flow-stubborn-escalation");
    let mut handle = handle(ticket, child("stubborn-child-wait", Some(&marker)));
    let root_pid = worker_pid(&handle);
    let descendant_pid = wait_for_marker_pid(&marker);
    let error = handle
        .terminate_worker_with(
            Duration::from_millis(100),
            Box::new(|_, _| TerminationAttempt {
                proof: Err(anyhow!("injected tree proof failure")),
                root_stop: Err(anyhow!("injected root proof failure")),
            }),
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("injected tree proof failure"));
    assert!(matches!(
        handle.worker,
        Some(WorkerState::Escalating { .. })
    ));
    wait_for_terminal_worker_failure(&mut handle, Duration::from_secs(4));
    assert!(!qol_process::is_pid_alive(root_pid));
    assert!(!qol_process::is_pid_alive(descendant_pid));
    let WaitState::Failed { worker_exit, .. } = handle.poll().unwrap() else {
        panic!("proof-complete descendant cleanup failure was not terminal");
    };
    assert!(worker_exit.contains("injected tree proof failure"));
}

#[cfg(target_os = "linux")]
#[test]
fn natural_root_exit_cleans_a_stubborn_descendant_before_terminal_state() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("natural-stubborn-descendant");
    let ticket = ticket(temp.path(), "flow-natural-stubborn");
    write_report(&ticket.report_path, "flow-natural-stubborn", "pass", true);
    let mut handle = handle(ticket, child("stubborn-child-exit", Some(&marker)));
    let descendant_pid = wait_for_marker_pid(&marker);
    assert!(matches!(
        handle.wait().unwrap(),
        WaitState::Terminal {
            worker_success: true,
            ..
        }
    ));
    assert!(!qol_process::is_pid_alive(descendant_pid));
}

#[cfg(target_os = "linux")]
#[test]
fn detached_owner_cleans_a_stubborn_descendant_after_root_exit() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("detached-stubborn-descendant");
    let ticket = ticket(temp.path(), "flow-detached-stubborn");
    let handle = handle(ticket, child("stubborn-child-exit", Some(&marker)));
    let descendant_pid = wait_for_marker_pid(&marker);
    let _ = handle.detach();
    wait_for_exit(descendant_pid);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn lifecycle_owner_keeps_drop_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let ticket = ticket(temp.path(), "flow-bounded-drop");
    let worker = child("bounded-exit", None);
    let pid = worker.pid();
    let handle = RunHandle {
        ticket,
        worker: Some(WorkerState::Running(worker)),
    };
    let started = Instant::now();
    drop(handle);
    assert!(started.elapsed() < Duration::from_millis(400));
    wait_for_exit(pid);
}

#[test]
fn cancellation_is_idempotent() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let run_id = format!("orchestrator-cancel-{unique}");
    let ticket = ticket(&std::env::temp_dir().join(&run_id), &run_id);
    let first = ticket.cancel().unwrap();
    let second = ticket.cancel().unwrap();
    assert_eq!(first, second);
    qol_dev_env::clear_cancellation_request(&run_id).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn dropping_or_detaching_a_handle_reaps_without_killing_the_worker() {
    for operation in ["drop", "detach"] {
        assert_drop_or_detach(operation);
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn assert_drop_or_detach(operation: &str) {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join(operation);
    let run_id = format!("worker-{operation}");
    let ticket = operation_ticket(temp.path(), &run_id, operation);
    let handle = handle(ticket, child("mark", Some(&marker)));
    let pid = worker_pid(&handle);
    if operation == "detach" {
        let _ = handle.detach();
    } else {
        drop(handle);
    }
    wait_for_path(&marker);
    wait_for_exit(pid);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn operation_ticket(root: &Path, run_id: &str, operation: &str) -> RunTicket {
    if operation == "detach" {
        return image_import_ticket(root, run_id);
    }
    ticket(root, run_id)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn typed_request_write_failure_terminates_a_stubborn_exact_tree() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("stdin-closed-descendant");
    let (registration, worker, process_tree) = stdin_closed_stubborn_worker(&marker);
    let root_pid = worker.id();
    let descendant_pid = wait_for_marker_pid(&marker);
    let ticket = ticket(temp.path(), "flow-stdin-closed");
    let error = match attach_worker(
        registration,
        worker,
        process_tree,
        b"request\n".to_vec(),
        ticket,
    ) {
        Ok(_) => panic!("closed worker stdin accepted a typed request"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("exact tree termination was scheduled"));
    wait_for_exit(root_pid);
    wait_for_exit(descendant_pid);
}

#[cfg(target_os = "linux")]
fn stdin_closed_stubborn_worker(
    marker: &Path,
) -> (
    LifecycleRegistration,
    std::process::Child,
    qol_process::ProcessTreeGuard,
) {
    let registration = LifecycleRegistration::new("stdin-closed").unwrap();
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            "exec 0<&-; trap '' TERM; sleep 30 & descendant=$!; printf '%s' \"$descendant\" > \"$QOL_STDIN_CLOSED_MARKER\"; wait",
        ])
        .env("QOL_STDIN_CLOSED_MARKER", marker)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    qol_process::isolate_owned_session(&mut command).unwrap();
    let process_tree =
        qol_process::own_current_process_tree_with_guardian(guardian_command()).unwrap();
    let worker = process_tree
        .prepare_command(command)
        .unwrap()
        .spawn()
        .unwrap();
    (registration, worker, process_tree)
}

#[cfg(unix)]
#[test]
fn worker_isolation_creates_a_process_group() {
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30"]);
    qol_process::isolate_owned_command(&mut command).unwrap();
    let mut worker = command.spawn().unwrap();
    let pid = worker.id();
    assert!(qol_process::is_group_alive(pid));
    qol_process::terminate_group(pid, Duration::from_secs(1));
    let _ = worker.wait();
}

#[test]
#[allow(clippy::zombie_processes)]
fn subprocess_helper() {
    let Ok(mode) = std::env::var("QOL_ORCHESTRATOR_TEST_MODE") else {
        return;
    };
    if mode == "slow-exit" {
        thread::sleep(Duration::from_millis(200));
        return;
    }
    if mode == "bounded-exit" {
        thread::sleep(Duration::from_millis(900));
        return;
    }
    if mode == "exit" {
        thread::sleep(Duration::from_millis(50));
        return;
    }
    spawn_stubborn_descendant(&mode);
    if mode.starts_with("stubborn-child-") {
        return;
    }
    let marker = std::env::var_os("QOL_ORCHESTRATOR_TEST_MARKER").unwrap();
    thread::sleep(Duration::from_millis(100));
    fs::File::create(marker).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn process_tree_guardian_helper() {
    if std::env::var_os("QOL_PROCESS_GUARDIAN_PROTOCOL").is_some() {
        qol_process::run_process_tree_guardian_entry().unwrap();
    }
}

#[cfg(target_os = "linux")]
#[allow(clippy::zombie_processes)]
fn spawn_stubborn_descendant(mode: &str) {
    if !matches!(mode, "stubborn-child-exit" | "stubborn-child-wait") {
        return;
    }
    let marker = std::env::var_os("QOL_ORCHESTRATOR_TEST_MARKER").unwrap();
    let mut command = Command::new("sh");
    command.args(["-c", "trap '' TERM; exec sleep 30"]);
    qol_process::isolate_owned_session(&mut command).unwrap();
    let child = command.spawn().unwrap();
    fs::write(marker, child.id().to_string()).unwrap();
    let delay = if mode == "stubborn-child-wait" {
        Duration::from_secs(30)
    } else {
        Duration::from_millis(150)
    };
    thread::sleep(delay);
}

#[cfg(not(target_os = "linux"))]
fn spawn_stubborn_descendant(_: &str) {}
