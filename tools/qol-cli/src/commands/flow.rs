use crate::commands::dev_env::registry::{EnvironmentState, ResolvedEnvironment};
use crate::commands::dev_env::resources as dev_resources;
use crate::commands::{dev_env, emu};
use crate::progress::{print_hint, print_title, step_label, StepKind};
use crate::workspace::repo_root;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_REPEAT: u32 = 128;
const SUPERVISOR_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const SUPERVISOR_WAIT_INTERVAL: Duration = Duration::from_millis(25);
const LANE_OWNERS_DIR: &str = "lanes";

#[derive(Clone, Debug, PartialEq, Eq)]
struct FlowOptions {
    workflow: String,
    environment_id: String,
    repeat: u32,
    jobs: u32,
    memory_mb: Option<u32>,
    cpus: Option<u16>,
    force: bool,
}

struct ActiveLane {
    run_id: String,
    report_path: PathBuf,
    log_path: PathBuf,
    supervisor: Box<dyn Supervisor>,
}

struct LaneLaunch<'a> {
    executable: &'a Path,
    logs_dir: &'a Path,
    case_root: &'a Path,
    flow_run_id: &'a str,
    flow_report_path: &'a Path,
    owner_pid: u32,
}

struct PendingLane {
    run_id: String,
    args: Vec<OsString>,
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

trait Supervisor {
    fn try_wait(&mut self) -> Result<Option<SupervisorExit>>;
    fn shutdown(&mut self, reason: &str) -> ShutdownOutcome;
}

trait LaneSpawner {
    fn spawn(&mut self, launch: &LaneLaunch<'_>, pending: &PendingLane) -> Result<ActiveLane>;
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
    let mut process_tree = qol_process::guard_current_process_tree()
        .context("failed to guard the flow process tree")?;
    let result = run_flow_inner(options, verbose);
    result?;
    process_tree
        .disarm()
        .context("failed to disarm flow process-tree ownership")
}

fn run_flow_inner(options: &FlowOptions, verbose: bool) -> Result<()> {
    let cancellation = qol_process::CancellationToken::install()
        .context("failed to install flow cancellation handlers")?;
    crate::commands::env::reconcile_for_admission()?;
    reconcile_all()?;
    dev_resources::reconcile()?;
    let environment = environment(&options.environment_id)?;
    let image_path = environment
        .image_path
        .as_deref()
        .ok_or_else(|| anyhow!("environment `{}` has no image path", options.environment_id))?;
    let configured_memory_mb = options.memory_mb.unwrap_or(
        u32::try_from(environment.definition.boot.memory_mb).with_context(|| {
            format!(
                "environment `{}` memory does not fit in u32",
                options.environment_id
            )
        })?,
    );
    let configured_cpus = options.cpus.unwrap_or(environment.definition.boot.cpus);
    let resources =
        dev_resources::profile(u64::from(configured_memory_mb), u64::from(configured_cpus))?;
    let memory_mb = resources.memory_mb;
    let cpus = resources.cpus;
    let concurrent = options.jobs.min(options.repeat);
    let run_root = environment
        .run_root
        .clone()
        .unwrap_or(repo_root()?.join("target/qol-env"));
    let case_root = run_root.join("cases");
    let batch_id = emu::new_run_id(&format!("flow-{}", options.workflow))?;
    let run_dir = run_root.join("flows").join(&batch_id);
    let flow_report_path = run_dir.join("report.json");
    let (admission, resource_lease) = dev_resources::reserve(
        &batch_id,
        &flow_report_path,
        dev_resources::AdmissionRequest {
            concurrent_lanes: u64::from(concurrent),
            profile: resources,
            recommended_size_gb: environment.definition.image.recommended_size_gb,
            capacity: dev_resources::host_capacity(&run_root),
            force: options.force,
        },
    )?;

    let mut pending = Vec::with_capacity(options.repeat as usize);
    for index in 0..options.repeat {
        let run_id = emu::new_run_id(&format!("{}-lane-{}", options.environment_id, index + 1))?;
        let args = emu::child_launch_args(emu::ChildLaunch {
            operation: emu::ChildOperation::Run(&options.workflow),
            target: image_path,
            environment_id: &options.environment_id,
            run_id: &run_id,
            run_root: Some(&case_root),
            image_kind: Some(environment.definition.image.kind.as_str()),
            display: emu::DisplayMode::None,
            resources,
            acceleration: environment
                .definition
                .capabilities
                .get("acceleration")
                .map(String::as_str),
            arch: environment.definition.image.arch.as_deref(),
            firmware: environment.definition.image.firmware.as_deref(),
        })?;
        pending.push(PendingLane { run_id, args });
    }

    let logs_dir = run_dir.join("logs");
    let artifacts_dir = run_dir.join("artifacts");
    let steps_dir = run_dir.join("steps");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;
    fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
    fs::create_dir_all(&steps_dir)
        .with_context(|| format!("failed to create {}", steps_dir.display()))?;
    let executable = std::env::current_exe().context("failed to resolve the qol executable")?;
    let started_at = unix_millis()?;
    let lane_launch = LaneLaunch {
        executable: &executable,
        logs_dir: &logs_dir,
        case_root: &case_root,
        flow_run_id: &batch_id,
        flow_report_path: &flow_report_path,
        owner_pid: std::process::id(),
    };
    let planned = pending
        .iter()
        .map(|lane| planned_lane(&lane_launch, lane))
        .collect::<Vec<_>>();
    write_preflight(&run_dir, options, memory_mb, cpus, concurrent, admission)?;
    write_effective_environment(&run_dir, &environment, image_path, memory_mb, cpus)?;
    write_aggregate_report(
        &run_dir,
        &batch_id,
        options,
        &environment,
        image_path,
        memory_mb,
        cpus,
        admission,
        started_at,
        "running",
        None,
        &planned,
    )?;
    if let Err(error) = prepare_lane_owners(&lane_launch, &pending) {
        let message = format!("failed to persist flow lane ownership: {error:#}");
        let results = pending
            .iter()
            .map(|lane| not_started(&lane_launch, lane, &message))
            .collect::<Vec<_>>();
        write_aggregate_report(
            &run_dir,
            &batch_id,
            options,
            &environment,
            image_path,
            memory_mb,
            cpus,
            admission,
            started_at,
            "failed",
            Some(&message),
            &results,
        )?;
        resource_lease
            .release()
            .context("failed to release the flow resource lease")?;
        bail!(message);
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
    let passed = !cancelled
        && execution_error.is_none()
        && results.len() == options.repeat as usize
        && results.iter().all(|result| result.passed);
    let cleanup_complete = results.len() == options.repeat as usize
        && results.iter().all(|result| result.cleanup.complete);
    let status = flow_status(passed, cancelled, cleanup_complete);
    let terminal_error = terminal_error(execution_error.as_deref(), &results, options.repeat);
    write_aggregate_report(
        &run_dir,
        &batch_id,
        options,
        &environment,
        image_path,
        memory_mb,
        cpus,
        admission,
        started_at,
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

fn cmd_runs(args: &[OsString]) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol flow runs");
    }
    let runs = reconcile_all()?
        .into_iter()
        .filter(|run| !flow_status_is_terminal(&run.status))
        .collect::<Vec<_>>();
    dev_resources::reconcile()?;
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
    if flow_status_is_terminal(&status) {
        return Ok(Some(FlowRunSummary {
            run_id,
            status,
            report_path: path.to_path_buf(),
        }));
    }
    validate_flow_lanes(run_dir, &report)?;
    let owner_state = report
        .get("owner")
        .and_then(|owner| owner.get("state"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let owner_pid = report
        .get("owner")
        .and_then(|owner| owner.get("pid"))
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid != 0);
    let Some(owner_state) = owner_state else {
        return Ok(Some(FlowRunSummary {
            run_id,
            status,
            report_path: path.to_path_buf(),
        }));
    };
    let owner_is_active = matches!(owner_state.as_str(), "running" | "cancelling")
        && owner_pid.is_some_and(qol_process::is_pid_alive);
    if owner_is_active {
        return Ok(Some(FlowRunSummary {
            run_id,
            status,
            report_path: path.to_path_buf(),
        }));
    }
    let observed_at = unix_millis()?;
    let interrupted_path = run_dir.join("report.interrupted.json");
    if fs::symlink_metadata(&interrupted_path).is_err() {
        atomic_write(&interrupted_path, content.as_bytes())?;
    }
    let recovered_status = reconcile_flow_lanes(run_dir, &run_id, &mut report, &owner_state)?;
    report["status"] = json!(recovered_status);
    report["reconciliation"] = json!({
        "status": if flow_status_is_terminal(recovered_status) { "complete" } else { "in-progress" },
        "previous_status": status,
        "owner_pid": owner_pid,
        "owner_state": owner_state,
        "observed_at_unix_ms": observed_at,
        "interrupted_report": interrupted_path,
    });
    report["owner"]["state"] = json!(if flow_status_is_terminal(recovered_status) {
        "released"
    } else {
        "orphaned"
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

fn lock_flow_run(run_dir: &Path) -> Result<File> {
    fs::create_dir_all(run_dir)
        .with_context(|| format!("failed to create {}", run_dir.display()))?;
    let path = run_dir.join("reconcile.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    lock.lock()
        .with_context(|| format!("failed to lock {}", path.display()))?;
    Ok(lock)
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
    !run_id.is_empty()
        && run_id != "."
        && run_id != ".."
        && !run_id.contains('/')
        && !run_id.contains('\\')
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
    let child = match read_optional_json(&report_path) {
        Ok(child) => child,
        Err(error) => {
            mark_lane_incomplete(lane, None, error.clone())?;
            return Ok(LaneRecovery::Incomplete);
        }
    };
    let supervisor_alive = journal
        .as_ref()
        .and_then(|journal| journal.get("supervisor_pid"))
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .is_some_and(process_tree_alive);
    let Some(child) = child else {
        if supervisor_alive {
            mark_lane_active(lane, None)?;
            return Ok(LaneRecovery::Active);
        }
        let phase = journal
            .as_ref()
            .and_then(|journal| journal.get("phase"))
            .and_then(Value::as_str)
            .or_else(|| lane.get("phase").and_then(Value::as_str));
        if phase == Some("planned") {
            mark_lane_not_started(lane, "flow owner exited before this lane launched")?;
            return Ok(LaneRecovery::Resolved {
                passed: false,
                completed: false,
            });
        }
        mark_lane_incomplete(
            lane,
            None,
            "lane may have spawned but has no child report or verified cleanup".to_string(),
        )?;
        return Ok(LaneRecovery::Incomplete);
    };
    if child.get("run_id").and_then(Value::as_str) != Some(run_id.as_str()) {
        mark_lane_incomplete(lane, None, "child report identity mismatch".to_string())?;
        return Ok(LaneRecovery::Incomplete);
    }
    let child_status = child
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let child_alive = supervisor_alive || child_process_alive(&child);
    if matches!(child_status, "starting" | "running" | "stopping") {
        if child_alive {
            mark_lane_active(lane, Some(child_status))?;
            return Ok(LaneRecovery::Active);
        }
        mark_lane_incomplete(
            lane,
            Some(child_status),
            "active child report has no live owner and no verified cleanup".to_string(),
        )?;
        return Ok(LaneRecovery::Incomplete);
    }
    if child_alive {
        mark_lane_active(lane, Some(child_status))?;
        return Ok(LaneRecovery::Active);
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

fn process_tree_alive(pid: u32) -> bool {
    qol_process::is_pid_alive(pid) || qol_process::is_group_alive(pid)
}

fn child_process_alive(report: &Value) -> bool {
    let runtime = report.get("runtime");
    let supervisor_alive = runtime
        .and_then(|runtime| runtime.get("supervisor_pid"))
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .is_some_and(process_tree_alive);
    let qemu_alive = runtime
        .and_then(|runtime| runtime.get("qemu_pid"))
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .is_some_and(qol_process::is_pid_alive);
    supervisor_alive || qemu_alive
}

fn child_cleanup_complete(report: &Value, status: &str) -> std::result::Result<(), String> {
    if matches!(
        status,
        "cleanup-incomplete" | "rollback-incomplete" | "cancellation-cleanup-incomplete"
    ) {
        return Err(report
            .get("teardown")
            .and_then(|teardown| teardown.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("child cleanup is incomplete")
            .to_string());
    }
    if !matches!(
        status,
        "pass" | "failed" | "skipped" | "abandoned" | "cancelled"
    ) {
        return Err(format!("child report has unknown status `{status}`"));
    }
    let teardown = report
        .get("teardown")
        .filter(|teardown| !teardown.is_null())
        .ok_or_else(|| "terminal child report has no teardown evidence".to_string())?;
    if status != "abandoned" {
        return Ok(());
    }
    let complete = teardown.get("status").and_then(Value::as_str) == Some("complete");
    let qemu_exit = teardown.get("qemu_exit_verified").and_then(Value::as_bool) == Some(true);
    let tree_exit = teardown.get("tree_exit_verified").and_then(Value::as_bool) == Some(true);
    if complete && qemu_exit && tree_exit {
        return Ok(());
    }
    Err("abandoned child lacks verified process-tree exit or artifact cleanup".to_string())
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

fn mark_lane_not_started(lane: &mut Value, error: &str) -> Result<()> {
    let object = lane.as_object_mut().context("flow lane is not an object")?;
    object.insert("phase".to_string(), json!("not-started"));
    object.insert("passed".to_string(), json!(false));
    object.insert("completed".to_string(), json!(false));
    object.insert("process_status".to_string(), json!("not started"));
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

fn environment(id: &str) -> Result<ResolvedEnvironment> {
    let environment = dev_env::find(id)?
        .ok_or_else(|| anyhow!("unknown environment `{id}`; run `qol env list`"))?;
    if environment.state != EnvironmentState::Ready {
        let detail = environment.messages.join("; ");
        bail!(
            "environment `{id}` is {}: {detail}",
            environment.state.as_str()
        );
    }
    require_flow_adapter(&environment)?;
    Ok(environment)
}

fn require_flow_adapter(environment: &ResolvedEnvironment) -> Result<()> {
    if supported_flow_adapter(&environment.definition.capabilities) {
        return Ok(());
    }
    bail!(
        "environment `{}` supports manual `qol env up` sessions but has no automated flow adapter",
        environment.definition.id
    )
}

fn supported_flow_adapter(capabilities: &std::collections::BTreeMap<String, String>) -> bool {
    capabilities
        .get("flow_adapter")
        .is_some_and(|adapter| adapter == "debian-nocloud")
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
            .current_dir(repo_root()?)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        dev_env::clear_host_session(&mut command);
        isolate_supervisor(&mut command);
        let process_tree = qol_process::own_current_process_tree()
            .context("failed to create supervisor process-tree ownership")?;
        write_lane_owner(launch, pending, "launching", None)?;
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = write_lane_owner(launch, pending, "spawn-failed", None);
                return Err(error)
                    .with_context(|| format!("failed to start flow lane `{}`", pending.run_id));
            }
        };
        if let Err(assign_error) = process_tree.assign(&child) {
            let cleanup = qol_process::terminate_owned(&mut child, SUPERVISOR_SHUTDOWN_GRACE);
            if let Err(cleanup_error) = cleanup {
                bail!(
                    "failed to own flow lane `{}`: {assign_error}; supervisor cleanup also failed: {cleanup_error}",
                    pending.run_id
                );
            }
            let _ = write_lane_owner(launch, pending, "spawn-failed", None);
            return Err(assign_error)
                .with_context(|| format!("failed to own flow lane `{}`", pending.run_id));
        }
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
    let journal = json!({
        "kind": "flow-lane-owner",
        "run_id": pending.run_id,
        "flow_run_id": launch.flow_run_id,
        "flow_report": launch.flow_report_path,
        "owner_pid": launch.owner_pid,
        "supervisor_pid": supervisor_pid,
        "phase": phase,
        "observed_at_unix_ms": unix_millis()?,
    });
    atomic_json(&path, &journal)?;
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

#[cfg(unix)]
fn isolate_supervisor(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
fn isolate_supervisor(_: &mut Command) {}

fn execute_lanes(
    spawner: &mut impl LaneSpawner,
    launch: &LaneLaunch<'_>,
    pending: &[PendingLane],
    concurrent: usize,
    progress: bool,
    cancellation: &qol_process::CancellationToken,
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
            let result = finish_lane(lane, exit);
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
    while let Some(lane) = active.pop() {
        results.push(abort_lane(lane, reason));
    }
}

fn abort_lane(mut lane: ActiveLane, reason: &str) -> LaneResult {
    let shutdown = lane.supervisor.shutdown(reason);
    let (report_status, verdict) = report_outcome(&lane.report_path);
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
    match read_optional_json(report_path) {
        Ok(Some(report)) => {
            let status = report
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if child_cleanup_complete(&report, status).is_ok() && !child_process_alive(&report) {
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

fn finish_lane(lane: ActiveLane, exit: SupervisorExit) -> LaneResult {
    let report = read_json(&lane.report_path);
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
    let report_error = report.is_none().then(|| {
        format!(
            "child report is missing or invalid: {}",
            lane.report_path.display()
        )
    });
    let error = combine_errors(report_error, exit.cleanup.error.clone());
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

fn read_json(path: &Path) -> Option<Value> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn report_outcome(path: &Path) -> (Option<String>, Option<String>) {
    let report = read_json(path);
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

#[allow(clippy::too_many_arguments)]
fn write_aggregate_report(
    run_dir: &Path,
    batch_id: &str,
    options: &FlowOptions,
    environment: &ResolvedEnvironment,
    image_path: &Path,
    memory_mb: u32,
    cpus: u16,
    admission: dev_resources::Admission,
    started_at: u64,
    status: &str,
    error: Option<&str>,
    results: &[LaneResult],
) -> Result<()> {
    let lanes = lane_reports(results);
    let mut report = json!({
        "name": "qol-flow-run",
        "kind": "flow-fanout",
        "run_id": batch_id,
        "started_at_unix_ms": started_at,
        "status": status,
        "owner": {
            "pid": std::process::id(),
            "state": if status == "running" { "running" } else { "released" },
        },
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
                "name": "preflight",
                "status": "pass",
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
            format!("Rerun with `qol flow run {} --env {} --repeat {} --jobs {}`.", options.workflow, options.environment_id, options.repeat, options.jobs),
        ],
    });
    let finished_at = flow_status_is_terminal(status)
        .then(unix_millis)
        .transpose()?;
    apply_report_lifecycle(&mut report, error, finished_at);
    let _lock = lock_flow_run(run_dir)?;
    atomic_json(&run_dir.join("steps/lifecycle.json"), &report["steps"])?;
    atomic_json(&run_dir.join("report.json"), &report)
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
    let content = serde_json::to_string_pretty(value).context("failed to serialize JSON")?;
    atomic_write(path, format!("{content}\n").as_bytes())
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    qol_fs::atomic_write(path, content)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn parse_options(args: &[OsString]) -> Result<FlowOptions> {
    let mut workflow = None;
    let mut environment_id = None;
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
        repeat: repeat.unwrap_or(1),
        jobs: jobs.unwrap_or(1),
        memory_mb,
        cpus,
        force,
    })
}

fn option_value<'a>(args: &'a [OsString], index: usize, option: &str) -> Result<&'a str> {
    let value = args
        .get(index + 1)
        .ok_or_else(|| anyhow!("{option} requires a value"))?;
    let value = utf8(value)?;
    if value.starts_with('-') {
        bail!("{option} requires a value");
    }
    Ok(value)
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

fn unix_millis() -> Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before UNIX_EPOCH")?
        .as_millis();
    u64::try_from(millis).context("timestamp does not fit in u64")
}

fn print_help() {
    print!("{}", help_text());
}

fn help_text() -> &'static str {
    "qol flow commands:\n  qol flow run <workflow> --env <environment> [--repeat N] [--jobs N]\n               [--memory-mb N] [--cpus N] [--force]\n  qol flow runs\n\nFlows run headlessly in disposable environment lanes. --jobs bounds concurrent\nVMs; --repeat controls the total number of independent runs. `qol flow runs`\nreconciles interrupted fan-outs and lists active or incomplete flow reports.\nRun placement and acceleration come from the selected environment definition.\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn argv(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
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
            if let Some(report) = report {
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
            logs_dir: temp.path(),
            case_root: temp.path(),
            flow_run_id: "flow-test",
            flow_report_path: temp.path(),
            owner_pid: std::process::id(),
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
    fn live_owner_keeps_active_report_byte_identical() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = recovery_fixture(&temp, std::process::id(), "running", "planned");
        let before = fs::read(&fixture.report_path).unwrap();

        let summary = reconcile_flow_report_file(&fixture.report_path)
            .unwrap()
            .unwrap();

        assert_eq!(summary.status, "running");
        assert_eq!(fs::read(&fixture.report_path).unwrap(), before);
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

        assert_eq!(summary.status, "abandoned");
        assert_eq!(report["lanes"][0]["passed"], true);
        assert_eq!(report["lanes"][0]["cleanup"]["complete"], true);
        assert_eq!(report["status"], "abandoned");
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
                "teardown": { "removed": [] },
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
    fn parses_defaults_and_complete_options_in_any_order() {
        let defaults = parse_options(&argv(&["leaves-no-trace", "--env", "linux/debian"])).unwrap();
        assert_eq!(defaults.repeat, 1);
        assert_eq!(defaults.jobs, 1);
        assert_eq!(defaults.memory_mb, None);
        assert_eq!(defaults.cpus, None);
        assert!(!defaults.force);

        let complete = parse_options(&argv(&[
            "--jobs",
            "10",
            "--env",
            "linux/debian",
            "--force",
            "leaves-no-trace",
            "--repeat",
            "12",
            "--cpus",
            "2",
            "--memory-mb",
            "1536",
        ]))
        .unwrap();
        assert_eq!(
            complete,
            FlowOptions {
                workflow: "leaves-no-trace".to_string(),
                environment_id: "linux/debian".to_string(),
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
    fn only_declared_debian_nocloud_guests_run_automated_flows() {
        let cases = [
            (Some("debian-nocloud"), true),
            (Some("mint"), false),
            (None, false),
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
            assert_eq!(supported_flow_adapter(&capabilities), expected);
        }
    }
}
