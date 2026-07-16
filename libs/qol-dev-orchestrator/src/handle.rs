use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use qol_dev_env::{CleanupState, ReportKind, RunSummary};
use serde::Serialize;

use crate::{
    FlowWorkerRequest, ImageImportWorkerRequest, FLOW_WORKER_COMMAND, IMAGE_IMPORT_WORKER_COMMAND,
};

const WAIT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunTicket {
    pub run_id: String,
    pub kind: ReportKind,
    pub report_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitState {
    Starting,
    Running(RunSummary),
    Terminal {
        report: RunSummary,
        worker_success: bool,
    },
    Failed {
        report: Option<RunSummary>,
        worker_exit: String,
    },
}

pub struct RunHandle {
    ticket: RunTicket,
    worker: Option<WorkerState>,
}

enum WorkerState {
    Running {
        child: Child,
        process_tree: qol_process::ProcessTreeGuard,
        reaper: SyncSender<OwnedWorker>,
    },
    Exited(ExitStatus),
}

struct OwnedWorker {
    child: Child,
    _process_tree: qol_process::ProcessTreeGuard,
}

trait TypedWorkerRequest: Serialize {
    fn command_name(&self) -> &'static str;
    fn worktree(&self) -> &Path;
    fn run_id(&self) -> &str;
    fn report_kind(&self) -> ReportKind;
}

impl TypedWorkerRequest for FlowWorkerRequest {
    fn command_name(&self) -> &'static str {
        FLOW_WORKER_COMMAND
    }

    fn worktree(&self) -> &Path {
        &self.start.worktree
    }

    fn run_id(&self) -> &str {
        &self.start.run_id
    }

    fn report_kind(&self) -> ReportKind {
        ReportKind::FlowFanout
    }
}

impl TypedWorkerRequest for ImageImportWorkerRequest {
    fn command_name(&self) -> &'static str {
        IMAGE_IMPORT_WORKER_COMMAND
    }

    fn worktree(&self) -> &Path {
        &self.start.worktree
    }

    fn run_id(&self) -> &str {
        &self.start.run_id
    }

    fn report_kind(&self) -> ReportKind {
        ReportKind::ImageImport
    }
}

impl RunTicket {
    pub fn new(run_id: String, kind: ReportKind, report_path: PathBuf) -> Result<Self> {
        qol_dev_env::validate_run_id(&run_id)?;
        crate::request::validate_absolute_path(&report_path, "run report path")?;
        Ok(Self {
            run_id,
            kind,
            report_path,
        })
    }

    pub fn read(&self) -> Result<Option<qol_dev_env::RunReport>> {
        qol_dev_env::read_report_checked(&self.report_path, &self.run_id, &self.kind)
    }

    pub fn cancel(&self) -> Result<PathBuf> {
        qol_dev_env::request_cancellation(&self.run_id)
    }

    pub fn worker_log_path(&self) -> Result<PathBuf> {
        let worker_root = self.validate_worker_layout()?;
        Ok(worker_root
            .join(".workers")
            .join(format!("{}.log", self.run_id)))
    }

    fn validate_worker_layout(&self) -> Result<&Path> {
        if self.report_path.file_name().and_then(|name| name.to_str()) != Some("report.json") {
            bail!("worker report must be named report.json");
        }
        let run_dir = self
            .report_path
            .parent()
            .context("worker report has no run directory")?;
        if run_dir.file_name().and_then(|name| name.to_str()) != Some(self.run_id.as_str()) {
            bail!("worker report directory does not match its run id");
        }
        let worker_root = run_dir
            .parent()
            .context("worker report has no worker root directory")?;
        match self.kind {
            ReportKind::FlowFanout => validate_parent_name(worker_root, "flows")?,
            ReportKind::ImageImport => {
                let verified = worker_root
                    .parent()
                    .context("image import report has no verified directory")?;
                let image_root = verified
                    .parent()
                    .context("image import report has no image root")?;
                let expected =
                    qol_dev_env::managed_verification_report_path(image_root, &self.run_id)?;
                if self.report_path != expected {
                    bail!("image import report is outside the managed verification layout");
                }
            }
            _ => bail!(
                "report kind `{}` has no typed worker layout",
                self.kind.as_str()
            ),
        }
        Ok(worker_root)
    }
}

fn validate_parent_name(path: &Path, expected: &str) -> Result<()> {
    if path.file_name().and_then(|name| name.to_str()) != Some(expected) {
        bail!("worker report is outside a {expected} directory");
    }
    Ok(())
}

impl WaitState {
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Terminal { .. } | Self::Failed { .. })
    }
}

impl RunHandle {
    pub fn ticket(&self) -> &RunTicket {
        &self.ticket
    }

    pub fn cancel(&self) -> Result<PathBuf> {
        self.ticket.cancel()
    }

    pub fn poll(&mut self) -> Result<WaitState> {
        let worker_exit = self.poll_worker()?;
        let report = match self.ticket.read() {
            Ok(report) => report.map(|report| report.summary()),
            Err(error) => {
                let Some(worker_exit) = worker_exit else {
                    return Err(error);
                };
                return Ok(WaitState::Failed {
                    report: None,
                    worker_exit: format!(
                        "{worker_exit}; authoritative report is unreadable: {error:#}"
                    ),
                });
            }
        };
        let Some(worker_exit) = worker_exit else {
            return Ok(match report {
                Some(report) => WaitState::Running(report),
                None => WaitState::Starting,
            });
        };
        let terminal = report.as_ref().is_some_and(|report| {
            report.status.is_terminal() && matches!(report.cleanup, CleanupState::Complete)
        });
        if terminal {
            let Some(report) = report else {
                return Err(anyhow!("terminal worker report disappeared"));
            };
            return Ok(WaitState::Terminal {
                report,
                worker_success: worker_exit.success(),
            });
        }
        Ok(WaitState::Failed {
            report,
            worker_exit: worker_exit.to_string(),
        })
    }

    pub fn wait(&mut self) -> Result<WaitState> {
        loop {
            let state = self.poll()?;
            if state.is_finished() {
                return Ok(state);
            }
            thread::sleep(WAIT_INTERVAL);
        }
    }

    pub fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<WaitState>> {
        let started = Instant::now();
        loop {
            let state = self.poll()?;
            if state.is_finished() {
                return Ok(Some(state));
            }
            let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
                return Ok(None);
            };
            if remaining.is_zero() {
                return Ok(None);
            }
            thread::sleep(WAIT_INTERVAL.min(remaining));
        }
    }

    pub fn detach(self) -> RunTicket {
        self.ticket.clone()
    }

    pub fn terminate_worker(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<qol_process::TerminatedProcessTree>> {
        let Some(WorkerState::Running {
            mut child,
            process_tree,
            reaper: _,
        }) = self.worker.take()
        else {
            return Ok(None);
        };
        let worker_pid = child.id();
        let (proof, status) = thread::scope(|scope| {
            let waiter = scope.spawn(|| child.wait().context("failed to reap terminated worker"));
            let proof = process_tree
                .terminate_and_wait(timeout)
                .context("typed worker process tree survived termination");
            if proof.is_err() {
                qol_process::terminate_group(worker_pid, timeout);
            }
            let status = waiter
                .join()
                .map_err(|_| anyhow!("typed worker reaper panicked"))??;
            Ok::<_, anyhow::Error>((proof, status))
        })?;
        self.worker = Some(WorkerState::Exited(status));
        Ok(Some(proof?))
    }

    fn poll_worker(&mut self) -> Result<Option<ExitStatus>> {
        let status = match self.worker.as_mut() {
            Some(WorkerState::Running { child, .. }) => {
                child.try_wait().context("failed to poll worker")?
            }
            Some(WorkerState::Exited(status)) => return Ok(Some(*status)),
            None => return Err(anyhow!("run worker is detached")),
        };
        if status.is_none() {
            return Ok(None);
        }
        self.worker.take();
        self.worker = status.map(WorkerState::Exited);
        Ok(status)
    }
}

impl Drop for RunHandle {
    fn drop(&mut self) {
        let Some(WorkerState::Running {
            child,
            process_tree,
            reaper,
        }) = self.worker.take()
        else {
            return;
        };
        background_reap(reaper, child, process_tree);
    }
}

pub fn start_flow_worker(
    executable: &Path,
    request: FlowWorkerRequest,
    ticket: RunTicket,
) -> Result<RunHandle> {
    request.validate()?;
    let expected = request.start.ticket(&request.run_root)?;
    if ticket != expected {
        bail!("flow worker ticket does not match its immutable run plan");
    }
    start_worker(executable, request, ticket)
}

pub fn start_image_import_worker(
    executable: &Path,
    request: ImageImportWorkerRequest,
    ticket: RunTicket,
) -> Result<RunHandle> {
    request.validate()?;
    let expected = request.start.ticket(&request.image_root)?;
    if ticket != expected {
        bail!("image-import worker ticket does not match its immutable image plan");
    }
    start_worker(executable, request, ticket)
}

fn start_worker<T: TypedWorkerRequest>(
    executable: &Path,
    request: T,
    ticket: RunTicket,
) -> Result<RunHandle> {
    if !executable.is_absolute() {
        bail!("worker executable must be absolute");
    }
    if ticket.run_id != request.run_id() {
        bail!(
            "run ticket `{}` does not match worker `{}`",
            ticket.run_id,
            request.run_id()
        );
    }
    let report_kind = request.report_kind();
    if ticket.kind != report_kind {
        bail!(
            "worker requires a `{}` ticket, received `{}`",
            report_kind.as_str(),
            ticket.kind.as_str()
        );
    }
    ticket.validate_worker_layout()?;
    let log_path = ticket.worker_log_path()?;
    let log_dir = log_path
        .parent()
        .context("worker log has no parent directory")?;
    fs::create_dir_all(log_dir)
        .with_context(|| format!("failed to create {}", log_dir.display()))?;
    let log = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&log_path)
        .with_context(|| format!("failed to create {}", log_path.display()))?;
    let stderr = log
        .try_clone()
        .with_context(|| format!("failed to clone {}", log_path.display()))?;
    let reaper = spawn_reaper(&ticket.run_id)?;
    let process_tree = qol_process::own_current_process_tree()
        .context("failed to create typed worker process-tree ownership")?;
    let mut command = Command::new(executable);
    command
        .arg(request.command_name())
        .current_dir(request.worktree())
        .stdin(Stdio::piped())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    qol_process::isolate_owned_command(&mut command)
        .context("failed to isolate typed worker process tree")?;
    qol_dev_env::clear_host_session(&mut command);
    let mut worker = command
        .spawn()
        .with_context(|| format!("failed to start {}", executable.display()))?;
    if let Err(error) = process_tree.assign(&worker) {
        let cleanup = qol_process::terminate_owned(&mut worker, Duration::from_secs(2));
        return Err(anyhow!(
            "failed to own typed worker process tree: {error}; fallback cleanup: {}",
            cleanup.err().map_or_else(
                || "direct child stopped".to_string(),
                |error| format!("{error:#}")
            )
        ));
    }
    attach_worker(worker, process_tree, reaper, request, ticket)
}

fn attach_worker<T: Serialize>(
    mut worker: Child,
    process_tree: qol_process::ProcessTreeGuard,
    reaper: SyncSender<OwnedWorker>,
    request: T,
    ticket: RunTicket,
) -> Result<RunHandle> {
    if let Err(error) = send_request(&mut worker, &request) {
        background_reap(reaper, worker, process_tree);
        return Err(error);
    }
    Ok(RunHandle {
        ticket,
        worker: Some(WorkerState::Running {
            child: worker,
            process_tree,
            reaper,
        }),
    })
}

fn send_request(worker: &mut Child, request: &impl Serialize) -> Result<()> {
    let stdin = worker
        .stdin
        .take()
        .context("worker did not expose its typed input")?;
    let mut input = BufWriter::new(stdin);
    serde_json::to_writer(&mut input, request).context("failed to encode worker request")?;
    input
        .write_all(b"\n")
        .context("failed to terminate worker request")?;
    input.flush().context("failed to send worker request")
}

fn spawn_reaper(run_id: &str) -> Result<SyncSender<OwnedWorker>> {
    let (sender, receiver) = mpsc::sync_channel::<OwnedWorker>(1);
    thread::Builder::new()
        .name(format!("qol-worker-reaper-{run_id}"))
        .spawn(move || {
            let Ok(mut worker) = receiver.recv() else {
                return;
            };
            let _ = worker.child.wait();
        })
        .context("failed to start worker reaper")?;
    Ok(sender)
}

fn background_reap(
    reaper: SyncSender<OwnedWorker>,
    child: Child,
    process_tree: qol_process::ProcessTreeGuard,
) {
    let _ = reaper.send(OwnedWorker {
        child,
        _process_tree: process_tree,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FlowStart, ImageImportStart};
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    fn child(mode: &str, marker: Option<&Path>) -> Child {
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
        qol_process::isolate_owned_command(&mut command).unwrap();
        command.spawn().unwrap()
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

    fn handle(ticket: RunTicket, worker: Child) -> RunHandle {
        let reaper = spawn_reaper(&ticket.run_id).unwrap();
        let process_tree = qol_process::own_current_process_tree().unwrap();
        process_tree.assign(&worker).unwrap();
        RunHandle {
            ticket,
            worker: Some(WorkerState::Running {
                child: worker,
                process_tree,
                reaper,
            }),
        }
    }

    fn worker_pid(handle: &RunHandle) -> u32 {
        let Some(WorkerState::Running { child, .. }) = handle.worker.as_ref() else {
            panic!("worker is not running");
        };
        child.id()
    }

    fn wait_for_exit(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while qol_process::is_pid_alive(pid) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!qol_process::is_pid_alive(pid));
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

        for invalid in [
            RunTicket::new(
                "image-layout".to_string(),
                ReportKind::ImageImport,
                temp.path().join("images/imports/image-layout/report.json"),
            )
            .unwrap(),
            RunTicket::new(
                "image-layout".to_string(),
                ReportKind::ImageImport,
                temp.path()
                    .join("images/verified/wrong/image-layout/report.json"),
            )
            .unwrap(),
            RunTicket::new(
                "environment-layout".to_string(),
                ReportKind::Environment,
                temp.path()
                    .join("environments/environment-layout/report.json"),
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
        assert!(start_flow_worker(Path::new("qol"), request, ticket).is_err());
    }

    #[test]
    fn flow_worker_rejects_a_different_run_root_ticket() {
        let temp = tempfile::tempdir().unwrap();
        let run_id = "flow-wrong-ticket";
        let request = flow_request(temp.path(), run_id);
        let wrong_ticket = ticket(temp.path(), run_id);
        assert!(
            start_flow_worker(&std::env::current_exe().unwrap(), request, wrong_ticket).is_err()
        );
    }

    #[test]
    fn image_import_worker_rejects_a_different_ticket_kind() {
        let temp = tempfile::tempdir().unwrap();
        let run_id = "image-wrong-ticket";
        let request = image_import_request(temp.path(), run_id);
        let wrong_ticket = ticket(temp.path(), run_id);
        assert!(start_image_import_worker(
            &std::env::current_exe().unwrap(),
            request,
            wrong_ticket
        )
        .is_err());
    }

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
            request,
            ticket
        )
        .is_err());
        assert!(log_path.is_file());
        assert!(!run_dir.exists());
    }

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

    #[test]
    fn wait_timeout_keeps_the_worker_owned() {
        let temp = tempfile::tempdir().unwrap();
        let ticket = ticket(temp.path(), "flow-1");
        let mut handle = handle(ticket, child("slow-exit", None));
        assert_eq!(handle.wait_timeout(Duration::ZERO).unwrap(), None);
        assert!(matches!(handle.wait().unwrap(), WaitState::Failed { .. }));
    }

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

    #[test]
    fn dropping_or_detaching_a_handle_reaps_without_killing_the_worker() {
        for operation in ["drop", "detach"] {
            let temp = tempfile::tempdir().unwrap();
            let marker = temp.path().join(operation);
            let run_id = format!("worker-{operation}");
            let ticket = if operation == "detach" {
                image_import_ticket(temp.path(), &run_id)
            } else {
                ticket(temp.path(), &run_id)
            };
            let handle = handle(ticket, child("mark", Some(&marker)));
            let pid = worker_pid(&handle);
            if operation == "detach" {
                let _ = handle.detach();
            } else {
                drop(handle);
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while !marker.exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert!(marker.exists(), "{operation}");
            wait_for_exit(pid);
        }
    }

    #[test]
    fn request_write_failure_reaps_without_killing_the_worker() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("request-error");
        let ticket = image_import_ticket(temp.path(), "image-request-error");
        let worker = child("mark", Some(&marker));
        let pid = worker.id();
        let reaper = spawn_reaper(&ticket.run_id).unwrap();
        let process_tree = qol_process::own_current_process_tree().unwrap();
        process_tree.assign(&worker).unwrap();
        let request = image_import_request(temp.path(), &ticket.run_id);
        assert!(attach_worker(worker, process_tree, reaper, request, ticket).is_err());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists());
        wait_for_exit(pid);
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
    fn subprocess_helper() {
        let Ok(mode) = std::env::var("QOL_ORCHESTRATOR_TEST_MODE") else {
            return;
        };
        if mode == "slow-exit" {
            thread::sleep(Duration::from_millis(200));
            return;
        }
        if mode == "exit" {
            return;
        }
        let marker = std::env::var_os("QOL_ORCHESTRATOR_TEST_MARKER").unwrap();
        thread::sleep(Duration::from_millis(100));
        fs::File::create(marker).unwrap();
    }
}
