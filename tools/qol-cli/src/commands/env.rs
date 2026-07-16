use crate::commands::dev_env;
use crate::commands::dev_env::resources::{self as dev_resources, Admission};
use crate::commands::emu;
use crate::commands::flow;
use crate::host_facade;
use anyhow::{anyhow, bail, Context, Result};
use qol_dev_env::{ReportStatus, ResolutionState, ResolvedEnvironment};
use qol_dev_orchestrator::{
    ImageImportStart, ImageImportWorkerRequest, RunHandle, RunTicket, WaitState,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const READY_TIMEOUT: Duration = Duration::from_secs(15);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(25);
const TEARDOWN_TIMEOUT: Duration = Duration::from_secs(15);
const UP_USAGE: &str =
    "qol env up <environment> [--count N] [--memory-mb N] [--cpus N] [--windowed] [--force]";
const IMAGE_IMPORT_USAGE: &str =
    "qol env image import <environment> <source> --worktree <absolute-path> [--run-id ID]";

#[derive(Clone, Debug, PartialEq, Eq)]
struct UpArgs {
    environment_id: String,
    run_id: Option<String>,
    count: usize,
    memory_mb: Option<u64>,
    cpus: Option<u16>,
    windowed: bool,
    force: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImageImportArgs {
    environment_id: String,
    source: PathBuf,
    worktree: PathBuf,
    run_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DoctorAction {
    Inspect,
    Repair,
    Clear(dev_resources::LeaseClearSelection),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LanePhase {
    Attempting,
    Launching,
    Spawned,
    Running,
}

impl LanePhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Attempting => "attempting",
            Self::Launching => "launching",
            Self::Spawned => "spawned",
            Self::Running => "running",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Lane {
    run_id: String,
    run_dir: PathBuf,
    report_path: PathBuf,
    phase: LanePhase,
}

impl Lane {
    fn attempted(run_id: String, runs_root: &Path) -> Self {
        let run_dir = runs_root.join(&run_id);
        let report_path = run_dir.join("report.json");
        Self {
            run_id,
            run_dir,
            report_path,
            phase: LanePhase::Attempting,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TeardownResult {
    run_id: String,
    status: &'static str,
    verification: &'static str,
    report_status: Option<String>,
    stop_error: Option<String>,
}

impl TeardownResult {
    fn succeeded(&self) -> bool {
        self.status == "pass"
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CleanupVerification {
    Pending,
    Complete,
    Incomplete(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReportLifecycle {
    status: String,
    cleanup: CleanupVerification,
}

struct Batch<'a> {
    run_id: &'a str,
    run_dir: &'a Path,
    environment: &'a ResolvedEnvironment,
    count: usize,
    memory_mb: u64,
    cpus: u16,
    windowed: bool,
    admission: Admission,
    lanes: &'a [Lane],
    teardown: &'a [TeardownResult],
    status: &'a str,
    error: Option<&'a str>,
    started_at_unix_ms: u64,
    finished_at_unix_ms: Option<u64>,
}

enum BatchExecution {
    Running(Vec<Lane>),
    RolledBack {
        lanes: Vec<Lane>,
        error: String,
        teardown: Vec<TeardownResult>,
        cancelled: bool,
    },
}

trait CancellationSource {
    fn is_cancelled(&self) -> bool;
}

impl CancellationSource for qol_process::CancellationToken {
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }
}

struct EnvironmentCancellation<'a> {
    signals: &'a qol_process::CancellationToken,
    inbox: &'a qol_dev_env::CancellationInbox,
}

impl CancellationSource for EnvironmentCancellation<'_> {
    fn is_cancelled(&self) -> bool {
        self.signals.is_cancelled() || self.inbox.is_requested().unwrap_or(true)
    }
}

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        print_help();
        return Ok(());
    };
    let rest = &args[1..];
    if crate::cli::help_only(rest) {
        print_help();
        return Ok(());
    }
    match command {
        "list" => cmd_list(rest),
        "doctor" => cmd_doctor(rest),
        "up" => cmd_up(rest, verbose),
        "image" => cmd_image(rest, verbose),
        "cancel" => cmd_cancel(rest),
        "runs" => cmd_runs(rest),
        "down" => cmd_down(rest, verbose),
        "shot" => cmd_shot(rest, verbose),
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        other => bail!("unknown env command `{other}`\n\n{}", help_text()),
    }
}

fn cmd_image(args: &[OsString], verbose: bool) -> Result<()> {
    let Some(command) = args.first().and_then(|argument| argument.to_str()) else {
        bail!("usage: {IMAGE_IMPORT_USAGE}");
    };
    match command {
        "import" => cmd_image_import(&args[1..], verbose),
        "help" | "-h" | "--help" => {
            println!("{IMAGE_IMPORT_USAGE}");
            Ok(())
        }
        other => bail!("unknown env image command `{other}`\nusage: {IMAGE_IMPORT_USAGE}"),
    }
}

fn cmd_image_import(args: &[OsString], verbose: bool) -> Result<()> {
    let parsed = parse_image_import_args(args)?;
    let run_id = match parsed.run_id {
        Some(run_id) => run_id,
        None => emu::new_run_id("image-import")?,
    };
    let start = ImageImportStart {
        environment_id: parsed.environment_id,
        source: parsed.source,
        worktree: parsed.worktree,
        run_id,
    };
    let executable = std::env::current_exe().context("failed to resolve the qol executable")?;
    let cancellation = qol_process::CancellationToken::install()
        .context("failed to install image-import cancellation handlers")?;
    let mut handle = start_typed_image_import(&executable, start, verbose)?;
    println!("Image import run: {}", handle.ticket().run_id);
    println!(
        "Worker log: {}",
        handle.ticket().worker_log_path()?.display()
    );
    let receipt = wait_for_typed_image_import(&mut handle, &cancellation)?;
    println!("Verified image: {}", receipt.image_path.display());
    println!("Verification report: {}", receipt.report_path.display());
    Ok(())
}

pub(crate) fn run_image_import_worker(args: &[OsString]) -> Result<()> {
    if !args.is_empty() {
        bail!("internal image-import worker accepts typed standard input only");
    }
    qol_dev_env::require_host_session_cleared()
        .context("internal image-import worker refused host session access")?;
    let request = qol_dev_orchestrator::read_image_import_worker_request(std::io::stdin().lock())?;
    let plan = emu::image_import::plan_image_import(
        emu::image_import::ImageImportRequest {
            environment_id: request.start.environment_id.clone(),
            source: request.start.source.clone(),
            run_id: Some(request.start.run_id.clone()),
            worktree: request.start.worktree.clone(),
        },
        request.verbose,
    )?;
    let expected_ticket = request.start.ticket(&request.image_root)?;
    if plan.report_path != expected_ticket.report_path {
        bail!("image-import configuration changed after the worker ticket was issued");
    }
    if plan.fingerprint()? != request.plan_fingerprint {
        bail!("image-import plan changed before the typed worker started; retry the import");
    }
    emu::image_import::execute_image_import(plan, request.verbose)?;
    Ok(())
}

pub(crate) fn start_typed_image_import(
    executable: &Path,
    start: ImageImportStart,
    verbose: bool,
) -> Result<RunHandle> {
    start.validate()?;
    let plan = emu::image_import::plan_image_import(
        emu::image_import::ImageImportRequest {
            environment_id: start.environment_id.clone(),
            source: start.source.clone(),
            run_id: Some(start.run_id.clone()),
            worktree: start.worktree.clone(),
        },
        verbose,
    )?;
    let canonical_start = ImageImportStart {
        environment_id: start.environment_id,
        source: start.source.canonicalize().with_context(|| {
            format!("failed to resolve image source {}", start.source.display())
        })?,
        worktree: start.worktree.canonicalize().with_context(|| {
            format!(
                "failed to resolve image-import worktree {}",
                start.worktree.display()
            )
        })?,
        run_id: start.run_id,
    };
    let ticket = canonical_start.ticket(&plan.image_root)?;
    if ticket.report_path != plan.report_path {
        bail!("image-import plan produced a mismatched report path");
    }
    let plan_fingerprint = plan.fingerprint()?;
    qol_dev_orchestrator::start_image_import_worker(
        executable,
        ImageImportWorkerRequest {
            start: canonical_start,
            image_root: plan.image_root,
            plan_fingerprint,
            verbose,
        },
        ticket,
    )
}

fn wait_for_typed_image_import(
    handle: &mut RunHandle,
    cancellation: &qol_process::CancellationToken,
) -> Result<emu::image_import::ImageImportReceipt> {
    let mut cancellation_requested = false;
    loop {
        if cancellation.is_cancelled() && !cancellation_requested {
            handle.cancel()?;
            cancellation_requested = true;
        }
        match handle.poll()? {
            WaitState::Starting | WaitState::Running(_) => {}
            WaitState::Terminal {
                report,
                worker_success,
            } => return image_import_receipt(handle.ticket(), report, worker_success),
            WaitState::Failed {
                report,
                worker_exit,
            } => {
                let status = report
                    .as_ref()
                    .map(|report| report.status.as_str())
                    .unwrap_or("missing");
                bail!(
                    "image-import worker exited {worker_exit} without terminal cleanup proof; report status: {status}; evidence: {}; worker log: {}",
                    handle.ticket().report_path.display(),
                    handle.ticket().worker_log_path()?.display()
                );
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn image_import_receipt(
    ticket: &RunTicket,
    summary: qol_dev_env::RunSummary,
    worker_success: bool,
) -> Result<emu::image_import::ImageImportReceipt> {
    if summary.status != ReportStatus::Pass {
        let error = summary
            .error
            .as_deref()
            .unwrap_or("image verification did not pass");
        bail!(
            "image import ended `{}`: {error}; report: {}; worker log: {}",
            summary.status.as_str(),
            summary.report_path.display(),
            ticket.worker_log_path()?.display()
        );
    }
    if !worker_success {
        bail!(
            "image-import worker exited unsuccessfully after publishing a passing report: {}; worker log: {}",
            summary.report_path.display(),
            ticket.worker_log_path()?.display()
        );
    }
    let report = ticket
        .read()?
        .context("passing image-import report disappeared")?;
    let promotion = report
        .document()
        .pointer("/workflow/promotion")
        .context("passing image-import report has no promotion receipt")?;
    if promotion.get("status").and_then(Value::as_str) != Some("published") {
        bail!("passing image-import report has no published image receipt");
    }
    let image_path = promotion
        .get("image_path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .context("passing image-import report has no verified image path")?;
    if !image_path.is_absolute() {
        bail!("passing image-import report has a non-absolute image path");
    }
    Ok(emu::image_import::ImageImportReceipt {
        run_id: ticket.run_id.clone(),
        image_path,
        report_path: ticket.report_path.clone(),
    })
}

fn cmd_list(args: &[OsString]) -> Result<()> {
    require_no_args(args, "qol env list")?;
    let environments = dev_env::discover()?;
    if environments.is_empty() {
        println!("No development environments are defined in flows/envs.");
        return Ok(());
    }
    println!(
        "{:<30} {:<12} {:<8} IMAGE",
        "ENVIRONMENT", "STATE", "BACKEND"
    );
    for environment in environments {
        println!(
            "{:<30} {:<12} {:<8} {}",
            environment.definition.id,
            environment.state.as_str(),
            environment.definition.backend,
            display_optional_path(environment.image_path.as_deref())
        );
    }
    Ok(())
}

fn cmd_cancel(args: &[OsString]) -> Result<()> {
    let run_id = required_selector(args, "qol env cancel <batch-run-id>")?;
    let path = qol_dev_env::request_cancellation(&run_id)?;
    println!("Cancellation requested for {run_id}: {}", path.display());
    Ok(())
}

fn cmd_doctor(args: &[OsString]) -> Result<()> {
    let action = parse_doctor_action(args)?;
    match action {
        DoctorAction::Inspect => {}
        DoctorAction::Repair => {
            reconcile_for_admission()?;
            flow::reconcile_all()?;
            let reserved = dev_env::reconcile_resources()?;
            println!(
                "Repair complete: {} lane(s), {} MiB, {} CPU(s) remain reserved.",
                reserved.lanes, reserved.memory_mb, reserved.cpus
            );
        }
        DoctorAction::Clear(selection) => {
            let outcome = dev_resources::clear_leases(selection)?;
            if outcome.removed.is_empty() {
                println!("No matching resource leases were recorded.");
            } else {
                println!("Cleared resource leases: {}", outcome.removed.join(", "));
            }
            if let Some(path) = outcome.backup_path {
                println!("Backup: {}", path.display());
            }
        }
    }
    let environments = dev_env::discover()?;
    let config = dev_env::config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "unavailable".to_string());
    println!("Config: {config}");
    println!(
        "Available memory: {}",
        display_optional_memory(host_facade::available_memory_mb())
    );
    let inspection = dev_resources::inspect().with_context(|| {
        "resource lease inspection failed; use `qol env doctor --lease-clear --all` only after verifying no sandbox processes are live"
    })?;
    println!(
        "Resource leases: {} record(s), {} lane(s), {} MiB, {} CPU(s)",
        inspection.leases.len(),
        inspection.reserved.lanes,
        inspection.reserved.memory_mb,
        inspection.reserved.cpus
    );
    for lease in inspection.leases {
        println!(
            "  {}: owner {}, {} lane(s), report {}",
            lease.lease_id,
            lease.owner_pid,
            lease.resources.lanes,
            lease.report_path.display()
        );
    }
    for diagnostic in inspection.diagnostics {
        println!("  warning: {diagnostic}");
    }
    if environments.is_empty() {
        println!("Definitions: none");
        return Ok(());
    }
    for environment in environments {
        println!(
            "{}: {} ({})",
            environment.definition.id,
            environment.state.as_str(),
            environment.definition.source.display()
        );
        println!(
            "  image: {}",
            display_optional_path(environment.image_path.as_deref())
        );
        println!(
            "  launch: {} MiB, {} CPU(s), {}",
            environment.definition.boot.memory_mb,
            environment.definition.boot.cpus,
            environment.definition.boot.display
        );
        for message in environment.messages {
            println!("  {message}");
        }
    }
    Ok(())
}

fn parse_doctor_action(args: &[OsString]) -> Result<DoctorAction> {
    if args.is_empty() {
        return Ok(DoctorAction::Inspect);
    }
    if args.len() == 1
        && args[0]
            .to_str()
            .is_some_and(|value| matches!(value, "--repair" | "--fix"))
    {
        return Ok(DoctorAction::Repair);
    }
    if args.len() == 2 && args[0].to_str() == Some("--lease-clear") {
        let target = args[1].to_str().context("lease id must be valid UTF-8")?;
        let selection = if target == "--all" {
            dev_resources::LeaseClearSelection::All
        } else {
            dev_resources::LeaseClearSelection::One(target.to_string())
        };
        return Ok(DoctorAction::Clear(selection));
    }
    bail!("usage: qol env doctor [--repair|--fix|--lease-clear <run-id|--all>]")
}

fn cmd_up(args: &[OsString], verbose: bool) -> Result<()> {
    let parsed = parse_up_args(args)?;
    let signal_cancellation = qol_process::CancellationToken::install()
        .context("failed to install environment cancellation handlers")?;
    reconcile_for_admission()?;
    flow::reconcile_all()?;
    dev_env::reconcile_resources()?;
    let environment = dev_env::find(&parsed.environment_id)?
        .ok_or_else(|| unknown_environment(&parsed.environment_id))?;
    ensure_ready(&environment)?;
    let image_path = environment
        .image_path
        .as_deref()
        .context("ready environment has no image path")?;
    let run_root = environment
        .run_root
        .as_deref()
        .context("development environment run root is not configured")?;
    let memory_mb = parsed
        .memory_mb
        .unwrap_or(environment.definition.boot.memory_mb);
    let cpus = parsed.cpus.unwrap_or(environment.definition.boot.cpus);
    let resources = dev_resources::profile(memory_mb, u64::from(cpus))?;
    let memory_mb = u64::from(resources.memory_mb);
    let cpus = resources.cpus;
    let concurrent_lanes = u64::try_from(parsed.count).context("lane count exceeds u64")?;
    let batch_id = match parsed.run_id {
        Some(run_id) => run_id,
        None => emu::new_run_id(&format!("{}-batch", environment.definition.id))?,
    };
    let cancellation_inbox = qol_dev_env::CancellationInbox::for_run(&batch_id)?;
    let cancellation = EnvironmentCancellation {
        signals: &signal_cancellation,
        inbox: &cancellation_inbox,
    };
    if cancellation.is_cancelled() {
        bail!("environment launch cancelled before admission");
    }
    let batch_dir = prepare_batch_dir(run_root, &batch_id)?;
    let case_root = run_root.join("cases");
    let setup = (|| -> Result<_> {
        let started_at_unix_ms = qol_dev_env::unix_millis()?;
        let mut planned_lanes = Vec::with_capacity(parsed.count);
        for _ in 0..parsed.count {
            planned_lanes.push(Lane::attempted(
                emu::new_run_id(&environment.definition.id)?,
                &case_root,
            ));
        }
        Ok((started_at_unix_ms, planned_lanes))
    })();
    let (started_at_unix_ms, planned_lanes) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            let cleanup = remove_unpublished_batch_dir(&batch_dir).err();
            return Err(combine_unpublished_errors(error, cleanup, None));
        }
    };
    let batch_report_path = run_root.join(&batch_id).join("report.json");
    let (admission, resource_lease) = match dev_resources::reserve(
        &batch_id,
        &batch_report_path,
        dev_resources::AdmissionRequest {
            concurrent_lanes,
            profile: resources,
            recommended_size_gb: environment.definition.image.recommended_size_gb,
            capacity: dev_env::host_capacity(run_root),
            force: parsed.force,
        },
    ) {
        Ok(reservation) => reservation,
        Err(error) => {
            let cleanup = remove_unpublished_batch_dir(&batch_dir).err();
            return Err(combine_unpublished_errors(error, cleanup, None));
        }
    };
    let parent_lease = match resource_lease.child_claim() {
        Ok(parent_lease) => parent_lease,
        Err(error) => {
            return Err(rollback_unpublished_batch(
                resource_lease,
                &batch_dir,
                &batch_id,
                error,
            ))
        }
    };
    let initial_report = write_batch_files(&Batch {
        run_id: &batch_id,
        run_dir: &batch_dir,
        environment: &environment,
        count: parsed.count,
        memory_mb,
        cpus,
        windowed: parsed.windowed,
        admission,
        lanes: &planned_lanes,
        teardown: &[],
        status: "starting",
        error: None,
        started_at_unix_ms,
        finished_at_unix_ms: None,
    });
    if let Err(error) = initial_report {
        return Err(rollback_unpublished_batch(
            resource_lease,
            &batch_dir,
            &batch_id,
            error.context("failed to publish initial environment ownership"),
        ));
    }
    let execution = execute_owned_batch(
        planned_lanes,
        &cancellation,
        |lane| {
            if verbose {
                println!("Starting lane: {}", lane.run_id);
            }
            let child_args = emu_up_args(EmuUpRequest {
                image_path,
                environment: &environment,
                parent_lease: &parent_lease,
                run_id: &lane.run_id,
                memory_mb,
                cpus,
                windowed: parsed.windowed,
                case_root: &case_root,
            })?;
            if let Err(error) = spawn_lane(&child_args, parsed.windowed) {
                lane.phase = LanePhase::Attempting;
                return Err(error);
            }
            lane.phase = LanePhase::Spawned;
            wait_until_running(lane, &cancellation)?;
            lane.phase = LanePhase::Running;
            Ok(())
        },
        |lanes, status| {
            write_batch_files(&Batch {
                run_id: &batch_id,
                run_dir: &batch_dir,
                environment: &environment,
                count: parsed.count,
                memory_mb,
                cpus,
                windowed: parsed.windowed,
                admission,
                lanes,
                teardown: &[],
                status,
                error: None,
                started_at_unix_ms,
                finished_at_unix_ms: None,
            })
        },
        |lanes| teardown_lanes(lanes, verbose),
    );
    let lanes = match execution {
        BatchExecution::Running(lanes) => {
            resource_lease.retain();
            lanes
        }
        BatchExecution::RolledBack {
            lanes,
            error,
            teardown,
            cancelled,
        } => {
            let cleanup_complete = teardown.iter().all(TeardownResult::succeeded);
            let status = rollback_status(cancelled, cleanup_complete);
            let (finished_at_unix_ms, timestamp_error) =
                match cleanup_complete.then(qol_dev_env::unix_millis).transpose() {
                    Ok(timestamp) => (timestamp, None),
                    Err(error) => (None, Some(error)),
                };
            let report_result = write_batch_files(&Batch {
                run_id: &batch_id,
                run_dir: &batch_dir,
                environment: &environment,
                count: parsed.count,
                memory_mb,
                cpus,
                windowed: parsed.windowed,
                admission,
                lanes: &lanes,
                teardown: &teardown,
                status,
                error: Some(&error),
                started_at_unix_ms,
                finished_at_unix_ms,
            });
            let report_written = report_result.is_ok();
            let recording_error = combine_recording_errors(timestamp_error, report_result.err());
            let lease_error = if cleanup_complete && report_written {
                resource_lease
                    .release()
                    .context("failed to release the environment resource lease")
                    .err()
            } else {
                resource_lease.retain();
                None
            };
            let recording_error = combine_optional_errors(recording_error, lease_error);
            return Err(batch_failure(error, &teardown, recording_error));
        }
    };
    println!(
        "Started {} lane(s) for {}.",
        lanes.len(),
        parsed.environment_id
    );
    for lane in &lanes {
        println!("  {}", lane.run_id);
    }
    println!("Report: {}", batch_dir.join("report.json").display());
    Ok(())
}

fn rollback_status(cancelled: bool, cleanup_complete: bool) -> &'static str {
    match (cancelled, cleanup_complete) {
        (true, true) => "cancelled",
        (true, false) => "cancellation-cleanup-incomplete",
        (false, true) => "failed",
        (false, false) => "rollback-incomplete",
    }
}

fn combine_recording_errors(
    timestamp_error: Option<anyhow::Error>,
    report_error: Option<anyhow::Error>,
) -> Option<anyhow::Error> {
    match (timestamp_error, report_error) {
        (Some(timestamp), Some(report)) => Some(anyhow!(
            "failed to timestamp rollback: {timestamp:#}; failed to record rollback: {report:#}"
        )),
        (Some(timestamp), None) => Some(anyhow!("failed to timestamp rollback: {timestamp:#}")),
        (None, Some(report)) => Some(report),
        (None, None) => None,
    }
}

fn combine_optional_errors(
    first: Option<anyhow::Error>,
    second: Option<anyhow::Error>,
) -> Option<anyhow::Error> {
    match (first, second) {
        (Some(first), Some(second)) => Some(anyhow!("{first:#}; {second:#}")),
        (Some(first), None) => Some(first),
        (None, Some(second)) => Some(second),
        (None, None) => None,
    }
}

fn execute_owned_batch(
    mut lanes: Vec<Lane>,
    cancellation: &impl CancellationSource,
    mut launch_lane: impl FnMut(&mut Lane) -> Result<()>,
    mut persist: impl FnMut(&[Lane], &str) -> Result<()>,
    mut rollback: impl FnMut(&[Lane]) -> Vec<TeardownResult>,
) -> BatchExecution {
    let count = lanes.len();
    for index in 0..count {
        if cancellation.is_cancelled() {
            return rolled_back(
                lanes,
                "environment launch cancelled".to_string(),
                true,
                &mut rollback,
            );
        }
        lanes[index].phase = LanePhase::Launching;
        if let Err(error) = persist(&lanes, "starting") {
            lanes[index].phase = LanePhase::Attempting;
            return rolled_back(
                lanes,
                format!(
                    "lane {}/{} ownership could not be recorded: {error:#}",
                    index + 1,
                    count
                ),
                cancellation.is_cancelled(),
                &mut rollback,
            );
        }
        if cancellation.is_cancelled() {
            lanes[index].phase = LanePhase::Attempting;
            return rolled_back(
                lanes,
                "environment launch cancelled".to_string(),
                true,
                &mut rollback,
            );
        }
        if let Err(error) = launch_lane(&mut lanes[index]) {
            return rolled_back(
                lanes,
                format!("lane {}/{} failed: {error:#}", index + 1, count),
                cancellation.is_cancelled(),
                &mut rollback,
            );
        }
        if cancellation.is_cancelled() {
            return rolled_back(
                lanes,
                "environment launch cancelled".to_string(),
                true,
                &mut rollback,
            );
        }
        if let Err(error) = persist(&lanes, "starting") {
            return rolled_back(
                lanes,
                format!(
                    "lane {}/{} running state could not be recorded: {error:#}",
                    index + 1,
                    count
                ),
                cancellation.is_cancelled(),
                &mut rollback,
            );
        }
    }
    if cancellation.is_cancelled() {
        return rolled_back(
            lanes,
            "environment launch cancelled".to_string(),
            true,
            &mut rollback,
        );
    }
    if let Err(error) = persist(&lanes, "running") {
        return rolled_back(
            lanes,
            format!("running batch ownership could not be committed: {error:#}"),
            cancellation.is_cancelled(),
            &mut rollback,
        );
    }
    if cancellation.is_cancelled() {
        return rolled_back(
            lanes,
            "environment launch cancelled".to_string(),
            true,
            &mut rollback,
        );
    }
    BatchExecution::Running(lanes)
}

fn rolled_back(
    lanes: Vec<Lane>,
    error: String,
    cancelled: bool,
    rollback: &mut impl FnMut(&[Lane]) -> Vec<TeardownResult>,
) -> BatchExecution {
    let teardown = rollback(&lanes);
    BatchExecution::RolledBack {
        lanes,
        error,
        teardown,
        cancelled,
    }
}

fn batch_failure(
    error: String,
    teardown: &[TeardownResult],
    report_error: Option<anyhow::Error>,
) -> anyhow::Error {
    let failures = teardown
        .iter()
        .filter(|result| !result.succeeded())
        .map(|result| {
            format!(
                "{}: {}",
                result.run_id,
                result
                    .stop_error
                    .as_deref()
                    .unwrap_or("verification failed")
            )
        })
        .collect::<Vec<_>>();
    let mut details = vec![error];
    if !failures.is_empty() {
        details.push(format!("rollback failures:\n{}", failures.join("\n")));
    }
    if let Some(report_error) = report_error {
        details.push(format!("failed to record rollback: {report_error:#}"));
    }
    anyhow!(details.join("\n"))
}

pub(crate) fn reconcile_for_admission() -> Result<()> {
    let roots = environment_case_roots()?;
    let _ = emu::live_runs_in_roots(&roots);
    reconcile_batch_reports(&[], false)
}

fn cmd_runs(args: &[OsString]) -> Result<()> {
    require_no_args(args, "qol env runs")?;
    let roots = environment_case_roots()?;
    let runs = emu::live_runs_in_roots(&roots);
    reconcile_batch_reports(&[], false)?;
    flow::reconcile_all()?;
    dev_env::reconcile_resources()?;
    if runs.is_empty() {
        println!("No development environments are running.");
        return Ok(());
    }
    println!("{:<58} {:<30} {:<6} REPORT", "RUN ID", "ENVIRONMENT", "QMP");
    for run in runs {
        println!(
            "{:<58} {:<30} {:<6} {}",
            run.run_id,
            run.environment_id,
            run.qmp_port,
            run.run_dir.join("report.json").display()
        );
    }
    Ok(())
}

fn cmd_down(args: &[OsString], verbose: bool) -> Result<()> {
    let selector = required_selector(args, "qol env down <run-id|environment|--all>")?;
    let roots = environment_case_roots()?;
    let live_runs = emu::live_runs_in_roots(&roots);
    let runs = match select_runs_to_stop(&live_runs, &selector) {
        Ok(runs) => runs,
        Err(selection_error) => {
            let reconciliation = reconcile_batch_reports(&[], false)
                .and_then(|()| dev_env::reconcile_resources().map(|_| ()));
            if let Err(reconciliation_error) = reconciliation {
                bail!("{selection_error:#}\nbatch reconciliation: {reconciliation_error:#}");
            }
            return Err(selection_error);
        }
    };
    if runs.is_empty() {
        reconcile_batch_reports(&[], false)?;
        dev_env::reconcile_resources()?;
        println!("No development environments are running.");
        return Ok(());
    }
    let lanes = runs
        .into_iter()
        .map(|run| Lane {
            run_id: run.run_id,
            run_dir: run.run_dir.clone(),
            report_path: run.run_dir.join("report.json"),
            phase: LanePhase::Running,
        })
        .collect::<Vec<_>>();
    let teardown = teardown_lanes(&lanes, verbose);
    let reconciliation = reconcile_batch_reports(&teardown, true)
        .and_then(|()| dev_env::reconcile_resources().map(|_| ()));
    let failures = teardown
        .iter()
        .filter(|result| !result.succeeded())
        .map(|result| {
            format!(
                "{}: {}",
                result.run_id,
                result
                    .stop_error
                    .as_deref()
                    .unwrap_or("verification failed")
            )
        })
        .collect::<Vec<_>>();
    if failures.is_empty() && reconciliation.is_ok() {
        return Ok(());
    }
    let mut details = failures;
    if let Err(error) = reconciliation {
        details.push(format!("batch reconciliation: {error:#}"));
    }
    bail!(
        "failed to stop development environments:\n{}",
        details.join("\n")
    )
}

fn select_runs_to_stop(runs: &[emu::LiveRun], selector: &str) -> Result<Vec<emu::LiveRun>> {
    if selector == "--all" {
        return Ok(runs.to_vec());
    }
    let exact = runs
        .iter()
        .filter(|run| run.run_id == selector)
        .cloned()
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return Ok(exact);
    }
    if exact.len() > 1 {
        let paths = exact
            .iter()
            .map(|run| run.run_dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        bail!("duplicate running environment identity `{selector}` in: {paths}");
    }
    let matches = runs
        .iter()
        .filter(|run| run.environment_id == selector)
        .cloned()
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        return Ok(matches);
    }
    if matches.is_empty() {
        bail!("no running environment matches `{selector}`");
    }
    let ids = matches
        .iter()
        .map(|run| run.run_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    bail!("multiple running environments match `{selector}`: {ids}")
}

fn cmd_shot(args: &[OsString], verbose: bool) -> Result<()> {
    let selector = required_selector(args, "qol env shot <run-id|environment>")?;
    if selector == "--all" {
        bail!("qol env shot requires one run ID or unambiguous environment ID");
    }
    forward_emu("shot", &selector, verbose, &environment_case_roots()?)
}

fn environment_case_roots() -> Result<Vec<PathBuf>> {
    Ok(dev_env::discover()?
        .into_iter()
        .filter_map(|environment| environment.run_root)
        .map(|run_root| run_root.join("cases"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn parse_up_args(args: &[OsString]) -> Result<UpArgs> {
    let mut environment_id = None;
    let mut run_id = None;
    let mut count = None;
    let mut memory_mb = None;
    let mut cpus = None;
    let mut windowed = false;
    let mut force = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("--count") => {
                set_once(
                    &mut count,
                    parse_usize(next_value(&mut iter, "--count")?, "--count")?,
                    "--count",
                )?;
            }
            Some("--memory-mb") => {
                set_once(
                    &mut memory_mb,
                    parse_u64(next_value(&mut iter, "--memory-mb")?, "--memory-mb")?,
                    "--memory-mb",
                )?;
            }
            Some("--cpus") => {
                set_once(
                    &mut cpus,
                    parse_u16(next_value(&mut iter, "--cpus")?, "--cpus")?,
                    "--cpus",
                )?;
            }
            Some("--run-id") => {
                let value = next_value(&mut iter, "--run-id")?;
                dev_resources::ParentLeaseClaim::parse(value)?;
                set_once(&mut run_id, value.to_string(), "--run-id")?;
            }
            Some("--windowed") => set_flag(&mut windowed, "--windowed")?,
            Some("--force") => set_flag(&mut force, "--force")?,
            Some(value) if value.starts_with('-') => {
                bail!("unknown option `{value}`\nusage: {UP_USAGE}")
            }
            Some(value) => set_once(&mut environment_id, value.to_string(), "environment")?,
            None => bail!("arguments must be valid UTF-8\nusage: {UP_USAGE}"),
        }
    }
    let environment_id = environment_id.ok_or_else(|| anyhow!("usage: {UP_USAGE}"))?;
    let count = count.unwrap_or(1);
    let maximum = usize::try_from(dev_resources::MAX_CONCURRENT_LANES).unwrap_or(usize::MAX);
    if count == 0 || count > maximum {
        bail!("--count must be between 1 and {maximum}");
    }
    Ok(UpArgs {
        environment_id,
        run_id,
        count,
        memory_mb,
        cpus,
        windowed,
        force,
    })
}

fn parse_image_import_args(args: &[OsString]) -> Result<ImageImportArgs> {
    let mut environment_id = None;
    let mut source = None;
    let mut worktree = None;
    let mut run_id = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].to_str() {
            Some("--worktree") => {
                if worktree.is_some() {
                    bail!("--worktree was provided more than once");
                }
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.to_string_lossy().starts_with('-'))
                    .context("--worktree needs a value")?;
                let path = PathBuf::from(value);
                if !path.is_absolute() {
                    bail!("--worktree requires an absolute path");
                }
                worktree = Some(path);
                index += 2;
            }
            Some("--run-id") => {
                if run_id.is_some() {
                    bail!("--run-id was provided more than once");
                }
                let value = args
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .filter(|value| !value.starts_with('-'))
                    .context("--run-id needs a value")?;
                qol_dev_env::validate_run_id(value)?;
                run_id = Some(value.to_string());
                index += 2;
            }
            Some(option) if option.starts_with('-') => {
                bail!("unknown image-import option `{option}`\nusage: {IMAGE_IMPORT_USAGE}")
            }
            Some(value) if environment_id.is_none() => {
                if value.is_empty() {
                    bail!("usage: {IMAGE_IMPORT_USAGE}");
                }
                environment_id = Some(value.to_string());
                index += 1;
            }
            Some(_) | None if source.is_none() => {
                let path = PathBuf::from(&args[index]);
                if !path.is_absolute() {
                    bail!("image source requires an absolute path");
                }
                source = Some(path);
                index += 1;
            }
            Some(_) | None => bail!("usage: {IMAGE_IMPORT_USAGE}"),
        }
    }
    Ok(ImageImportArgs {
        environment_id: environment_id.ok_or_else(|| anyhow!("usage: {IMAGE_IMPORT_USAGE}"))?,
        source: source.ok_or_else(|| anyhow!("usage: {IMAGE_IMPORT_USAGE}"))?,
        worktree: worktree.ok_or_else(|| anyhow!("--worktree is required"))?,
        run_id,
    })
}

fn next_value<'a>(iter: &mut impl Iterator<Item = &'a OsString>, option: &str) -> Result<&'a str> {
    iter.next()
        .and_then(|value| value.to_str())
        .with_context(|| format!("{option} needs a value"))
}

fn parse_usize(value: &str, option: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .with_context(|| format!("{option} must be a positive integer"))
}

fn parse_u64(value: &str, option: &str) -> Result<u64> {
    let value = value
        .parse::<u64>()
        .with_context(|| format!("{option} must be a positive integer"))?;
    if value > 0 {
        return Ok(value);
    }
    bail!("{option} must be a positive integer")
}

fn parse_u16(value: &str, option: &str) -> Result<u16> {
    let value = value
        .parse::<u16>()
        .with_context(|| format!("{option} must be a positive integer"))?;
    if value > 0 {
        return Ok(value);
    }
    bail!("{option} must be a positive integer")
}

fn set_once<T>(slot: &mut Option<T>, value: T, label: &str) -> Result<()> {
    if slot.is_none() {
        *slot = Some(value);
        return Ok(());
    }
    bail!("{label} was provided more than once")
}

fn set_flag(flag: &mut bool, label: &str) -> Result<()> {
    if !*flag {
        *flag = true;
        return Ok(());
    }
    bail!("{label} was provided more than once")
}

fn ensure_ready(environment: &ResolvedEnvironment) -> Result<()> {
    if environment.state == ResolutionState::Ready {
        return Ok(());
    }
    let detail = environment.messages.join("; ");
    bail!(
        "environment `{}` is {}: {detail}",
        environment.definition.id,
        environment.state.as_str()
    )
}

fn unknown_environment(id: &str) -> anyhow::Error {
    anyhow!("unknown development environment `{id}`; inspect available environments with `qol env list`")
}

fn prepare_batch_dir(run_root: &Path, run_id: &str) -> Result<PathBuf> {
    fs::create_dir_all(run_root)
        .with_context(|| format!("failed to create {}", run_root.display()))?;
    let run_dir = run_root.join(run_id);
    fs::create_dir(&run_dir).with_context(|| format!("failed to create {}", run_dir.display()))?;
    let prepared = (|| -> Result<()> {
        fs::create_dir(run_dir.join("artifacts"))
            .with_context(|| format!("failed to create artifacts for {run_id}"))?;
        fs::create_dir(run_dir.join("logs"))
            .with_context(|| format!("failed to create logs for {run_id}"))?;
        fs::create_dir(run_dir.join("steps"))
            .with_context(|| format!("failed to create steps for {run_id}"))
    })();
    match prepared {
        Ok(()) => Ok(run_dir),
        Err(error) => {
            let cleanup = remove_unpublished_batch_dir(&run_dir).err();
            Err(combine_unpublished_errors(error, cleanup, None))
        }
    }
}

fn rollback_unpublished_batch(
    resource_lease: dev_resources::ResourceLease,
    run_dir: &Path,
    batch_id: &str,
    error: anyhow::Error,
) -> anyhow::Error {
    let cleanup = remove_unpublished_batch_dir(run_dir).err();
    if let Some(cleanup) = cleanup {
        let mut failures = vec![
            format!("{error:#}"),
            format!("batch directory cleanup failed: {cleanup:#}"),
        ];
        if let Err(error) = write_unpublished_batch_failure(run_dir, batch_id, &failures.join("; "))
        {
            failures.push(format!(
                "failed to persist unresolved cleanup evidence: {error:#}"
            ));
        }
        resource_lease.retain();
        return anyhow!(failures.join("; "));
    }
    let rollback = resource_lease.rollback_unpublished().err();
    combine_unpublished_errors(error, None, rollback)
}

fn write_unpublished_batch_failure(run_dir: &Path, batch_id: &str, error: &str) -> Result<()> {
    let report_path = run_dir.join("report.json");
    let report = json!({
        "name": "qol-env-setup-failure",
        "kind": "environment",
        "run_id": batch_id,
        "status": "cleanup-incomplete",
        "owner": dev_env::run_owner("environment-setup", "released"),
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

fn remove_unpublished_batch_dir(run_dir: &Path) -> Result<()> {
    match fs::remove_dir_all(run_dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to remove unpublished environment batch {}",
                run_dir.display()
            )
        }),
    }
}

fn combine_unpublished_errors(
    error: anyhow::Error,
    cleanup: Option<anyhow::Error>,
    rollback: Option<anyhow::Error>,
) -> anyhow::Error {
    let mut failures = vec![format!("{error:#}")];
    if let Some(cleanup) = cleanup {
        failures.push(format!("batch directory cleanup failed: {cleanup:#}"));
    }
    if let Some(rollback) = rollback {
        failures.push(format!(
            "resource reservation rollback failed: {rollback:#}"
        ));
    }
    anyhow!(failures.join("; "))
}

struct EmuUpRequest<'a> {
    image_path: &'a Path,
    environment: &'a ResolvedEnvironment,
    parent_lease: &'a dev_resources::ParentLeaseClaim,
    run_id: &'a str,
    memory_mb: u64,
    cpus: u16,
    windowed: bool,
    case_root: &'a Path,
}

fn emu_up_args(request: EmuUpRequest<'_>) -> Result<Vec<OsString>> {
    let EmuUpRequest {
        image_path,
        environment,
        parent_lease,
        run_id,
        memory_mb,
        cpus,
        windowed,
        case_root,
    } = request;
    emu::child_launch_args(emu::ChildLaunch {
        operation: emu::ChildOperation::Up,
        target: image_path,
        environment_id: &environment.definition.id,
        run_id,
        parent_lease,
        guest_adapter: None,
        guest_image_revision: None,
        payload_manifest: None,
        payload_image: None,
        run_root: Some(case_root),
        image_kind: Some(environment.definition.image.kind.as_str()),
        display: if windowed {
            emu::DisplayMode::Host
        } else {
            emu::DisplayMode::None
        },
        offline: false,
        resources: dev_resources::profile(memory_mb, u64::from(cpus))?,
        acceleration: environment
            .definition
            .capabilities
            .get("acceleration")
            .map(String::as_str),
        arch: environment.definition.image.arch.as_deref(),
        firmware: environment.definition.image.firmware.as_deref(),
    })
}

fn spawn_lane(args: &[OsString], windowed: bool) -> Result<()> {
    let executable =
        std::env::current_exe().context("failed to locate the current qol executable")?;
    let mut command = Command::new(&executable);
    command.args(args);
    if !windowed {
        dev_env::clear_host_session(&mut command);
    }
    qol_process::spawn_detached(&mut command)
        .with_context(|| format!("failed to start {}", executable.display()))
}

fn wait_until_running(lane: &Lane, cancellation: &impl CancellationSource) -> Result<()> {
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if cancellation.is_cancelled() {
            bail!("environment launch cancelled");
        }
        if let Some(state) = read_report_state(&lane.report_path)? {
            if state == "running" {
                return Ok(());
            }
            if matches!(state.as_str(), "failed" | "skipped" | "pass") {
                bail!("emu `{}` reported status `{state}`", lane.run_id);
            }
        }
        thread::sleep(READY_POLL_INTERVAL);
    }
    bail!(
        "timed out after {} seconds waiting for emu `{run_id}` to report running",
        READY_TIMEOUT.as_secs(),
        run_id = lane.run_id,
    )
}

fn read_report_state(path: &Path) -> Result<Option<String>> {
    Ok(read_report(path)?
        .as_ref()
        .and_then(|report| report.get("status"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

fn read_report_lifecycle(path: &Path) -> Result<Option<ReportLifecycle>> {
    let Some(report) = read_report(path)? else {
        return Ok(None);
    };
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .context("run report has no status")?
        .to_string();
    let cleanup = cleanup_verification(&report, &status);
    Ok(Some(ReportLifecycle { status, cleanup }))
}

fn read_report(path: &Path) -> Result<Option<Value>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };
    let report = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(report))
}

fn cleanup_verification(report: &Value, status: &str) -> CleanupVerification {
    let status = qol_dev_env::ReportStatus::parse(status);
    match qol_dev_env::report::child_cleanup(report, &status) {
        qol_dev_env::CleanupState::Pending => CleanupVerification::Pending,
        qol_dev_env::CleanupState::Complete => CleanupVerification::Complete,
        qol_dev_env::CleanupState::Incomplete(error) => CleanupVerification::Incomplete(error),
    }
}

fn teardown_lanes(lanes: &[Lane], verbose: bool) -> Vec<TeardownResult> {
    lanes
        .iter()
        .rev()
        .map(|lane| teardown_lane(lane, verbose))
        .collect()
}

fn teardown_lane(lane: &Lane, verbose: bool) -> TeardownResult {
    if lane.phase == LanePhase::Attempting {
        return TeardownResult {
            run_id: lane.run_id.clone(),
            status: "pass",
            verification: "not-started",
            report_status: None,
            stop_error: None,
        };
    }
    let deadline = Instant::now() + TEARDOWN_TIMEOUT;
    let mut stop_attempted = false;
    let mut stop_error = None;
    let mut last_report_status = None;
    while Instant::now() < deadline {
        match read_report_lifecycle(&lane.report_path) {
            Ok(Some(report)) => {
                last_report_status = Some(report.status.clone());
                if report.cleanup == CleanupVerification::Complete {
                    return TeardownResult {
                        run_id: lane.run_id.clone(),
                        status: "pass",
                        verification: "verified-cleanup",
                        report_status: Some(report.status),
                        stop_error,
                    };
                }
                if let CleanupVerification::Incomplete(error) = report.cleanup {
                    if !stop_attempted {
                        stop_attempted = true;
                        if let Err(stop) =
                            forward_emu("down", &lane.run_id, verbose, &lane_control_roots(lane))
                        {
                            stop_error = Some(format!("cleanup retry failed: {stop:#}"));
                        }
                        thread::sleep(READY_POLL_INTERVAL);
                        continue;
                    }
                    let error = append_error(stop_error, error);
                    return TeardownResult {
                        run_id: lane.run_id.clone(),
                        status: "failed",
                        verification: "cleanup-incomplete",
                        report_status: Some(report.status),
                        stop_error: Some(error),
                    };
                }
                let running = report.status == "running";
                if running && !stop_attempted {
                    stop_attempted = true;
                    if let Err(error) =
                        forward_emu("down", &lane.run_id, verbose, &lane_control_roots(lane))
                    {
                        stop_error = Some(format!("stop request failed: {error:#}"));
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                stop_error = Some(format!("report inspection failed: {error:#}"));
            }
        }
        thread::sleep(READY_POLL_INTERVAL);
    }
    if !stop_attempted {
        if let Err(error) = forward_emu("down", &lane.run_id, verbose, &lane_control_roots(lane)) {
            stop_error = Some(format!("stop request failed: {error:#}"));
        }
    }
    let verification_error = format!(
        "could not verify terminal state after {} seconds",
        TEARDOWN_TIMEOUT.as_secs()
    );
    let stop_error = stop_error
        .map(|error| format!("{error}; {verification_error}"))
        .or(Some(verification_error));
    TeardownResult {
        run_id: lane.run_id.clone(),
        status: "failed",
        verification: "timeout",
        report_status: last_report_status,
        stop_error,
    }
}

fn append_error(existing: Option<String>, error: String) -> String {
    existing
        .map(|existing| format!("{existing}; {error}"))
        .unwrap_or(error)
}

fn lane_control_roots(lane: &Lane) -> Vec<PathBuf> {
    lane.run_dir
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .collect()
}

fn reconcile_batch_reports(
    teardown: &[TeardownResult],
    require_cleanup_complete: bool,
) -> Result<()> {
    let teardown = teardown
        .iter()
        .map(|result| (result.run_id.clone(), result.clone()))
        .collect::<BTreeMap<_, _>>();
    let roots = dev_env::discover()?
        .into_iter()
        .filter_map(|environment| environment.run_root)
        .collect::<BTreeSet<_>>();
    let mut failures = Vec::new();
    for root in roots {
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                failures.push(format!("{}: {error}", root.display()));
                continue;
            }
        };
        for entry in entries.flatten() {
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
            if let Err(error) =
                reconcile_batch_report_file(&report_path, &teardown, require_cleanup_complete)
            {
                failures.push(format!("{}: {error:#}", report_path.display()));
            }
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    bail!(
        "failed to reconcile environment batches:\n{}",
        failures.join("\n")
    )
}

fn reconcile_batch_report_file(
    path: &Path,
    teardown: &BTreeMap<String, TeardownResult>,
    require_cleanup_complete: bool,
) -> Result<()> {
    let Some(run_dir) = path.parent() else {
        bail!("environment batch report has no run directory");
    };
    let _lock = lock_batch_run(run_dir)?;
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };
    let report: Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if report.get("kind").and_then(Value::as_str) != Some("environment-batch") {
        return Ok(());
    }
    let require_cleanup_complete = require_cleanup_complete
        && report
            .get("runs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|run| run.get("run_id").and_then(Value::as_str))
            .any(|run_id| teardown.contains_key(run_id));
    if batch_owner_active(&report) {
        return Ok(());
    }
    let mut lane_states = BTreeMap::new();
    for run in report
        .get("runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(run_id) = run.get("run_id").and_then(Value::as_str) else {
            continue;
        };
        let lifecycle = run
            .get("report")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .map(|report_path| read_report_lifecycle(&report_path))
            .transpose()?
            .flatten();
        lane_states.insert(run_id.to_string(), lifecycle);
    }
    let reconciled =
        reconciled_batch_report(&report, &lane_states, teardown, qol_dev_env::unix_millis()?);
    if reconciled == report {
        if require_cleanup_complete {
            ensure_batch_cleanup_complete(&reconciled)?;
        }
        return Ok(());
    }
    if let Some(run_dir) = path.parent() {
        fs::create_dir_all(run_dir.join("steps"))
            .with_context(|| format!("failed to create steps for {}", run_dir.display()))?;
        write_json(&run_dir.join("steps/lifecycle.json"), &reconciled["steps"])?;
    }
    write_json(path, &reconciled)?;
    if require_cleanup_complete {
        ensure_batch_cleanup_complete(&reconciled)?;
    }
    Ok(())
}

fn reconciled_batch_report(
    report: &Value,
    lane_states: &BTreeMap<String, Option<ReportLifecycle>>,
    teardown: &BTreeMap<String, TeardownResult>,
    finished_at_unix_ms: u64,
) -> Value {
    let mut reconciled = report.clone();
    let Some(object) = reconciled.as_object_mut() else {
        return reconciled;
    };
    let owner_interrupted = batch_owner_interrupted(object);
    let mut active = false;
    let mut incomplete = false;
    let mut affected = false;
    let mut child_failed = false;
    let mut child_abandoned = false;
    let mut child_cancelled = false;
    let mut teardown_lanes = Vec::new();
    if let Some(runs) = object.get_mut("runs").and_then(Value::as_array_mut) {
        for run in runs {
            let Some(run_id) = run
                .get("run_id")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                active = true;
                continue;
            };
            let lifecycle = lane_states.get(&run_id).cloned().flatten();
            let phase = run.get("phase").and_then(Value::as_str);
            let planned_not_started = owner_interrupted
                && lifecycle.is_none()
                && phase == Some(LanePhase::Attempting.as_str());
            let uncertain_launch = owner_interrupted && lifecycle.is_none() && !planned_not_started;
            let mut state = lifecycle.as_ref().map(|lifecycle| lifecycle.status.clone());
            let mut cleanup_complete = lifecycle
                .as_ref()
                .is_some_and(|lifecycle| lifecycle.cleanup == CleanupVerification::Complete);
            let mut cleanup_incomplete = lifecycle.as_ref().is_some_and(|lifecycle| {
                matches!(lifecycle.cleanup, CleanupVerification::Incomplete(_))
            });
            if planned_not_started {
                state = Some("not-started".to_string());
                cleanup_complete = true;
                child_abandoned = true;
            }
            if uncertain_launch {
                cleanup_incomplete = true;
            }
            if cleanup_complete {
                match state.as_deref() {
                    Some("failed" | "skipped") => child_failed = true,
                    Some("abandoned") => child_abandoned = true,
                    Some("cancelled") => child_cancelled = true,
                    _ => {}
                }
            }
            let lane_active = !planned_not_started
                && !uncertain_launch
                && lifecycle
                    .as_ref()
                    .is_none_or(|lifecycle| lifecycle.cleanup == CleanupVerification::Pending);
            active |= lane_active;
            if let Some(run_object) = run.as_object_mut() {
                run_object.insert(
                    "status".to_string(),
                    state.clone().map(Value::String).unwrap_or(Value::Null),
                );
            }
            let outcome = teardown.get(&run_id);
            if outcome.is_some() {
                affected = true;
            }
            let outcome_failed = outcome.is_some_and(|result| !result.succeeded());
            let lane_incomplete = cleanup_incomplete || outcome_failed;
            incomplete |= lane_incomplete;
            let teardown_lane_status = if lane_incomplete {
                "failed"
            } else if cleanup_complete {
                "pass"
            } else {
                outcome.map(|result| result.status).unwrap_or("pending")
            };
            let verification = if cleanup_incomplete {
                "cleanup-incomplete"
            } else if cleanup_complete {
                "verified-cleanup"
            } else {
                outcome
                    .map(|result| result.verification)
                    .unwrap_or("active-or-unknown")
            };
            teardown_lanes.push(json!({
                "run_id": run_id,
                "status": teardown_lane_status,
                "verification": verification,
                "report_status": state,
                "stop_error": outcome.and_then(|result| result.stop_error.as_deref()),
            }));
        }
    }
    let teardown_status = if incomplete {
        "incomplete"
    } else if active {
        "in-progress"
    } else {
        "complete"
    };
    object.insert(
        "teardown".to_string(),
        json!({ "status": teardown_status, "lanes": teardown_lanes }),
    );
    if incomplete {
        object.remove("finished_at_unix_ms");
        mark_batch_owner_orphaned(object, owner_interrupted);
        let status = incomplete_batch_status(object);
        object.insert("status".to_string(), Value::String(status.to_string()));
        update_report_steps(object, status, teardown_status);
        return reconciled;
    }
    if active {
        object.remove("finished_at_unix_ms");
        if owner_interrupted {
            object.insert(
                "status".to_string(),
                Value::String("recovering".to_string()),
            );
            mark_batch_owner_orphaned(object, true);
            update_report_steps(object, "recovering", teardown_status);
        } else if affected {
            object.insert("status".to_string(), Value::String("stopping".to_string()));
            update_report_steps(object, "stopping", teardown_status);
        }
        return reconciled;
    }
    let failed = object.get("error").is_some_and(|error| !error.is_null());
    let cancelled = object.get("status").and_then(Value::as_str) == Some("cancelled");
    let status = if cancelled || child_cancelled {
        "cancelled"
    } else if failed || child_failed {
        "failed"
    } else if owner_interrupted || child_abandoned {
        "abandoned"
    } else {
        "stopped"
    };
    object.insert("status".to_string(), Value::String(status.to_string()));
    object.insert(
        "finished_at_unix_ms".to_string(),
        Value::from(finished_at_unix_ms),
    );
    if owner_interrupted {
        if let Some(owner) = object.get_mut("owner").and_then(Value::as_object_mut) {
            owner.insert("state".to_string(), Value::String("released".to_string()));
        }
    }
    update_report_steps(object, status, teardown_status);
    reconciled
}

fn batch_owner_interrupted(report: &serde_json::Map<String, Value>) -> bool {
    let owner = report.get("owner");
    let state = owner
        .and_then(|owner| owner.get("state"))
        .and_then(Value::as_str);
    if state == Some("orphaned") {
        return true;
    }
    if state != Some("running") {
        return false;
    }
    owner.is_none_or(|owner| !batch_owner_may_be_active(owner))
}

fn batch_owner_active(report: &Value) -> bool {
    report
        .get("owner")
        .filter(|owner| owner.get("state").and_then(Value::as_str) == Some("running"))
        .is_some_and(batch_owner_may_be_active)
}

fn batch_owner_may_be_active(owner: &Value) -> bool {
    let Some(pid) = owner
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
    else {
        return false;
    };
    if !qol_process::is_pid_alive(pid) {
        return false;
    }
    let Some(identity) = owner.get("process_identity").and_then(Value::as_str) else {
        return true;
    };
    qol_process::process_identity_matches(pid, identity)
}

fn mark_batch_owner_orphaned(report: &mut serde_json::Map<String, Value>, interrupted: bool) {
    if !interrupted {
        return;
    }
    if let Some(owner) = report.get_mut("owner").and_then(Value::as_object_mut) {
        owner.insert("state".to_string(), Value::String("orphaned".to_string()));
    }
}

fn incomplete_batch_status(report: &serde_json::Map<String, Value>) -> &'static str {
    let status = report.get("status").and_then(Value::as_str);
    if matches!(
        status,
        Some("cancelled" | "cancellation-cleanup-incomplete")
    ) {
        return "cancellation-cleanup-incomplete";
    }
    "rollback-incomplete"
}

fn ensure_batch_cleanup_complete(report: &Value) -> Result<()> {
    if report
        .get("teardown")
        .and_then(|teardown| teardown.get("status"))
        .and_then(Value::as_str)
        != Some("incomplete")
    {
        return Ok(());
    }
    let run_id = report
        .get("run_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    bail!("environment batch `{run_id}` has incomplete cleanup")
}

fn update_report_steps(
    report: &mut serde_json::Map<String, Value>,
    status: &str,
    teardown_status: &str,
) {
    let launch_status = match status {
        "cancelled" | "cancellation-cleanup-incomplete" => "cancelled",
        "failed" | "abandoned" | "rollback-incomplete" => "failed",
        "recovering" | "starting" | "stopping" => "running",
        _ => "pass",
    };
    let teardown_step_status = match teardown_status {
        "complete" => "pass",
        "incomplete" => "failed",
        _ => "running",
    };
    report.insert(
        "steps".to_string(),
        json!([
            { "id": "prepare", "status": "pass" },
            { "id": "launch", "status": launch_status },
            { "id": "teardown", "status": teardown_step_status },
        ]),
    );
}

fn write_batch_files(batch: &Batch<'_>) -> Result<()> {
    let _lock = lock_batch_run(batch.run_dir)?;
    let effective_environment = effective_environment(batch);
    write_json(
        &batch.run_dir.join("effective-env.json"),
        &effective_environment,
    )?;
    let preflight = host_preflight(batch);
    atomic_write(&batch.run_dir.join("host-preflight.txt"), &preflight)?;
    let report = batch_report(batch);
    write_json(
        &batch.run_dir.join("steps/lifecycle.json"),
        &report["steps"],
    )?;
    write_json(&batch.run_dir.join("report.json"), &report)
}

fn lock_batch_run(run_dir: &Path) -> Result<File> {
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

fn batch_report(batch: &Batch<'_>) -> Value {
    let lanes = batch
        .lanes
        .iter()
        .map(|lane| {
            json!({
                "run_id": lane.run_id,
                "run_dir": lane.run_dir,
                "report": lane.report_path,
                "phase": lane.phase.as_str(),
            })
        })
        .collect::<Vec<_>>();
    let teardown = teardown_json(batch.teardown);
    let teardown_status = teardown["status"].as_str().unwrap_or("not-started");
    let launch_status = match batch.status {
        "running" | "stopped" => "pass",
        "cancelled" => "cancelled",
        "failed" | "rollback-incomplete" | "cancellation-cleanup-incomplete" => "failed",
        _ => "running",
    };
    let teardown_step_status = match teardown_status {
        "complete" => "pass",
        "incomplete" => "failed",
        "not-started" => "pending",
        _ => "running",
    };
    let mut report = json!({
        "name": "qol-env-up",
        "kind": "environment-batch",
        "run_id": batch.run_id,
        "started_at_unix_ms": batch.started_at_unix_ms,
        "status": batch.status,
        "error": batch.error,
        "owner": dev_env::run_owner(
            "interactive-environment",
            if batch.status == "starting" { "running" } else { "released" },
        ),
        "environment": {
            "id": batch.environment.definition.id,
            "name": batch.environment.definition.name,
            "family": batch.environment.definition.family,
            "backend": batch.environment.definition.backend,
            "image_path": batch.environment.image_path,
            "source": batch.environment.definition.source,
        },
        "launch": {
            "count": batch.count,
            "memory_mb": batch.memory_mb,
            "cpus": batch.cpus,
            "display": if batch.windowed { "windowed" } else { "none" },
        },
        "admission": {
            "available_memory_mb": batch.admission.available_memory_mb,
            "budget_percent": dev_resources::MEMORY_BUDGET_PERCENT,
            "budget_memory_mb": batch.admission.budget_memory_mb,
            "requested_memory_mb": batch.admission.requested_memory_mb,
            "reserved_lanes": batch.admission.reserved_lanes,
            "reserved_memory_mb": batch.admission.reserved_memory_mb,
            "available_cpus": batch.admission.available_cpus,
            "cpu_budget_percent": dev_resources::CPU_BUDGET_PERCENT,
            "budget_cpus": batch.admission.budget_cpus,
            "requested_cpus": batch.admission.requested_cpus,
            "reserved_cpus": batch.admission.reserved_cpus,
            "available_disk_bytes": batch.admission.available_disk_bytes,
            "disk_budget_percent": dev_resources::DISK_BUDGET_PERCENT,
            "budget_disk_bytes": batch.admission.budget_disk_bytes,
            "requested_disk_bytes": batch.admission.requested_disk_bytes,
            "reserved_disk_bytes": batch.admission.reserved_disk_bytes,
            "forced": batch.admission.forced,
        },
        "runs": lanes,
        "steps": [
            { "id": "prepare", "status": "pass" },
            { "id": "launch", "status": launch_status, "attempted": batch.lanes.len(), "requested": batch.count },
            { "id": "teardown", "status": teardown_step_status },
        ],
        "teardown": teardown,
        "artifacts": {
            "run_dir": batch.run_dir,
            "report": batch.run_dir.join("report.json"),
            "effective_environment": batch.run_dir.join("effective-env.json"),
            "host_preflight": batch.run_dir.join("host-preflight.txt"),
            "logs": batch.run_dir.join("logs"),
            "steps": batch.run_dir.join("steps"),
            "artifacts": batch.run_dir.join("artifacts"),
        },
        "next": [
            "Inspect live lanes with `qol env runs`.",
            "Capture a lane with `qol env shot <run-id>`.",
            "Stop all lanes with `qol env down --all`.",
        ],
    });
    if let Some(finished_at_unix_ms) = batch.finished_at_unix_ms {
        report["finished_at_unix_ms"] = Value::from(finished_at_unix_ms);
    }
    report
}

fn teardown_json(teardown: &[TeardownResult]) -> Value {
    if teardown.is_empty() {
        return json!({ "status": "not-started", "lanes": [] });
    }
    let complete = teardown.iter().all(TeardownResult::succeeded);
    let status = if complete { "complete" } else { "incomplete" };
    let lanes = teardown
        .iter()
        .map(|result| {
            json!({
                "run_id": result.run_id,
                "status": result.status,
                "verification": result.verification,
                "report_status": result.report_status,
                "stop_error": result.stop_error,
            })
        })
        .collect::<Vec<_>>();
    json!({ "status": status, "lanes": lanes })
}

fn effective_environment(batch: &Batch<'_>) -> Value {
    json!({
        "id": batch.environment.definition.id,
        "name": batch.environment.definition.name,
        "family": batch.environment.definition.family,
        "backend": batch.environment.definition.backend,
        "image": {
            "kind": batch.environment.definition.image.kind,
            "path": batch.environment.image_path,
            "arch": batch.environment.definition.image.arch,
            "firmware": batch.environment.definition.image.firmware,
        },
        "launch": {
            "count": batch.count,
            "memory_mb": batch.memory_mb,
            "cpus": batch.cpus,
            "display": if batch.windowed { "windowed" } else { "none" },
        },
        "mounts": {
            "workspace": batch.environment.definition.mounts.workspace,
        },
        "capabilities": batch.environment.definition.capabilities,
    })
}

fn host_preflight(batch: &Batch<'_>) -> String {
    let available_memory = display_optional_number(batch.admission.available_memory_mb);
    let budget_memory = display_optional_number(batch.admission.budget_memory_mb);
    let available_cpus = display_optional_number(batch.admission.available_cpus);
    let budget_cpus = display_optional_number(batch.admission.budget_cpus);
    let available_disk = display_optional_number(batch.admission.available_disk_bytes);
    let budget_disk = display_optional_number(batch.admission.budget_disk_bytes);
    let admission = if batch.admission.forced {
        "forced"
    } else {
        "passed"
    };
    format!(
        "environment={}\ncount={}\nmemory_mb={}\ncpus={}\navailable_memory_mb={}\nmemory_budget_percent={}\nbudget_memory_mb={}\nrequested_memory_mb={}\nreserved_lanes={}\nreserved_memory_mb={}\navailable_cpus={}\ncpu_budget_percent={}\nbudget_cpus={}\nrequested_cpus={}\nreserved_cpus={}\navailable_disk_bytes={}\ndisk_budget_percent={}\nbudget_disk_bytes={}\nrequested_disk_bytes={}\nreserved_disk_bytes={}\nadmission={}\n",
        batch.environment.definition.id,
        batch.count,
        batch.memory_mb,
        batch.cpus,
        available_memory,
        dev_resources::MEMORY_BUDGET_PERCENT,
        budget_memory,
        batch.admission.requested_memory_mb,
        batch.admission.reserved_lanes,
        batch.admission.reserved_memory_mb,
        available_cpus,
        dev_resources::CPU_BUDGET_PERCENT,
        budget_cpus,
        batch.admission.requested_cpus,
        batch.admission.reserved_cpus,
        available_disk,
        dev_resources::DISK_BUDGET_PERCENT,
        budget_disk,
        batch.admission.requested_disk_bytes,
        batch.admission.reserved_disk_bytes,
        admission,
    )
}

fn display_optional_number(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let content = serde_json::to_string_pretty(value).context("failed to serialize JSON")?;
    atomic_write(path, &format!("{content}\n"))
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    qol_fs::atomic_write(path, content.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))
}

fn forward_emu(command: &str, selector: &str, verbose: bool, run_roots: &[PathBuf]) -> Result<()> {
    let mut args = vec![OsString::from(command)];
    for root in run_roots {
        args.extend([
            OsString::from("--run-root"),
            root.as_os_str().to_os_string(),
        ]);
    }
    args.push(OsString::from(selector));
    emu::run(&args, verbose)
}

fn required_selector(args: &[OsString], usage: &str) -> Result<String> {
    if args.len() != 1 {
        bail!("usage: {usage}");
    }
    args[0]
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("selector must be valid UTF-8\nusage: {usage}"))
}

fn require_no_args(args: &[OsString], usage: &str) -> Result<()> {
    if args.is_empty() {
        return Ok(());
    }
    bail!("usage: {usage}")
}

fn display_optional_path(path: Option<&Path>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "unconfigured".to_string())
}

fn display_optional_memory(memory_mb: Option<u64>) -> String {
    memory_mb
        .map(|memory| format!("{memory} MiB"))
        .unwrap_or_else(|| "unknown".to_string())
}

fn help_text() -> &'static str {
    "qol env\n\n  list\n  doctor [--repair|--fix|--lease-clear <run-id|--all>]\n  up <environment> [--count N] [--memory-mb N] [--cpus N] [--windowed] [--force]\n  image import <environment> <source> --worktree <absolute-path> [--run-id ID]\n  cancel <batch-run-id>\n  runs\n  down <run-id|environment|--all>\n  shot <run-id|environment>\n\nDefinitions live in flows/envs/*.toml. Local image_root, run_root, and [images]\noverrides live in the dev-envs.toml path shown by `qol env doctor`. Environment\ncapabilities select the required acceleration policy."
}

fn print_help() {
    println!("{}", help_text());
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_dev_env::{BootDefinition, EnvironmentDefinition, ImageDefinition, MountDefinition};
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn os_args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn report_lifecycle(status: &str, cleanup: CleanupVerification) -> Option<ReportLifecycle> {
        Some(ReportLifecycle {
            status: status.to_string(),
            cleanup,
        })
    }

    fn resolved_environment(run_root: &Path) -> ResolvedEnvironment {
        ResolvedEnvironment {
            definition: EnvironmentDefinition {
                id: "linux-debian".to_string(),
                name: "Debian".to_string(),
                family: "linux".to_string(),
                backend: "qemu".to_string(),
                image: ImageDefinition {
                    kind: "qcow2".to_string(),
                    base: PathBuf::from("debian.qcow2"),
                    recommended_size_gb: 16,
                    arch: Some("x86_64".to_string()),
                    firmware: Some("bios".to_string()),
                },
                boot: BootDefinition {
                    memory_mb: 1024,
                    cpus: 1,
                    display: "none".to_string(),
                },
                mounts: MountDefinition { workspace: true },
                capabilities: BTreeMap::from([(
                    "acceleration".to_string(),
                    "hardware".to_string(),
                )]),
                source: PathBuf::from("flows/envs/linux-debian.toml"),
            },
            state: ResolutionState::Ready,
            image_path: Some(PathBuf::from("/images/debian.qcow2")),
            verified_image: None,
            run_root: Some(run_root.to_path_buf()),
            messages: Vec::new(),
        }
    }

    #[test]
    fn parses_up_options_in_any_order() {
        let cases = [
            (
                vec!["debian"],
                UpArgs {
                    environment_id: "debian".to_string(),
                    run_id: None,
                    count: 1,
                    memory_mb: None,
                    cpus: None,
                    windowed: false,
                    force: false,
                },
            ),
            (
                vec![
                    "--count",
                    "10",
                    "--windowed",
                    "debian",
                    "--cpus",
                    "2",
                    "--memory-mb",
                    "768",
                    "--force",
                    "--run-id",
                    "debian-batch-test",
                ],
                UpArgs {
                    environment_id: "debian".to_string(),
                    run_id: Some("debian-batch-test".to_string()),
                    count: 10,
                    memory_mb: Some(768),
                    cpus: Some(2),
                    windowed: true,
                    force: true,
                },
            ),
        ];
        for (args, expected) in cases {
            assert_eq!(parse_up_args(&os_args(&args)).unwrap(), expected);
        }
    }

    #[test]
    fn image_import_parser_requires_exact_absolute_inputs() {
        let parsed = parse_image_import_args(&os_args(&[
            "linux/mint-cinnamon",
            "/images/mint.qcow2",
            "--run-id",
            "mint-import-1",
            "--worktree",
            "/worktrees/mint",
        ]))
        .unwrap();
        assert_eq!(parsed.environment_id, "linux/mint-cinnamon");
        assert_eq!(parsed.source, PathBuf::from("/images/mint.qcow2"));
        assert_eq!(parsed.worktree, PathBuf::from("/worktrees/mint"));
        assert_eq!(parsed.run_id.as_deref(), Some("mint-import-1"));

        let cases = [
            (
                vec!["linux/mint-cinnamon", "/images/mint.qcow2"],
                "--worktree is required",
            ),
            (
                vec![
                    "linux/mint-cinnamon",
                    "relative.qcow2",
                    "--worktree",
                    "/worktree",
                ],
                "absolute path",
            ),
            (
                vec![
                    "linux/mint-cinnamon",
                    "/images/mint.qcow2",
                    "--worktree",
                    "relative",
                ],
                "absolute path",
            ),
            (
                vec![
                    "linux/mint-cinnamon",
                    "/images/mint.qcow2",
                    "--worktree",
                    "/worktree",
                    "--run-id",
                    "../escape",
                ],
                "invalid run id",
            ),
            (
                vec![
                    "linux/mint-cinnamon",
                    "/images/mint.qcow2",
                    "--worktree",
                    "/worktree",
                    "--unknown",
                ],
                "unknown image-import option",
            ),
        ];
        for (args, expected) in cases {
            let error = parse_image_import_args(&os_args(&args)).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "args {args:?}: {error:#}"
            );
        }
    }

    #[test]
    fn image_import_receipt_comes_from_the_exact_terminal_report() {
        let temp = tempdir().unwrap();
        let run_id = "mint-import-1";
        let image_root = temp.path().join("images");
        let report_path =
            qol_dev_env::managed_verification_report_path(&image_root, run_id).unwrap();
        let image_path = image_root.join("verified/images/digest.qcow2");
        write_json(
            &report_path,
            &json!({
                "kind": "image-import",
                "run_id": run_id,
                "status": "pass",
                "workflow": {
                    "promotion": {
                        "status": "published",
                        "image_path": image_path,
                    },
                },
                "teardown": {
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": true,
                    "staging_removed": true,
                },
            }),
        )
        .unwrap();
        let ticket = RunTicket::new(
            run_id.to_string(),
            qol_dev_env::ReportKind::ImageImport,
            report_path.clone(),
        )
        .unwrap();
        let summary = ticket.read().unwrap().unwrap().summary();

        let receipt = image_import_receipt(&ticket, summary, true).unwrap();

        assert_eq!(receipt.run_id, run_id);
        assert_eq!(receipt.image_path, image_path);
        assert_eq!(receipt.report_path, report_path);
    }

    #[test]
    fn doctor_actions_require_explicit_repair_and_clear_modes() {
        let cases = [
            (Vec::new(), DoctorAction::Inspect),
            (vec!["--repair"], DoctorAction::Repair),
            (vec!["--fix"], DoctorAction::Repair),
            (
                vec!["--lease-clear", "run-1"],
                DoctorAction::Clear(dev_resources::LeaseClearSelection::One("run-1".to_string())),
            ),
            (
                vec!["--lease-clear", "--all"],
                DoctorAction::Clear(dev_resources::LeaseClearSelection::All),
            ),
        ];
        for (args, expected) in cases {
            assert_eq!(parse_doctor_action(&os_args(&args)).unwrap(), expected);
        }
        for args in [
            vec!["--repair", "extra"],
            vec!["--lease-clear"],
            vec!["--unknown"],
        ] {
            assert!(
                parse_doctor_action(&os_args(&args)).is_err(),
                "args: {args:?}"
            );
        }
    }

    #[test]
    fn rejects_invalid_up_options() {
        let cases = [
            (vec![], "usage"),
            (vec!["debian", "mint"], "more than once"),
            (vec!["debian", "--count", "0"], "between 1 and 32"),
            (vec!["debian", "--count", "33"], "between 1 and 32"),
            (vec!["debian", "--memory-mb", "0"], "positive integer"),
            (vec!["debian", "--cpus", "0"], "positive integer"),
            (vec!["debian", "--wat"], "unknown option"),
            (vec!["debian", "--force", "--force"], "more than once"),
            (
                vec!["debian", "--run-id", "../bad"],
                "invalid resource lease",
            ),
        ];
        for (args, expected) in cases {
            let error = parse_up_args(&os_args(&args)).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "args {args:?}: {error:#}"
            );
        }
    }

    #[test]
    fn emu_arguments_are_identity_safe_and_headless_by_default() {
        let temp = tempdir().unwrap();
        let environment = resolved_environment(temp.path());
        let case_root = temp.path().join("cases");
        let parent_lease = dev_resources::ParentLeaseClaim::parse("linux-debian-batch-1").unwrap();
        let args = emu_up_args(EmuUpRequest {
            image_path: Path::new("/images/debian.qcow2"),
            environment: &environment,
            parent_lease: &parent_lease,
            run_id: "linux-debian-run-1",
            memory_mb: 768,
            cpus: 2,
            windowed: false,
            case_root: &case_root,
        })
        .unwrap();
        let actual = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            [
                "emu",
                "up",
                "/images/debian.qcow2",
                "--headless",
                "--memory-mb",
                "768",
                "--cpus",
                "2",
                "--run-id",
                "linux-debian-run-1",
                "--parent-lease",
                "linux-debian-batch-1",
                "--environment-id",
                "linux-debian",
                "--run-root",
                case_root.to_str().unwrap(),
                "--image-kind",
                "qcow2",
                "--acceleration",
                "hardware",
                "--arch",
                "x86_64",
                "--firmware",
                "bios",
            ]
        );
        let windowed = emu_up_args(EmuUpRequest {
            image_path: Path::new("/images/debian.qcow2"),
            environment: &environment,
            parent_lease: &parent_lease,
            run_id: "linux-debian-run-2",
            memory_mb: 768,
            cpus: 2,
            windowed: true,
            case_root: &case_root,
        })
        .unwrap();
        assert!(windowed.contains(&OsString::from("--windowed")));
        assert!(!windowed.contains(&OsString::from("--headless")));
    }

    #[test]
    fn batch_report_and_stable_evidence_files_share_one_source() {
        let temp = tempdir().unwrap();
        let environment = resolved_environment(temp.path());
        let run_dir = prepare_batch_dir(temp.path(), "batch-1").unwrap();
        let lane = Lane {
            run_id: "lane-1".to_string(),
            run_dir: PathBuf::from("/runs/lane-1"),
            report_path: PathBuf::from("/runs/lane-1/report.json"),
            phase: LanePhase::Running,
        };
        let batch = Batch {
            run_id: "batch-1",
            run_dir: &run_dir,
            environment: &environment,
            count: 1,
            memory_mb: 1024,
            cpus: 1,
            windowed: false,
            admission: Admission {
                available_memory_mb: Some(4096),
                budget_memory_mb: Some(3072),
                requested_memory_mb: 1024,
                reserved_lanes: 0,
                reserved_memory_mb: 0,
                available_cpus: Some(8),
                budget_cpus: Some(16),
                requested_cpus: 1,
                reserved_cpus: 0,
                available_disk_bytes: Some(128_000_000_000),
                budget_disk_bytes: Some(115_200_000_000),
                requested_disk_bytes: 16 * 1_073_741_824,
                reserved_disk_bytes: 0,
                forced: false,
            },
            lanes: &[lane],
            teardown: &[],
            status: "running",
            error: None,
            started_at_unix_ms: 1,
            finished_at_unix_ms: None,
        };
        write_batch_files(&batch).unwrap();
        let report: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(report["status"], "running");
        assert_eq!(report["environment"]["id"], "linux-debian");
        assert_eq!(report["runs"][0]["run_id"], "lane-1");
        assert_eq!(report["runs"][0]["phase"], "running");
        assert_eq!(report["launch"]["display"], "none");
        assert_eq!(
            report["owner"]["process_identity"],
            qol_process::process_identity(std::process::id()).unwrap()
        );
        assert!(report.get("finished_at_unix_ms").is_none());
        assert!(run_dir.join("effective-env.json").is_file());
        assert!(run_dir.join("host-preflight.txt").is_file());
        assert!(run_dir.join("artifacts").is_dir());
        assert!(run_dir.join("logs").is_dir());
        assert!(run_dir.join("steps/lifecycle.json").is_file());
        let preflight = fs::read_to_string(run_dir.join("host-preflight.txt")).unwrap();
        assert!(preflight.contains("budget_memory_mb=3072"));
        assert!(preflight.contains("admission=passed"));
    }

    #[test]
    fn unpublished_batch_directories_are_owned_exclusively_and_removable() {
        let temp = tempdir().unwrap();
        let owned = prepare_batch_dir(temp.path(), "owned").unwrap();
        assert!(owned.join("artifacts").is_dir());
        assert!(owned.join("logs").is_dir());
        assert!(owned.join("steps").is_dir());

        remove_unpublished_batch_dir(&owned).unwrap();
        assert!(!owned.exists());

        let existing = temp.path().join("existing");
        fs::create_dir(&existing).unwrap();
        let marker = existing.join("keep");
        fs::write(&marker, b"keep").unwrap();
        assert!(prepare_batch_dir(temp.path(), "existing").is_err());
        assert_eq!(fs::read(&marker).unwrap(), b"keep");
    }

    #[test]
    fn unresolved_unpublished_batch_cleanup_writes_durable_quarantine_evidence() {
        let temp = tempdir().unwrap();
        let run_dir = temp.path().join("quarantined");

        write_unpublished_batch_failure(&run_dir, "quarantined", "cleanup failed").unwrap();

        let report = qol_dev_env::read_report(&run_dir.join("report.json"))
            .unwrap()
            .unwrap();
        assert_eq!(report.run_id, "quarantined");
        assert_eq!(report.kind, qol_dev_env::ReportKind::Environment);
        assert_eq!(report.status, qol_dev_env::ReportStatus::CleanupIncomplete);
        assert!(matches!(
            report.cleanup,
            qol_dev_env::CleanupState::Incomplete(_)
        ));
    }

    #[test]
    fn lane_failure_rolls_back_every_attempt_in_reverse_order() {
        let temp = tempdir().unwrap();
        let mut persisted = Vec::new();
        let mut rolled_back_ids = Vec::new();
        let cancellation = qol_process::CancellationToken::new();
        let execution = execute_owned_batch(
            (0..3)
                .map(|index| Lane::attempted(format!("lane-{index}"), temp.path()))
                .collect(),
            &cancellation,
            |lane| {
                lane.phase = LanePhase::Spawned;
                if lane.run_id == "lane-1" {
                    bail!("injected launch failure");
                }
                lane.phase = LanePhase::Running;
                Ok(())
            },
            |lanes, status| {
                persisted.push((status.to_string(), lanes.len()));
                Ok(())
            },
            |lanes| {
                rolled_back_ids = lanes.iter().rev().map(|lane| lane.run_id.clone()).collect();
                lanes
                    .iter()
                    .rev()
                    .map(|lane| TeardownResult {
                        run_id: lane.run_id.clone(),
                        status: "pass",
                        verification: "terminal-report",
                        report_status: Some("pass".to_string()),
                        stop_error: None,
                    })
                    .collect()
            },
        );
        let BatchExecution::RolledBack {
            lanes,
            error,
            teardown,
            ..
        } = execution
        else {
            panic!("injected lane failure transferred ownership");
        };
        assert_eq!(lanes.len(), 3);
        assert_eq!(lanes[1].phase, LanePhase::Spawned);
        assert!(error.contains("injected launch failure"));
        assert_eq!(rolled_back_ids, ["lane-2", "lane-1", "lane-0"]);
        assert_eq!(teardown.len(), 3);
        assert!(!persisted.iter().any(|(status, _)| status == "running"));
    }

    #[test]
    fn failed_running_commit_retains_ownership_until_rollback() {
        let temp = tempdir().unwrap();
        let mut rollback_count = 0;
        let cancellation = qol_process::CancellationToken::new();
        let execution = execute_owned_batch(
            (0..2)
                .map(|index| Lane::attempted(format!("lane-{index}"), temp.path()))
                .collect(),
            &cancellation,
            |lane| {
                lane.phase = LanePhase::Running;
                Ok(())
            },
            |_, status| {
                if status == "running" {
                    bail!("injected atomic commit failure");
                }
                Ok(())
            },
            |lanes| {
                rollback_count = lanes.len();
                lanes
                    .iter()
                    .map(|lane| TeardownResult {
                        run_id: lane.run_id.clone(),
                        status: "pass",
                        verification: "terminal-report",
                        report_status: Some("pass".to_string()),
                        stop_error: None,
                    })
                    .collect()
            },
        );
        let BatchExecution::RolledBack { error, .. } = execution else {
            panic!("failed running commit transferred ownership");
        };
        assert!(error.contains("atomic commit failure"));
        assert_eq!(rollback_count, 2);
    }

    #[test]
    fn cancellation_stops_launching_and_rolls_back_every_attempt() {
        let temp = tempdir().unwrap();
        let cancellation = qol_process::CancellationToken::new();
        let launch_cancellation = cancellation.clone();
        let mut launches = 0;
        let mut rolled_back_ids = Vec::new();
        let execution = execute_owned_batch(
            (0..3)
                .map(|index| Lane::attempted(format!("lane-{index}"), temp.path()))
                .collect(),
            &cancellation,
            |lane| {
                launches += 1;
                lane.phase = LanePhase::Running;
                launch_cancellation.cancel();
                Ok(())
            },
            |_, _| Ok(()),
            |lanes| {
                rolled_back_ids = lanes.iter().rev().map(|lane| lane.run_id.clone()).collect();
                lanes
                    .iter()
                    .rev()
                    .map(|lane| TeardownResult {
                        run_id: lane.run_id.clone(),
                        status: "pass",
                        verification: "terminal-report",
                        report_status: Some("failed".to_string()),
                        stop_error: None,
                    })
                    .collect()
            },
        );
        let BatchExecution::RolledBack {
            lanes,
            error,
            teardown,
            cancelled,
        } = execution
        else {
            panic!("cancelled environment batch transferred ownership");
        };
        assert!(cancelled);
        assert_eq!(error, "environment launch cancelled");
        assert_eq!(launches, 1);
        assert_eq!(lanes.len(), 3);
        assert_eq!(rolled_back_ids, ["lane-2", "lane-1", "lane-0"]);
        assert!(teardown.iter().all(TeardownResult::succeeded));
        assert_eq!(rollback_status(cancelled, true), "cancelled");
        assert_eq!(
            rollback_status(cancelled, false),
            "cancellation-cleanup-incomplete"
        );
    }

    #[test]
    fn teardown_report_preserves_successes_and_failures() {
        let teardown = teardown_json(&[
            TeardownResult {
                run_id: "lane-1".to_string(),
                status: "pass",
                verification: "terminal-report",
                report_status: Some("pass".to_string()),
                stop_error: None,
            },
            TeardownResult {
                run_id: "lane-2".to_string(),
                status: "failed",
                verification: "timeout",
                report_status: Some("running".to_string()),
                stop_error: Some("injected stop failure".to_string()),
            },
        ]);
        assert_eq!(teardown["status"], "incomplete");
        assert_eq!(teardown["lanes"][0]["status"], "pass");
        assert_eq!(teardown["lanes"][1]["status"], "failed");
        assert_eq!(teardown["lanes"][1]["stop_error"], "injected stop failure");
    }

    #[test]
    fn terminal_cleanup_requires_complete_orphan_evidence() {
        let cases = [
            (
                "preparing",
                json!({"status": "preparing", "teardown": null}),
                CleanupVerification::Pending,
            ),
            (
                "running",
                json!({"status": "running", "teardown": null}),
                CleanupVerification::Pending,
            ),
            (
                "cleanup-incomplete",
                json!({
                    "status": "cleanup-incomplete",
                    "teardown": {"status": "incomplete", "error": "identity mismatch"},
                }),
                CleanupVerification::Incomplete("identity mismatch".to_string()),
            ),
            (
                "abandoned",
                json!({
                    "status": "abandoned",
                    "teardown": {"status": "complete", "qemu_exit_verified": false},
                }),
                CleanupVerification::Incomplete(
                    "terminal child lacks verified process-tree exit or artifact cleanup"
                        .to_string(),
                ),
            ),
            (
                "abandoned",
                json!({
                    "status": "abandoned",
                    "teardown": {
                        "status": "complete",
                        "qemu_exit_verified": true,
                        "tree_exit_verified": true,
                    },
                }),
                CleanupVerification::Complete,
            ),
            (
                "pass",
                json!({"status": "pass", "teardown": {"removed": []}}),
                CleanupVerification::Incomplete(
                    "terminal child lacks verified process-tree exit or artifact cleanup"
                        .to_string(),
                ),
            ),
            (
                "pass",
                json!({
                    "status": "pass",
                    "teardown": {
                        "status": "complete",
                        "qemu_exit_verified": true,
                        "tree_exit_verified": true,
                        "removed": []
                    }
                }),
                CleanupVerification::Complete,
            ),
            (
                "future-status",
                json!({
                    "status": "future-status",
                    "teardown": {
                        "status": "complete",
                        "qemu_exit_verified": true,
                        "tree_exit_verified": true
                    }
                }),
                CleanupVerification::Pending,
            ),
        ];
        for (status, report, expected) in cases {
            assert_eq!(cleanup_verification(&report, status), expected);
        }
    }

    #[test]
    fn reconciliation_only_finishes_batches_after_every_lane_is_terminal() {
        let report = json!({
            "kind": "environment-batch",
            "status": "running",
            "error": null,
            "finished_at_unix_ms": 9,
            "runs": [
                { "run_id": "lane-1", "report": "/runs/lane-1/report.json" },
                { "run_id": "lane-2", "report": "/runs/lane-2/report.json" },
            ],
        });
        let active = BTreeMap::from([
            (
                "lane-1".to_string(),
                report_lifecycle("pass", CleanupVerification::Complete),
            ),
            (
                "lane-2".to_string(),
                report_lifecycle("running", CleanupVerification::Pending),
            ),
        ]);
        let untouched = reconciled_batch_report(&report, &active, &BTreeMap::new(), 99);
        assert_eq!(untouched["status"], "running");
        assert!(untouched.get("finished_at_unix_ms").is_none());
        let teardown = BTreeMap::from([(
            "lane-2".to_string(),
            TeardownResult {
                run_id: "lane-2".to_string(),
                status: "failed",
                verification: "timeout",
                report_status: Some("running".to_string()),
                stop_error: Some("injected timeout".to_string()),
            },
        )]);
        let stopping = reconciled_batch_report(&report, &active, &teardown, 100);
        assert_eq!(stopping["status"], "rollback-incomplete");
        assert!(stopping.get("finished_at_unix_ms").is_none());
        assert_eq!(stopping["teardown"]["status"], "incomplete");

        let terminal = BTreeMap::from([
            (
                "lane-1".to_string(),
                report_lifecycle("pass", CleanupVerification::Complete),
            ),
            (
                "lane-2".to_string(),
                report_lifecycle("failed", CleanupVerification::Complete),
            ),
        ]);
        let failed = reconciled_batch_report(&stopping, &terminal, &BTreeMap::new(), 101);
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["finished_at_unix_ms"], 101);
        assert_eq!(failed["teardown"]["status"], "complete");

        let abandoned = BTreeMap::from([
            (
                "lane-1".to_string(),
                report_lifecycle("pass", CleanupVerification::Complete),
            ),
            (
                "lane-2".to_string(),
                report_lifecycle("abandoned", CleanupVerification::Complete),
            ),
        ]);
        let abandoned = reconciled_batch_report(&report, &abandoned, &BTreeMap::new(), 102);
        assert_eq!(abandoned["status"], "abandoned");
        assert_eq!(abandoned["finished_at_unix_ms"], 102);
        assert_eq!(abandoned["steps"][1]["status"], "failed");
    }

    #[test]
    fn dead_batch_owner_resolves_planned_lane_as_not_started() {
        let report = json!({
            "kind": "environment-batch",
            "run_id": "batch-1",
            "status": "starting",
            "error": null,
            "owner": { "pid": u32::MAX, "state": "running" },
            "runs": [{
                "run_id": "lane-1",
                "phase": "attempting",
                "report": "/runs/lane-1/report.json",
            }],
        });
        let states = BTreeMap::from([("lane-1".to_string(), None)]);

        let reconciled = reconciled_batch_report(&report, &states, &BTreeMap::new(), 300);

        assert_eq!(reconciled["status"], "abandoned");
        assert_eq!(reconciled["owner"]["state"], "released");
        assert_eq!(reconciled["runs"][0]["status"], "not-started");
        assert_eq!(reconciled["teardown"]["status"], "complete");
        assert_eq!(reconciled["finished_at_unix_ms"], 300);
    }

    #[test]
    fn reused_batch_owner_pid_resolves_planned_lane_as_not_started() {
        let report = json!({
            "kind": "environment-batch",
            "run_id": "batch-1",
            "status": "starting",
            "error": null,
            "owner": {
                "pid": std::process::id(),
                "process_identity": "stale-process-identity",
                "state": "running",
            },
            "runs": [{
                "run_id": "lane-1",
                "phase": "attempting",
                "report": "/runs/lane-1/report.json",
            }],
        });
        let states = BTreeMap::from([("lane-1".to_string(), None)]);

        assert!(!batch_owner_active(&report));
        let reconciled = reconciled_batch_report(&report, &states, &BTreeMap::new(), 300);

        assert_eq!(reconciled["status"], "abandoned");
        assert_eq!(reconciled["owner"]["state"], "released");
        assert_eq!(reconciled["runs"][0]["status"], "not-started");
        assert_eq!(reconciled["teardown"]["status"], "complete");
    }

    #[test]
    fn matching_and_legacy_batch_owner_evidence_remain_active() {
        let pid = std::process::id();
        let identity = qol_process::process_identity(pid).unwrap();
        let report = json!({
            "owner": {
                "pid": pid,
                "process_identity": identity,
                "state": "running",
            },
        });
        let legacy_report = json!({
            "owner": {
                "pid": pid,
                "state": "running",
            },
        });

        assert!(batch_owner_active(&report));
        assert!(!batch_owner_interrupted(report.as_object().unwrap()));
        assert!(batch_owner_active(&legacy_report));
        assert!(!batch_owner_interrupted(legacy_report.as_object().unwrap()));
    }

    #[test]
    fn dead_batch_owner_keeps_uncertain_launch_nonterminal() {
        let report = json!({
            "kind": "environment-batch",
            "run_id": "batch-1",
            "status": "starting",
            "error": null,
            "owner": { "pid": u32::MAX, "state": "running" },
            "runs": [{
                "run_id": "lane-1",
                "phase": "launching",
                "report": "/runs/lane-1/report.json",
            }],
        });
        let states = BTreeMap::from([("lane-1".to_string(), None)]);

        let reconciled = reconciled_batch_report(&report, &states, &BTreeMap::new(), 301);

        assert_eq!(reconciled["status"], "rollback-incomplete");
        assert_eq!(reconciled["owner"]["state"], "orphaned");
        assert_eq!(reconciled["teardown"]["status"], "incomplete");
        assert!(reconciled.get("finished_at_unix_ms").is_none());
    }

    #[test]
    fn cleanup_incomplete_child_can_never_produce_successful_batch_teardown() {
        let report = json!({
            "kind": "environment-batch",
            "run_id": "batch-1",
            "status": "running",
            "error": null,
            "runs": [
                { "run_id": "lane-1", "report": "/runs/lane-1/report.json" },
            ],
        });
        let states = BTreeMap::from([(
            "lane-1".to_string(),
            report_lifecycle(
                "cleanup-incomplete",
                CleanupVerification::Incomplete("QMP identity mismatch".to_string()),
            ),
        )]);

        let reconciled = reconciled_batch_report(&report, &states, &BTreeMap::new(), 200);

        assert_eq!(reconciled["status"], "rollback-incomplete");
        assert_eq!(reconciled["teardown"]["status"], "incomplete");
        assert_eq!(reconciled["teardown"]["lanes"][0]["status"], "failed");
        assert!(reconciled.get("finished_at_unix_ms").is_none());
        assert!(ensure_batch_cleanup_complete(&reconciled).is_err());
    }

    #[test]
    fn admission_recovery_tolerates_unrelated_incomplete_history() {
        let temp = tempfile::tempdir().unwrap();
        let batch_dir = temp.path().join("batch");
        let child_dir = temp.path().join("cases/lane-1");
        fs::create_dir_all(&batch_dir).unwrap();
        fs::create_dir_all(&child_dir).unwrap();
        let child_report = child_dir.join("report.json");
        write_json(
            &child_report,
            &json!({
                "status": "cleanup-incomplete",
                "teardown": { "status": "incomplete", "error": "historic failure" },
            }),
        )
        .unwrap();
        let batch_report = batch_dir.join("report.json");
        write_json(
            &batch_report,
            &json!({
                "kind": "environment-batch",
                "run_id": "batch-1",
                "status": "rollback-incomplete",
                "owner": { "state": "released" },
                "runs": [{
                    "run_id": "lane-1",
                    "phase": "running",
                    "report": child_report,
                }],
                "teardown": { "status": "incomplete" },
            }),
        )
        .unwrap();
        let teardown = BTreeMap::from([(
            "lane-1".to_string(),
            TeardownResult {
                run_id: "lane-1".to_string(),
                status: "failed",
                verification: "cleanup-incomplete",
                report_status: Some("cleanup-incomplete".to_string()),
                stop_error: Some("historic failure".to_string()),
            },
        )]);

        assert!(reconcile_batch_report_file(&batch_report, &teardown, false).is_ok());
        assert!(reconcile_batch_report_file(&batch_report, &teardown, true).is_err());
    }
}
