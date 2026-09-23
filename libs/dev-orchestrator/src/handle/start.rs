use std::fs::{self, OpenOptions};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use qol_dev_env::ReportKind;
use serde::Serialize;

use crate::{
    FlowWorkerRequest, ImageImportWorkerRequest, FLOW_WORKER_COMMAND, IMAGE_IMPORT_WORKER_COMMAND,
};

use super::lifecycle::{terminate_process_tree, LifecycleRegistration};
use super::run::WorkerState;
use super::{RunHandle, RunTicket, BACKGROUND_CLEANUP_TIMEOUT};

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

pub fn start_flow_worker(
    executable: &Path,
    guardian_command: Command,
    request: FlowWorkerRequest,
    ticket: RunTicket,
) -> Result<RunHandle> {
    request.validate()?;
    let expected = request.start.ticket(&request.run_root)?;
    if ticket != expected {
        bail!("flow worker ticket does not match its immutable run plan");
    }
    start_worker(executable, guardian_command, request, ticket)
}

pub fn start_image_import_worker(
    executable: &Path,
    guardian_command: Command,
    request: ImageImportWorkerRequest,
    ticket: RunTicket,
) -> Result<RunHandle> {
    request.validate()?;
    let expected = request.start.ticket(&request.image_root)?;
    if ticket != expected {
        bail!("image-import worker ticket does not match its immutable image plan");
    }
    start_worker(executable, guardian_command, request, ticket)
}

fn start_worker<T: TypedWorkerRequest>(
    executable: &Path,
    guardian_command: Command,
    request: T,
    ticket: RunTicket,
) -> Result<RunHandle> {
    validate_worker_start(executable, &request, &ticket)?;
    qol_process::process_tree_containment_support()
        .context("typed worker process-tree containment is unsupported")?;
    let input = encode_worker_request(&request)?;
    let (stdout, stderr) = open_worker_logs(&ticket)?;
    let registration = LifecycleRegistration::new(&ticket.run_id)?;
    let process_tree = qol_process::own_current_process_tree_with_guardian(guardian_command)
        .context("failed to create typed worker process-tree ownership")?;
    let mut command = worker_command(executable, &request, stdout, stderr);
    qol_process::isolate_owned_session(&mut command)
        .context("failed to isolate typed worker process session")?;
    qol_dev_env::clear_host_session(&mut command);
    let prepared = process_tree
        .prepare_command(command)
        .context("failed to prepare typed worker process-tree ownership")?;
    let worker = prepared
        .spawn()
        .map_err(|error| worker_spawn_error(executable, error))?;
    attach_worker(registration, worker, process_tree, input, ticket)
}

fn worker_spawn_error(path: &Path, error: qol_process::PreparedSpawnError) -> anyhow::Error {
    let evidence = spawn_cleanup_evidence(error.cleanup());
    anyhow::Error::new(error).context(format!("failed to start {}; {evidence}", path.display()))
}

pub(super) fn spawn_cleanup_evidence(cleanup: qol_process::PreparedSpawnCleanup) -> &'static str {
    match cleanup {
        qol_process::PreparedSpawnCleanup::NotStarted => "process creation did not start",
        qol_process::PreparedSpawnCleanup::Verified => "prepared process-tree cleanup was verified",
        qol_process::PreparedSpawnCleanup::RecoveryPending => {
            "prepared process-tree cleanup is unresolved; owned processes may still be running"
        }
    }
}

pub(super) fn attach_worker(
    registration: LifecycleRegistration,
    worker: std::process::Child,
    process_tree: qol_process::ProcessTreeGuard,
    input: Vec<u8>,
    ticket: RunTicket,
) -> Result<RunHandle> {
    let mut worker = registration.attach(worker, process_tree);
    if let Err(error) = worker.write_input(&input) {
        let events = worker.terminate(BACKGROUND_CLEANUP_TIMEOUT, Box::new(terminate_process_tree));
        drop(events);
        return Err(error).context(
            "failed to send worker request; exact tree termination was scheduled and lifecycle ownership remains until proof",
        );
    }
    Ok(RunHandle {
        ticket,
        worker: Some(WorkerState::Running(worker)),
    })
}

fn validate_worker_start<T: TypedWorkerRequest>(
    executable: &Path,
    request: &T,
    ticket: &RunTicket,
) -> Result<()> {
    if !executable.is_absolute() {
        bail!("worker executable must be absolute");
    }
    if ticket.run_id != request.run_id() {
        bail!("run ticket does not match the typed worker run id");
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
    Ok(())
}

fn encode_worker_request(request: &impl Serialize) -> Result<Vec<u8>> {
    let mut input = serde_json::to_vec(request).context("failed to encode worker request")?;
    input.push(b'\n');
    Ok(input)
}

fn open_worker_logs(ticket: &RunTicket) -> Result<(fs::File, fs::File)> {
    let path = ticket.worker_log_path()?;
    let parent = path
        .parent()
        .context("worker log has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let stdout = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("failed to clone {}", path.display()))?;
    Ok((stdout, stderr))
}

fn worker_command<T: TypedWorkerRequest>(
    executable: &Path,
    request: &T,
    stdout: fs::File,
    stderr: fs::File,
) -> Command {
    let mut command = Command::new(executable);
    command
        .arg(request.command_name())
        .current_dir(request.worktree())
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    command
}
