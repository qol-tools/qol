use crate::commands::dev_env::resources as dev_resources;
use crate::commands::{dev_env, emu};
use crate::progress::{print_hint, print_title, step_label, StepKind};
use crate::workspace::repo_root;
use anyhow::{anyhow, bail, Context, Result};
use qol_dev_env::{EnvironmentDefinition, ResolutionState, ResolvedEnvironment};
use qol_dev_orchestrator::{FlowStart, FlowWorkerRequest, RunHandle, RunTicket, MAX_FLOW_REPEATS};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MAX_REPEAT: u32 = MAX_FLOW_REPEATS;
const SUPERVISOR_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const SUPERVISOR_WAIT_INTERVAL: Duration = Duration::from_millis(25);
const PREPARATION_POLL_INTERVAL: Duration = Duration::from_millis(25);
const LANE_OWNERS_DIR: &str = "lanes";
const PAYLOAD_TRANSPORT: &str = "read-only-iso9660";

#[derive(Clone, Debug, PartialEq, Eq)]
struct FlowOptions {
    workflow: String,
    environment_id: String,
    run_id: Option<String>,
    worktree: Option<PathBuf>,
    repeat: u32,
    jobs: u32,
    memory_mb: Option<u32>,
    cpus: Option<u16>,
    force: bool,
}

#[derive(Clone)]
struct FlowPlan {
    start: FlowStart,
    environment: ResolvedEnvironment,
    workflow: emu::WorkflowDefinition,
    guest_adapter: emu::GuestAdapter,
    image_path: PathBuf,
    resources: dev_resources::ResourceProfile,
    concurrent: u32,
    run_root: PathBuf,
    ticket: RunTicket,
}

impl FlowPlan {
    fn fingerprint(&self) -> Result<String> {
        let workflow_kind = if self.workflow.requires_payload() {
            "desktop"
        } else {
            "serial"
        };
        let identity = json!({
            "schema": 1,
            "start": self.start,
            "ticket": {
                "run_id": self.ticket.run_id,
                "kind": self.ticket.kind.as_str(),
                "report_path": self.ticket.report_path,
            },
            "run_root": self.run_root,
            "environment": {
                "definition": {
                    "id": self.environment.definition.id,
                    "name": self.environment.definition.name,
                    "family": self.environment.definition.family,
                    "backend": self.environment.definition.backend,
                    "image": {
                        "kind": self.environment.definition.image.kind,
                        "base": self.environment.definition.image.base,
                        "recommended_size_gb": self.environment.definition.image.recommended_size_gb,
                        "arch": self.environment.definition.image.arch,
                        "firmware": self.environment.definition.image.firmware,
                    },
                    "boot": {
                        "memory_mb": self.environment.definition.boot.memory_mb,
                        "cpus": self.environment.definition.boot.cpus,
                        "display": self.environment.definition.boot.display,
                    },
                    "mounts": { "workspace": self.environment.definition.mounts.workspace },
                    "capabilities": self.environment.definition.capabilities,
                    "source": self.environment.definition.source,
                },
                "image_path": self.image_path,
                "verified_image": self.environment.verified_image,
                "run_root": self.environment.run_root,
            },
            "workflow": {
                "id": self.workflow.id(),
                "kind": workflow_kind,
                "requires_payload": self.workflow.requires_payload(),
            },
            "guest_adapter": self.guest_adapter.as_str(),
            "resources": {
                "memory_mb": self.resources.memory_mb,
                "cpus": self.resources.cpus,
                "concurrent": self.concurrent,
            },
            "payload_transport": PAYLOAD_TRANSPORT,
        });
        let encoded =
            serde_json::to_vec(&identity).context("failed to encode the immutable flow plan")?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }
}

struct ActiveLane {
    run_id: String,
    report_path: PathBuf,
    log_path: PathBuf,
    supervisor: Box<dyn Supervisor>,
}

struct LaneLaunch<'a> {
    executable: &'a Path,
    worktree: &'a Path,
    logs_dir: &'a Path,
    case_root: &'a Path,
    flow_run_id: &'a str,
    flow_report_path: &'a Path,
    owner_pid: u32,
    owner_process_identity: Option<String>,
}

struct PendingLane {
    run_id: String,
    args: Vec<OsString>,
}

#[derive(Debug)]
struct FlowPayload {
    root: PathBuf,
    manifest_path: PathBuf,
    image_path: PathBuf,
    manifest_sha256: String,
    cleanup: PayloadCleanup,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DesktopPayloadRecipe {
    companion: Option<DesktopCompanionRecipe>,
    tray_features: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DesktopCompanionRecipe {
    package: &'static str,
    binary: &'static str,
    plugin_dir: &'static str,
    plugin_id: &'static str,
}

#[derive(Debug)]
struct PayloadCleanup {
    status: String,
    complete: bool,
    removed: Vec<PathBuf>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct PreparationCleanup {
    status: String,
    complete: bool,
    verification: Option<String>,
    error: Option<String>,
}

impl PreparationCleanup {
    fn pending() -> Self {
        Self {
            status: "pending".to_string(),
            complete: false,
            verification: None,
            error: None,
        }
    }

    fn not_required() -> Self {
        Self {
            status: "not-required".to_string(),
            complete: true,
            verification: Some("no-process-spawned".to_string()),
            error: None,
        }
    }

    fn verified() -> Self {
        Self {
            status: "complete".to_string(),
            complete: true,
            verification: Some("owned-process-tree-exit".to_string()),
            error: None,
        }
    }

    fn incomplete(error: impl Into<String>) -> Self {
        let error = error.into();
        Self {
            status: "incomplete".to_string(),
            complete: false,
            verification: None,
            error: Some(error),
        }
    }
}

#[derive(Clone, Debug)]
struct FlowPreparation {
    status: String,
    build_status: String,
    process_status: Option<String>,
    cleanup: PreparationCleanup,
    iso_status: String,
    iso_process_status: Option<String>,
    iso_cleanup: PreparationCleanup,
}

#[derive(Clone, Debug)]
struct PreparationCommandJournal {
    run_id: String,
    phase: &'static str,
    path: PathBuf,
}

struct PreparationJournals {
    build: PreparationCommandJournal,
    iso: PreparationCommandJournal,
}

impl PreparationJournals {
    fn initialize(run_dir: &Path) -> Result<Self> {
        let run_id = run_dir
            .file_name()
            .and_then(|name| name.to_str())
            .context("flow preparation run directory has no UTF-8 identity")?;
        if !safe_run_id(run_id) {
            bail!("flow preparation run directory has an unsafe identity");
        }
        let root = run_dir.join("preparation");
        let build = PreparationCommandJournal {
            run_id: run_id.to_string(),
            phase: "build",
            path: root.join("build.json"),
        };
        let iso = PreparationCommandJournal {
            run_id: run_id.to_string(),
            phase: "iso",
            path: root.join("iso.json"),
        };
        build.record(
            "not-started",
            None,
            None,
            None,
            &PreparationCleanup::not_required(),
        )?;
        iso.record(
            "not-started",
            None,
            None,
            None,
            &PreparationCleanup::not_required(),
        )?;
        Ok(Self { build, iso })
    }
}

impl PreparationCommandJournal {
    fn record(
        &self,
        state: &str,
        process_id: Option<u32>,
        process_identity: Option<&str>,
        process_status: Option<&str>,
        cleanup: &PreparationCleanup,
    ) -> Result<()> {
        atomic_json_durable(
            &self.path,
            &json!({
                "kind": "flow-preparation-process",
                "run_id": self.run_id,
                "phase": self.phase,
                "state": state,
                "process": {
                    "pid": process_id,
                    "identity": process_identity,
                    "status": process_status,
                },
                "cleanup": {
                    "status": cleanup.status,
                    "complete": cleanup.complete,
                    "verification": cleanup.verification,
                    "error": cleanup.error,
                },
                "observed_at_unix_ms": qol_dev_env::unix_millis()?,
            }),
        )
    }
}

impl FlowPreparation {
    fn pending(requires_payload: bool) -> Self {
        if !requires_payload {
            return Self {
                status: "complete".to_string(),
                build_status: "skipped".to_string(),
                process_status: None,
                cleanup: PreparationCleanup::not_required(),
                iso_status: "skipped".to_string(),
                iso_process_status: None,
                iso_cleanup: PreparationCleanup::not_required(),
            };
        }
        Self {
            status: "preparing".to_string(),
            build_status: "pending".to_string(),
            process_status: None,
            cleanup: PreparationCleanup::pending(),
            iso_status: "pending".to_string(),
            iso_process_status: None,
            iso_cleanup: PreparationCleanup::pending(),
        }
    }

    fn cleanup_complete(&self) -> bool {
        self.cleanup.complete && self.iso_cleanup.complete
    }
}

#[derive(Debug)]
struct PayloadPreparationFailure {
    error: anyhow::Error,
    cancelled: bool,
    preparation: Box<FlowPreparation>,
}

impl PayloadPreparationFailure {
    fn before_spawn(error: anyhow::Error, cancelled: bool) -> Self {
        Self {
            error,
            cancelled,
            preparation: Box::new(FlowPreparation {
                status: if cancelled { "cancelled" } else { "failed" }.to_string(),
                build_status: if cancelled { "cancelled" } else { "failed" }.to_string(),
                process_status: None,
                cleanup: PreparationCleanup::not_required(),
                iso_status: "skipped".to_string(),
                iso_process_status: None,
                iso_cleanup: PreparationCleanup::not_required(),
            }),
        }
    }
}

impl PayloadCleanup {
    fn pending() -> Self {
        Self {
            status: "pending".to_string(),
            complete: false,
            removed: Vec::new(),
            error: None,
        }
    }
}

#[derive(Clone, Debug)]
struct SupervisorExit {
    success: bool,
    description: String,
    cleanup: LaneCleanup,
}

#[derive(Debug)]
struct ShutdownOutcome {
    process_status: String,
    error: Option<String>,
    cleanup: LaneCleanup,
}

#[derive(Clone, Debug)]
struct LaneCleanup {
    status: String,
    complete: bool,
    evidence_path: Option<PathBuf>,
    removed: Vec<PathBuf>,
    error: Option<String>,
}

struct GracefulShutdown {
    status: Option<ExitStatus>,
    error: Option<String>,
}

trait Supervisor: Send {
    fn try_wait(&mut self) -> Result<Option<SupervisorExit>>;
    fn shutdown(&mut self, reason: &str) -> ShutdownOutcome;
}

trait LaneSpawner {
    fn spawn(&mut self, launch: &LaneLaunch<'_>, pending: &PendingLane) -> Result<ActiveLane>;
}

trait CancellationSource {
    fn is_cancelled(&self) -> bool;
}

impl CancellationSource for qol_process::CancellationToken {
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }
}

impl CancellationSource for qol_dev_env::CancellationInbox {
    fn is_cancelled(&self) -> bool {
        self.is_requested().unwrap_or(true)
    }
}

struct FlowCancellation<'a> {
    signals: &'a dyn CancellationSource,
    inbox: &'a dyn CancellationSource,
}

impl CancellationSource for FlowCancellation<'_> {
    fn is_cancelled(&self) -> bool {
        self.signals.is_cancelled() || self.inbox.is_cancelled()
    }
}

struct ProcessLaneSpawner;

struct FlowJournal {
    report_path: PathBuf,
}

struct ChildSupervisor {
    executable: PathBuf,
    case_root: PathBuf,
    run_id: String,
    child: Option<Child>,
    process_tree: qol_process::ProcessTreeGuard,
}

impl Supervisor for ChildSupervisor {
    fn try_wait(&mut self) -> Result<Option<SupervisorExit>> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| anyhow!("supervisor process was already released"))?;
        let Some(status) = child
            .try_wait()
            .context("failed to poll supervisor process")?
        else {
            return Ok(None);
        };
        self.child = None;
        let proof = self
            .process_tree
            .terminate_and_wait(SUPERVISOR_SHUTDOWN_GRACE)
            .context("flow supervisor process tree did not terminate")?;
        let cleanup = self.reconcile_owned_run(&proof, "flow supervisor exited")?;
        Ok(Some(SupervisorExit {
            success: status.success(),
            description: status.to_string(),
            cleanup,
        }))
    }

    fn shutdown(&mut self, reason: &str) -> ShutdownOutcome {
        let (process_status, mut error, reaped) = self.stop_direct_supervisor();
        if !reaped {
            let cleanup = LaneCleanup::incomplete("supervisor process tree is not reaped");
            error = combine_errors(error, cleanup.error.clone());
            return ShutdownOutcome {
                process_status,
                error,
                cleanup,
            };
        }
        let proof = match self
            .process_tree
            .terminate_and_wait(SUPERVISOR_SHUTDOWN_GRACE)
        {
            Ok(proof) => proof,
            Err(tree_error) => {
                let cleanup = LaneCleanup::incomplete(format!(
                    "flow supervisor process tree did not terminate: {tree_error}"
                ));
                error = combine_errors(error, cleanup.error.clone());
                return ShutdownOutcome {
                    process_status,
                    error,
                    cleanup,
                };
            }
        };
        let cleanup = match self.reconcile_owned_run(&proof, reason) {
            Ok(cleanup) => cleanup,
            Err(cleanup_error) => LaneCleanup::incomplete(format!(
                "owned run cleanup could not be reconciled: {cleanup_error:#}"
            )),
        };
        error = combine_errors(error, cleanup.error.clone());
        ShutdownOutcome {
            process_status,
            error,
            cleanup,
        }
    }
}

impl ChildSupervisor {
    fn stop_direct_supervisor(&mut self) -> (String, Option<String>, bool) {
        let Some(mut child) = self.child.take() else {
            return ("exited".to_string(), None, true);
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                return (status.to_string(), None, true);
            }
            Ok(None) => {}
            Err(error) => {
                return self.fallback(child, Some(format!("failed to poll supervisor: {error}")))
            }
        }
        let graceful = request_shutdown_until_exit(
            &mut child,
            &self.executable,
            &self.case_root,
            &self.run_id,
            SUPERVISOR_SHUTDOWN_GRACE,
        );
        match graceful.status {
            Some(status) => (status.to_string(), graceful.error, true),
            None => self.fallback(child, graceful.error),
        }
    }

    fn fallback(
        &mut self,
        mut child: Child,
        prior_error: Option<String>,
    ) -> (String, Option<String>, bool) {
        match qol_process::terminate_owned(&mut child, SUPERVISOR_SHUTDOWN_GRACE) {
            Ok(()) => ("terminated".to_string(), prior_error, true),
            Err(error) => {
                self.child = Some(child);
                (
                    "termination failed".to_string(),
                    combine_errors(
                        prior_error,
                        format!("failed to terminate owned supervisor: {error}"),
                    ),
                    false,
                )
            }
        }
    }

    fn reconcile_owned_run(
        &self,
        proof: &qol_process::TerminatedProcessTree,
        reason: &str,
    ) -> Result<LaneCleanup> {
        let run_dir = self.case_root.join(&self.run_id);
        let cleanup = emu::reconcile_owned_terminated(&run_dir, &self.run_id, reason, proof)?;
        Ok(LaneCleanup::complete(cleanup))
    }
}

impl Drop for ChildSupervisor {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.shutdown("flow supervisor dropped");
        }
    }
}

impl LaneCleanup {
    fn complete(cleanup: emu::OwnedRunCleanup) -> Self {
        Self {
            status: "complete".to_string(),
            complete: true,
            evidence_path: Some(cleanup.evidence_path),
            removed: cleanup.removed,
            error: None,
        }
    }

    fn not_required() -> Self {
        Self {
            status: "not-required".to_string(),
            complete: true,
            evidence_path: None,
            removed: Vec::new(),
            error: None,
        }
    }

    fn incomplete(error: impl Into<String>) -> Self {
        let error = error.into();
        Self {
            status: "incomplete".to_string(),
            complete: false,
            evidence_path: None,
            removed: Vec::new(),
            error: Some(error),
        }
    }

    fn pending() -> Self {
        Self {
            status: "pending".to_string(),
            complete: false,
            evidence_path: None,
            removed: Vec::new(),
            error: None,
        }
    }
}

#[derive(Debug)]
struct LaneResult {
    run_id: String,
    report_path: PathBuf,
    log_path: PathBuf,
    phase: String,
    process_status: String,
    report_status: Option<String>,
    verdict: Option<String>,
    passed: bool,
    completed: bool,
    cleanup: LaneCleanup,
    error: Option<String>,
}

#[derive(Debug)]
struct ExecutionOutcome {
    results: Vec<LaneResult>,
    error: Option<String>,
    cancelled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FlowRunSummary {
    pub(crate) run_id: String,
    pub(crate) status: String,
    pub(crate) report_path: PathBuf,
}

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let Some(command) = args.first().and_then(|argument| argument.to_str()) else {
        print_help();
        return Ok(());
    };
    if crate::cli::help_only(&args[1..]) {
        print_help();
        return Ok(());
    }
    match command {
        "run" => run_flow(&parse_options(&args[1..])?, verbose),
        "runs" => cmd_runs(&args[1..]),
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        other => bail!("unknown flow command `{other}`\n\n{}", help_text()),
    }
}

fn run_flow(options: &FlowOptions, verbose: bool) -> Result<()> {
    let start = flow_start(options)?;
    let executable = std::env::current_exe().context("failed to resolve the qol executable")?;
    let signal_cancellation = qol_process::CancellationToken::install()
        .context("failed to install flow adapter cancellation handlers")?;
    let mut handle = start_typed_flow(&executable, start, verbose)?;
    print_title("qol flow run");
    print_hint(verbose);
    step_label(
        "start",
        StepKind::Info,
        &format!(
            "worker log {}",
            handle.ticket().worker_log_path()?.display()
        ),
    );
    wait_for_typed_flow(&mut handle, &signal_cancellation)
}

pub(crate) fn run_worker(args: &[OsString], executable: &Path) -> Result<()> {
    if !args.is_empty() {
        bail!("internal flow worker accepts typed standard input only");
    }
    qol_dev_env::require_host_session_cleared()
        .context("internal flow worker refused host session access")?;
    let request = qol_dev_orchestrator::read_flow_worker_request(std::io::stdin().lock())?;
    let plan = plan_flow(request.start.clone())?;
    validate_flow_worker_plan(&request, &plan)?;
    run_flow_coordinator(plan, executable, request.verbose)
}

pub(crate) fn start_typed_flow(
    executable: &Path,
    start: FlowStart,
    verbose: bool,
) -> Result<RunHandle> {
    let plan = plan_flow(start)?;
    let plan_fingerprint = plan.fingerprint()?;
    let guardian = qol_process::process_tree_guardian_command(executable);
    qol_dev_orchestrator::start_flow_worker(
        executable,
        guardian,
        FlowWorkerRequest {
            start: plan.start,
            run_root: plan.run_root,
            plan_fingerprint,
            verbose,
        },
        plan.ticket,
    )
}

fn validate_flow_worker_plan(request: &FlowWorkerRequest, plan: &FlowPlan) -> Result<()> {
    let expected_ticket = request.start.ticket(&request.run_root)?;
    if plan.run_root != request.run_root || plan.ticket != expected_ticket {
        bail!("flow configuration changed after the worker ticket was issued");
    }
    if plan.fingerprint()? != request.plan_fingerprint {
        bail!("flow plan changed before the typed worker started; retry the flow");
    }
    Ok(())
}

fn wait_for_typed_flow(
    handle: &mut RunHandle,
    cancellation: &qol_process::CancellationToken,
) -> Result<()> {
    let report_path = handle.ticket().report_path.clone();
    let mut previous_status = None;
    handle.wait_with_cancellation(
        cancellation,
        "flow",
        SUPERVISOR_SHUTDOWN_GRACE,
        None,
        |report| {
            let status = report.status.as_str().to_string();
            if previous_status.as_ref() != Some(&status) {
                step_label("status", StepKind::Info, &status);
                previous_status = Some(status);
            }
        },
        || step_label("cancel", StepKind::Info, "requested"),
        || {
            step_label(
                "cancel",
                StepKind::Info,
                "second signal · terminating owned worker tree",
            )
        },
        || {
            reconcile_flow_report_file(&report_path)
                .context("flow worker tree stopped, but its report could not be reconciled")?;
            dev_env::reconcile_resources().context(
                "flow worker tree stopped, but its resource lease could not be reconciled",
            )?;
            Ok(())
        },
        |_handle, report, worker_success| finish_typed_flow(report, worker_success),
    )
}

fn finish_typed_flow(report: qol_dev_env::RunSummary, worker_success: bool) -> Result<()> {
    step_label(
        "report",
        StepKind::Info,
        &report.report_path.display().to_string(),
    );
    if report.status == qol_dev_env::ReportStatus::Pass && worker_success {
        step_label("verdict", StepKind::Success, "all lanes passed");
        return Ok(());
    }
    if report.status == qol_dev_env::ReportStatus::Pass {
        bail!(
            "flow worker exited unsuccessfully after publishing a passing report: {}",
            report.report_path.display()
        );
    }
    let error = report
        .error
        .as_deref()
        .unwrap_or_else(|| report.status.as_str());
    bail!("flow finished with {}: {error}", report.status.as_str())
}

fn flow_start(options: &FlowOptions) -> Result<FlowStart> {
    let worktree = resolve_flow_worktree(options)?;
    let run_id = match &options.run_id {
        Some(run_id) => run_id.clone(),
        None => emu::new_run_id(&format!("flow-{}", options.workflow))?,
    };
    let start = FlowStart {
        workflow: options.workflow.clone(),
        environment_id: options.environment_id.clone(),
        worktree,
        run_id,
        repeat: options.repeat,
        jobs: options.jobs,
        memory_mb: options.memory_mb,
        cpus: options.cpus,
        force: options.force,
    };
    start.validate()?;
    Ok(start)
}

impl From<FlowStart> for FlowOptions {
    fn from(start: FlowStart) -> Self {
        Self {
            workflow: start.workflow,
            environment_id: start.environment_id,
            run_id: Some(start.run_id),
            worktree: Some(start.worktree),
            repeat: start.repeat,
            jobs: start.jobs,
            memory_mb: start.memory_mb,
            cpus: start.cpus,
            force: start.force,
        }
    }
}

fn validate_payload_admission(
    definition: &EnvironmentDefinition,
    workflow: emu::WorkflowDefinition,
) -> Result<()> {
    if workflow.requires_payload() && definition.mounts.workspace {
        bail!(
            "environment `{}` must disable the workspace mount before running immutable payload workflows",
            definition.id
        );
    }
    if workflow.requires_guest_revision() && !definition.capabilities.contains_key("image_revision")
    {
        bail!(
            "environment `{}` must declare image_revision before running desktop workflows",
            definition.id
        );
    }
    Ok(())
}

fn plan_flow(mut start: FlowStart) -> Result<FlowPlan> {
    start.validate()?;
    let worktree = resolve_flow_worktree(&FlowOptions::from(start.clone()))?;
    start.worktree = worktree.clone();
    let environment = environment(&worktree, &start.environment_id)?;
    let workflow = emu::workflow_definition(&start.workflow)?;
    let guest_adapter = require_flow_adapter(&environment)?;
    emu::validate_workflow_adapter(workflow, guest_adapter)?;
    validate_payload_admission(&environment.definition, workflow)?;
    let image_path = environment
        .image_path
        .clone()
        .ok_or_else(|| anyhow!("environment `{}` has no image path", start.environment_id))?;
    let configured_memory_mb = start.memory_mb.unwrap_or(
        u32::try_from(environment.definition.boot.memory_mb).with_context(|| {
            format!(
                "environment `{}` memory does not fit in u32",
                start.environment_id
            )
        })?,
    );
    let configured_cpus = start.cpus.unwrap_or(environment.definition.boot.cpus);
    let resources =
        dev_resources::profile(u64::from(configured_memory_mb), u64::from(configured_cpus))?;
    let concurrent = start.jobs.min(start.repeat);
    let run_root = environment
        .run_root
        .clone()
        .unwrap_or_else(|| worktree.join("target/qol-env"));
    let ticket = start.ticket(&run_root)?;
    Ok(FlowPlan {
        start,
        environment,
        workflow,
        guest_adapter,
        image_path,
        resources,
        concurrent,
        run_root,
        ticket,
    })
}

fn run_flow_coordinator(plan: FlowPlan, executable: &Path, verbose: bool) -> Result<()> {
    let mut process_tree = qol_process::guard_current_process_tree()
        .context("failed to guard the flow process tree")?;
    run_flow_inner(plan, executable, verbose)?;
    process_tree
        .disarm()
        .context("failed to disarm flow process-tree ownership")
}

fn resolve_flow_worktree(options: &FlowOptions) -> Result<PathBuf> {
    let root = match options.worktree.as_deref() {
        Some(worktree) => qol_workspace::workspace_root_from(worktree)
            .with_context(|| format!("invalid flow worktree {}", worktree.display()))?,
        None => repo_root()?,
    };
    root.canonicalize()
        .with_context(|| format!("failed to resolve flow worktree {}", root.display()))
}

fn run_flow_inner(plan: FlowPlan, executable: &Path, verbose: bool) -> Result<()> {
    let FlowPlan {
        start,
        environment,
        workflow,
        guest_adapter,
        image_path,
        resources,
        concurrent,
        run_root,
        ticket,
    } = plan;
    let options = FlowOptions::from(start.clone());
    let worktree = start.worktree;
    let memory_mb = resources.memory_mb;
    let cpus = resources.cpus;
    let batch_id = ticket.run_id;
    let flow_report_path = ticket.report_path;
    let signal_cancellation = qol_process::CancellationToken::install()
        .context("failed to install flow cancellation handlers")?;
    crate::commands::env::reconcile_for_admission()?;
    reconcile_all()?;
    dev_env::reconcile_resources()?;
    let case_root = run_root.join("cases");
    let cancellation_inbox = qol_dev_env::CancellationInbox::for_run(&batch_id)?;
    let cancellation = FlowCancellation {
        signals: &signal_cancellation,
        inbox: &cancellation_inbox,
    };
    if cancellation.is_cancelled() {
        bail!("flow execution cancelled before admission");
    }
    let run_dir = flow_report_path
        .parent()
        .context("flow ticket report has no run directory")?
        .to_path_buf();
    let logs_dir = run_dir.join("logs");
    let artifacts_dir = run_dir.join("artifacts");
    let steps_dir = run_dir.join("steps");
    let flows_dir = run_dir
        .parent()
        .context("flow run has no flows directory")?;
    fs::create_dir_all(flows_dir)
        .with_context(|| format!("failed to create {}", flows_dir.display()))?;
    fs::create_dir(&run_dir).with_context(|| {
        format!(
            "flow run `{batch_id}` already exists or {} could not be created",
            run_dir.display()
        )
    })?;
    let directories = (|| -> Result<()> {
        fs::create_dir(&logs_dir)
            .with_context(|| format!("failed to create {}", logs_dir.display()))?;
        fs::create_dir(&artifacts_dir)
            .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
        fs::create_dir(&steps_dir)
            .with_context(|| format!("failed to create {}", steps_dir.display()))
    })();
    if let Err(error) = directories {
        let cleanup = remove_unpublished_run_dir(&run_dir).err();
        return Err(combine_setup_errors(error, cleanup));
    }
    let (admission, resource_lease) = match dev_resources::reserve(
        &batch_id,
        &flow_report_path,
        dev_resources::AdmissionRequest {
            concurrent_lanes: u64::from(concurrent),
            profile: resources,
            recommended_size_gb: environment.definition.image.recommended_size_gb,
            capacity: dev_env::host_capacity(&run_root),
            force: options.force,
        },
    ) {
        Ok(reservation) => reservation,
        Err(error) => {
            let cleanup = remove_unpublished_run_dir(&run_dir).err();
            return Err(combine_setup_errors(error, cleanup));
        }
    };
    let mut payload = None;
    let unpublished_setup = (|| -> Result<_> {
        let started_at = qol_dev_env::unix_millis()?;
        let parent_lease = resource_lease.child_claim()?;
        let mut pending = Vec::with_capacity(options.repeat as usize);
        for index in 0..options.repeat {
            let run_id =
                emu::new_run_id(&format!("{}-lane-{}", options.environment_id, index + 1))?;
            pending.push(PendingLane {
                run_id,
                args: Vec::new(),
            });
        }
        Ok((pending, parent_lease, started_at))
    })();
    let (mut pending, parent_lease, started_at) = match unpublished_setup {
        Ok(setup) => setup,
        Err(error) => {
            return Err(rollback_unpublished_flow(
                resource_lease,
                &mut payload,
                &run_dir,
                &batch_id,
                &worktree,
                error,
            ))
        }
    };
    let lane_launch = LaneLaunch {
        executable,
        worktree: &worktree,
        logs_dir: &logs_dir,
        case_root: &case_root,
        flow_run_id: &batch_id,
        flow_report_path: &flow_report_path,
        owner_pid: std::process::id(),
        owner_process_identity: qol_process::process_identity(std::process::id()).ok(),
    };
    let planned = pending
        .iter()
        .map(|lane| planned_lane(&lane_launch, lane))
        .collect::<Vec<_>>();
    let mut preparation = FlowPreparation::pending(workflow.requires_payload());
    if let Err(error) = write_aggregate_report(
        &run_dir,
        &batch_id,
        &options,
        &worktree,
        &environment,
        &image_path,
        memory_mb,
        cpus,
        admission,
        started_at,
        payload.as_ref(),
        &preparation,
        "preparing",
        None,
        &planned,
    ) {
        return Err(rollback_unpublished_flow(
            resource_lease,
            &mut payload,
            &run_dir,
            &batch_id,
            &worktree,
            error.context("failed to publish flow preparation ownership"),
        ));
    }
    match prepare_workflow_payload(workflow, &worktree, &run_dir, verbose, &cancellation) {
        Ok((prepared_payload, prepared_state)) => {
            payload = prepared_payload;
            preparation = prepared_state;
        }
        Err(failure) => {
            preparation = *failure.preparation;
            let message = format!("failed to prepare workflow payload: {:#}", failure.error);
            return Err(finalize_pre_fanout_failure(
                resource_lease,
                &run_dir,
                &batch_id,
                &options,
                &worktree,
                &environment,
                &image_path,
                memory_mb,
                cpus,
                admission,
                started_at,
                &lane_launch,
                &pending,
                &mut payload,
                &preparation,
                failure.cancelled,
                message,
            ));
        }
    }
    let guest_image_revision = environment
        .definition
        .capabilities
        .get("image_revision")
        .map(String::as_str);
    let launch_arguments = (|| -> Result<()> {
        for pending in &mut pending {
            pending.args = emu::child_launch_args(emu::ChildLaunch {
                operation: emu::ChildOperation::Run(&options.workflow),
                target: &image_path,
                environment_id: &options.environment_id,
                run_id: &pending.run_id,
                parent_lease: &parent_lease,
                guest_adapter: Some(guest_adapter),
                guest_image_revision,
                payload_manifest: payload
                    .as_ref()
                    .map(|payload| payload.manifest_path.as_path()),
                payload_image: payload.as_ref().map(|payload| payload.image_path.as_path()),
                run_root: Some(&case_root),
                image_kind: Some(environment.definition.image.kind.as_str()),
                display: emu::DisplayMode::None,
                offline: workflow.requires_payload(),
                resources,
                acceleration: environment
                    .definition
                    .capabilities
                    .get("acceleration")
                    .map(String::as_str),
                arch: environment.definition.image.arch.as_deref(),
                firmware: environment.definition.image.firmware.as_deref(),
                usb_host: None,
            })?;
        }
        Ok(())
    })();
    if let Err(error) = launch_arguments {
        return Err(finalize_pre_fanout_failure(
            resource_lease,
            &run_dir,
            &batch_id,
            &options,
            &worktree,
            &environment,
            &image_path,
            memory_mb,
            cpus,
            admission,
            started_at,
            &lane_launch,
            &pending,
            &mut payload,
            &preparation,
            cancellation.is_cancelled(),
            format!("failed to prepare flow launch arguments: {error:#}"),
        ));
    }
    if cancellation.is_cancelled() {
        return Err(finalize_pre_fanout_failure(
            resource_lease,
            &run_dir,
            &batch_id,
            &options,
            &worktree,
            &environment,
            &image_path,
            memory_mb,
            cpus,
            admission,
            started_at,
            &lane_launch,
            &pending,
            &mut payload,
            &preparation,
            true,
            "flow execution cancelled during preparation".to_string(),
        ));
    }
    if let Err(error) = write_aggregate_report(
        &run_dir,
        &batch_id,
        &options,
        &worktree,
        &environment,
        &image_path,
        memory_mb,
        cpus,
        admission,
        started_at,
        payload.as_ref(),
        &preparation,
        "running",
        None,
        &planned,
    ) {
        return Err(finalize_pre_fanout_failure(
            resource_lease,
            &run_dir,
            &batch_id,
            &options,
            &worktree,
            &environment,
            &image_path,
            memory_mb,
            cpus,
            admission,
            started_at,
            &lane_launch,
            &pending,
            &mut payload,
            &preparation,
            cancellation.is_cancelled(),
            format!("failed to publish runnable flow ownership: {error:#}"),
        ));
    }
    let pre_spawn = write_preflight(&run_dir, &options, memory_mb, cpus, concurrent, admission)
        .and_then(|()| {
            write_effective_environment(&run_dir, &environment, &image_path, memory_mb, cpus)
        })
        .and_then(|()| prepare_lane_owners(&lane_launch, &pending));
    if let Err(error) = pre_spawn {
        return Err(finalize_pre_fanout_failure(
            resource_lease,
            &run_dir,
            &batch_id,
            &options,
            &worktree,
            &environment,
            &image_path,
            memory_mb,
            cpus,
            admission,
            started_at,
            &lane_launch,
            &pending,
            &mut payload,
            &preparation,
            cancellation.is_cancelled(),
            format!("failed to prepare flow launch: {error:#}"),
        ));
    }

    print_title("qol flow run");
    print_hint(verbose);
    step_label(
        "plan",
        StepKind::Info,
        &format!(
            "{} × {} · {} concurrent · {} MiB / {} CPU each",
            options.repeat, options.workflow, concurrent, memory_mb, cpus
        ),
    );

    let mut spawner = ProcessLaneSpawner;
    let ExecutionOutcome {
        mut results,
        error: execution_error,
        cancelled,
    } = execute_lanes(
        &mut spawner,
        &lane_launch,
        &pending,
        concurrent as usize,
        true,
        &cancellation,
        Some(&FlowJournal {
            report_path: flow_report_path.clone(),
        }),
    );
    let cancelled = cancelled || cancellation.is_cancelled();
    results.sort_by_key(|result| {
        pending
            .iter()
            .position(|lane| lane.run_id == result.run_id)
            .unwrap_or(usize::MAX)
    });
    let workflows_passed = !cancelled
        && execution_error.is_none()
        && results.len() == options.repeat as usize
        && results.iter().all(|result| result.passed);
    let lane_cleanup_complete = results.len() == options.repeat as usize
        && results.iter().all(|result| result.cleanup.complete);
    let payload_cleanup_error = if lane_cleanup_complete {
        cleanup_workflow_payload(&mut payload)
            .err()
            .map(|error| format!("payload cleanup failed: {error:#}"))
    } else {
        retain_payload_for_recovery(
            &mut payload,
            "payload retained because one or more lanes lack verified cleanup",
        );
        None
    };
    let cleanup_complete = lane_cleanup_complete && payload_cleanup_complete(payload.as_ref());
    let passed = workflows_passed && cleanup_complete;
    let status = flow_status(passed, cancelled, cleanup_complete);
    let terminal_error = combine_errors(
        terminal_error(execution_error.as_deref(), &results, options.repeat),
        payload_cleanup_error.clone(),
    );
    write_aggregate_report(
        &run_dir,
        &batch_id,
        &options,
        &worktree,
        &environment,
        &image_path,
        memory_mb,
        cpus,
        admission,
        started_at,
        payload.as_ref(),
        &preparation,
        status,
        terminal_error.as_deref(),
        &results,
    )?;
    match cleanup_complete {
        true => resource_lease
            .release()
            .context("failed to release the flow resource lease")?,
        false => resource_lease.retain(),
    }
    step_label(
        "report",
        StepKind::Info,
        &run_dir.join("report.json").display().to_string(),
    );
    if !passed {
        let failed = results
            .iter()
            .filter(|result| !result.passed)
            .map(|result| result.run_id.as_str())
            .collect::<Vec<_>>();
        if cancelled {
            bail!("flow execution cancelled");
        }
        if let Some(error) = execution_error {
            bail!("flow execution failed: {error}");
        }
        if let Some(error) = payload_cleanup_error {
            bail!("flow cleanup failed: {error}");
        }
        bail!(
            "{} flow lane(s) failed: {}",
            failed.len(),
            failed.join(", ")
        );
    }
    step_label(
        "verdict",
        StepKind::Success,
        &format!("{} lane(s) passed", results.len()),
    );
    Ok(())
}

fn flow_status(passed: bool, cancelled: bool, cleanup_complete: bool) -> &'static str {
    if !cleanup_complete {
        if cancelled {
            return "cancellation-cleanup-incomplete";
        }
        return "cleanup-incomplete";
    }
    if cancelled {
        return "cancelled";
    }
    if passed {
        return "pass";
    }
    "failed"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaneRecovery {
    Active,
    Resolved { passed: bool, completed: bool },
    Incomplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordedProcessState {
    VerifiedAlive,
    VerifiedDead,
    Uncertain,
}

fn recorded_process_state(
    pid: Option<u32>,
    identity: Option<&str>,
    process_group_counts_as_live: bool,
) -> RecordedProcessState {
    let Some(pid) = pid else {
        return RecordedProcessState::VerifiedDead;
    };
    let pid_alive = qol_process::is_pid_alive(pid);
    let group_alive = process_group_counts_as_live && qol_process::is_group_alive(pid);
    if !pid_alive && !group_alive {
        return RecordedProcessState::VerifiedDead;
    }
    let Some(identity) = identity else {
        return RecordedProcessState::Uncertain;
    };
    if pid_alive && qol_process::process_identity_matches(pid, identity) {
        return RecordedProcessState::VerifiedAlive;
    }
    if group_alive && !pid_alive {
        return RecordedProcessState::Uncertain;
    }
    RecordedProcessState::VerifiedDead
}

fn cmd_runs(args: &[OsString]) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol flow runs");
    }
    let runs = reconcile_all()?
        .into_iter()
        .filter(|run| !flow_status_is_terminal(&run.status))
        .collect::<Vec<_>>();
    dev_env::reconcile_resources()?;
    if runs.is_empty() {
        println!("No active or incomplete flows.");
        return Ok(());
    }
    println!("{:<64} {:<32} REPORT", "RUN ID", "STATUS");
    for run in runs {
        println!(
            "{:<64} {:<32} {}",
            run.run_id,
            run.status,
            run.report_path.display()
        );
    }
    Ok(())
}

pub(crate) fn reconcile_all() -> Result<Vec<FlowRunSummary>> {
    let roots = flow_run_roots()?;
    let case_roots = roots
        .iter()
        .map(|root| root.join("cases"))
        .collect::<Vec<_>>();
    let _ = emu::live_runs_in_roots(&case_roots);
    let mut summaries = Vec::new();
    let mut failures = Vec::new();
    for root in roots {
        let flows_root = root.join("flows");
        let entries = match fs::read_dir(&flows_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                failures.push(format!("{}: {error}", flows_root.display()));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    failures.push(format!("{}: {error}", flows_root.display()));
                    continue;
                }
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    failures.push(format!("{}: {error}", entry.path().display()));
                    continue;
                }
            };
            if !file_type.is_dir() {
                continue;
            }
            let report_path = entry.path().join("report.json");
            match reconcile_flow_report_file(&report_path) {
                Ok(Some(summary)) => summaries.push(summary),
                Ok(None) => {}
                Err(error) => failures.push(format!("{}: {error:#}", report_path.display())),
            }
        }
    }
    summaries.sort_by(|left, right| left.run_id.cmp(&right.run_id));
    if failures.is_empty() {
        return Ok(summaries);
    }
    bail!("failed to reconcile flows:\n{}", failures.join("\n"))
}

fn flow_run_roots() -> Result<Vec<PathBuf>> {
    Ok(dev_env::discover()?
        .into_iter()
        .filter_map(|environment| environment.run_root)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn reconcile_flow_report_file(path: &Path) -> Result<Option<FlowRunSummary>> {
    let Some(run_dir) = path.parent() else {
        bail!("flow report has no run directory");
    };
    let _lock = lock_flow_run(run_dir)?;
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };
    let mut report: Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if report.get("kind").and_then(Value::as_str) != Some("flow-fanout") {
        return Ok(None);
    }
    let run_id = report
        .get("run_id")
        .and_then(Value::as_str)
        .context("flow report has no run_id")?
        .to_string();
    validate_flow_run_directory(run_dir, &run_id)?;
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .context("flow report has no status")?
        .to_string();
    let cleanup_complete = qol_dev_env::parse_report(path, content.as_bytes())?
        .cleanup
        .is_complete();
    if flow_status_is_terminal(&status) && cleanup_complete {
        return Ok(Some(FlowRunSummary {
            run_id,
            status,
            report_path: path.to_path_buf(),
        }));
    }
    let owner_state = report
        .get("owner")
        .and_then(|owner| owner.get("state"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if owner_state.is_none()
        && repair_ownerless_legacy_flow(path, run_dir, &content, &mut report, &status)?
    {
        return Ok(Some(FlowRunSummary {
            run_id,
            status,
            report_path: path.to_path_buf(),
        }));
    }
    let owner_state = match owner_state {
        Some(owner_state) => owner_state,
        None if ownerless_terminal_flow_is_repairable(run_dir, &report, &status) => {
            report["owner"] = json!({ "state": "released" });
            "released".to_string()
        }
        None => {
            return Ok(Some(FlowRunSummary {
                run_id,
                status,
                report_path: path.to_path_buf(),
            }));
        }
    };
    validate_flow_lanes(run_dir, &report)?;
    let owner_pid = report
        .get("owner")
        .and_then(|owner| owner.get("pid"))
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid != 0);
    let owner_process_identity = report
        .get("owner")
        .and_then(|owner| owner.get("process_identity"))
        .and_then(Value::as_str);
    let owner_process_state = recorded_process_state(owner_pid, owner_process_identity, false);
    let owner_claims_live = matches!(owner_state.as_str(), "running" | "cancelling");
    let owner_identity_uncertain =
        matches!(owner_state.as_str(), "running" | "cancelling" | "orphaned")
            && matches!(owner_process_state, RecordedProcessState::Uncertain);
    if owner_claims_live && matches!(owner_process_state, RecordedProcessState::VerifiedAlive) {
        return Ok(Some(FlowRunSummary {
            run_id,
            status,
            report_path: path.to_path_buf(),
        }));
    }
    let observed_at = qol_dev_env::unix_millis()?;
    let interrupted_path = run_dir.join("report.interrupted.json");
    if fs::symlink_metadata(&interrupted_path).is_err() {
        atomic_write(&interrupted_path, content.as_bytes())?;
    }
    let mut recovered_status = reconcile_flow_lanes(run_dir, &run_id, &mut report, &owner_state)?;
    if owner_identity_uncertain {
        recovered_status = if owner_state == "cancelling" {
            "cancellation-cleanup-incomplete"
        } else {
            "cleanup-incomplete"
        };
    }
    reconcile_preparation_evidence(run_dir, &mut report);
    let preparation_cleanup_complete = report.get("preparation").is_none_or(|preparation| {
        ["build", "iso"].into_iter().all(|phase| {
            preparation
                .get(phase)
                .and_then(|phase| phase.get("cleanup"))
                .and_then(|cleanup| cleanup.get("complete"))
                .and_then(Value::as_bool)
                == Some(true)
        })
    });
    let terminal_candidate = matches!(
        recovered_status,
        "abandoned" | "cancelled" | "failed" | "pass"
    );
    let payload_cleanup_complete =
        !terminal_candidate || reconcile_recovered_payload(run_dir, &mut report).is_ok();
    if terminal_candidate && (!preparation_cleanup_complete || !payload_cleanup_complete) {
        recovered_status = if recovered_status == "cancelled" {
            "cancellation-cleanup-incomplete"
        } else {
            "cleanup-incomplete"
        };
    }
    report["status"] = json!(recovered_status);
    let recovered_report_status = qol_dev_env::ReportStatus::parse(recovered_status);
    let reconciliation_status = match &recovered_report_status {
        qol_dev_env::ReportStatus::CleanupIncomplete
        | qol_dev_env::ReportStatus::RollbackIncomplete
        | qol_dev_env::ReportStatus::CancellationCleanupIncomplete => recovered_status,
        status if status.is_terminal() => "complete",
        _ => "in-progress",
    };
    report["reconciliation"] = json!({
        "status": reconciliation_status,
        "previous_status": status,
        "owner_pid": owner_pid,
        "owner_state": owner_state,
        "observed_at_unix_ms": observed_at,
        "interrupted_report": interrupted_path,
    });
    report["owner"]["state"] = json!(if recovered_report_status.is_active()
        || owner_identity_uncertain
    {
        "orphaned"
    } else {
        "released"
    });
    update_recovered_flow_lifecycle(&mut report, recovered_status, observed_at);
    atomic_json(&run_dir.join("steps/lifecycle.json"), &report["steps"])?;
    atomic_json(path, &report)?;
    Ok(Some(FlowRunSummary {
        run_id,
        status: recovered_status.to_string(),
        report_path: path.to_path_buf(),
    }))
}

fn repair_ownerless_legacy_flow(
    path: &Path,
    run_dir: &Path,
    content: &str,
    report: &mut Value,
    status: &str,
) -> Result<bool> {
    if !flow_status_is_terminal(status) {
        return Ok(false);
    }
    let Some(run_root) = run_dir.parent().and_then(Path::parent) else {
        return Ok(false);
    };
    let current_root = run_root.join("cases");
    let legacy_root = run_root.parent().unwrap_or(run_root).join("qol-emu");
    let Some(lanes) = report.get("lanes").and_then(Value::as_array) else {
        return Ok(false);
    };
    let requested = report
        .get("workflow")
        .and_then(|workflow| workflow.get("repeat"))
        .and_then(Value::as_u64);
    if requested != u64::try_from(lanes.len()).ok() {
        return Ok(false);
    }
    let mut seen = BTreeSet::new();
    let mut repairs = Vec::with_capacity(lanes.len());
    for lane in lanes {
        let Some(run_id) = lane.get("run_id").and_then(Value::as_str) else {
            return Ok(false);
        };
        if !safe_run_id(run_id) || !seen.insert(run_id.to_string()) {
            return Ok(false);
        }
        let recorded_report = lane
            .get("report")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        let current_report = current_root.join(run_id).join("report.json");
        let legacy_report = legacy_root.join(run_id).join("report.json");
        let Some(recorded_report) =
            recorded_report.filter(|path| path == &current_report || path == &legacy_report)
        else {
            return Ok(false);
        };
        let expected_log = run_dir.join("logs").join(format!("{run_id}.log"));
        let recorded_log = lane.get("log").and_then(Value::as_str).map(PathBuf::from);
        if recorded_log.as_ref() != Some(&expected_log) {
            return Ok(false);
        }
        let child = qol_dev_env::read_report_checked(
            &recorded_report,
            run_id,
            &qol_dev_env::ReportKind::Flow,
        )?;
        if let Some(child) = child {
            if !child.cleanup.is_complete() {
                return Ok(false);
            }
            repairs.push(LegacyFlowLaneRepair::Child {
                report_path: recorded_report,
                document: child.document().clone(),
            });
            continue;
        }
        let not_started = status == "cancelled"
            && recorded_report == current_report
            && lane.get("process_status").and_then(Value::as_str) == Some("not started")
            && lane.get("report_status").is_none_or(Value::is_null)
            && lane.get("completed").and_then(Value::as_bool) == Some(false)
            && lane.get("passed").and_then(Value::as_bool) == Some(false)
            && fs::symlink_metadata(lane_owner_path(run_dir, run_id)).is_err();
        if !not_started {
            return Ok(false);
        }
        repairs.push(LegacyFlowLaneRepair::NotStarted);
    }
    let lanes = report
        .get_mut("lanes")
        .and_then(Value::as_array_mut)
        .context("flow report has no mutable lane plan")?;
    for (lane, repair) in lanes.iter_mut().zip(repairs) {
        match repair {
            LegacyFlowLaneRepair::Child {
                report_path,
                document,
            } => {
                let report_status = document
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let verdict = document
                    .get("workflow")
                    .and_then(|workflow| workflow.get("verdict"))
                    .and_then(Value::as_str);
                let passed = report_status == "pass" && verdict == Some("pass");
                mark_lane_resolved(
                    lane,
                    &report_path,
                    &document,
                    report_status,
                    verdict,
                    passed,
                )?;
            }
            LegacyFlowLaneRepair::NotStarted => mark_lane_unspawned(
                lane,
                "not-started",
                "not started",
                "flow was cancelled before this lane launched",
            )?,
        }
    }
    let observed_at = qol_dev_env::unix_millis()?;
    let interrupted_path = run_dir.join("report.interrupted.json");
    if fs::symlink_metadata(&interrupted_path).is_err() {
        atomic_write(&interrupted_path, content.as_bytes())?;
    }
    report["owner"] = json!({ "state": "released" });
    report["reconciliation"] = json!({
        "status": "complete",
        "source": "qol-flow-legacy-lane-root-v1",
        "observed_at_unix_ms": observed_at,
        "interrupted_report": interrupted_path,
    });
    let repaired = serde_json::to_vec_pretty(report).context("failed to serialize flow report")?;
    if !qol_dev_env::parse_report(path, &repaired)?
        .cleanup
        .is_complete()
    {
        return Ok(false);
    }
    atomic_write(path, &[repaired.as_slice(), b"\n"].concat())?;
    Ok(true)
}

enum LegacyFlowLaneRepair {
    Child {
        report_path: PathBuf,
        document: Value,
    },
    NotStarted,
}

fn ownerless_terminal_flow_is_repairable(run_dir: &Path, report: &Value, status: &str) -> bool {
    if !flow_status_is_terminal(status) || validate_flow_lanes(run_dir, report).is_err() {
        return false;
    }
    let Some(lanes) = report.get("lanes").and_then(Value::as_array) else {
        return false;
    };
    lanes.iter().all(|lane| {
        let Some(run_id) = lane.get("run_id").and_then(Value::as_str) else {
            return false;
        };
        let Ok((report_path, _)) = canonical_lane_paths(run_dir, run_id) else {
            return false;
        };
        qol_dev_env::read_report_checked(&report_path, run_id, &qol_dev_env::ReportKind::Flow)
            .ok()
            .flatten()
            .is_some_and(|report| report.cleanup.is_complete())
    })
}

fn reconcile_preparation_evidence(run_dir: &Path, report: &mut Value) {
    let Some(preparation) = report.get_mut("preparation").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(run_id) = run_dir.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    for phase in ["build", "iso"] {
        let Some(phase_report) = preparation.get_mut(phase).and_then(Value::as_object_mut) else {
            continue;
        };
        let already_complete = phase_report
            .get("cleanup")
            .and_then(|cleanup| cleanup.get("complete"))
            .and_then(Value::as_bool)
            == Some(true);
        if already_complete {
            continue;
        }
        let path = run_dir.join("preparation").join(format!("{phase}.json"));
        let evidence = match read_optional_json(&path) {
            Ok(Some(evidence)) => evidence,
            Ok(None) => continue,
            Err(error) => {
                phase_report.insert("cleanup".to_string(), preparation_cleanup_error(error));
                continue;
            }
        };
        let valid = evidence.get("kind").and_then(Value::as_str)
            == Some("flow-preparation-process")
            && evidence.get("run_id").and_then(Value::as_str) == Some(run_id)
            && evidence.get("phase").and_then(Value::as_str) == Some(phase);
        if !valid {
            phase_report.insert(
                "cleanup".to_string(),
                preparation_cleanup_error(format!(
                    "preparation evidence identity mismatch: {}",
                    path.display()
                )),
            );
            continue;
        }
        let cleanup = match validated_preparation_evidence_cleanup(&evidence, &path) {
            Ok(cleanup) => cleanup,
            Err(error) => {
                phase_report.insert("cleanup".to_string(), preparation_cleanup_error(error));
                continue;
            }
        };
        phase_report.insert("cleanup".to_string(), cleanup);
        let process_status = evidence
            .get("process")
            .and_then(|process| process.get("status"))
            .cloned()
            .unwrap_or(Value::Null);
        phase_report.insert("process_status".to_string(), process_status);
        if evidence.get("state").and_then(Value::as_str) == Some("not-started") {
            phase_report.insert("status".to_string(), json!("skipped"));
        }
    }
}

fn validated_preparation_evidence_cleanup(
    evidence: &Value,
    path: &Path,
) -> std::result::Result<Value, String> {
    let state = evidence
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("preparation evidence has no state: {}", path.display()))?;
    let cleanup = evidence
        .get("cleanup")
        .filter(|cleanup| cleanup.is_object())
        .ok_or_else(|| {
            format!(
                "preparation evidence has no cleanup state: {}",
                path.display()
            )
        })?;
    let status = cleanup.get("status").and_then(Value::as_str);
    let complete = cleanup.get("complete").and_then(Value::as_bool);
    let verification = cleanup.get("verification").and_then(Value::as_str);
    let valid = match state {
        "not-started" => {
            status == Some("not-required")
                && complete == Some(true)
                && verification == Some("no-process-spawned")
                && evidence
                    .get("process")
                    .and_then(|process| process.get("pid"))
                    .is_none_or(Value::is_null)
        }
        "complete" => {
            status == Some("complete")
                && complete == Some(true)
                && verification == Some("owned-process-tree-exit")
        }
        "launching" | "running" => status == Some("pending") && complete == Some(false),
        "cleanup-incomplete" => status == Some("incomplete") && complete == Some(false),
        _ => false,
    };
    if valid {
        return Ok(cleanup.clone());
    }
    Err(format!(
        "preparation evidence cleanup contract is invalid: {}",
        path.display()
    ))
}

fn preparation_cleanup_error(error: String) -> Value {
    json!({
        "status": "incomplete",
        "complete": false,
        "verification": null,
        "error": error,
    })
}

fn reconcile_recovered_payload(run_dir: &Path, report: &mut Value) -> Result<()> {
    let Some(payload) = report.get("payload").filter(|payload| !payload.is_null()) else {
        return Ok(());
    };
    if payload
        .get("cleanup")
        .and_then(|cleanup| cleanup.get("complete"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Ok(());
    }
    let run_dir = run_dir
        .canonicalize()
        .with_context(|| format!("failed to canonicalize flow run {}", run_dir.display()))?;
    let payload_dir = run_dir.join("payload");
    let root = payload_dir.join("root");
    let manifest_path = payload
        .get("manifest")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .context("flow payload has no manifest path")?;
    if manifest_path != root.join("manifest.json") {
        bail!("flow payload manifest is outside its owned run directory");
    }
    let manifest_sha256 = payload
        .get("manifest_sha256")
        .and_then(Value::as_str)
        .context("flow payload has no manifest digest")?
        .to_string();
    if manifest_sha256.len() != 64
        || !manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("flow payload manifest digest is invalid");
    }
    let image_path = payload
        .get("image")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .context("flow payload has no image path")?;
    if image_path != payload_dir.join(format!("{manifest_sha256}.iso")) {
        bail!("flow payload image is outside its owned run directory");
    }
    if root
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        bail!("flow payload root is a symlink");
    }
    let mut recovered = Some(FlowPayload {
        root,
        manifest_path,
        image_path,
        manifest_sha256,
        cleanup: PayloadCleanup::pending(),
    });
    let cleanup = cleanup_workflow_payload(&mut recovered);
    report["payload"]["cleanup"] = recovered
        .as_ref()
        .map(|payload| {
            json!({
                "status": payload.cleanup.status,
                "complete": payload.cleanup.complete,
                "removed": payload.cleanup.removed,
                "error": payload.cleanup.error,
            })
        })
        .unwrap_or(Value::Null);
    cleanup
}

fn lock_flow_run(run_dir: &Path) -> Result<File> {
    qol_dev_env::lock_run_directory(run_dir, "reconcile.lock")
}

fn validate_flow_run_directory(run_dir: &Path, run_id: &str) -> Result<()> {
    if !safe_run_id(run_id) {
        bail!("invalid flow run identity `{run_id}`");
    }
    let directory_id = run_dir
        .file_name()
        .and_then(|name| name.to_str())
        .context("flow run directory has no UTF-8 identity")?;
    if directory_id != run_id {
        bail!("flow run directory identity mismatch: expected `{run_id}`, got `{directory_id}`");
    }
    if run_dir
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        != Some("flows")
    {
        bail!("flow run is not inside a canonical `flows` directory");
    }
    Ok(())
}

fn validate_flow_lanes(run_dir: &Path, report: &Value) -> Result<()> {
    let lanes = report
        .get("lanes")
        .and_then(Value::as_array)
        .context("flow report has no lane plan")?;
    let requested = report
        .get("workflow")
        .and_then(|workflow| workflow.get("repeat"))
        .and_then(Value::as_u64)
        .context("flow report has no repeat count")?;
    if lanes.len() != requested as usize {
        bail!(
            "flow lane plan has {} entries but requested {requested}",
            lanes.len()
        );
    }
    let mut identities = BTreeSet::new();
    for lane in lanes {
        let run_id = lane
            .get("run_id")
            .and_then(Value::as_str)
            .context("flow lane has no run_id")?;
        if !safe_run_id(run_id) {
            bail!("invalid flow lane identity `{run_id}`");
        }
        if !identities.insert(run_id) {
            bail!("duplicate flow lane identity `{run_id}`");
        }
        let (report_path, log_path) = canonical_lane_paths(run_dir, run_id)?;
        let recorded_report = lane
            .get("report")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .context("flow lane has no report path")?;
        let recorded_log = lane
            .get("log")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .context("flow lane has no log path")?;
        if recorded_report != report_path || recorded_log != log_path {
            bail!("flow lane `{run_id}` path contract does not match its canonical location");
        }
    }
    Ok(())
}

fn safe_run_id(run_id: &str) -> bool {
    qol_dev_env::is_safe_run_id_component(run_id)
}

fn canonical_lane_paths(run_dir: &Path, run_id: &str) -> Result<(PathBuf, PathBuf)> {
    let run_root = run_dir
        .parent()
        .and_then(Path::parent)
        .context("flow run has no canonical run root")?;
    Ok((
        run_root.join("cases").join(run_id).join("report.json"),
        run_dir.join("logs").join(format!("{run_id}.log")),
    ))
}

fn lane_owner_path(flow_run_dir: &Path, run_id: &str) -> PathBuf {
    flow_run_dir
        .join(LANE_OWNERS_DIR)
        .join(format!("{run_id}.json"))
}

fn reconcile_flow_lanes(
    run_dir: &Path,
    flow_run_id: &str,
    report: &mut Value,
    owner_state: &str,
) -> Result<&'static str> {
    let prior_status = report
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let lanes = report
        .get_mut("lanes")
        .and_then(Value::as_array_mut)
        .context("flow report has no mutable lane plan")?;
    let mut active = false;
    let mut incomplete = false;
    let mut all_passed = true;
    for lane in lanes {
        match reconcile_flow_lane(run_dir, flow_run_id, lane)? {
            LaneRecovery::Active => {
                active = true;
                all_passed = false;
            }
            LaneRecovery::Resolved { passed, .. } => all_passed &= passed,
            LaneRecovery::Incomplete => {
                incomplete = true;
                all_passed = false;
            }
        }
    }
    let cancellation = owner_state == "cancelling"
        || matches!(
            prior_status.as_str(),
            "cancelled" | "cancelling" | "cancellation-cleanup-incomplete"
        );
    if active {
        return Ok("recovering");
    }
    if incomplete {
        if cancellation {
            return Ok("cancellation-cleanup-incomplete");
        }
        return Ok("cleanup-incomplete");
    }
    if cancellation {
        return Ok("cancelled");
    }
    if prior_status == "abandoned" {
        return Ok("abandoned");
    }
    if prior_status == "failed" {
        return Ok("failed");
    }
    if matches!(owner_state, "running" | "orphaned") {
        return Ok("abandoned");
    }
    if all_passed && prior_status == "pass" {
        return Ok("pass");
    }
    Ok("failed")
}

fn reconcile_flow_lane(
    run_dir: &Path,
    flow_run_id: &str,
    lane: &mut Value,
) -> Result<LaneRecovery> {
    let run_id = lane
        .get("run_id")
        .and_then(Value::as_str)
        .context("flow lane has no run_id")?
        .to_string();
    let (report_path, _) = canonical_lane_paths(run_dir, &run_id)?;
    let journal_path = lane_owner_path(run_dir, &run_id);
    let journal = match read_optional_json(&journal_path) {
        Ok(journal) => journal,
        Err(error) => {
            mark_lane_incomplete(lane, None, error.clone())?;
            return Ok(LaneRecovery::Incomplete);
        }
    };
    if let Some(journal) = &journal {
        if journal.get("kind").and_then(Value::as_str) != Some("flow-lane-owner")
            || journal.get("run_id").and_then(Value::as_str) != Some(run_id.as_str())
            || journal.get("flow_run_id").and_then(Value::as_str) != Some(flow_run_id)
        {
            mark_lane_incomplete(
                lane,
                None,
                "flow lane ownership journal identity mismatch".to_string(),
            )?;
            return Ok(LaneRecovery::Incomplete);
        }
    }
    let child = match read_optional_flow_report(&report_path, &run_id) {
        Ok(child) => child,
        Err(error) => {
            mark_lane_incomplete(lane, None, error.clone())?;
            return Ok(LaneRecovery::Incomplete);
        }
    };
    let supervisor_pid = journal
        .as_ref()
        .and_then(|journal| journal.get("supervisor_pid"))
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok());
    let supervisor_process_identity = journal
        .as_ref()
        .and_then(|journal| journal.get("supervisor_process_identity"))
        .and_then(Value::as_str);
    let supervisor_state =
        recorded_process_state(supervisor_pid, supervisor_process_identity, true);
    let Some(child) = child else {
        match supervisor_state {
            RecordedProcessState::VerifiedAlive => {
                mark_lane_active(lane, None)?;
                return Ok(LaneRecovery::Active);
            }
            RecordedProcessState::Uncertain => {
                mark_lane_incomplete(
                    lane,
                    None,
                    "lane supervisor liveness is uncertain and no child report proves cleanup"
                        .to_string(),
                )?;
                return Ok(LaneRecovery::Incomplete);
            }
            RecordedProcessState::VerifiedDead => {}
        }
        let phase = journal
            .as_ref()
            .and_then(|journal| journal.get("phase"))
            .and_then(Value::as_str)
            .or_else(|| lane.get("phase").and_then(Value::as_str));
        match phase {
            Some("planned") => mark_lane_unspawned(
                lane,
                "not-started",
                "not started",
                "flow owner exited before this lane launched",
            )?,
            Some("spawn-failed") => mark_lane_unspawned(
                lane,
                "spawn-failed",
                "spawn failed",
                "lane spawn failed without leaving an owned child process",
            )?,
            _ => {
                mark_lane_incomplete(
                    lane,
                    None,
                    "lane may have spawned but has no child report or verified cleanup".to_string(),
                )?;
                return Ok(LaneRecovery::Incomplete);
            }
        }
        return Ok(LaneRecovery::Resolved {
            passed: false,
            completed: false,
        });
    };
    if child.get("run_id").and_then(Value::as_str) != Some(run_id.as_str()) {
        mark_lane_incomplete(lane, None, "child report identity mismatch".to_string())?;
        return Ok(LaneRecovery::Incomplete);
    }
    let child_status = child
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if matches!(child_status, "starting" | "running" | "stopping") {
        let child_process_state =
            combine_process_states(supervisor_state, child_process_state(&child));
        match child_process_state {
            RecordedProcessState::VerifiedAlive => {
                mark_lane_active(lane, Some(child_status))?;
                return Ok(LaneRecovery::Active);
            }
            RecordedProcessState::Uncertain => {
                mark_lane_incomplete(
                    lane,
                    Some(child_status),
                    "active child report has uncertain process identity and no verified cleanup"
                        .to_string(),
                )?;
                return Ok(LaneRecovery::Incomplete);
            }
            RecordedProcessState::VerifiedDead => {}
        }
        mark_lane_incomplete(
            lane,
            Some(child_status),
            "active child report has no live owner and no verified cleanup".to_string(),
        )?;
        return Ok(LaneRecovery::Incomplete);
    }
    if let Some(error) = child_cleanup_complete(&child, child_status).err() {
        mark_lane_incomplete(lane, Some(child_status), error)?;
        return Ok(LaneRecovery::Incomplete);
    }
    let verdict = child
        .get("workflow")
        .and_then(|workflow| workflow.get("verdict"))
        .and_then(Value::as_str);
    let passed = child_status == "pass" && verdict == Some("pass");
    mark_lane_resolved(lane, &report_path, &child, child_status, verdict, passed)?;
    Ok(LaneRecovery::Resolved {
        passed,
        completed: true,
    })
}

fn read_optional_json(path: &Path) -> std::result::Result<Option<Value>, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
    serde_json::from_str(&content)
        .map(Some)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn read_optional_flow_report(
    path: &Path,
    run_id: &str,
) -> std::result::Result<Option<Value>, String> {
    qol_dev_env::read_report_checked(path, run_id, &qol_dev_env::ReportKind::Flow)
        .map(|report| report.map(|report| report.document().clone()))
        .map_err(|error| format!("{error:#}"))
}

fn child_process_state(report: &Value) -> RecordedProcessState {
    let runtime = report.get("runtime");
    let supervisor = recorded_process_state(
        runtime
            .and_then(|runtime| runtime.get("supervisor_pid"))
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok()),
        runtime
            .and_then(|runtime| runtime.get("supervisor_process_identity"))
            .and_then(Value::as_str),
        true,
    );
    let qemu = recorded_process_state(
        runtime
            .and_then(|runtime| runtime.get("qemu_pid"))
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok()),
        runtime
            .and_then(|runtime| runtime.get("qemu_process_identity"))
            .and_then(Value::as_str),
        false,
    );
    combine_process_states(supervisor, qemu)
}

fn combine_process_states(
    first: RecordedProcessState,
    second: RecordedProcessState,
) -> RecordedProcessState {
    if matches!(first, RecordedProcessState::VerifiedAlive)
        || matches!(second, RecordedProcessState::VerifiedAlive)
    {
        return RecordedProcessState::VerifiedAlive;
    }
    if matches!(first, RecordedProcessState::Uncertain)
        || matches!(second, RecordedProcessState::Uncertain)
    {
        return RecordedProcessState::Uncertain;
    }
    RecordedProcessState::VerifiedDead
}

fn child_cleanup_complete(report: &Value, status: &str) -> std::result::Result<(), String> {
    let parsed_status = qol_dev_env::ReportStatus::parse(status);
    match qol_dev_env::report::child_cleanup(report, &parsed_status) {
        qol_dev_env::CleanupState::Complete => Ok(()),
        qol_dev_env::CleanupState::Incomplete(error) => Err(error),
        qol_dev_env::CleanupState::Pending => {
            Err(format!("child report has nonterminal status `{status}`"))
        }
    }
}

fn mark_lane_active(lane: &mut Value, report_status: Option<&str>) -> Result<()> {
    let object = lane.as_object_mut().context("flow lane is not an object")?;
    object.insert("phase".to_string(), json!("spawned"));
    object.insert("passed".to_string(), json!(false));
    object.insert("completed".to_string(), json!(false));
    object.insert("process_status".to_string(), json!("active"));
    object.insert("report_status".to_string(), json!(report_status));
    object.insert(
        "cleanup".to_string(),
        json!({
            "status": "pending",
            "complete": false,
            "evidence": null,
            "removed": [],
            "error": null,
        }),
    );
    object.remove("error");
    Ok(())
}

fn mark_lane_incomplete(
    lane: &mut Value,
    report_status: Option<&str>,
    error: String,
) -> Result<()> {
    let object = lane.as_object_mut().context("flow lane is not an object")?;
    object.insert("passed".to_string(), json!(false));
    object.insert("completed".to_string(), json!(false));
    object.insert("process_status".to_string(), json!("cleanup incomplete"));
    object.insert("report_status".to_string(), json!(report_status));
    object.insert(
        "cleanup".to_string(),
        json!({
            "status": "incomplete",
            "complete": false,
            "evidence": null,
            "removed": [],
            "error": error,
        }),
    );
    object.insert("error".to_string(), json!(error));
    Ok(())
}

fn mark_lane_unspawned(
    lane: &mut Value,
    phase: &str,
    process_status: &str,
    error: &str,
) -> Result<()> {
    let object = lane.as_object_mut().context("flow lane is not an object")?;
    object.insert("phase".to_string(), json!(phase));
    object.insert("passed".to_string(), json!(false));
    object.insert("completed".to_string(), json!(false));
    object.insert("process_status".to_string(), json!(process_status));
    object.insert("report_status".to_string(), Value::Null);
    object.insert("verdict".to_string(), Value::Null);
    object.insert(
        "cleanup".to_string(),
        json!({
            "status": "not-required",
            "complete": true,
            "evidence": null,
            "removed": [],
            "error": null,
        }),
    );
    object.insert("error".to_string(), json!(error));
    Ok(())
}

fn mark_lane_resolved(
    lane: &mut Value,
    report_path: &Path,
    child: &Value,
    report_status: &str,
    verdict: Option<&str>,
    passed: bool,
) -> Result<()> {
    let object = lane.as_object_mut().context("flow lane is not an object")?;
    let removed = child
        .get("teardown")
        .and_then(|teardown| teardown.get("removed"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    object.insert("phase".to_string(), json!("completed"));
    object.insert("passed".to_string(), json!(passed));
    object.insert("completed".to_string(), json!(true));
    object.insert("process_status".to_string(), json!("reconciled"));
    object.insert("report_status".to_string(), json!(report_status));
    object.insert("verdict".to_string(), json!(verdict));
    object.insert(
        "cleanup".to_string(),
        json!({
            "status": "complete",
            "complete": true,
            "evidence": report_path,
            "removed": removed,
            "error": null,
        }),
    );
    object.remove("error");
    Ok(())
}

fn update_recovered_flow_lifecycle(report: &mut Value, status: &str, observed_at: u64) {
    let completed_lanes = report
        .get("lanes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|lane| lane.get("completed").and_then(Value::as_bool) == Some(true))
        .count();
    let reported_lanes = report
        .get("lanes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let requested_lanes = report
        .get("workflow")
        .and_then(|workflow| workflow.get("repeat"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if let Some(fanout) = report
        .get_mut("steps")
        .and_then(Value::as_array_mut)
        .and_then(|steps| {
            steps
                .iter_mut()
                .find(|step| step.get("name").and_then(Value::as_str) == Some("fanout"))
        })
        .and_then(Value::as_object_mut)
    {
        fanout.insert("status".to_string(), json!(status));
        fanout.insert("completed_lanes".to_string(), json!(completed_lanes));
        fanout.insert("reported_lanes".to_string(), json!(reported_lanes));
        fanout.insert("requested_lanes".to_string(), json!(requested_lanes));
    }
    if flow_status_is_terminal(status) {
        report["finished_at_unix_ms"] = json!(observed_at);
    } else if let Some(object) = report.as_object_mut() {
        object.remove("finished_at_unix_ms");
    }
    let error = match status {
        "abandoned" => Some("flow owner exited before aggregate finalization"),
        "cleanup-incomplete" | "cancellation-cleanup-incomplete" => {
            Some("one or more flow lanes lack verified cleanup")
        }
        _ => None,
    };
    if let Some(error) = error {
        report["error"] = json!(error);
    }
}

fn environment(worktree: &Path, id: &str) -> Result<ResolvedEnvironment> {
    let environment = dev_env::find_in(worktree, id)?
        .ok_or_else(|| anyhow!("unknown environment `{id}`; run `qol env list`"))?;
    if environment.state != ResolutionState::Ready {
        let detail = environment.messages.join("; ");
        bail!(
            "environment `{id}` is {}: {detail}",
            environment.state.as_str()
        );
    }
    Ok(environment)
}

fn require_flow_adapter(environment: &ResolvedEnvironment) -> Result<emu::GuestAdapter> {
    configured_flow_adapter(&environment.definition.capabilities).with_context(|| {
        format!(
            "environment `{}` cannot run automated flows",
            environment.definition.id
        )
    })
}

fn prepare_workflow_payload(
    workflow: emu::WorkflowDefinition,
    worktree: &Path,
    run_dir: &Path,
    verbose: bool,
    cancellation: &impl CancellationSource,
) -> std::result::Result<(Option<FlowPayload>, FlowPreparation), PayloadPreparationFailure> {
    match workflow.payload_recipe() {
        None | Some(emu::PayloadRecipe::None) => Ok((None, FlowPreparation::pending(false))),
        Some(emu::PayloadRecipe::Desktop) => {
            prepare_desktop_workflow_payload(workflow, worktree, run_dir, verbose, cancellation)
        }
        Some(emu::PayloadRecipe::ResidentWave2) => {
            prepare_resident_workflow_payload(workflow.id(), worktree, run_dir, cancellation)
        }
    }
}

fn prepare_resident_workflow_payload(
    workflow_id: &str,
    worktree: &Path,
    run_dir: &Path,
    cancellation: &impl CancellationSource,
) -> std::result::Result<(Option<FlowPayload>, FlowPreparation), PayloadPreparationFailure> {
    if !crate::host_facade::supports_immutable_payload_build() {
        return Err(PayloadPreparationFailure::before_spawn(
            anyhow!("workflow `{workflow_id}` currently requires an x86_64 Linux build host"),
            false,
        ));
    }
    if cancellation.is_cancelled() {
        return Err(PayloadPreparationFailure::before_spawn(
            anyhow!("flow execution cancelled before bundle preparation"),
            true,
        ));
    }
    let journals = PreparationJournals::initialize(run_dir)
        .map_err(|error| PayloadPreparationFailure::before_spawn(error, false))?;
    let cache_root = worktree.join(emu::resident_wave2::bundle::BUNDLE_CACHE_ROOT);
    let snapshot_dir = run_dir.join("bundle-snapshot");
    step_label(
        "bundle",
        StepKind::Pending,
        "resolving the resident product bundle",
    );
    let current = std::env::current_exe().map_err(|error| {
        PayloadPreparationFailure::before_spawn(
            anyhow!("failed to resolve the qol executable: {error}"),
            false,
        )
    })?;
    let mut prepare = Command::new(current);
    prepare.args(emu::resident_wave2::bundle::prepare_argv(
        worktree,
        &cache_root,
        &snapshot_dir,
    ));
    let status = run_owned_preparation_command(prepare, cancellation, &journals.build).map_err(
        |mut failure| {
            failure.error = failure.error.context("failed to run bundle preparation");
            failure
        },
    )?;
    if !status.success() {
        return Err(PayloadPreparationFailure {
            error: anyhow!("resident bundle preparation exited with {status}"),
            cancelled: false,
            preparation: Box::new(FlowPreparation {
                status: "failed".to_string(),
                build_status: "failed".to_string(),
                process_status: Some(status.to_string()),
                cleanup: PreparationCleanup::verified(),
                iso_status: "skipped".to_string(),
                iso_process_status: None,
                iso_cleanup: PreparationCleanup::not_required(),
            }),
        });
    }
    step_label("bundle", StepKind::Success, "product bundle is ready");
    let mut preparation = FlowPreparation {
        status: "complete".to_string(),
        build_status: "pass".to_string(),
        process_status: Some(status.to_string()),
        cleanup: PreparationCleanup::verified(),
        iso_status: "pending".to_string(),
        iso_process_status: None,
        iso_cleanup: PreparationCleanup::pending(),
    };
    if cancellation.is_cancelled() {
        preparation.status = "cancelled".to_string();
        preparation.iso_status = "skipped".to_string();
        preparation.iso_cleanup = PreparationCleanup::not_required();
        return Err(PayloadPreparationFailure {
            error: anyhow!("flow execution cancelled after bundle preparation"),
            cancelled: true,
            preparation: Box::new(preparation),
        });
    }
    let files = emu::resident_wave2::bundle::snapshot_payload_files(&snapshot_dir, run_dir)
        .map_err(|error| {
            let cancelled = cancellation.is_cancelled();
            PayloadPreparationFailure {
                error,
                cancelled,
                preparation: Box::new(failed_preparation(preparation.clone(), cancelled)),
            }
        })?;
    let payload_dir = run_dir.join("payload");
    let prepared =
        qol_dev_env::payload::stage_payload(&payload_dir.join("root"), workflow_id, &files)
            .map_err(|error| {
                let cancelled = cancellation.is_cancelled();
                PayloadPreparationFailure {
                    error,
                    cancelled,
                    preparation: Box::new(failed_preparation(preparation.clone(), cancelled)),
                }
            })?;
    let iso_tool = emu::find_on_path("genisoimage")
        .or_else(|| emu::find_on_path("mkisofs"))
        .context("missing genisoimage or mkisofs on PATH")
        .map_err(|error| {
            let cancelled = cancellation.is_cancelled();
            PayloadPreparationFailure {
                error,
                cancelled,
                preparation: Box::new(failed_preparation(preparation.clone(), cancelled)),
            }
        })?;
    let mut iso_process_failure = None;
    let mut iso_process_status = None;
    let image = match qol_dev_env::payload::create_read_only_iso_with_runner(
        &prepared,
        &payload_dir,
        iso_tool.as_os_str(),
        |mut command| {
            dev_env::clear_host_session(&mut command);
            match run_owned_preparation_command(command, cancellation, &journals.iso) {
                Ok(status) => {
                    iso_process_status = Some(status);
                    Ok(status)
                }
                Err(failure) => {
                    let detail = format!("{:#}", failure.error);
                    iso_process_failure = Some(failure);
                    Err(anyhow!(detail))
                }
            }
        },
    ) {
        Ok(image) => {
            if let Some(status) = iso_process_status {
                preparation.iso_status = "pass".to_string();
                preparation.iso_process_status = Some(status.to_string());
                preparation.iso_cleanup = PreparationCleanup::verified();
            } else {
                preparation.iso_status = "reused".to_string();
                preparation.iso_cleanup = PreparationCleanup::not_required();
            }
            image
        }
        Err(error) => {
            let mut preparation =
                failed_preparation(preparation.clone(), cancellation.is_cancelled());
            if let Some(failure) = iso_process_failure {
                preparation.cleanup = failure.preparation.cleanup;
                preparation.iso_cleanup = failure.preparation.iso_cleanup;
                preparation.iso_process_status = failure.preparation.iso_process_status;
                preparation.iso_status = failure.preparation.iso_status.clone();
            }
            return Err(PayloadPreparationFailure {
                error: error.context("failed to create the resident payload ISO"),
                cancelled: false,
                preparation: Box::new(preparation),
            });
        }
    };
    Ok((
        Some(FlowPayload {
            root: prepared.root,
            manifest_path: prepared.manifest_path,
            image_path: image.path,
            manifest_sha256: image.manifest_sha256,
            cleanup: PayloadCleanup::pending(),
        }),
        preparation,
    ))
}

fn prepare_desktop_workflow_payload(
    workflow: emu::WorkflowDefinition,
    worktree: &Path,
    run_dir: &Path,
    verbose: bool,
    cancellation: &impl CancellationSource,
) -> std::result::Result<(Option<FlowPayload>, FlowPreparation), PayloadPreparationFailure> {
    let recipe = desktop_payload_recipe(workflow.id()).ok_or_else(|| {
        PayloadPreparationFailure::before_spawn(
            anyhow!(
                "workflow `{}` declares a payload but has no desktop payload recipe",
                workflow.id()
            ),
            false,
        )
    })?;
    if !crate::host_facade::supports_immutable_payload_build() {
        return Err(PayloadPreparationFailure::before_spawn(
            anyhow!(
                "workflow `{}` currently requires an x86_64 Linux build host",
                workflow.id()
            ),
            false,
        ));
    }
    if cancellation.is_cancelled() {
        return Err(PayloadPreparationFailure::before_spawn(
            anyhow!("flow execution cancelled before payload build"),
            true,
        ));
    }
    let journals = PreparationJournals::initialize(run_dir)
        .map_err(|error| PayloadPreparationFailure::before_spawn(error, false))?;
    let cargo = emu::find_on_path("cargo")
        .context("missing cargo on PATH")
        .map_err(|error| PayloadPreparationFailure::before_spawn(error, false))?;
    step_label("build", StepKind::Pending, &payload_build_label(recipe));
    let mut build = Command::new(&cargo);
    build.current_dir(worktree).args([
        "build",
        "--profile",
        "sandbox",
        "-p",
        "qol-tray",
        "--bin",
        "qol-tray",
    ]);
    if let Some(features) = recipe.tray_features {
        build.args(["--features", features]);
    }
    if let Some(companion) = recipe.companion {
        build.args(["-p", companion.package, "--bin", companion.binary]);
    }
    if !verbose {
        build.arg("--quiet");
    }
    let identity =
        qol_build_identity::BuildIdentityEnvironment::sandbox(worktree).map_err(|error| {
            PayloadPreparationFailure::before_spawn(
                anyhow!("failed to resolve sandbox build identity: {error}"),
                false,
            )
        })?;
    identity.apply_to(&mut build);
    dev_env::clear_host_session(&mut build);
    let status = run_owned_preparation_command(build, cancellation, &journals.build).map_err(
        |mut failure| {
            failure.error = failure
                .error
                .context(format!("failed to run {}", cargo.display()));
            failure
        },
    )?;
    if !status.success() {
        return Err(PayloadPreparationFailure {
            error: anyhow!("sandbox payload build exited with {status}"),
            cancelled: false,
            preparation: Box::new(FlowPreparation {
                status: "failed".to_string(),
                build_status: "failed".to_string(),
                process_status: Some(status.to_string()),
                cleanup: PreparationCleanup::verified(),
                iso_status: "skipped".to_string(),
                iso_process_status: None,
                iso_cleanup: PreparationCleanup::not_required(),
            }),
        });
    }
    step_label("build", StepKind::Success, "sandbox binaries are ready");
    let mut preparation = FlowPreparation {
        status: "complete".to_string(),
        build_status: "pass".to_string(),
        process_status: Some(status.to_string()),
        cleanup: PreparationCleanup::verified(),
        iso_status: "pending".to_string(),
        iso_process_status: None,
        iso_cleanup: PreparationCleanup::pending(),
    };
    if cancellation.is_cancelled() {
        preparation.status = "cancelled".to_string();
        preparation.iso_status = "skipped".to_string();
        preparation.iso_cleanup = PreparationCleanup::not_required();
        return Err(PayloadPreparationFailure {
            error: anyhow!("flow execution cancelled after payload build"),
            cancelled: true,
            preparation: Box::new(preparation),
        });
    }

    let target_root = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(target) if Path::new(&target).is_absolute() => PathBuf::from(target),
        Some(target) => worktree.join(target),
        None => worktree.join("target"),
    };
    let binary_dir = target_root.join("sandbox");
    let mut files = vec![qol_dev_env::payload::PayloadFileSpec {
        source: binary_dir.join(crate::workspace::exe_name("qol-tray")),
        relative_path: PathBuf::from("bin/qol-tray"),
        executable: true,
    }];
    if let Some(companion) = recipe.companion {
        let plugin_files = desktop_plugin_payload_files(worktree, companion).map_err(|error| {
            PayloadPreparationFailure {
                error: error.context("failed to collect desktop plugin payload files"),
                cancelled: false,
                preparation: Box::new(failed_preparation(preparation.clone(), false)),
            }
        })?;
        files.extend(plugin_files);
        files.push(qol_dev_env::payload::PayloadFileSpec {
            source: binary_dir.join(crate::workspace::exe_name(companion.binary)),
            relative_path: Path::new("plugins")
                .join(companion.plugin_id)
                .join(companion.binary),
            executable: true,
        });
    }
    files.push(qol_dev_env::payload::PayloadFileSpec {
        source: worktree.join("flows/envs/linux-mint-cinnamon/qol-sandbox-payload"),
        relative_path: PathBuf::from("installer/qol-sandbox-payload"),
        executable: true,
    });
    let payload_dir = run_dir.join("payload");
    let prepared =
        qol_dev_env::payload::stage_payload(&payload_dir.join("root"), workflow.id(), &files)
            .map_err(|error| {
                let cancelled = cancellation.is_cancelled();
                PayloadPreparationFailure {
                    error,
                    cancelled,
                    preparation: Box::new(failed_preparation(preparation.clone(), cancelled)),
                }
            })?;
    let iso_tool = emu::find_on_path("genisoimage")
        .or_else(|| emu::find_on_path("mkisofs"))
        .context("missing genisoimage or mkisofs on PATH")
        .map_err(|error| {
            let cancelled = cancellation.is_cancelled();
            PayloadPreparationFailure {
                error,
                cancelled,
                preparation: Box::new(failed_preparation(preparation.clone(), cancelled)),
            }
        })?;
    let mut iso_process_failure = None;
    let mut iso_process_status = None;
    let image = match qol_dev_env::payload::create_read_only_iso_with_runner(
        &prepared,
        &payload_dir,
        iso_tool.as_os_str(),
        |mut command| {
            dev_env::clear_host_session(&mut command);
            match run_owned_preparation_command(command, cancellation, &journals.iso) {
                Ok(status) => {
                    iso_process_status = Some(status);
                    Ok(status)
                }
                Err(failure) => {
                    let detail = format!("{:#}", failure.error);
                    iso_process_failure = Some(failure);
                    Err(anyhow!(detail))
                }
            }
        },
    ) {
        Ok(image) => {
            if let Some(status) = iso_process_status {
                preparation.iso_status = "pass".to_string();
                preparation.iso_process_status = Some(status.to_string());
                preparation.iso_cleanup = PreparationCleanup::verified();
            } else {
                preparation.iso_status = "reused".to_string();
                preparation.iso_cleanup = PreparationCleanup::not_required();
            }
            image
        }
        Err(error) => {
            if let Some(failure) = iso_process_failure {
                let process_preparation = *failure.preparation;
                preparation.status = process_preparation.status;
                preparation.iso_status = process_preparation.build_status;
                preparation.iso_process_status = process_preparation.process_status;
                preparation.iso_cleanup = process_preparation.cleanup;
                return Err(PayloadPreparationFailure {
                    error: failure
                        .error
                        .context("immutable payload ISO process failed"),
                    cancelled: failure.cancelled,
                    preparation: Box::new(preparation),
                });
            }
            let cancelled = cancellation.is_cancelled();
            if let Some(status) = iso_process_status {
                preparation.iso_status = "failed".to_string();
                preparation.iso_process_status = Some(status.to_string());
                preparation.iso_cleanup = PreparationCleanup::verified();
            }
            return Err(PayloadPreparationFailure {
                error: error.context("failed to create the immutable workflow payload"),
                cancelled,
                preparation: Box::new(failed_preparation(preparation, cancelled)),
            });
        }
    };
    if cancellation.is_cancelled() {
        preparation.status = "cancelled".to_string();
        return Err(PayloadPreparationFailure {
            error: anyhow!("flow execution cancelled after payload ISO creation"),
            cancelled: true,
            preparation: Box::new(preparation),
        });
    }
    step_label(
        "payload",
        StepKind::Success,
        &format!("{} · shared read-only by every lane", image.path.display()),
    );
    Ok((
        Some(FlowPayload {
            root: prepared.root,
            manifest_path: prepared.manifest_path,
            image_path: image.path,
            manifest_sha256: image.manifest_sha256,
            cleanup: PayloadCleanup::pending(),
        }),
        preparation,
    ))
}

fn desktop_plugin_payload_files(
    worktree: &Path,
    recipe: DesktopCompanionRecipe,
) -> Result<Vec<qol_dev_env::payload::PayloadFileSpec>> {
    let plugin_root = worktree.join("plugins").join(recipe.plugin_dir);
    let executable_name = crate::workspace::exe_name(recipe.binary);
    let files =
        qol_workspace::plugin_delivery_files(&plugin_root, &[recipe.binary, &executable_name])?;
    Ok(files
        .into_iter()
        .map(|file| qol_dev_env::payload::PayloadFileSpec {
            executable: false,
            source: file.source,
            relative_path: Path::new("plugins")
                .join(recipe.plugin_id)
                .join(file.relative_path),
        })
        .collect())
}

fn payload_build_label(recipe: DesktopPayloadRecipe) -> String {
    match recipe.companion {
        Some(companion) => format!(
            "qol-tray + {} · optimized probe-enabled sandbox profile",
            companion.package
        ),
        None => "qol-tray · optimized probe-enabled sandbox profile".to_string(),
    }
}

fn desktop_payload_recipe(workflow_id: &str) -> Option<DesktopPayloadRecipe> {
    if matches!(
        workflow_id,
        "hotkey-shadow" | "hotkey-shadow-boot" | "hotkey-storm"
    ) {
        return Some(DesktopPayloadRecipe {
            companion: Some(DesktopCompanionRecipe {
                package: "launcher",
                binary: "launcher",
                plugin_dir: "launcher",
                plugin_id: "plugin-launcher",
            }),
            tray_features: Some("linux_evdev"),
        });
    }
    let companion = match workflow_id {
        "alt-tab-performance" | "alt-tab-storm" => DesktopCompanionRecipe {
            package: "alt-tab",
            binary: "alt-tab",
            plugin_dir: "alt-tab",
            plugin_id: "plugin-alt-tab",
        },
        "bluetooth-storm" => DesktopCompanionRecipe {
            package: "plugin-bluetooth",
            binary: "plugin-bluetooth",
            plugin_dir: "bluetooth",
            plugin_id: "plugin-bluetooth",
        },
        "launcher-storm" | "portable-session" => DesktopCompanionRecipe {
            package: "launcher",
            binary: "launcher",
            plugin_dir: "launcher",
            plugin_id: "plugin-launcher",
        },
        "qol-shot-capture" | "qol-shot-storm" => DesktopCompanionRecipe {
            package: "qol-shot",
            binary: "qol-shot",
            plugin_dir: "qol-shot",
            plugin_id: "qol-shot",
        },
        "shortcut-storm" => {
            return Some(DesktopPayloadRecipe {
                companion: None,
                tray_features: None,
            });
        }
        "window-actions-storm" => DesktopCompanionRecipe {
            package: "window-actions",
            binary: "window-actions",
            plugin_dir: "window-actions",
            plugin_id: "plugin-window-actions",
        },
        _ => return None,
    };
    Some(DesktopPayloadRecipe {
        companion: Some(companion),
        tray_features: None,
    })
}

fn failed_preparation(mut preparation: FlowPreparation, cancelled: bool) -> FlowPreparation {
    preparation.status = if cancelled { "cancelled" } else { "failed" }.to_string();
    if preparation.iso_status == "pending" {
        preparation.iso_status = "skipped".to_string();
        preparation.iso_cleanup = PreparationCleanup::not_required();
    }
    preparation
}

fn run_owned_preparation_command(
    mut command: Command,
    cancellation: &impl CancellationSource,
    journal: &PreparationCommandJournal,
) -> std::result::Result<ExitStatus, PayloadPreparationFailure> {
    if cancellation.is_cancelled() {
        return Err(PayloadPreparationFailure::before_spawn(
            anyhow!("flow execution cancelled before preparation command"),
            true,
        ));
    }
    let process_tree = crate::process_guardian::own_process_tree()
        .context("failed to create preparation command process-tree ownership")
        .map_err(|error| PayloadPreparationFailure::before_spawn(error, false))?;
    qol_process::isolate_owned_command(&mut command)
        .context("failed to isolate preparation command process tree")
        .map_err(|error| PayloadPreparationFailure::before_spawn(error, false))?;
    journal
        .record(
            "launching",
            None,
            None,
            None,
            &PreparationCleanup::pending(),
        )
        .map_err(|error| PayloadPreparationFailure::before_spawn(error, false))?;
    let prepared = match process_tree.prepare_command(command) {
        Ok(prepared) => prepared,
        Err(error) => {
            let mut error =
                anyhow!(error).context("failed to contain preparation command before exec");
            if let Err(evidence_error) = journal.record(
                "not-started",
                None,
                None,
                Some("prepare-failed"),
                &PreparationCleanup::not_required(),
            ) {
                error = error.context(format!(
                    "failed to persist preparation failure evidence: {evidence_error:#}"
                ));
            }
            return Err(PayloadPreparationFailure::before_spawn(error, false));
        }
    };
    let mut child = match prepared.spawn() {
        Ok(child) => child,
        Err(error) => {
            let mut failure = preparation_spawn_failure(error, cancellation.is_cancelled());
            let cleanup = &failure.preparation.cleanup;
            let state = if cleanup.complete {
                "complete"
            } else {
                "cleanup-incomplete"
            };
            if let Err(error) = journal.record(state, None, None, Some("spawn-failed"), cleanup) {
                failure.error = failure.error.context(format!(
                    "failed to persist preparation spawn cleanup: {error:#}"
                ));
            }
            return Err(failure);
        }
    };
    let child_pid = child.id();
    let child_identity = qol_process::process_identity(child_pid).ok();
    if let Err(error) = journal.record(
        "running",
        Some(child_pid),
        child_identity.as_deref(),
        None,
        &PreparationCleanup::pending(),
    ) {
        let cleanup = terminate_preparation_process_recorded(
            &mut child,
            &process_tree,
            journal,
            child_pid,
            child_identity.as_deref(),
            "journal-failed",
        );
        return Err(preparation_process_failure(
            error.context("failed to persist preparation process ownership"),
            cancellation.is_cancelled(),
            "journal-failed",
            cleanup,
        ));
    }
    loop {
        if cancellation.is_cancelled() {
            let cleanup = terminate_preparation_process_recorded(
                &mut child,
                &process_tree,
                journal,
                child_pid,
                child_identity.as_deref(),
                "cancelled",
            );
            return Err(preparation_process_failure(
                anyhow!("flow execution cancelled while running a preparation command"),
                true,
                "cancelled",
                cleanup,
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let process_status = status.to_string();
                let cleanup = terminate_preparation_process_recorded(
                    &mut child,
                    &process_tree,
                    journal,
                    child_pid,
                    child_identity.as_deref(),
                    &process_status,
                );
                if let Err(error) = cleanup {
                    return Err(preparation_process_failure(
                        anyhow!("preparation command exited with {status}"),
                        false,
                        &process_status,
                        Err(error),
                    ));
                }
                return Ok(status);
            }
            Ok(None) => thread::sleep(PREPARATION_POLL_INTERVAL),
            Err(error) => {
                let cleanup = terminate_preparation_process_recorded(
                    &mut child,
                    &process_tree,
                    journal,
                    child_pid,
                    child_identity.as_deref(),
                    "wait-failed",
                );
                return Err(preparation_process_failure(
                    anyhow!(error).context("failed to poll preparation command"),
                    cancellation.is_cancelled(),
                    "wait-failed",
                    cleanup,
                ));
            }
        }
    }
}

fn terminate_preparation_process_recorded(
    child: &mut Child,
    process_tree: &qol_process::ProcessTreeGuard,
    journal: &PreparationCommandJournal,
    child_pid: u32,
    child_identity: Option<&str>,
    process_status: &str,
) -> Result<()> {
    let interrupted_cleanup = PreparationCleanup::incomplete(
        "preparation cleanup began but terminal process-tree proof was not persisted",
    );
    let intent = journal
        .record(
            "cleanup-incomplete",
            Some(child_pid),
            child_identity,
            Some(process_status),
            &interrupted_cleanup,
        )
        .context("failed to persist terminal preparation cleanup intent");
    let cleanup = terminate_preparation_process(child, process_tree);
    let cleanup_evidence = match &cleanup {
        Ok(()) => PreparationCleanup::verified(),
        Err(error) => PreparationCleanup::incomplete(format!("{error:#}")),
    };
    let state = if cleanup_evidence.complete {
        "complete"
    } else {
        "cleanup-incomplete"
    };
    let terminal = journal
        .record(
            state,
            Some(child_pid),
            child_identity,
            Some(process_status),
            &cleanup_evidence,
        )
        .context("failed to persist terminal preparation cleanup evidence");
    let errors = [intent.err(), cleanup.err(), terminal.err()]
        .into_iter()
        .flatten()
        .map(|error| format!("{error:#}"))
        .collect::<Vec<_>>();
    if errors.is_empty() {
        return Ok(());
    }
    bail!(errors.join("; "))
}

fn preparation_spawn_failure(
    error: qol_process::PreparedSpawnError,
    cancelled: bool,
) -> PayloadPreparationFailure {
    let cleanup = match error.cleanup() {
        qol_process::PreparedSpawnCleanup::NotStarted => PreparationCleanup::not_required(),
        qol_process::PreparedSpawnCleanup::Verified => PreparationCleanup::verified(),
        qol_process::PreparedSpawnCleanup::RecoveryPending => {
            PreparationCleanup::incomplete(error.to_string())
        }
    };
    let cleanup_pending = !cleanup.complete;
    PayloadPreparationFailure {
        error: anyhow!(error).context("failed to spawn preparation command"),
        cancelled,
        preparation: Box::new(FlowPreparation {
            status: if cleanup_pending {
                "cleanup-incomplete"
            } else if cancelled {
                "cancelled"
            } else {
                "failed"
            }
            .to_string(),
            build_status: if cancelled { "cancelled" } else { "failed" }.to_string(),
            process_status: Some("spawn-failed".to_string()),
            cleanup,
            iso_status: "skipped".to_string(),
            iso_process_status: None,
            iso_cleanup: PreparationCleanup::not_required(),
        }),
    }
}

fn preparation_process_failure(
    error: anyhow::Error,
    cancelled: bool,
    process_status: &str,
    cleanup: Result<()>,
) -> PayloadPreparationFailure {
    let (status, cleanup) = match cleanup {
        Ok(()) => (
            if cancelled { "cancelled" } else { "failed" },
            PreparationCleanup::verified(),
        ),
        Err(cleanup) => (
            if cancelled {
                "cancellation-cleanup-incomplete"
            } else {
                "cleanup-incomplete"
            },
            PreparationCleanup::incomplete(format!("{cleanup:#}")),
        ),
    };
    PayloadPreparationFailure {
        error,
        cancelled,
        preparation: Box::new(FlowPreparation {
            status: status.to_string(),
            build_status: if cancelled { "cancelled" } else { "failed" }.to_string(),
            process_status: Some(process_status.to_string()),
            cleanup,
            iso_status: "skipped".to_string(),
            iso_process_status: None,
            iso_cleanup: PreparationCleanup::not_required(),
        }),
    }
}

fn terminate_preparation_process(
    child: &mut Child,
    process_tree: &qol_process::ProcessTreeGuard,
) -> Result<()> {
    let direct = qol_process::terminate_owned(child, SUPERVISOR_SHUTDOWN_GRACE)
        .context("failed to stop preparation child");
    let reaped = child.wait().context("failed to reap preparation child");
    let tree = process_tree
        .terminate_and_wait(SUPERVISOR_SHUTDOWN_GRACE)
        .map(|_proof| ())
        .context("preparation command descendants survived cleanup");
    let errors = [direct.err(), reaped.err(), tree.err()]
        .into_iter()
        .flatten()
        .map(|error| format!("{error:#}"))
        .collect::<Vec<_>>();
    if errors.is_empty() {
        return Ok(());
    }
    bail!(errors.join("; "))
}

fn retain_payload_for_recovery(payload: &mut Option<FlowPayload>, reason: &str) {
    if let Some(payload) = payload {
        payload.cleanup.status = "retained".to_string();
        payload.cleanup.complete = false;
        payload.cleanup.error = Some(reason.to_string());
    }
}

fn cleanup_workflow_payload(payload: &mut Option<FlowPayload>) -> Result<()> {
    let Some(payload) = payload else {
        return Ok(());
    };
    let mut removed = Vec::new();
    let cleanup = (|| -> Result<()> {
        match fs::remove_file(&payload.image_path) {
            Ok(()) => removed.push(payload.image_path.clone()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to remove payload image {}",
                        payload.image_path.display()
                    )
                })
            }
        }
        if payload.root.exists() {
            removed.push(qol_dev_env::payload::remove_payload(&payload.root)?);
        }
        if let Some(parent) = payload.root.parent() {
            match fs::remove_dir(parent) {
                Ok(()) => removed.push(parent.to_path_buf()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to remove payload directory {}", parent.display())
                    })
                }
            }
        }
        Ok(())
    })();
    payload.cleanup.removed = removed;
    match cleanup {
        Ok(()) => {
            payload.cleanup.status = "complete".to_string();
            payload.cleanup.complete = true;
            payload.cleanup.error = None;
            Ok(())
        }
        Err(error) => {
            payload.cleanup.status = "incomplete".to_string();
            payload.cleanup.complete = false;
            payload.cleanup.error = Some(format!("{error:#}"));
            Err(error)
        }
    }
}

fn payload_cleanup_complete(payload: Option<&FlowPayload>) -> bool {
    payload.is_none_or(|payload| payload.cleanup.complete)
}

#[allow(clippy::too_many_arguments)]
fn finalize_pre_fanout_failure(
    resource_lease: dev_resources::ResourceLease,
    run_dir: &Path,
    batch_id: &str,
    options: &FlowOptions,
    worktree: &Path,
    environment: &ResolvedEnvironment,
    image_path: &Path,
    memory_mb: u32,
    cpus: u16,
    admission: dev_resources::Admission,
    started_at: u64,
    lane_launch: &LaneLaunch<'_>,
    pending: &[PendingLane],
    payload: &mut Option<FlowPayload>,
    preparation: &FlowPreparation,
    cancelled: bool,
    mut message: String,
) -> anyhow::Error {
    let results = pending
        .iter()
        .map(|lane| not_started(lane_launch, lane, &message))
        .collect::<Vec<_>>();
    let payload_cleanup = cleanup_workflow_payload(payload);
    if let Err(error) = &payload_cleanup {
        message = format!("{message}; payload cleanup failed: {error:#}");
    }
    let preparation_artifacts_cleanup = if payload.is_none() {
        cleanup_payload_preparation_artifacts(run_dir)
    } else {
        Ok(())
    };
    if let Err(error) = &preparation_artifacts_cleanup {
        message = format!("{message}; preparation artifact cleanup failed: {error:#}");
    }
    let cleanup_complete = preparation.cleanup_complete()
        && payload_cleanup.is_ok()
        && preparation_artifacts_cleanup.is_ok()
        && payload_cleanup_complete(payload.as_ref());
    let status = flow_status(false, cancelled, cleanup_complete);
    if let Err(error) = write_aggregate_report(
        run_dir,
        batch_id,
        options,
        worktree,
        environment,
        image_path,
        memory_mb,
        cpus,
        admission,
        started_at,
        payload.as_ref(),
        preparation,
        status,
        Some(&message),
        &results,
    ) {
        resource_lease.retain();
        return anyhow!("{message}; failed to persist terminal flow report: {error:#}");
    }
    if cleanup_complete {
        if let Err(error) = resource_lease
            .release()
            .context("failed to release the flow resource lease")
        {
            return anyhow!("{message}; {error:#}");
        }
    } else {
        resource_lease.retain();
    }
    anyhow!(message)
}

fn cleanup_payload_preparation_artifacts(run_dir: &Path) -> Result<()> {
    let payload_dir = run_dir.join("payload");
    let root = payload_dir.join("root");
    if root.exists() {
        qol_dev_env::payload::remove_payload(&root)?;
    }
    match fs::remove_dir_all(&payload_dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to remove payload directory {}",
                payload_dir.display()
            )
        }),
    }
}

fn rollback_unpublished_flow(
    resource_lease: dev_resources::ResourceLease,
    payload: &mut Option<FlowPayload>,
    run_dir: &Path,
    batch_id: &str,
    worktree: &Path,
    error: anyhow::Error,
) -> anyhow::Error {
    let mut failures = vec![format!("{error:#}")];
    let payload_cleanup = cleanup_workflow_payload(payload);
    if let Err(error) = &payload_cleanup {
        failures.push(format!("payload cleanup failed: {error:#}"));
    }
    let run_cleanup = payload_cleanup
        .is_ok()
        .then(|| remove_unpublished_run_dir(run_dir))
        .transpose();
    if let Err(error) = &run_cleanup {
        failures.push(format!("run directory cleanup failed: {error:#}"));
    }
    if payload_cleanup.is_err() || run_cleanup.is_err() {
        let evidence_error = write_unpublished_flow_failure(
            run_dir,
            batch_id,
            payload.as_ref(),
            worktree,
            &failures.join("; "),
        )
        .err();
        if let Some(error) = evidence_error {
            failures.push(format!(
                "failed to persist unresolved cleanup evidence: {error:#}"
            ));
        }
        resource_lease.retain();
        return anyhow!(failures.join("; "));
    }
    if let Err(error) = resource_lease.rollback_unpublished() {
        failures.push(format!("resource reservation rollback failed: {error:#}"));
    }
    anyhow!(failures.join("; "))
}

fn write_unpublished_flow_failure(
    run_dir: &Path,
    batch_id: &str,
    payload: Option<&FlowPayload>,
    worktree: &Path,
    error: &str,
) -> Result<()> {
    let report_path = run_dir.join("report.json");
    let report = json!({
        "name": "qol-flow-setup-failure",
        "kind": "flow",
        "run_id": batch_id,
        "status": "cleanup-incomplete",
        "owner": dev_env::run_owner_in("flow-setup", "released", worktree),
        "payload": payload.map(payload_report),
        "teardown": {
            "status": "incomplete",
            "error": error,
        },
        "artifacts": {
            "run_dir": run_dir,
            "report": report_path,
        },
        "error": error,
        "next": [
            format!("Inspect retained setup artifacts under {}.", run_dir.display()),
            format!("After verifying no sandbox process is live, run qol env doctor --lease-clear {batch_id}."),
        ],
    });
    let content = serde_json::to_vec_pretty(&report).context("failed to serialize JSON")?;
    qol_fs::atomic_write_durable(&report_path, &content)
        .with_context(|| format!("failed to write {}", report_path.display()))
}

fn remove_unpublished_run_dir(run_dir: &Path) -> Result<()> {
    qol_dev_env::remove_unpublished_run_dir(run_dir, "flow")
}

fn combine_setup_errors(error: anyhow::Error, cleanup: Option<anyhow::Error>) -> anyhow::Error {
    match cleanup {
        Some(cleanup) => anyhow!("{error:#}; setup cleanup failed: {cleanup:#}"),
        None => error,
    }
}

fn configured_flow_adapter(
    capabilities: &std::collections::BTreeMap<String, String>,
) -> Result<emu::GuestAdapter> {
    let adapter_id = capabilities
        .get("flow_adapter")
        .context("manual sessions are available, but no automated flow adapter is declared")?;
    let adapter = emu::GuestAdapter::parse(adapter_id).ok_or_else(|| {
        anyhow!("unknown flow adapter `{adapter_id}` declared by the environment manifest")
    })?;
    Ok(adapter)
}

impl LaneSpawner for ProcessLaneSpawner {
    fn spawn(&mut self, launch: &LaneLaunch<'_>, pending: &PendingLane) -> Result<ActiveLane> {
        let log_path = launch.logs_dir.join(format!("{}.log", pending.run_id));
        let stdout = File::create(&log_path)
            .with_context(|| format!("failed to create {}", log_path.display()))?;
        let stderr = stdout
            .try_clone()
            .with_context(|| format!("failed to clone {}", log_path.display()))?;
        let report_path = launch.case_root.join(&pending.run_id).join("report.json");
        let mut command = Command::new(launch.executable);
        command
            .args(&pending.args)
            .current_dir(launch.worktree)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        dev_env::clear_host_session(&mut command);
        let process_tree = crate::process_guardian::own_process_tree()
            .context("failed to create supervisor process-tree ownership")?;
        qol_process::isolate_owned_command(&mut command).with_context(|| {
            format!(
                "failed to isolate flow lane supervisor `{}`",
                pending.run_id
            )
        })?;
        let prepared = process_tree.prepare_command(command).with_context(|| {
            format!(
                "failed to contain flow lane supervisor `{}` before exec",
                pending.run_id
            )
        })?;
        write_lane_owner(launch, pending, "launching", None)?;
        let child = match prepared.spawn() {
            Ok(child) => child,
            Err(error) => {
                if error.cleanup() != qol_process::PreparedSpawnCleanup::RecoveryPending {
                    let _ = write_lane_owner(launch, pending, "spawn-failed", None);
                }
                return Err(anyhow!(error))
                    .with_context(|| format!("failed to start flow lane `{}`", pending.run_id));
            }
        };
        if let Err(journal_error) = write_lane_owner(launch, pending, "spawned", Some(child.id())) {
            let mut supervisor = ChildSupervisor {
                executable: launch.executable.to_path_buf(),
                case_root: launch.case_root.to_path_buf(),
                run_id: pending.run_id.clone(),
                child: Some(child),
                process_tree,
            };
            let shutdown = supervisor.shutdown("flow ownership journal update failed");
            let cleanup_error = shutdown.error.unwrap_or_else(|| {
                if shutdown.cleanup.complete {
                    return "owned process tree was cleaned".to_string();
                }
                "owned process-tree cleanup is incomplete".to_string()
            });
            bail!(
                "failed to update flow lane `{}` ownership: {journal_error:#}; {cleanup_error}",
                pending.run_id
            );
        }
        Ok(ActiveLane {
            run_id: pending.run_id.clone(),
            report_path,
            log_path,
            supervisor: Box::new(ChildSupervisor {
                executable: launch.executable.to_path_buf(),
                case_root: launch.case_root.to_path_buf(),
                run_id: pending.run_id.clone(),
                child: Some(child),
                process_tree,
            }),
        })
    }
}

fn prepare_lane_owners(launch: &LaneLaunch<'_>, pending: &[PendingLane]) -> Result<()> {
    for lane in pending {
        write_lane_owner(launch, lane, "planned", None)?;
    }
    Ok(())
}

fn write_lane_owner(
    launch: &LaneLaunch<'_>,
    pending: &PendingLane,
    phase: &str,
    supervisor_pid: Option<u32>,
) -> Result<PathBuf> {
    let flow_run_dir = launch
        .flow_report_path
        .parent()
        .context("flow report has no run directory")?;
    let owners_dir = flow_run_dir.join(LANE_OWNERS_DIR);
    fs::create_dir_all(&owners_dir)
        .with_context(|| format!("failed to create {}", owners_dir.display()))?;
    let path = owners_dir.join(format!("{}.json", pending.run_id));
    let supervisor_process_identity =
        supervisor_pid.and_then(|pid| qol_process::process_identity(pid).ok());
    let journal = json!({
        "kind": "flow-lane-owner",
        "run_id": pending.run_id,
        "flow_run_id": launch.flow_run_id,
        "flow_report": launch.flow_report_path,
        "owner_pid": launch.owner_pid,
        "owner_process_identity": launch.owner_process_identity.as_deref(),
        "supervisor_pid": supervisor_pid,
        "supervisor_process_identity": supervisor_process_identity,
        "phase": phase,
        "observed_at_unix_ms": qol_dev_env::unix_millis()?,
    });
    atomic_json_durable(&path, &journal)?;
    Ok(path)
}

impl FlowJournal {
    fn mark_cancelling(&self) -> Result<()> {
        let run_dir = self
            .report_path
            .parent()
            .context("flow report has no run directory")?;
        let _lock = lock_flow_run(run_dir)?;
        let content = fs::read_to_string(&self.report_path)
            .with_context(|| format!("failed to read {}", self.report_path.display()))?;
        let mut report: Value = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", self.report_path.display()))?;
        if report.get("kind").and_then(Value::as_str) != Some("flow-fanout") {
            bail!("flow cancellation journal has the wrong report kind");
        }
        if flow_status_is_terminal(
            report
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ) {
            return Ok(());
        }
        report["status"] = json!("cancelling");
        report["owner"]["state"] = json!("cancelling");
        if let Some(fanout) = report
            .get_mut("steps")
            .and_then(Value::as_array_mut)
            .and_then(|steps| {
                steps
                    .iter_mut()
                    .find(|step| step.get("name").and_then(Value::as_str) == Some("fanout"))
            })
        {
            fanout["status"] = json!("cancelling");
        }
        atomic_json(&run_dir.join("steps/lifecycle.json"), &report["steps"])?;
        atomic_json(&self.report_path, &report)
    }
}

fn execute_lanes(
    spawner: &mut impl LaneSpawner,
    launch: &LaneLaunch<'_>,
    pending: &[PendingLane],
    concurrent: usize,
    progress: bool,
    cancellation: &impl CancellationSource,
    journal: Option<&FlowJournal>,
) -> ExecutionOutcome {
    let mut active = Vec::<ActiveLane>::new();
    let mut results = Vec::<LaneResult>::new();
    let mut next_lane = 0;
    while next_lane < pending.len() || !active.is_empty() {
        if cancellation.is_cancelled() {
            return cancel_execution(&mut active, results, launch, pending, next_lane, journal);
        }
        while next_lane < pending.len() && active.len() < concurrent {
            match spawner.spawn(launch, &pending[next_lane]) {
                Ok(lane) => active.push(lane),
                Err(error) => {
                    let message = format!(
                        "failed to start flow lane `{}`: {error:#}",
                        pending[next_lane].run_id
                    );
                    results.push(spawn_failure(launch, &pending[next_lane], &message));
                    abort_lanes(&mut active, &mut results, &message);
                    results.extend(
                        pending[next_lane + 1..]
                            .iter()
                            .map(|lane| not_started(launch, lane, &message)),
                    );
                    return ExecutionOutcome {
                        results,
                        error: Some(message),
                        cancelled: false,
                    };
                }
            }
            show_lane_started(progress, next_lane, &pending[next_lane].run_id);
            next_lane += 1;
            if cancellation.is_cancelled() {
                return cancel_execution(&mut active, results, launch, pending, next_lane, journal);
            }
        }

        let mut index = 0;
        while index < active.len() {
            let run_id = active[index].run_id.clone();
            let completed = match active[index].supervisor.try_wait() {
                Ok(completed) => completed,
                Err(error) => {
                    let message = format!("failed to wait for lane `{run_id}`: {error:#}");
                    let lane = active.swap_remove(index);
                    results.push(abort_lane(lane, &message));
                    abort_lanes(&mut active, &mut results, &message);
                    results.extend(
                        pending[next_lane..]
                            .iter()
                            .map(|lane| not_started(launch, lane, &message)),
                    );
                    return ExecutionOutcome {
                        results,
                        error: Some(message),
                        cancelled: false,
                    };
                }
            };
            let Some(exit) = completed else {
                index += 1;
                continue;
            };
            let lane = active.swap_remove(index);
            let result = finish_lane(lane, exit, launch);
            show_lane_finished(progress, &result);
            results.push(result);
        }
        if !active.is_empty() {
            thread::sleep(SUPERVISOR_WAIT_INTERVAL);
        }
    }
    ExecutionOutcome {
        results,
        error: None,
        cancelled: false,
    }
}

fn cancel_execution(
    active: &mut Vec<ActiveLane>,
    mut results: Vec<LaneResult>,
    launch: &LaneLaunch<'_>,
    pending: &[PendingLane],
    next_lane: usize,
    journal: Option<&FlowJournal>,
) -> ExecutionOutcome {
    let reason = "flow execution cancelled";
    let journal_error = journal
        .and_then(|journal| journal.mark_cancelling().err())
        .map(|error| format!("failed to persist cancellation state: {error:#}"));
    abort_lanes(active, &mut results, reason);
    results.extend(
        pending[next_lane..]
            .iter()
            .map(|lane| not_started(launch, lane, reason)),
    );
    ExecutionOutcome {
        results,
        error: combine_errors(Some(reason.to_string()), journal_error),
        cancelled: true,
    }
}

fn show_lane_started(progress: bool, index: usize, run_id: &str) {
    if !progress {
        return;
    }
    step_label(
        "start",
        StepKind::Pending,
        &format!("lane {} · {run_id}", index + 1),
    );
}

fn show_lane_finished(progress: bool, result: &LaneResult) {
    if !progress {
        return;
    }
    step_label(
        "lane",
        if result.passed {
            StepKind::Success
        } else {
            StepKind::Info
        },
        &format!("{} · {}", result.run_id, result.process_status),
    );
}

fn abort_lanes(active: &mut Vec<ActiveLane>, results: &mut Vec<LaneResult>, reason: &str) {
    let lanes = std::mem::take(active);
    let mut aborted = thread::scope(|scope| {
        lanes
            .into_iter()
            .map(|lane| scope.spawn(move || abort_lane(lane, reason)))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|worker| worker.join().expect("flow lane shutdown worker panicked"))
            .collect::<Vec<_>>()
    });
    results.append(&mut aborted);
}

fn abort_lane(mut lane: ActiveLane, reason: &str) -> LaneResult {
    let shutdown = lane.supervisor.shutdown(reason);
    let (report_status, verdict) = report_outcome(&lane.report_path, &lane.run_id);
    LaneResult {
        run_id: lane.run_id,
        report_path: lane.report_path,
        log_path: lane.log_path,
        phase: "aborted".to_string(),
        process_status: shutdown.process_status,
        report_status,
        verdict,
        passed: false,
        completed: false,
        cleanup: shutdown.cleanup,
        error: combine_errors(Some(reason.to_string()), shutdown.error),
    }
}

fn spawn_failure(launch: &LaneLaunch<'_>, pending: &PendingLane, error: &str) -> LaneResult {
    let report_path = launch.case_root.join(&pending.run_id).join("report.json");
    LaneResult {
        run_id: pending.run_id.clone(),
        report_path: report_path.clone(),
        log_path: launch.logs_dir.join(format!("{}.log", pending.run_id)),
        phase: "spawn-failed".to_string(),
        process_status: "spawn failed".to_string(),
        report_status: None,
        verdict: None,
        passed: false,
        completed: false,
        cleanup: spawn_failure_cleanup(launch, pending, &report_path),
        error: Some(error.to_string()),
    }
}

fn spawn_failure_cleanup(
    launch: &LaneLaunch<'_>,
    pending: &PendingLane,
    report_path: &Path,
) -> LaneCleanup {
    match read_optional_flow_report(report_path, &pending.run_id) {
        Ok(Some(report)) => {
            let status = report
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if child_cleanup_complete(&report, status).is_ok()
                && matches!(
                    child_process_state(&report),
                    RecordedProcessState::VerifiedDead
                )
            {
                let removed = report
                    .get("teardown")
                    .and_then(|teardown| teardown.get("removed"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(PathBuf::from)
                    .collect();
                return LaneCleanup {
                    status: "complete".to_string(),
                    complete: true,
                    evidence_path: Some(report_path.to_path_buf()),
                    removed,
                    error: None,
                };
            }
            LaneCleanup::incomplete("spawned lane lacks verified cleanup")
        }
        Err(error) => LaneCleanup::incomplete(error),
        Ok(None) => {
            let owner_path = launch
                .flow_report_path
                .parent()
                .map(|run_dir| lane_owner_path(run_dir, &pending.run_id));
            let phase = owner_path
                .as_deref()
                .and_then(|path| read_optional_json(path).ok().flatten())
                .and_then(|owner| {
                    owner
                        .get("phase")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
            if phase
                .as_deref()
                .is_none_or(|phase| matches!(phase, "planned" | "spawn-failed"))
            {
                return LaneCleanup::not_required();
            }
            LaneCleanup::incomplete("lane may have spawned without a child report")
        }
    }
}

fn not_started(launch: &LaneLaunch<'_>, pending: &PendingLane, error: &str) -> LaneResult {
    LaneResult {
        run_id: pending.run_id.clone(),
        report_path: launch.case_root.join(&pending.run_id).join("report.json"),
        log_path: launch.logs_dir.join(format!("{}.log", pending.run_id)),
        phase: "not-started".to_string(),
        process_status: "not started".to_string(),
        report_status: None,
        verdict: None,
        passed: false,
        completed: false,
        cleanup: LaneCleanup::not_required(),
        error: Some(error.to_string()),
    }
}

fn planned_lane(launch: &LaneLaunch<'_>, pending: &PendingLane) -> LaneResult {
    LaneResult {
        run_id: pending.run_id.clone(),
        report_path: launch.case_root.join(&pending.run_id).join("report.json"),
        log_path: launch.logs_dir.join(format!("{}.log", pending.run_id)),
        phase: "planned".to_string(),
        process_status: "planned".to_string(),
        report_status: None,
        verdict: None,
        passed: false,
        completed: false,
        cleanup: LaneCleanup::pending(),
        error: None,
    }
}

fn finish_lane(lane: ActiveLane, exit: SupervisorExit, launch: &LaneLaunch<'_>) -> LaneResult {
    let report = read_optional_flow_report(&lane.report_path, &lane.run_id);
    let report_error = match &report {
        Ok(Some(_)) => None,
        Ok(None) => Some(format!(
            "child report is missing: {}",
            lane.report_path.display()
        )),
        Err(error) => Some(format!("child report is invalid: {error}")),
    };
    let report = report.ok().flatten();
    let report_status = report
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let verdict = report
        .as_ref()
        .and_then(|value| value.get("workflow"))
        .and_then(|value| value.get("verdict"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let passed = exit.cleanup.complete
        && lane_passed(exit.success, report_status.as_deref(), verdict.as_deref());
    let artifacts_error =
        copy_lane_artifacts(report.as_ref(), &lane.run_id, launch.flow_report_path);
    let error = combine_errors(report_error, exit.cleanup.error.clone());
    let error = combine_errors(error, artifacts_error);
    LaneResult {
        run_id: lane.run_id.clone(),
        report_path: lane.report_path.clone(),
        log_path: lane.log_path.clone(),
        phase: "completed".to_string(),
        process_status: exit.description,
        report_status,
        verdict,
        passed,
        completed: true,
        cleanup: exit.cleanup,
        error,
    }
}

fn lane_passed(process_success: bool, report_status: Option<&str>, verdict: Option<&str>) -> bool {
    process_success && report_status == Some("pass") && verdict == Some("pass")
}

fn copy_lane_artifacts(
    report: Option<&Value>,
    run_id: &str,
    flow_report_path: &Path,
) -> Option<String> {
    let paths = report
        .and_then(|value| value.get("workflow"))
        .and_then(|value| value.get("artifacts"))
        .and_then(Value::as_array)?;
    let destination = flow_report_path.parent()?.join("artifacts").join(run_id);
    let mut failures = Vec::new();
    for path in paths.iter().filter_map(Value::as_str) {
        let source = Path::new(path);
        let Some(name) = source.file_name() else {
            failures.push(format!("{path} has no file name"));
            continue;
        };
        let copied = fs::create_dir_all(&destination)
            .and_then(|()| fs::copy(source, destination.join(name)).map(|_| ()));
        if let Err(error) = copied {
            failures.push(format!("{path}: {error}"));
        }
    }
    (!failures.is_empty())
        .then(|| format!("failed to copy lane artifacts: {}", failures.join(", ")))
}

fn terminal_error(
    execution_error: Option<&str>,
    results: &[LaneResult],
    requested: u32,
) -> Option<String> {
    if let Some(error) = execution_error {
        return Some(error.to_string());
    }
    let failed = results
        .iter()
        .filter(|result| !result.passed)
        .map(|result| result.run_id.as_str())
        .collect::<Vec<_>>();
    if !failed.is_empty() {
        return Some(format!("failed lanes: {}", failed.join(", ")));
    }
    if results.len() != requested as usize {
        return Some(format!(
            "reported {} of {requested} requested lanes",
            results.len()
        ));
    }
    None
}

fn report_outcome(path: &Path, run_id: &str) -> (Option<String>, Option<String>) {
    let report = read_optional_flow_report(path, run_id).ok().flatten();
    let status = report
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let verdict = report
        .as_ref()
        .and_then(|value| value.get("workflow"))
        .and_then(|value| value.get("verdict"))
        .and_then(Value::as_str)
        .map(str::to_string);
    (status, verdict)
}

fn request_verified_shutdown(executable: &Path, case_root: &Path, run_id: &str) -> Result<()> {
    let status = Command::new(executable)
        .arg("emu")
        .arg("down")
        .arg("--run-root")
        .arg(case_root)
        .arg(run_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to request verified shutdown for `{run_id}`"))?;
    if status.success() {
        return Ok(());
    }
    bail!("verified shutdown for `{run_id}` exited with {status}")
}

fn request_shutdown_until_exit(
    child: &mut Child,
    executable: &Path,
    case_root: &Path,
    run_id: &str,
    timeout: Duration,
) -> GracefulShutdown {
    let deadline = Instant::now() + timeout;
    let mut control_error = None;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return GracefulShutdown {
                    status: Some(status),
                    error: None,
                }
            }
            Ok(None) => {}
            Err(error) => {
                return GracefulShutdown {
                    status: None,
                    error: combine_errors(
                        control_error,
                        format!("failed to wait for supervisor: {error}"),
                    ),
                }
            }
        }
        match request_verified_shutdown(executable, case_root, run_id) {
            Ok(()) => return wait_for_exit_until(child, deadline),
            Err(error) => control_error = Some(error.to_string()),
        }
        if Instant::now() >= deadline {
            return GracefulShutdown {
                status: None,
                error: combine_errors(
                    control_error,
                    "verified shutdown was unavailable before the deadline".to_string(),
                ),
            };
        }
        thread::sleep(SUPERVISOR_WAIT_INTERVAL);
    }
}

fn wait_for_exit_until(child: &mut Child, deadline: Instant) -> GracefulShutdown {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return GracefulShutdown {
                    status: Some(status),
                    error: None,
                }
            }
            Ok(None) => {}
            Err(error) => {
                return GracefulShutdown {
                    status: None,
                    error: Some(format!("failed to wait for supervisor: {error}")),
                }
            }
        }
        if Instant::now() >= deadline {
            return GracefulShutdown {
                status: None,
                error: Some("supervisor did not exit after verified shutdown".to_string()),
            };
        }
        thread::sleep(SUPERVISOR_WAIT_INTERVAL);
    }
}

fn combine_errors(first: Option<String>, second: impl Into<Option<String>>) -> Option<String> {
    let second = second.into();
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}; {second}")),
        (Some(first), None) => Some(first),
        (None, Some(second)) => Some(second),
        (None, None) => None,
    }
}

fn write_preflight(
    run_dir: &Path,
    options: &FlowOptions,
    memory_mb: u32,
    cpus: u16,
    concurrent: u32,
    admission: dev_resources::Admission,
) -> Result<()> {
    let content = format!(
        "workflow={}\nenvironment={}\nrepeat={}\njobs={}\nconcurrent={}\nmemory_mb={}\ncpus={}\navailable_memory_mb={}\nmemory_budget_mb={}\nrequested_memory_mb={}\nreserved_lanes={}\nreserved_memory_mb={}\navailable_cpus={}\ncpu_budget={}\nrequested_cpus={}\nreserved_cpus={}\navailable_disk_bytes={}\ndisk_budget_bytes={}\nrequested_disk_bytes={}\nreserved_disk_bytes={}\nforced={}\n",
        options.workflow,
        options.environment_id,
        options.repeat,
        options.jobs,
        concurrent,
        memory_mb,
        cpus,
        optional_number(admission.available_memory_mb),
        optional_number(admission.budget_memory_mb),
        admission.requested_memory_mb,
        admission.reserved_lanes,
        admission.reserved_memory_mb,
        optional_number(admission.available_cpus),
        optional_number(admission.budget_cpus),
        admission.requested_cpus,
        admission.reserved_cpus,
        optional_number(admission.available_disk_bytes),
        optional_number(admission.budget_disk_bytes),
        admission.requested_disk_bytes,
        admission.reserved_disk_bytes,
        admission.forced,
    );
    atomic_write(&run_dir.join("host-preflight.txt"), content.as_bytes())
}

fn write_effective_environment(
    run_dir: &Path,
    environment: &ResolvedEnvironment,
    image_path: &Path,
    memory_mb: u32,
    cpus: u16,
) -> Result<()> {
    let content = json!({
        "id": environment.definition.id,
        "name": environment.definition.name,
        "family": environment.definition.family,
        "backend": environment.definition.backend,
        "source": environment.definition.source,
        "image": {
            "path": image_path,
            "kind": environment.definition.image.kind,
            "arch": environment.definition.image.arch,
            "firmware": environment.definition.image.firmware,
        },
        "boot": {
            "display": "headless",
            "memory_mb": memory_mb,
            "cpus": cpus,
        },
        "mounts": {
            "workspace": environment.definition.mounts.workspace,
        },
        "capabilities": environment.definition.capabilities,
    });
    atomic_json(&run_dir.join("effective-env.json"), &content)
}

fn payload_report(payload: &FlowPayload) -> Value {
    json!({
        "manifest": payload.manifest_path,
        "image": payload.image_path,
        "manifest_sha256": payload.manifest_sha256,
        "transport": PAYLOAD_TRANSPORT,
        "cleanup": {
            "status": payload.cleanup.status,
            "complete": payload.cleanup.complete,
            "removed": payload.cleanup.removed,
            "error": payload.cleanup.error,
        },
    })
}

fn preparation_report(preparation: &FlowPreparation) -> Value {
    json!({
        "status": preparation.status,
        "build": {
            "status": preparation.build_status,
            "process_status": preparation.process_status,
            "cleanup": {
                "status": preparation.cleanup.status,
                "complete": preparation.cleanup.complete,
                "verification": preparation.cleanup.verification,
                "error": preparation.cleanup.error,
            },
        },
        "iso": {
            "status": preparation.iso_status,
            "process_status": preparation.iso_process_status,
            "cleanup": {
                "status": preparation.iso_cleanup.status,
                "complete": preparation.iso_cleanup.complete,
                "verification": preparation.iso_cleanup.verification,
                "error": preparation.iso_cleanup.error,
            },
        },
    })
}

fn add_payload_runtime_boundary_evidence(report: &mut Value, payload: Option<&FlowPayload>) {
    let Some(payload) = payload else {
        return;
    };
    report["desktop_runtime_boundary"] = json!({
        "scope": "guest-runtime-only",
        "host_worker": {
            "session_environment_cleared": true,
            "os_security_boundary": false,
        },
        "guest": {
            "headless": true,
            "offline": true,
            "workspace_mounted": false,
        },
        "guest_payload": {
            "identity": {
                "kind": "manifest-sha256",
                "value": payload.manifest_sha256,
                "immutable": true,
            },
            "transport": PAYLOAD_TRANSPORT,
            "read_only": true,
        },
    });
}

#[allow(clippy::too_many_arguments)]
fn write_aggregate_report(
    run_dir: &Path,
    batch_id: &str,
    options: &FlowOptions,
    worktree: &Path,
    environment: &ResolvedEnvironment,
    image_path: &Path,
    memory_mb: u32,
    cpus: u16,
    admission: dev_resources::Admission,
    started_at: u64,
    payload: Option<&FlowPayload>,
    preparation: &FlowPreparation,
    status: &str,
    error: Option<&str>,
    results: &[LaneResult],
) -> Result<()> {
    let lanes = lane_reports(results);
    let active = qol_dev_env::ReportStatus::parse(status).is_active();
    let preflight_status = if run_dir.join("host-preflight.txt").is_file() {
        "pass"
    } else if active {
        "pending"
    } else {
        "failed"
    };
    let mut report = json!({
        "name": "qol-flow-run",
        "kind": "flow-fanout",
        "run_id": batch_id,
        "started_at_unix_ms": started_at,
        "status": status,
        "owner": dev_env::run_owner_in(
            &options.workflow,
            if active { "running" } else { "released" },
            worktree,
        ),
        "workflow": {
            "id": options.workflow,
            "repeat": options.repeat,
            "jobs": options.jobs.min(options.repeat),
        },
        "environment": {
            "id": environment.definition.id,
            "source": environment.definition.source,
            "image_path": image_path,
        },
        "preparation": preparation_report(preparation),
        "payload": payload.map(payload_report),
        "resources": {
            "memory_mb_each": memory_mb,
            "cpus_each": cpus,
            "available_memory_mb": admission.available_memory_mb,
            "memory_budget_mb": admission.budget_memory_mb,
            "requested_memory_mb": admission.requested_memory_mb,
            "reserved_lanes": admission.reserved_lanes,
            "reserved_memory_mb": admission.reserved_memory_mb,
            "memory_budget_percent": dev_resources::MEMORY_BUDGET_PERCENT,
            "available_cpus": admission.available_cpus,
            "cpu_budget": admission.budget_cpus,
            "requested_cpus": admission.requested_cpus,
            "reserved_cpus": admission.reserved_cpus,
            "cpu_budget_percent": dev_resources::CPU_BUDGET_PERCENT,
            "available_disk_bytes": admission.available_disk_bytes,
            "disk_budget_bytes": admission.budget_disk_bytes,
            "requested_disk_bytes": admission.requested_disk_bytes,
            "reserved_disk_bytes": admission.reserved_disk_bytes,
            "disk_budget_percent": dev_resources::DISK_BUDGET_PERCENT,
            "forced": admission.forced,
        },
        "artifacts": {
            "run_dir": run_dir,
            "report": run_dir.join("report.json"),
            "effective_environment": run_dir.join("effective-env.json"),
            "host_preflight": run_dir.join("host-preflight.txt"),
            "logs": run_dir.join("logs"),
            "artifacts": run_dir.join("artifacts"),
            "steps": run_dir.join("steps"),
        },
        "steps": [
            {
                "name": "preparation",
                "status": preparation.status,
            },
            {
                "name": "preflight",
                "status": preflight_status,
                "artifact": run_dir.join("host-preflight.txt"),
            },
            {
                "name": "fanout",
                "status": status,
                "completed_lanes": results.iter().filter(|result| result.completed).count(),
                "reported_lanes": results.len(),
                "requested_lanes": options.repeat,
            },
        ],
        "lanes": lanes,
        "next": [
            format!("Inspect each child report and log under `{}`.", run_dir.display()),
            format!("Rerun with `qol flow run {} --env {} --repeat {} --jobs {} --worktree {}`.", options.workflow, options.environment_id, options.repeat, options.jobs, worktree.display()),
        ],
    });
    add_payload_runtime_boundary_evidence(&mut report, payload);
    let finished_at = flow_status_is_terminal(status)
        .then(qol_dev_env::unix_millis)
        .transpose()?;
    apply_report_lifecycle(&mut report, error, finished_at);
    let _lock = lock_flow_run(run_dir)?;
    atomic_json(&run_dir.join("steps/lifecycle.json"), &report["steps"])?;
    atomic_json_durable(&run_dir.join("report.json"), &report)
}

fn lane_reports(results: &[LaneResult]) -> Vec<Value> {
    results
        .iter()
        .map(|result| {
            json!({
                "run_id": result.run_id,
                "phase": result.phase,
                "passed": result.passed,
                "completed": result.completed,
                "process_status": result.process_status,
                "report_status": result.report_status,
                "verdict": result.verdict,
                "cleanup": {
                    "status": result.cleanup.status,
                    "complete": result.cleanup.complete,
                    "evidence": result.cleanup.evidence_path,
                    "removed": result.cleanup.removed,
                    "error": result.cleanup.error,
                },
                "error": result.error,
                "report": result.report_path,
                "log": result.log_path,
            })
        })
        .collect()
}

fn flow_status_is_terminal(status: &str) -> bool {
    matches!(status, "pass" | "failed" | "cancelled" | "abandoned")
}

fn apply_report_lifecycle(report: &mut Value, error: Option<&str>, finished_at: Option<u64>) {
    if let Some(error) = error {
        report["error"] = json!(error);
    }
    if let Some(finished_at) = finished_at {
        report["finished_at_unix_ms"] = json!(finished_at);
    }
}

fn optional_number(value: Option<u64>) -> String {
    value
        .map(|number| number.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    qol_dev_env::write_json_report(path, value)
}

fn atomic_json_durable(path: &Path, value: &Value) -> Result<()> {
    let content = serde_json::to_vec_pretty(value).context("failed to serialize JSON")?;
    qol_fs::atomic_write_durable(path, &content)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    qol_fs::atomic_write(path, content)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn parse_options(args: &[OsString]) -> Result<FlowOptions> {
    let mut workflow = None;
    let mut environment_id = None;
    let mut run_id = None;
    let mut worktree = None;
    let mut repeat = None;
    let mut jobs = None;
    let mut memory_mb = None;
    let mut cpus = None;
    let mut force = false;
    let mut index = 0;
    while index < args.len() {
        let argument = utf8(&args[index])?;
        match argument {
            "--env" => {
                reject_duplicate(environment_id.is_some(), "--env")?;
                environment_id = Some(option_value(args, index, "--env")?.to_string());
                index += 2;
            }
            "--repeat" => {
                reject_duplicate(repeat.is_some(), "--repeat")?;
                repeat = Some(parse_bounded(
                    option_value(args, index, "--repeat")?,
                    "--repeat",
                    1,
                    u64::from(MAX_REPEAT),
                )? as u32);
                index += 2;
            }
            "--run-id" => {
                reject_duplicate(run_id.is_some(), "--run-id")?;
                let value = option_value(args, index, "--run-id")?;
                qol_dev_env::validate_run_id(value)?;
                run_id = Some(value.to_string());
                index += 2;
            }
            "--worktree" => {
                reject_duplicate(worktree.is_some(), "--worktree")?;
                let path = PathBuf::from(option_os_value(args, index, "--worktree")?);
                if !path.is_absolute() {
                    bail!("--worktree requires an absolute path");
                }
                worktree = Some(path);
                index += 2;
            }
            "--jobs" => {
                reject_duplicate(jobs.is_some(), "--jobs")?;
                jobs = Some(parse_bounded(
                    option_value(args, index, "--jobs")?,
                    "--jobs",
                    1,
                    u64::from(dev_resources::MAX_CONCURRENT_LANES),
                )? as u32);
                index += 2;
            }
            "--memory-mb" => {
                reject_duplicate(memory_mb.is_some(), "--memory-mb")?;
                memory_mb = Some(parse_bounded(
                    option_value(args, index, "--memory-mb")?,
                    "--memory-mb",
                    dev_resources::MIN_MEMORY_MB,
                    dev_resources::MAX_MEMORY_MB,
                )? as u32);
                index += 2;
            }
            "--cpus" => {
                reject_duplicate(cpus.is_some(), "--cpus")?;
                cpus = Some(parse_bounded(
                    option_value(args, index, "--cpus")?,
                    "--cpus",
                    dev_resources::MIN_CPUS,
                    dev_resources::MAX_CPUS,
                )? as u16);
                index += 2;
            }
            "--force" => {
                if force {
                    bail!("duplicate flow option `--force`");
                }
                force = true;
                index += 1;
            }
            option if option.starts_with('-') => bail!("unknown flow option `{option}`"),
            value => {
                if workflow.is_some() || value.is_empty() {
                    bail!("usage: qol flow run <workflow> --env <environment> [options]");
                }
                workflow = Some(value.to_string());
                index += 1;
            }
        }
    }
    Ok(FlowOptions {
        workflow: workflow
            .ok_or_else(|| anyhow!("usage: qol flow run <workflow> --env <environment>"))?,
        environment_id: environment_id.ok_or_else(|| anyhow!("--env is required"))?,
        run_id,
        worktree,
        repeat: repeat.unwrap_or(1),
        jobs: jobs.unwrap_or(1),
        memory_mb,
        cpus,
        force,
    })
}

fn option_os_value<'a>(args: &'a [OsString], index: usize, option: &str) -> Result<&'a OsString> {
    let value = args
        .get(index + 1)
        .ok_or_else(|| anyhow!("{option} requires a value"))?;
    if value.to_str().is_some_and(|value| value.starts_with('-')) {
        bail!("{option} requires a value");
    }
    Ok(value)
}

fn option_value<'a>(args: &'a [OsString], index: usize, option: &str) -> Result<&'a str> {
    utf8(option_os_value(args, index, option)?)
}

fn utf8(value: &OsString) -> Result<&str> {
    value
        .to_str()
        .ok_or_else(|| anyhow!("flow argument is not valid UTF-8"))
}

fn reject_duplicate(duplicate: bool, option: &str) -> Result<()> {
    if duplicate {
        bail!("duplicate flow option `{option}`");
    }
    Ok(())
}

fn parse_bounded(value: &str, option: &str, minimum: u64, maximum: u64) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("{option} must be an integer from {minimum} to {maximum}");
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| anyhow!("{option} must be an integer from {minimum} to {maximum}"))?;
    if !(minimum..=maximum).contains(&parsed) {
        bail!("{option} must be from {minimum} to {maximum}");
    }
    Ok(parsed)
}

fn print_help() {
    print!("{}", help_text());
}

fn help_text() -> &'static str {
    "qol flow commands:\n  qol flow run <workflow> --env <environment> [--repeat N] [--jobs N]\n               [--memory-mb N] [--cpus N] [--worktree PATH] [--force]\n  qol flow runs\n\nFlows run headlessly in disposable environment lanes. --jobs bounds concurrent\nVMs; --repeat controls the total number of independent runs. `qol flow runs`\nreconciles interrupted fan-outs and lists active or incomplete flow reports.\nRun placement and acceleration come from the selected environment definition.\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Condvar, Mutex};

    fn argv(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn desktop_payload_recipes_cover_registered_payload_workflows() {
        let expected = [
            (
                "alt-tab-performance",
                DesktopPayloadRecipe {
                    companion: Some(DesktopCompanionRecipe {
                        package: "alt-tab",
                        binary: "alt-tab",
                        plugin_dir: "alt-tab",
                        plugin_id: "plugin-alt-tab",
                    }),
                    tray_features: None,
                },
            ),
            (
                "alt-tab-storm",
                DesktopPayloadRecipe {
                    companion: Some(DesktopCompanionRecipe {
                        package: "alt-tab",
                        binary: "alt-tab",
                        plugin_dir: "alt-tab",
                        plugin_id: "plugin-alt-tab",
                    }),
                    tray_features: None,
                },
            ),
            (
                "bluetooth-storm",
                DesktopPayloadRecipe {
                    companion: Some(DesktopCompanionRecipe {
                        package: "plugin-bluetooth",
                        binary: "plugin-bluetooth",
                        plugin_dir: "bluetooth",
                        plugin_id: "plugin-bluetooth",
                    }),
                    tray_features: None,
                },
            ),
            (
                "hotkey-shadow-boot",
                DesktopPayloadRecipe {
                    companion: Some(DesktopCompanionRecipe {
                        package: "launcher",
                        binary: "launcher",
                        plugin_dir: "launcher",
                        plugin_id: "plugin-launcher",
                    }),
                    tray_features: Some("linux_evdev"),
                },
            ),
            (
                "hotkey-shadow",
                DesktopPayloadRecipe {
                    companion: Some(DesktopCompanionRecipe {
                        package: "launcher",
                        binary: "launcher",
                        plugin_dir: "launcher",
                        plugin_id: "plugin-launcher",
                    }),
                    tray_features: Some("linux_evdev"),
                },
            ),
            (
                "hotkey-storm",
                DesktopPayloadRecipe {
                    companion: Some(DesktopCompanionRecipe {
                        package: "launcher",
                        binary: "launcher",
                        plugin_dir: "launcher",
                        plugin_id: "plugin-launcher",
                    }),
                    tray_features: Some("linux_evdev"),
                },
            ),
            (
                "launcher-storm",
                DesktopPayloadRecipe {
                    companion: Some(DesktopCompanionRecipe {
                        package: "launcher",
                        binary: "launcher",
                        plugin_dir: "launcher",
                        plugin_id: "plugin-launcher",
                    }),
                    tray_features: None,
                },
            ),
            (
                "portable-session",
                DesktopPayloadRecipe {
                    companion: Some(DesktopCompanionRecipe {
                        package: "launcher",
                        binary: "launcher",
                        plugin_dir: "launcher",
                        plugin_id: "plugin-launcher",
                    }),
                    tray_features: None,
                },
            ),
            (
                "qol-shot-capture",
                DesktopPayloadRecipe {
                    companion: Some(DesktopCompanionRecipe {
                        package: "qol-shot",
                        binary: "qol-shot",
                        plugin_dir: "qol-shot",
                        plugin_id: "qol-shot",
                    }),
                    tray_features: None,
                },
            ),
            (
                "qol-shot-storm",
                DesktopPayloadRecipe {
                    companion: Some(DesktopCompanionRecipe {
                        package: "qol-shot",
                        binary: "qol-shot",
                        plugin_dir: "qol-shot",
                        plugin_id: "qol-shot",
                    }),
                    tray_features: None,
                },
            ),
            (
                "shortcut-storm",
                DesktopPayloadRecipe {
                    companion: None,
                    tray_features: None,
                },
            ),
            (
                "window-actions-storm",
                DesktopPayloadRecipe {
                    companion: Some(DesktopCompanionRecipe {
                        package: "window-actions",
                        binary: "window-actions",
                        plugin_dir: "window-actions",
                        plugin_id: "plugin-window-actions",
                    }),
                    tray_features: None,
                },
            ),
        ];
        for (workflow, recipe) in expected {
            assert_eq!(desktop_payload_recipe(workflow), Some(recipe));
        }
        assert_eq!(desktop_payload_recipe("leaves-no-trace"), None);

        let uncovered: Vec<&str> = emu::workflow_ids()
            .into_iter()
            .filter(|id| {
                emu::workflow_definition(id).is_ok_and(|definition| {
                    definition.payload_recipe() == Some(emu::PayloadRecipe::Desktop)
                })
            })
            .filter(|id| desktop_payload_recipe(id).is_none())
            .collect();
        assert!(
            uncovered.is_empty(),
            "every desktop payload workflow needs a desktop recipe: {uncovered:?}"
        );
        let resident: Vec<&str> = emu::workflow_ids()
            .into_iter()
            .filter(|id| {
                emu::workflow_definition(id).is_ok_and(|definition| {
                    definition.payload_recipe() == Some(emu::PayloadRecipe::ResidentWave2)
                })
            })
            .collect();
        assert_eq!(
            resident,
            vec![
                "resident-wave2-apt-preferences",
                "resident-wave2-package-contract"
            ]
        );
    }

    #[test]
    fn payload_free_workflows_skip_preparation_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let workflow = emu::workflow_definition("leaves-no-trace").unwrap();
        assert!(!workflow.requires_payload());
        assert_eq!(workflow.payload_recipe(), Some(emu::PayloadRecipe::None));
        let (payload, preparation) = prepare_workflow_payload(
            workflow,
            dir.path(),
            &dir.path().join("run"),
            false,
            &FixedCancellation(false),
        )
        .unwrap();
        assert!(payload.is_none());
        assert_eq!(preparation.status, "complete");
        assert_eq!(preparation.build_status, "skipped");
        assert_eq!(preparation.iso_status, "skipped");
        assert!(preparation.process_status.is_none());
        assert_eq!(preparation.cleanup.status, "not-required");
        assert_eq!(preparation.iso_cleanup.status, "not-required");
    }

    #[test]
    fn desktop_plugin_payload_includes_runtime_contract_and_excludes_sources() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("plugins/qol-shot");
        fs::create_dir_all(plugin.join("src")).unwrap();
        for (path, content) in [
            ("plugin.toml", "manifest"),
            ("qol-config.toml", "config"),
            ("qol-runtime.toml", "runtime"),
            ("qol-shot", "stale binary"),
            ("src/main.rs", "source"),
        ] {
            fs::write(plugin.join(path), content).unwrap();
        }
        let recipe = desktop_payload_recipe("qol-shot-storm")
            .unwrap()
            .companion
            .unwrap();

        let files = desktop_plugin_payload_files(root.path(), recipe).unwrap();
        let relative = files
            .into_iter()
            .map(|file| file.relative_path)
            .collect::<Vec<_>>();

        assert_eq!(
            relative,
            [
                PathBuf::from("plugins/qol-shot/plugin.toml"),
                PathBuf::from("plugins/qol-shot/qol-config.toml"),
                PathBuf::from("plugins/qol-shot/qol-runtime.toml"),
            ]
        );
    }

    fn immutable_flow_plan(root: &Path) -> FlowPlan {
        let worktree = root.join("worktree");
        let run_root = root.join("runs");
        let image_path = root.join("images/mint.qcow2");
        let definition = qol_dev_env::registry::parse_definition(
            include_str!("../../../../flows/envs/linux-mint-cinnamon.toml"),
            Path::new("flows/envs/linux-mint-cinnamon.toml"),
        )
        .unwrap();
        let start = FlowStart {
            workflow: "qol-shot-capture".to_string(),
            environment_id: definition.id.clone(),
            worktree,
            run_id: "flow-plan-test".to_string(),
            repeat: 3,
            jobs: 2,
            memory_mb: Some(4096),
            cpus: Some(4),
            force: false,
        };
        let ticket = start.ticket(&run_root).unwrap();
        FlowPlan {
            start,
            environment: ResolvedEnvironment {
                definition,
                state: ResolutionState::Ready,
                image_path: Some(image_path.clone()),
                verified_image: None,
                run_root: Some(run_root.clone()),
                messages: Vec::new(),
            },
            workflow: emu::workflow_definition("qol-shot-capture").unwrap(),
            guest_adapter: emu::GuestAdapter::MintCinnamon,
            image_path,
            resources: dev_resources::profile(4096, 4).unwrap(),
            concurrent: 2,
            run_root,
            ticket,
        }
    }

    fn registered_serial_definition(toml: &str) -> EnvironmentDefinition {
        qol_dev_env::registry::parse_definition(toml, Path::new("flows/envs/registered.toml"))
            .unwrap()
    }

    #[test]
    fn resident_payload_workflows_admit_registered_debian_and_ubuntu_environments() {
        for (toml, id, adapter) in [
            (
                include_str!("../../../../flows/envs/linux-debian-nocloud.toml"),
                "linux/debian-nocloud",
                emu::GuestAdapter::DebianNocloud,
            ),
            (
                include_str!("../../../../flows/envs/linux-ubuntu-nocloud.toml"),
                "linux/ubuntu-nocloud",
                emu::GuestAdapter::UbuntuNocloud,
            ),
        ] {
            let definition = registered_serial_definition(toml);
            assert_eq!(definition.id, id);
            assert!(!definition.capabilities.contains_key("image_revision"));
            assert_eq!(
                configured_flow_adapter(&definition.capabilities).unwrap(),
                adapter
            );
            for workflow_id in [
                "resident-wave2-apt-preferences",
                "resident-wave2-package-contract",
            ] {
                let workflow = emu::workflow_definition(workflow_id).unwrap();
                assert!(workflow.requires_payload(), "{workflow_id}");
                assert!(!workflow.requires_guest_revision(), "{workflow_id}");
                emu::validate_workflow_adapter(workflow, adapter).unwrap();
                validate_payload_admission(&definition, workflow).unwrap();
            }
        }
    }

    fn serial_child_launch<'a>(
        definition: &'a EnvironmentDefinition,
        parent_lease: &'a qol_dev_env::resources::ParentLeaseClaim,
    ) -> emu::ChildLaunch<'a> {
        emu::ChildLaunch {
            operation: emu::ChildOperation::Run("resident-wave2-apt-preferences"),
            target: Path::new("/images/debian.qcow2"),
            environment_id: &definition.id,
            run_id: "debian-lane-1",
            parent_lease,
            guest_adapter: Some(configured_flow_adapter(&definition.capabilities).unwrap()),
            guest_image_revision: definition
                .capabilities
                .get("image_revision")
                .map(String::as_str),
            payload_manifest: None,
            payload_image: None,
            run_root: Some(Path::new("/runs/cases")),
            image_kind: Some("qcow2"),
            display: emu::DisplayMode::None,
            offline: true,
            resources: dev_resources::profile(1024, 1).unwrap(),
            acceleration: definition
                .capabilities
                .get("acceleration")
                .map(String::as_str),
            arch: definition.image.arch.as_deref(),
            firmware: definition.image.firmware.as_deref(),
            usb_host: None,
        }
    }

    #[test]
    fn serial_workflow_forwarding_preserves_a_declared_guest_revision() {
        let definition = registered_serial_definition(include_str!(
            "../../../../flows/envs/linux-debian-nocloud.toml"
        ));
        let mut revisioned = definition.clone();
        revisioned
            .capabilities
            .insert("image_revision".to_string(), "debian-13-qol-1".to_string());
        let workflow = emu::workflow_definition("resident-wave2-apt-preferences").unwrap();
        assert!(!workflow.requires_guest_revision());
        emu::validate_workflow_adapter(
            workflow,
            configured_flow_adapter(&revisioned.capabilities).unwrap(),
        )
        .unwrap();
        validate_payload_admission(&revisioned, workflow).unwrap();
        validate_payload_admission(&definition, workflow).unwrap();

        let parent_lease =
            qol_dev_env::resources::ParentLeaseClaim::parse("debian-batch-1").unwrap();
        let revisioned_args =
            emu::child_launch_args(serial_child_launch(&revisioned, &parent_lease)).unwrap();
        let revisioned_args: Vec<String> = revisioned_args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let index = revisioned_args
            .iter()
            .position(|arg| arg == "--guest-image-revision")
            .unwrap();
        assert_eq!(revisioned_args[index + 1], "debian-13-qol-1");

        let revisionless_args =
            emu::child_launch_args(serial_child_launch(&definition, &parent_lease)).unwrap();
        let revisionless_args: Vec<String> = revisionless_args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(!revisionless_args
            .iter()
            .any(|arg| arg == "--guest-image-revision"));
    }

    #[test]
    fn desktop_workflows_require_a_guest_revision_in_a_desktop_compatible_environment() {
        let definition = qol_dev_env::registry::parse_definition(
            include_str!("../../../../flows/envs/linux-mint-cinnamon.toml"),
            Path::new("flows/envs/linux-mint-cinnamon.toml"),
        )
        .unwrap();
        let mut revisionless = definition;
        revisionless.capabilities.remove("image_revision");
        let adapter = configured_flow_adapter(&revisionless.capabilities).unwrap();
        let desktop = emu::workflow_definition("bluetooth-storm").unwrap();
        assert!(desktop.requires_guest_revision());
        emu::validate_workflow_adapter(desktop, adapter).unwrap();
        let error = validate_payload_admission(&revisionless, desktop).unwrap_err();
        assert!(error.to_string().contains("image_revision"), "{error}");
    }

    #[test]
    fn workspace_mounts_stay_forbidden_for_every_immutable_payload_workflow() {
        let definition = registered_serial_definition(include_str!(
            "../../../../flows/envs/linux-ubuntu-nocloud.toml"
        ));
        let mut mounted = definition;
        mounted.mounts.workspace = true;
        let wave2 = emu::workflow_definition("resident-wave2-apt-preferences").unwrap();
        let error = validate_payload_admission(&mounted, wave2).unwrap_err();
        assert!(error.to_string().contains("immutable payload"), "{error}");
        let desktop = emu::workflow_definition("qol-shot-capture").unwrap();
        let error = validate_payload_admission(&mounted, desktop).unwrap_err();
        assert!(error.to_string().contains("immutable payload"), "{error}");
        validate_payload_admission(
            &mounted,
            emu::workflow_definition("leaves-no-trace").unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn resident_payload_consumption_never_touches_the_shared_cache_or_key() {
        let dir = tempfile::tempdir().unwrap();
        let run_dir = dir.path().join("run");
        fs::create_dir_all(&run_dir).unwrap();
        let snapshot_dir = run_dir.join("bundle-snapshot");
        let key = "a".repeat(64);
        emu::resident_wave2::bundle::write_fake_bundle(&snapshot_dir, &key);
        let cache_root = dir.path().join("cache");

        let files =
            emu::resident_wave2::bundle::snapshot_payload_files(&snapshot_dir, &run_dir).unwrap();
        assert_eq!(files.len(), 5);
        for file in &files {
            if file.relative_path == *Path::new("scenario.sh") {
                assert_eq!(file.source, run_dir.join("wave2-scenario.sh"));
            } else {
                assert!(
                    file.source.starts_with(&snapshot_dir),
                    "{}",
                    file.source.display()
                );
            }
        }
        assert!(
            !cache_root.exists(),
            "the parent consumption path must never touch the shared cache"
        );
        let snapshot_entries: Vec<String> = fs::read_dir(&snapshot_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            !snapshot_entries.iter().any(|name| name == "scenario.sh"),
            "the scenario must stay outside the snapshot bundle set"
        );
    }

    #[test]
    fn flow_plan_fingerprint_covers_semantic_config_and_ticket() {
        let temp = tempfile::tempdir().unwrap();
        let plan = immutable_flow_plan(temp.path());
        let fingerprint = plan.fingerprint().unwrap();
        assert_eq!(fingerprint.len(), 64);

        let mut changed_config = plan.clone();
        changed_config
            .environment
            .definition
            .capabilities
            .insert("image_revision".to_string(), "changed-revision".to_string());
        assert_ne!(changed_config.fingerprint().unwrap(), fingerprint);

        let mut changed_ticket = plan;
        changed_ticket.ticket.report_path = temp.path().join("other/report.json");
        assert_ne!(changed_ticket.fingerprint().unwrap(), fingerprint);
    }

    #[test]
    fn typed_worker_plan_rejects_drift_before_creating_a_report() {
        let temp = tempfile::tempdir().unwrap();
        let plan = immutable_flow_plan(temp.path());
        let run_dir = plan.ticket.report_path.parent().unwrap().to_path_buf();
        let mut request = FlowWorkerRequest {
            start: plan.start.clone(),
            run_root: plan.run_root.clone(),
            plan_fingerprint: plan.fingerprint().unwrap(),
            verbose: false,
        };
        validate_flow_worker_plan(&request, &plan).unwrap();

        request.plan_fingerprint = "b".repeat(64);
        let error = validate_flow_worker_plan(&request, &plan).unwrap_err();

        assert!(error.to_string().contains("plan changed"));
        assert!(!run_dir.exists());
    }

    struct FixedCancellation(bool);

    impl CancellationSource for FixedCancellation {
        fn is_cancelled(&self) -> bool {
            self.0
        }
    }

    #[cfg(target_os = "linux")]
    struct CancelAfterPoll {
        polls: AtomicUsize,
        cancel_at: usize,
    }

    #[cfg(target_os = "linux")]
    impl CancellationSource for CancelAfterPoll {
        fn is_cancelled(&self) -> bool {
            self.polls.fetch_add(1, Ordering::SeqCst) + 1 >= self.cancel_at
        }
    }

    struct RecoveryFixture {
        flow_dir: PathBuf,
        report_path: PathBuf,
        lane_dir: PathBuf,
        lane_id: String,
    }

    fn recovery_fixture(
        temp: &tempfile::TempDir,
        owner_pid: u32,
        owner_state: &str,
        lane_phase: &str,
    ) -> RecoveryFixture {
        let flow_id = "flow-recovery-test";
        let lane_id = "lane-recovery-test".to_string();
        let flow_dir = temp.path().join("flows").join(flow_id);
        let lane_dir = temp.path().join("cases").join(&lane_id);
        let report_path = flow_dir.join("report.json");
        let child_report_path = lane_dir.join("report.json");
        let log_path = flow_dir.join("logs").join(format!("{lane_id}.log"));
        fs::create_dir_all(flow_dir.join("steps")).unwrap();
        fs::create_dir_all(flow_dir.join("logs")).unwrap();
        fs::create_dir_all(flow_dir.join(LANE_OWNERS_DIR)).unwrap();
        fs::create_dir_all(&lane_dir).unwrap();
        let report = json!({
            "name": "qol-flow-run",
            "kind": "flow-fanout",
            "run_id": flow_id,
            "started_at_unix_ms": 1,
            "status": if owner_state == "cancelling" { "cancelling" } else { "running" },
            "owner": { "pid": owner_pid, "state": owner_state },
            "workflow": { "id": "leaves-no-trace", "repeat": 1, "jobs": 1 },
            "lanes": [{
                "run_id": lane_id,
                "phase": lane_phase,
                "passed": false,
                "completed": false,
                "process_status": lane_phase,
                "report_status": null,
                "verdict": null,
                "cleanup": { "status": "pending", "complete": false },
                "report": child_report_path,
                "log": log_path,
            }],
            "steps": [
                { "name": "preflight", "status": "pass" },
                {
                    "name": "fanout",
                    "status": "running",
                    "completed_lanes": 0,
                    "reported_lanes": 1,
                    "requested_lanes": 1,
                },
            ],
        });
        fs::write(
            &report_path,
            format!("{}\n", serde_json::to_string_pretty(&report).unwrap()),
        )
        .unwrap();
        let journal = json!({
            "kind": "flow-lane-owner",
            "run_id": lane_id,
            "flow_run_id": flow_id,
            "owner_pid": owner_pid,
            "supervisor_pid": null,
            "phase": lane_phase,
        });
        fs::write(
            lane_owner_path(&flow_dir, &lane_id),
            format!("{}\n", serde_json::to_string_pretty(&journal).unwrap()),
        )
        .unwrap();
        RecoveryFixture {
            flow_dir,
            report_path,
            lane_dir,
            lane_id,
        }
    }

    fn read_value(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    enum FakePlan {
        Spawn {
            wait: FakeWait,
            report: Option<Value>,
        },
        Fail(&'static str),
    }

    enum FakeWait {
        Exit(SupervisorExit),
        Error(&'static str),
    }

    struct FakeSupervisor {
        wait: Option<FakeWait>,
        live: bool,
        shutdowns: Arc<AtomicUsize>,
        abandoned: Arc<AtomicUsize>,
    }

    impl Supervisor for FakeSupervisor {
        fn try_wait(&mut self) -> Result<Option<SupervisorExit>> {
            let wait = self
                .wait
                .take()
                .ok_or_else(|| anyhow!("fake supervisor has no wait event"))?;
            match wait {
                FakeWait::Exit(exit) => {
                    self.live = false;
                    Ok(Some(exit))
                }
                FakeWait::Error(error) => Err(anyhow!(error)),
            }
        }

        fn shutdown(&mut self, _: &str) -> ShutdownOutcome {
            if self.live {
                self.shutdowns.fetch_add(1, Ordering::SeqCst);
                self.live = false;
            }
            ShutdownOutcome {
                process_status: "terminated".to_string(),
                error: None,
                cleanup: LaneCleanup::not_required(),
            }
        }
    }

    impl Drop for FakeSupervisor {
        fn drop(&mut self) {
            if self.live {
                self.abandoned.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    struct CoordinatedShutdown {
        started: mpsc::Sender<()>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Supervisor for CoordinatedShutdown {
        fn try_wait(&mut self) -> Result<Option<SupervisorExit>> {
            Ok(None)
        }

        fn shutdown(&mut self, _: &str) -> ShutdownOutcome {
            self.started.send(()).unwrap();
            let (released, wake) = &*self.release;
            let mut released = released.lock().unwrap();
            while !*released {
                released = wake.wait(released).unwrap();
            }
            ShutdownOutcome {
                process_status: "terminated".to_string(),
                error: None,
                cleanup: LaneCleanup::not_required(),
            }
        }
    }

    struct FakeSpawner {
        plans: VecDeque<FakePlan>,
        shutdowns: Arc<AtomicUsize>,
        abandoned: Arc<AtomicUsize>,
        cancel_after_spawn: Option<qol_process::CancellationToken>,
    }

    impl LaneSpawner for FakeSpawner {
        fn spawn(&mut self, launch: &LaneLaunch<'_>, pending: &PendingLane) -> Result<ActiveLane> {
            let plan = self
                .plans
                .pop_front()
                .ok_or_else(|| anyhow!("fake spawn plan exhausted"))?;
            let (wait, report) = match plan {
                FakePlan::Spawn { wait, report } => (wait, report),
                FakePlan::Fail(error) => bail!(error),
            };
            let report_path = launch.case_root.join(&pending.run_id).join("report.json");
            if let Some(mut report) = report {
                if report.get("kind").is_none() {
                    report["kind"] = json!("flow");
                }
                if report.get("run_id").is_none() {
                    report["run_id"] = json!(pending.run_id.as_str());
                }
                fs::create_dir_all(report_path.parent().unwrap()).unwrap();
                fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
            }
            let lane = ActiveLane {
                run_id: pending.run_id.clone(),
                report_path,
                log_path: launch.logs_dir.join(format!("{}.log", pending.run_id)),
                supervisor: Box::new(FakeSupervisor {
                    wait: Some(wait),
                    live: true,
                    shutdowns: Arc::clone(&self.shutdowns),
                    abandoned: Arc::clone(&self.abandoned),
                }),
            };
            if let Some(cancellation) = &self.cancel_after_spawn {
                cancellation.cancel();
            }
            Ok(lane)
        }
    }

    fn fake_spawner(plans: Vec<FakePlan>) -> (FakeSpawner, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let abandoned = Arc::new(AtomicUsize::new(0));
        (
            FakeSpawner {
                plans: plans.into(),
                shutdowns: Arc::clone(&shutdowns),
                abandoned: Arc::clone(&abandoned),
                cancel_after_spawn: None,
            },
            shutdowns,
            abandoned,
        )
    }

    fn fake_launch<'a>(temp: &'a tempfile::TempDir, executable: &'a Path) -> LaneLaunch<'a> {
        LaneLaunch {
            executable,
            worktree: temp.path(),
            logs_dir: temp.path(),
            case_root: temp.path(),
            flow_run_id: "flow-test",
            flow_report_path: temp.path(),
            owner_pid: std::process::id(),
            owner_process_identity: qol_process::process_identity(std::process::id()).ok(),
        }
    }

    fn pending(run_ids: &[&str]) -> Vec<PendingLane> {
        run_ids
            .iter()
            .map(|run_id| PendingLane {
                run_id: (*run_id).to_string(),
                args: Vec::new(),
            })
            .collect()
    }

    fn passing_exit() -> SupervisorExit {
        SupervisorExit {
            success: true,
            description: "exit status: 0".to_string(),
            cleanup: LaneCleanup::not_required(),
        }
    }

    fn passing_report() -> Value {
        json!({
            "status": "pass",
            "workflow": {
                "verdict": "pass",
            },
        })
    }

    #[test]
    fn abort_starts_every_active_lane_shutdown_concurrently() {
        let (started, starts) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let mut active = ["lane-1", "lane-2"]
            .into_iter()
            .map(|run_id| ActiveLane {
                run_id: run_id.to_string(),
                report_path: PathBuf::from(format!("/runs/{run_id}/report.json")),
                log_path: PathBuf::from(format!("/runs/{run_id}/run.log")),
                supervisor: Box::new(CoordinatedShutdown {
                    started: started.clone(),
                    release: Arc::clone(&release),
                }),
            })
            .collect::<Vec<_>>();
        let worker = std::thread::spawn(move || {
            let mut results = Vec::new();
            abort_lanes(&mut active, &mut results, "test cancellation");
            results
        });

        let first = starts.recv_timeout(Duration::from_secs(1));
        let second = starts.recv_timeout(Duration::from_secs(1));
        {
            let (released, wake) = &*release;
            *released.lock().unwrap() = true;
            wake.notify_all();
        }
        let results = worker.join().unwrap();

        assert!(first.is_ok(), "first lane did not begin shutdown");
        assert!(second.is_ok(), "second lane shutdown waited for the first");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn spawn_failure_stops_every_previously_owned_lane() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("qol");
        let launch = fake_launch(&temp, &executable);
        let (mut spawner, shutdowns, abandoned) = fake_spawner(vec![
            FakePlan::Spawn {
                wait: FakeWait::Exit(passing_exit()),
                report: None,
            },
            FakePlan::Fail("injected spawn failure"),
        ]);
        let cancellation = qol_process::CancellationToken::new();

        let outcome = execute_lanes(
            &mut spawner,
            &launch,
            &pending(&["lane-1", "lane-2", "lane-3"]),
            2,
            false,
            &cancellation,
            None,
        );

        assert!(outcome
            .error
            .as_deref()
            .is_some_and(|error| error.contains("injected spawn failure")));
        assert_eq!(outcome.results.len(), 3);
        assert!(outcome.results.iter().all(|result| !result.completed));
        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(abandoned.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn wait_failure_stops_failing_and_peer_supervisors_without_abandonment() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("qol");
        let launch = fake_launch(&temp, &executable);
        let (mut spawner, shutdowns, abandoned) = fake_spawner(vec![
            FakePlan::Spawn {
                wait: FakeWait::Error("injected wait failure"),
                report: None,
            },
            FakePlan::Spawn {
                wait: FakeWait::Exit(passing_exit()),
                report: None,
            },
        ]);
        let cancellation = qol_process::CancellationToken::new();

        let outcome = execute_lanes(
            &mut spawner,
            &launch,
            &pending(&["lane-1", "lane-2"]),
            2,
            false,
            &cancellation,
            None,
        );

        assert!(outcome
            .error
            .as_deref()
            .is_some_and(|error| error.contains("injected wait failure")));
        assert_eq!(outcome.results.len(), 2);
        assert_eq!(shutdowns.load(Ordering::SeqCst), 2);
        assert_eq!(abandoned.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn terminal_failure_preserves_completed_lane_results_in_report_data() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("qol");
        let launch = fake_launch(&temp, &executable);
        let (mut spawner, shutdowns, abandoned) = fake_spawner(vec![
            FakePlan::Spawn {
                wait: FakeWait::Exit(passing_exit()),
                report: Some(passing_report()),
            },
            FakePlan::Fail("injected second-lane failure"),
        ]);
        let cancellation = qol_process::CancellationToken::new();

        let outcome = execute_lanes(
            &mut spawner,
            &launch,
            &pending(&["lane-1", "lane-2"]),
            1,
            false,
            &cancellation,
            None,
        );
        let lanes = lane_reports(&outcome.results);

        assert_eq!(outcome.results.len(), 2);
        assert!(outcome.results[0].passed);
        assert!(outcome.results[0].completed);
        assert!(!outcome.results[1].passed);
        assert_eq!(lanes[0]["run_id"], "lane-1");
        assert_eq!(lanes[0]["passed"], true);
        assert_eq!(lanes[0]["completed"], true);
        assert_eq!(lanes[1]["run_id"], "lane-2");
        assert_eq!(lanes[1]["process_status"], "spawn failed");
        assert_eq!(shutdowns.load(Ordering::SeqCst), 0);
        assert_eq!(abandoned.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn finished_lane_copies_workflow_artifacts_into_the_flow_run_dir() {
        let temp = tempfile::tempdir().unwrap();
        let case_artifact = temp.path().join("case/artifacts/metrics.json");
        fs::create_dir_all(case_artifact.parent().unwrap()).unwrap();
        fs::write(&case_artifact, b"{\"p50\":1}").unwrap();
        let flow_report_path = temp.path().join("flow/report.json");
        fs::create_dir_all(flow_report_path.parent().unwrap()).unwrap();
        let report = json!({
            "workflow": { "artifacts": [case_artifact.to_string_lossy()] },
        });

        let error = copy_lane_artifacts(Some(&report), "lane-1", &flow_report_path);

        assert_eq!(error, None);
        assert_eq!(
            fs::read(temp.path().join("flow/artifacts/lane-1/metrics.json")).unwrap(),
            b"{\"p50\":1}"
        );
        let missing = json!({
            "workflow": { "artifacts": ["/a/b/does-not-exist.json"] },
        });
        let error = copy_lane_artifacts(Some(&missing), "lane-1", &flow_report_path);
        assert!(error
            .as_deref()
            .is_some_and(|error| error.contains("does-not-exist.json")));
    }

    #[test]
    fn finished_lane_rejects_wrong_kind_and_missing_run_identity() {
        let cases = [
            (
                json!({
                    "kind": "environment",
                    "run_id": "lane-1",
                    "status": "pass",
                    "workflow": { "verdict": "pass" },
                }),
                "expected `flow`",
            ),
            (
                json!({
                    "kind": "flow",
                    "status": "pass",
                    "workflow": { "verdict": "pass" },
                }),
                "has no run_id",
            ),
        ];
        for (report, expected) in cases {
            let temp = tempfile::tempdir().unwrap();
            let report_path = temp.path().join("lane-1/report.json");
            fs::create_dir_all(report_path.parent().unwrap()).unwrap();
            fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
            let lane = ActiveLane {
                run_id: "lane-1".to_string(),
                report_path,
                log_path: temp.path().join("lane-1.log"),
                supervisor: Box::new(FakeSupervisor {
                    wait: None,
                    live: false,
                    shutdowns: Arc::new(AtomicUsize::new(0)),
                    abandoned: Arc::new(AtomicUsize::new(0)),
                }),
            };

            let executable = temp.path().join("qol");
            let result = finish_lane(lane, passing_exit(), &fake_launch(&temp, &executable));

            assert!(!result.passed);
            assert!(result
                .error
                .as_deref()
                .is_some_and(|error| error.contains(expected)));
            assert!(result.report_status.is_none());
            assert!(result.verdict.is_none());
        }
    }

    #[test]
    fn cancellation_stops_active_lanes_and_records_unstarted_lanes() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("qol");
        let launch = fake_launch(&temp, &executable);
        let cancellation = qol_process::CancellationToken::new();
        let (mut spawner, shutdowns, abandoned) = fake_spawner(vec![FakePlan::Spawn {
            wait: FakeWait::Exit(passing_exit()),
            report: None,
        }]);
        spawner.cancel_after_spawn = Some(cancellation.clone());

        let outcome = execute_lanes(
            &mut spawner,
            &launch,
            &pending(&["lane-1", "lane-2", "lane-3"]),
            1,
            false,
            &cancellation,
            None,
        );

        assert!(outcome.cancelled);
        assert_eq!(outcome.error.as_deref(), Some("flow execution cancelled"));
        assert_eq!(outcome.results.len(), 3);
        assert_eq!(outcome.results[0].process_status, "terminated");
        assert_eq!(outcome.results[1].process_status, "not started");
        assert_eq!(outcome.results[2].process_status, "not started");
        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(abandoned.load(Ordering::SeqCst), 0);
        assert_eq!(flow_status(false, true, true), "cancelled");
        assert_eq!(
            flow_status(false, true, false),
            "cancellation-cleanup-incomplete"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cancellation_stops_the_owned_payload_build_tree_with_verified_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("flow-test");
        fs::create_dir_all(&run_dir).unwrap();
        let journals = PreparationJournals::initialize(&run_dir).unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & wait"]);
        let cancellation = CancelAfterPoll {
            polls: AtomicUsize::new(0),
            cancel_at: 2,
        };

        let failure =
            run_owned_preparation_command(command, &cancellation, &journals.build).unwrap_err();

        assert!(failure.cancelled);
        assert_eq!(failure.preparation.status, "cancelled");
        assert_eq!(failure.preparation.build_status, "cancelled");
        assert!(failure.preparation.cleanup.complete);
        assert_eq!(
            failure.preparation.cleanup.verification.as_deref(),
            Some("owned-process-tree-exit")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn successful_payload_build_waits_for_owned_tree_cleanup_proof() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("flow-test");
        fs::create_dir_all(&run_dir).unwrap();
        let journals = PreparationJournals::initialize(&run_dir).unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "exit 0"]);

        let status =
            run_owned_preparation_command(command, &FixedCancellation(false), &journals.build)
                .unwrap();

        assert!(status.success());
        let evidence = read_value(&journals.build.path);
        assert_eq!(evidence["state"], "complete");
        assert_eq!(evidence["cleanup"]["complete"], true);
    }

    #[test]
    fn preparation_report_distinguishes_pending_and_verified_process_cleanup() {
        let pending = FlowPreparation::pending(true);
        let pending_report = preparation_report(&pending);
        assert_eq!(pending_report["status"], "preparing");
        assert_eq!(pending_report["build"]["status"], "pending");
        assert_eq!(pending_report["build"]["cleanup"]["complete"], false);

        let verified = FlowPreparation {
            status: "complete".to_string(),
            build_status: "pass".to_string(),
            process_status: Some("exit status: 0".to_string()),
            cleanup: PreparationCleanup::verified(),
            iso_status: "pass".to_string(),
            iso_process_status: Some("exit status: 0".to_string()),
            iso_cleanup: PreparationCleanup::verified(),
        };
        let verified_report = preparation_report(&verified);
        assert_eq!(verified_report["build"]["cleanup"]["complete"], true);
        assert_eq!(
            verified_report["build"]["cleanup"]["verification"],
            "owned-process-tree-exit"
        );
        assert_eq!(
            verified_report["iso"]["cleanup"]["verification"],
            "owned-process-tree-exit"
        );
        assert!(verified_report.get("os_security_boundary").is_none());
    }

    #[test]
    fn durable_preparation_evidence_repairs_a_pending_aggregate_after_owner_loss() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("flow-test");
        fs::create_dir_all(&run_dir).unwrap();
        let journals = PreparationJournals::initialize(&run_dir).unwrap();
        journals
            .build
            .record(
                "complete",
                Some(42),
                Some("process-identity"),
                Some("exit status: 0"),
                &PreparationCleanup::verified(),
            )
            .unwrap();
        let mut report = json!({
            "preparation": preparation_report(&FlowPreparation::pending(true)),
        });

        reconcile_preparation_evidence(&run_dir, &mut report);

        assert_eq!(report["preparation"]["build"]["cleanup"]["complete"], true);
        assert_eq!(
            report["preparation"]["build"]["cleanup"]["verification"],
            "owned-process-tree-exit"
        );
        assert_eq!(report["preparation"]["iso"]["cleanup"]["complete"], true);
        assert_eq!(report["preparation"]["iso"]["status"], "skipped");
    }

    #[test]
    fn preparation_terminal_intent_is_durable_before_cleanup_proof() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("flow-test");
        fs::create_dir_all(&run_dir).unwrap();
        let journals = PreparationJournals::initialize(&run_dir).unwrap();

        journals
            .build
            .record(
                "cleanup-incomplete",
                Some(42),
                Some("process-identity"),
                Some("cancelled"),
                &PreparationCleanup::incomplete(
                    "preparation cleanup began but terminal process-tree proof was not persisted",
                ),
            )
            .unwrap();

        let evidence = read_value(&journals.build.path);
        assert_eq!(evidence["state"], "cleanup-incomplete");
        assert_eq!(evidence["cleanup"]["complete"], false);
        assert_eq!(evidence["process"]["pid"], 42);
        assert_eq!(evidence["process"]["identity"], "process-identity");
    }

    #[test]
    fn malformed_preparation_evidence_cannot_become_cleanup_proof() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("flow-test");
        fs::create_dir_all(run_dir.join("preparation")).unwrap();
        atomic_json_durable(
            &run_dir.join("preparation/build.json"),
            &json!({
                "kind": "flow-preparation-process",
                "run_id": "flow-test",
                "phase": "build",
                "state": "running",
                "process": { "pid": 42, "identity": "process-identity" },
                "cleanup": {
                    "status": "complete",
                    "complete": true,
                    "verification": "owned-process-tree-exit",
                    "error": null,
                },
            }),
        )
        .unwrap();
        let mut report = json!({
            "preparation": preparation_report(&FlowPreparation::pending(true)),
        });

        reconcile_preparation_evidence(&run_dir, &mut report);

        assert_eq!(report["preparation"]["build"]["cleanup"]["complete"], false);
        assert!(report["preparation"]["build"]["cleanup"]["error"]
            .as_str()
            .unwrap()
            .contains("contract is invalid"));
    }

    #[test]
    fn only_terminal_reports_receive_error_and_finish_timestamp() {
        let mut active = json!({"status": "running"});
        apply_report_lifecycle(&mut active, None, None);
        assert!(active.get("finished_at_unix_ms").is_none());
        assert!(active.get("error").is_none());

        let mut terminal = json!({"status": "failed"});
        apply_report_lifecycle(&mut terminal, Some("injected failure"), Some(42));
        assert_eq!(terminal["finished_at_unix_ms"], 42);
        assert_eq!(terminal["error"], "injected failure");
    }

    #[test]
    fn aggregate_runtime_boundary_is_payload_scoped_and_content_bound() {
        let mut report = json!({"kind": "flow-fanout"});
        add_payload_runtime_boundary_evidence(&mut report, None);
        assert!(report.get("desktop_runtime_boundary").is_none());

        let payload = FlowPayload {
            root: PathBuf::from("/runs/payload/root"),
            manifest_path: PathBuf::from("/runs/payload/root/manifest.json"),
            image_path: PathBuf::from("/runs/payload/digest.iso"),
            manifest_sha256: "digest".to_string(),
            cleanup: PayloadCleanup::pending(),
        };
        add_payload_runtime_boundary_evidence(&mut report, Some(&payload));

        assert_eq!(
            report["desktop_runtime_boundary"],
            json!({
                "scope": "guest-runtime-only",
                "host_worker": {
                    "session_environment_cleared": true,
                    "os_security_boundary": false,
                },
                "guest": {
                    "headless": true,
                    "offline": true,
                    "workspace_mounted": false,
                },
                "guest_payload": {
                    "identity": {
                        "kind": "manifest-sha256",
                        "value": "digest",
                        "immutable": true,
                    },
                    "transport": "read-only-iso9660",
                    "read_only": true,
                },
            })
        );
        assert_eq!(
            payload_report(&payload)["transport"],
            report["desktop_runtime_boundary"]["guest_payload"]["transport"]
        );
    }

    #[test]
    fn flow_cancellation_observes_either_signals_or_the_owner_inbox() {
        for (signals, inbox, expected) in [
            (false, false, false),
            (true, false, true),
            (false, true, true),
            (true, true, true),
        ] {
            let signals = FixedCancellation(signals);
            let inbox = FixedCancellation(inbox);
            let cancellation = FlowCancellation {
                signals: &signals,
                inbox: &inbox,
            };
            assert_eq!(cancellation.is_cancelled(), expected);
        }
    }

    #[test]
    fn successful_payload_cleanup_removes_image_root_and_payload_directory() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let source = base.join("qol-shot");
        fs::write(&source, b"sandbox binary").unwrap();
        let payload_dir = base.join("flow/payload");
        let prepared = qol_dev_env::payload::stage_payload(
            &payload_dir.join("root"),
            "qol-shot-capture",
            &[qol_dev_env::payload::PayloadFileSpec {
                source,
                relative_path: PathBuf::from("bin/qol-shot"),
                executable: true,
            }],
        )
        .unwrap();
        let image_path = payload_dir.join("payload.iso");
        fs::write(&image_path, b"iso").unwrap();
        let root = prepared.root.clone();
        let mut payload = Some(FlowPayload {
            root: prepared.root,
            manifest_path: prepared.manifest_path,
            image_path: image_path.clone(),
            manifest_sha256: "digest".to_string(),
            cleanup: PayloadCleanup::pending(),
        });

        cleanup_workflow_payload(&mut payload).unwrap();

        let payload = payload.unwrap();
        assert_eq!(payload.cleanup.status, "complete");
        assert!(payload.cleanup.complete);
        assert_eq!(
            payload.cleanup.removed,
            vec![image_path.clone(), root.clone(), payload_dir.clone()]
        );
        assert!(!image_path.exists());
        assert!(!root.exists());
        assert!(!payload_dir.exists());
    }

    #[test]
    fn unresolved_unpublished_flow_cleanup_writes_durable_quarantine_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("flows/quarantined");

        write_unpublished_flow_failure(
            &run_dir,
            "quarantined",
            None,
            temp.path(),
            "cleanup failed",
        )
        .unwrap();

        let report_path = run_dir.join("report.json");
        let report = qol_dev_env::read_report(&report_path).unwrap().unwrap();
        assert_eq!(report.run_id, "quarantined");
        assert_eq!(report.kind, qol_dev_env::ReportKind::Flow);
        assert_eq!(report.status, qol_dev_env::ReportStatus::CleanupIncomplete);
        assert_eq!(report.owner.worktree.as_deref(), Some(temp.path()));
        assert!(matches!(
            report.cleanup,
            qol_dev_env::CleanupState::Incomplete(_)
        ));
    }

    #[test]
    fn dead_flow_owner_reconciles_its_owned_payload_before_becoming_terminal() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "running", "planned");
        let source = temp.path().join("payload-source");
        fs::write(&source, b"sandbox binary").unwrap();
        let payload_dir = fixture.flow_dir.join("payload");
        let prepared = qol_dev_env::payload::stage_payload(
            &payload_dir.join("root"),
            "qol-shot-capture",
            &[qol_dev_env::payload::PayloadFileSpec {
                source,
                relative_path: PathBuf::from("bin/qol-shot"),
                executable: true,
            }],
        )
        .unwrap();
        let digest = "a".repeat(64);
        let image_path = payload_dir.join(format!("{digest}.iso"));
        fs::write(&image_path, b"iso").unwrap();
        let mut report = read_value(&fixture.report_path);
        report["payload"] = json!({
            "manifest": prepared.manifest_path,
            "image": image_path.canonicalize().unwrap(),
            "manifest_sha256": digest,
            "cleanup": { "status": "pending", "complete": false },
        });
        fs::write(
            &fixture.report_path,
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "abandoned");
        assert_eq!(report["payload"]["cleanup"]["status"], "complete");
        assert_eq!(report["payload"]["cleanup"]["complete"], true);
        assert!(!payload_dir.exists());
    }

    #[test]
    fn recovery_refuses_payload_paths_outside_the_owned_flow_directory() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "running", "planned");
        let outside = temp.path().join("outside-manifest.json");
        fs::write(&outside, b"keep").unwrap();
        let digest = "a".repeat(64);
        let mut report = read_value(&fixture.report_path);
        report["payload"] = json!({
            "manifest": outside,
            "image": fixture.flow_dir.join("payload").join(format!("{digest}.iso")),
            "manifest_sha256": digest,
            "cleanup": { "status": "pending", "complete": false },
        });
        fs::write(
            &fixture.report_path,
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();

        assert_eq!(summary.status, "cleanup-incomplete");
        assert_eq!(fs::read(&outside).unwrap(), b"keep");
    }

    #[test]
    fn dead_owner_with_planned_lane_reconciles_to_abandoned() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "running", "planned");

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "abandoned");
        assert_eq!(report["status"], "abandoned");
        assert_eq!(report["lanes"][0]["phase"], "not-started");
        assert_eq!(report["lanes"][0]["cleanup"]["complete"], true);
        assert!(report.get("finished_at_unix_ms").is_some());
        assert!(fixture.flow_dir.join("report.interrupted.json").is_file());
    }

    #[test]
    fn dead_owner_with_spawn_failed_lane_reconciles_to_abandoned() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "running", "spawn-failed");

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "abandoned");
        assert_eq!(report["status"], "abandoned");
        assert_eq!(report["lanes"][0]["phase"], "spawn-failed");
        assert_eq!(report["lanes"][0]["process_status"], "spawn failed");
        assert_eq!(report["lanes"][0]["cleanup"]["complete"], true);
    }

    #[test]
    fn released_owner_with_unspawned_lane_becomes_terminal_without_a_child_report() {
        for (phase, recovered_phase) in
            [("planned", "not-started"), ("spawn-failed", "spawn-failed")]
        {
            let temp = tempfile::tempdir().unwrap();
            let fixture = recovery_fixture(&temp, std::process::id(), "released", phase);

            let summary = reconcile_flow_report_file(&fixture.report_path)
                .unwrap()
                .unwrap();
            let reconciled = fs::read(&fixture.report_path).unwrap();
            let repeated = reconcile_flow_report_file(&fixture.report_path)
                .unwrap()
                .unwrap();
            let report = read_value(&fixture.report_path);
            let parsed = qol_dev_env::read_report(&fixture.report_path)
                .unwrap()
                .unwrap();

            assert_eq!(summary.status, "failed");
            assert_eq!(repeated.status, "failed");
            assert_eq!(fs::read(&fixture.report_path).unwrap(), reconciled);
            assert_eq!(report["status"], "failed");
            assert_eq!(report["owner"]["state"], "released");
            assert_eq!(report["reconciliation"]["status"], "complete");
            assert_eq!(report["lanes"][0]["phase"], recovered_phase);
            assert_eq!(report["lanes"][0]["cleanup"]["complete"], true);
            assert!(!parsed.status.is_active());
            assert!(matches!(
                parsed.cleanup,
                qol_dev_env::CleanupState::Complete
            ));
        }
    }

    #[test]
    fn dead_owner_with_unverified_preparation_cleanup_stays_incomplete() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "running", "planned");
        let mut report = read_value(&fixture.report_path);
        report["preparation"] = preparation_report(&FlowPreparation::pending(true));
        fs::write(
            &fixture.report_path,
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "cleanup-incomplete");
        assert_eq!(report["status"], "cleanup-incomplete");
        assert_eq!(report["preparation"]["build"]["cleanup"]["complete"], false);
    }

    #[test]
    fn dead_owner_with_maybe_spawned_lane_stays_cleanup_incomplete() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "running", "launching");

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "cleanup-incomplete");
        assert_eq!(report["status"], "cleanup-incomplete");
        assert_eq!(report["lanes"][0]["cleanup"]["complete"], false);
        assert!(report.get("finished_at_unix_ms").is_none());
        assert!(lane_owner_path(&fixture.flow_dir, &fixture.lane_id).is_file());
    }

    #[test]
    fn released_owner_with_uncertain_supervisor_stabilizes_as_cleanup_incomplete() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, std::process::id(), "released", "spawned");
        let journal_path = lane_owner_path(&fixture.flow_dir, &fixture.lane_id);
        let mut journal = read_value(&journal_path);
        journal["supervisor_pid"] = json!(std::process::id());
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let repeated = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);
        let parsed = qol_dev_env::read_report(&fixture.report_path)
            .unwrap()
            .unwrap();

        assert_eq!(summary.status, "cleanup-incomplete");
        assert_eq!(repeated.status, "cleanup-incomplete");
        assert_eq!(report["status"], "cleanup-incomplete");
        assert_eq!(report["owner"]["state"], "released");
        assert_eq!(report["reconciliation"]["status"], "cleanup-incomplete");
        assert!(!parsed.status.is_active());
        assert!(matches!(
            parsed.cleanup,
            qol_dev_env::CleanupState::Incomplete(_)
        ));
        assert!(report["lanes"][0]["cleanup"]["error"]
            .as_str()
            .unwrap()
            .contains("liveness is uncertain"));
    }

    #[test]
    fn released_owner_with_verified_live_supervisor_stays_recovering() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, std::process::id(), "released", "spawned");
        let journal_path = lane_owner_path(&fixture.flow_dir, &fixture.lane_id);
        let mut journal = read_value(&journal_path);
        journal["supervisor_pid"] = json!(std::process::id());
        journal["supervisor_process_identity"] =
            json!(qol_process::process_identity(std::process::id()).unwrap());
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "recovering");
        assert_eq!(report["status"], "recovering");
        assert_eq!(report["owner"]["state"], "orphaned");
        assert_eq!(report["reconciliation"]["status"], "in-progress");
        assert_eq!(report["lanes"][0]["process_status"], "active");
    }

    #[test]
    fn live_owner_keeps_active_report_byte_identical() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, std::process::id(), "running", "planned");
        let mut report = read_value(&fixture.report_path);
        report["owner"]["process_identity"] =
            json!(qol_process::process_identity(std::process::id()).unwrap());
        fs::write(
            &fixture.report_path,
            format!("{}\n", serde_json::to_string_pretty(&report).unwrap()),
        )
        .unwrap();
        let before = fs::read(&fixture.report_path).unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();

        assert_eq!(summary.status, "running");
        assert_eq!(fs::read(&fixture.report_path).unwrap(), before);
    }

    #[test]
    fn unidentified_live_owner_stabilizes_as_cleanup_incomplete() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, std::process::id(), "running", "planned");

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let repeated = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "cleanup-incomplete");
        assert_eq!(repeated.status, "cleanup-incomplete");
        assert_eq!(report["status"], "cleanup-incomplete");
        assert_eq!(report["reconciliation"]["status"], "cleanup-incomplete");
        assert!(report.get("finished_at_unix_ms").is_none());
    }

    #[test]
    fn reused_owner_pid_with_mismatched_identity_is_not_treated_as_live() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, std::process::id(), "running", "planned");
        let mut report = read_value(&fixture.report_path);
        report["owner"]["process_identity"] = json!("stale-process-identity");
        fs::write(
            &fixture.report_path,
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();

        assert_eq!(summary.status, "abandoned");
    }

    #[test]
    fn recorded_process_state_requires_identity_but_keeps_legacy_liveness_uncertain() {
        let pid = std::process::id();
        let identity = qol_process::process_identity(pid).unwrap();
        assert_eq!(
            recorded_process_state(Some(pid), Some(&identity), false),
            RecordedProcessState::VerifiedAlive
        );
        assert_eq!(
            recorded_process_state(Some(pid), Some("stale-process-identity"), false),
            RecordedProcessState::VerifiedDead
        );
        assert_eq!(
            recorded_process_state(Some(pid), None, false),
            RecordedProcessState::Uncertain
        );
        assert_eq!(
            recorded_process_state(Some(u32::MAX), None, false),
            RecordedProcessState::VerifiedDead
        );
    }

    #[test]
    fn reused_child_pid_with_mismatched_identity_is_not_treated_as_active() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "released", "spawned");
        fs::write(
            fixture.lane_dir.join("report.json"),
            serde_json::to_vec_pretty(&json!({
                "kind": "flow",
                "run_id": fixture.lane_id,
                "status": "failed",
                "runtime": {
                    "supervisor_pid": std::process::id(),
                    "supervisor_process_identity": "stale-process-identity",
                },
                "teardown": {
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": true,
                    "removed": []
                },
            }))
            .unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "failed");
        assert_eq!(report["status"], "failed");
        assert_eq!(report["lanes"][0]["completed"], true);
        assert_eq!(report["lanes"][0]["process_status"], "reconciled");
    }

    #[test]
    fn recovery_rejects_a_non_flow_lane_report() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "released", "spawned");
        fs::write(
            fixture.lane_dir.join("report.json"),
            serde_json::to_vec_pretty(&json!({
                "kind": "environment",
                "run_id": fixture.lane_id,
                "status": "pass",
                "teardown": {
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": true,
                    "removed": []
                },
            }))
            .unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "cleanup-incomplete");
        assert_eq!(report["status"], "cleanup-incomplete");
        assert!(report["lanes"][0]["cleanup"]["error"]
            .as_str()
            .unwrap()
            .contains("kind `environment`, expected `flow`"));
    }

    #[test]
    fn unidentified_live_child_pid_stabilizes_as_cleanup_incomplete() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "released", "spawned");
        fs::write(
            fixture.lane_dir.join("report.json"),
            serde_json::to_vec_pretty(&json!({
                "kind": "flow",
                "run_id": fixture.lane_id,
                "status": "running",
                "runtime": { "supervisor_pid": std::process::id() },
                "teardown": { "status": "pending" },
            }))
            .unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let repeated = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "cleanup-incomplete");
        assert_eq!(repeated.status, "cleanup-incomplete");
        assert_eq!(report["status"], "cleanup-incomplete");
        assert_eq!(report["lanes"][0]["process_status"], "cleanup incomplete");
        assert!(report["lanes"][0]["cleanup"]["error"]
            .as_str()
            .unwrap()
            .contains("uncertain process identity"));
    }

    #[test]
    fn completed_child_pass_after_parent_crash_is_still_abandoned() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "running", "spawned");
        fs::write(
            fixture.lane_dir.join("report.json"),
            serde_json::to_vec_pretty(&json!({
                "kind": "flow",
                "run_id": fixture.lane_id,
                "status": "pass",
                "runtime": { "supervisor_pid": std::process::id() },
                "workflow": { "verdict": "pass" },
                "teardown": {
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": true,
                    "removed": []
                },
            }))
            .unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "abandoned");
        assert_eq!(report["lanes"][0]["passed"], true);
        assert_eq!(report["lanes"][0]["cleanup"]["complete"], true);
        assert_eq!(report["status"], "abandoned");
    }

    #[test]
    fn terminal_child_without_explicit_cleanup_proof_stays_incomplete() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "running", "spawned");
        fs::write(
            fixture.lane_dir.join("report.json"),
            serde_json::to_vec_pretty(&json!({
                "kind": "flow",
                "run_id": fixture.lane_id,
                "status": "pass",
                "workflow": { "verdict": "pass" },
                "teardown": { "removed": [] },
            }))
            .unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let report = read_value(&fixture.report_path);

        assert_eq!(summary.status, "cleanup-incomplete");
        assert_eq!(report["status"], "cleanup-incomplete");
        assert_eq!(report["lanes"][0]["cleanup"]["complete"], false);
        assert!(report["lanes"][0]["cleanup"]["error"]
            .as_str()
            .unwrap()
            .contains("verified process-tree exit"));
    }

    #[test]
    fn completed_cancelled_flow_reconciles_as_cancelled() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "cancelling", "spawned");
        fs::write(
            fixture.lane_dir.join("report.json"),
            serde_json::to_vec_pretty(&json!({
                "kind": "flow",
                "run_id": fixture.lane_id,
                "status": "failed",
                "teardown": {
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": true,
                    "removed": []
                },
            }))
            .unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();

        assert_eq!(summary.status, "cancelled");
        assert_eq!(read_value(&fixture.report_path)["status"], "cancelled");
    }

    #[test]
    fn mismatched_lane_path_refuses_reconciliation_without_rewriting() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "running", "planned");
        let mut report = read_value(&fixture.report_path);
        report["lanes"][0]["report"] = json!(temp.path().join("foreign/report.json"));
        fs::write(
            &fixture.report_path,
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        let before = fs::read(&fixture.report_path).unwrap();

        let error = reconcile_flow_report_file(&fixture.report_path).unwrap_err();

        assert!(format!("{error:#}").contains("path contract"));
        assert_eq!(fs::read(&fixture.report_path).unwrap(), before);
        assert!(lane_owner_path(&fixture.flow_dir, &fixture.lane_id).is_file());
    }

    #[test]
    fn ownerless_legacy_terminal_flow_does_not_block_reconciliation() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "running", "planned");
        let mut report = read_value(&fixture.report_path);
        report.as_object_mut().unwrap().remove("owner");
        report["status"] = json!("failed");
        report["lanes"][0]["report"] = json!(temp.path().join("legacy/report.json"));
        fs::write(
            &fixture.report_path,
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        let before = fs::read(&fixture.report_path).unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();

        assert_eq!(summary.status, "failed");
        assert_eq!(fs::read(&fixture.report_path).unwrap(), before);
    }

    #[test]
    fn ownerless_terminal_flow_reconciles_from_typed_child_cleanup_proof() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, u32::MAX, "running", "spawned");
        fs::write(
            fixture.lane_dir.join("report.json"),
            serde_json::to_vec_pretty(&json!({
                "kind": "flow",
                "run_id": fixture.lane_id,
                "status": "pass",
                "workflow": { "verdict": "pass" },
                "teardown": {
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": true,
                    "removed": []
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let mut report = read_value(&fixture.report_path);
        report.as_object_mut().unwrap().remove("owner");
        report["status"] = json!("pass");
        fs::write(
            &fixture.report_path,
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();
        let repaired = read_value(&fixture.report_path);

        assert_eq!(summary.status, "pass");
        assert_eq!(repaired["lanes"][0]["cleanup"]["complete"], true);
        assert_eq!(repaired["reconciliation"]["status"], "complete");
        assert!(fixture.flow_dir.join("report.interrupted.json").is_file());
    }

    #[test]
    fn ownerless_legacy_root_flow_reconciles_from_its_recorded_children() {
        let temp = tempfile::tempdir().unwrap();
        let run_root = temp.path().join("target/qol-env");
        let flow_id = "legacy-flow";
        let lane_id = "legacy-lane";
        let flow_dir = run_root.join("flows").join(flow_id);
        let lane_dir = temp.path().join("target/qol-emu").join(lane_id);
        let report_path = flow_dir.join("report.json");
        let child_report_path = lane_dir.join("report.json");
        let log_path = flow_dir.join("logs").join(format!("{lane_id}.log"));
        fs::create_dir_all(flow_dir.join("logs")).unwrap();
        fs::create_dir_all(&lane_dir).unwrap();
        fs::write(
            &child_report_path,
            serde_json::to_vec_pretty(&json!({
                "kind": "flow",
                "run_id": lane_id,
                "status": "pass",
                "workflow": { "verdict": "pass" },
                "teardown": {
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": true,
                    "removed": []
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            &report_path,
            serde_json::to_vec_pretty(&json!({
                "kind": "flow-fanout",
                "run_id": flow_id,
                "status": "pass",
                "workflow": { "id": "leaves-no-trace", "repeat": 1 },
                "lanes": [{
                    "run_id": lane_id,
                    "cleanup": { "complete": false },
                    "report": child_report_path,
                    "log": log_path
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&report_path).unwrap().unwrap();
        let repaired = read_value(&report_path);

        assert_eq!(summary.status, "pass");
        assert_eq!(repaired["lanes"][0]["cleanup"]["complete"], true);
        assert_eq!(
            repaired["reconciliation"]["source"],
            "qol-flow-legacy-lane-root-v1"
        );
        assert!(flow_dir.join("report.interrupted.json").is_file());
    }

    #[test]
    fn ownerless_cancelled_flow_reconciles_children_and_not_started_lanes() {
        let temp = tempfile::tempdir().unwrap();
        let run_root = temp.path().join("target/qol-env");
        let flow_id = "cancelled-flow";
        let flow_dir = run_root.join("flows").join(flow_id);
        let report_path = flow_dir.join("report.json");
        fs::create_dir_all(flow_dir.join("logs")).unwrap();
        let lanes = ["completed-lane", "not-started-lane"];
        let child_dir = run_root.join("cases").join(lanes[0]);
        fs::create_dir_all(&child_dir).unwrap();
        fs::write(
            child_dir.join("report.json"),
            serde_json::to_vec_pretty(&json!({
                "kind": "flow",
                "run_id": lanes[0],
                "status": "abandoned",
                "teardown": {
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": true,
                    "removed": []
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let lane = |run_id: &str, process_status: &str| {
            json!({
                "run_id": run_id,
                "completed": false,
                "passed": false,
                "process_status": process_status,
                "report_status": null,
                "cleanup": { "complete": false },
                "report": run_root.join("cases").join(run_id).join("report.json"),
                "log": flow_dir.join("logs").join(format!("{run_id}.log"))
            })
        };
        fs::write(
            &report_path,
            serde_json::to_vec_pretty(&json!({
                "kind": "flow-fanout",
                "run_id": flow_id,
                "status": "cancelled",
                "workflow": { "id": "leaves-no-trace", "repeat": 2 },
                "lanes": [lane(lanes[0], "terminated"), lane(lanes[1], "not started")]
            }))
            .unwrap(),
        )
        .unwrap();

        let summary = reconcile_flow_report_file(&report_path).unwrap().unwrap();
        let repaired = read_value(&report_path);

        assert_eq!(summary.status, "cancelled");
        assert_eq!(repaired["lanes"][0]["cleanup"]["complete"], true);
        assert_eq!(repaired["lanes"][1]["phase"], "not-started");
        assert_eq!(repaired["lanes"][1]["cleanup"]["complete"], true);
        assert!(
            qol_dev_env::parse_report(&report_path, &fs::read(&report_path).unwrap())
                .unwrap()
                .cleanup
                .is_complete()
        );
    }

    #[test]
    fn parses_defaults_and_complete_options_in_any_order() {
        let defaults = parse_options(&argv(&["leaves-no-trace", "--env", "linux/debian"])).unwrap();
        assert_eq!(defaults.repeat, 1);
        assert_eq!(defaults.jobs, 1);
        assert_eq!(defaults.memory_mb, None);
        assert_eq!(defaults.cpus, None);
        assert_eq!(defaults.run_id, None);
        assert_eq!(defaults.worktree, None);
        assert!(!defaults.force);

        let worktree = std::env::temp_dir().join("qol-worktrees/feat-x");
        let mut complete_args = argv(&[
            "--jobs",
            "10",
            "--env",
            "linux/debian",
            "--run-id",
            "dev-flow-1",
            "--force",
            "leaves-no-trace",
            "--repeat",
            "12",
            "--cpus",
            "2",
            "--memory-mb",
            "1536",
        ]);
        complete_args.extend([
            OsString::from("--worktree"),
            worktree.clone().into_os_string(),
        ]);
        let complete = parse_options(&complete_args).unwrap();
        assert_eq!(
            complete,
            FlowOptions {
                workflow: "leaves-no-trace".to_string(),
                environment_id: "linux/debian".to_string(),
                run_id: Some("dev-flow-1".to_string()),
                worktree: Some(worktree),
                repeat: 12,
                jobs: 10,
                memory_mb: Some(1536),
                cpus: Some(2),
                force: true,
            }
        );
    }

    #[test]
    fn rejects_missing_duplicate_unknown_and_out_of_range_options() {
        let cases = [
            (vec![], "usage"),
            (vec!["flow"], "--env"),
            (vec!["flow", "--env"], "requires a value"),
            (vec!["flow", "--env", "a", "--env", "b"], "duplicate"),
            (vec!["flow", "--env", "a", "--run-id", "bad/run"], "invalid"),
            (
                vec!["flow", "--env", "a", "--run-id", "one", "--run-id", "two"],
                "duplicate",
            ),
            (vec!["flow", "--env", "a", "--worktree"], "requires a value"),
            (
                vec!["flow", "--env", "a", "--worktree", "relative"],
                "absolute path",
            ),
            (
                vec!["flow", "--env", "a", "--force", "--force"],
                "duplicate",
            ),
            (vec!["flow", "--env", "a", "--jobs", "0"], "from 1"),
            (vec!["flow", "--env", "a", "--repeat", "129"], "from 1"),
            (vec!["flow", "--env", "a", "--memory-mb", "255"], "from 256"),
            (vec!["flow", "--env", "a", "--wat"], "unknown"),
            (vec!["one", "two", "--env", "a"], "usage"),
        ];
        for (arguments, expected) in cases {
            let error = parse_options(&argv(&arguments)).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "arguments: {arguments:?}, error: {error:#}"
            );
        }
        let first = std::env::temp_dir().join("qol-worktree-one");
        let second = std::env::temp_dir().join("qol-worktree-two");
        let duplicate = vec![
            OsString::from("flow"),
            OsString::from("--env"),
            OsString::from("a"),
            OsString::from("--worktree"),
            first.into_os_string(),
            OsString::from("--worktree"),
            second.into_os_string(),
        ];
        assert!(parse_options(&duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate"));
    }

    #[test]
    fn lane_verdict_requires_process_report_and_workflow_success() {
        let cases = [
            (true, Some("pass"), Some("pass"), true),
            (false, Some("pass"), Some("pass"), false),
            (true, Some("failed"), Some("pass"), false),
            (true, Some("pass"), Some("fail"), false),
            (true, None, Some("pass"), false),
            (true, Some("pass"), None, false),
        ];
        for (process, report, verdict, expected) in cases {
            assert_eq!(lane_passed(process, report, verdict), expected);
        }
    }

    #[test]
    fn manifest_adapter_selection_accepts_typed_prepared_adapters_and_rejects_unknowns() {
        let cases = [
            (Some("debian-nocloud"), Ok(emu::GuestAdapter::DebianNocloud)),
            (Some("macos-desktop"), Ok(emu::GuestAdapter::MacosDesktop)),
            (Some("mint-cinnamon"), Ok(emu::GuestAdapter::MintCinnamon)),
            (
                Some("windows-desktop"),
                Ok(emu::GuestAdapter::WindowsDesktop),
            ),
            (Some("mint"), Err("unknown flow adapter")),
            (None, Err("no automated flow adapter")),
        ];
        for (adapter, expected) in cases {
            let capabilities = adapter
                .map(|adapter| {
                    std::collections::BTreeMap::from([(
                        "flow_adapter".to_string(),
                        adapter.to_string(),
                    )])
                })
                .unwrap_or_default();
            match expected {
                Ok(expected) => {
                    assert_eq!(configured_flow_adapter(&capabilities).unwrap(), expected)
                }
                Err(expected) => {
                    let error = configured_flow_adapter(&capabilities).unwrap_err();
                    assert!(error.to_string().contains(expected), "error: {error:#}");
                }
            }
        }
    }

    #[test]
    fn mint_manifest_selects_only_automated_desktop_workflows() {
        let definition = qol_dev_env::registry::parse_definition(
            include_str!("../../../../flows/envs/linux-mint-cinnamon.toml"),
            Path::new("flows/envs/linux-mint-cinnamon.toml"),
        )
        .unwrap();

        assert_eq!(
            definition
                .capabilities
                .get("flow_adapter")
                .map(String::as_str),
            Some("mint-cinnamon")
        );
        assert!(!definition.mounts.workspace);
        let adapter = configured_flow_adapter(&definition.capabilities).unwrap();
        let serial = emu::workflow_definition("leaves-no-trace").unwrap();
        let alt_tab = emu::workflow_definition("alt-tab-storm").unwrap();
        let bluetooth = emu::workflow_definition("bluetooth-storm").unwrap();
        let hotkeys = emu::workflow_definition("hotkey-storm").unwrap();
        let launcher = emu::workflow_definition("launcher-storm").unwrap();
        let portable = emu::workflow_definition("portable-session").unwrap();
        let desktop = emu::workflow_definition("qol-shot-capture").unwrap();
        let shot_storm = emu::workflow_definition("qol-shot-storm").unwrap();
        let shortcut_storm = emu::workflow_definition("shortcut-storm").unwrap();
        let window_actions = emu::workflow_definition("window-actions-storm").unwrap();
        assert!(emu::validate_workflow_adapter(serial, adapter).is_err());
        emu::validate_workflow_adapter(alt_tab, adapter).unwrap();
        emu::validate_workflow_adapter(bluetooth, adapter).unwrap();
        emu::validate_workflow_adapter(hotkeys, adapter).unwrap();
        emu::validate_workflow_adapter(launcher, adapter).unwrap();
        emu::validate_workflow_adapter(portable, adapter).unwrap();
        emu::validate_workflow_adapter(desktop, adapter).unwrap();
        emu::validate_workflow_adapter(shot_storm, adapter).unwrap();
        emu::validate_workflow_adapter(shortcut_storm, adapter).unwrap();
        emu::validate_workflow_adapter(window_actions, adapter).unwrap();
    }
}
