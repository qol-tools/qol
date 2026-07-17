use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::machine;
use super::qmp::{self, QmpClient};

const RUN_ID_ENVIRONMENT_CHARS: usize = 40;
const MAX_RUN_ID_LEN: usize = 64;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const LIST_TIMEOUT: Duration = Duration::from_millis(250);
const ORPHAN_EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const ORPHAN_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(25);
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LiveRun {
    pub(crate) run_id: String,
    pub(crate) environment_id: String,
    pub(crate) run_dir: PathBuf,
    pub(crate) qmp_port: u16,
    pub(crate) serial_port: Option<u16>,
    pub(crate) supervisor_pid: u32,
    pub(crate) supervisor_process_identity: String,
    pub(crate) qemu_pid: u32,
    pub(crate) qemu_process_identity: String,
    pub(crate) machine_name: String,
}

pub(crate) struct VerifiedRun {
    pub(crate) run: LiveRun,
    pub(crate) qmp: QmpClient,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OwnedRunCleanup {
    pub(crate) report_status: String,
    pub(crate) evidence_path: PathBuf,
    pub(crate) removed: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
struct RunningCandidate {
    run_id: String,
    directory_id: String,
    status: String,
    spawn_state: Option<String>,
    environment_id: Option<String>,
    run_dir: PathBuf,
    report_content: String,
    report: Value,
    qmp_port: Option<u16>,
    serial_port: Option<u16>,
    supervisor_pid: Option<u32>,
    supervisor_process_identity: Option<String>,
    qemu_pid: Option<u32>,
    qemu_process_identity: Option<String>,
}

enum CandidateSelection<'a> {
    Exact(&'a RunningCandidate),
    Environment(Vec<&'a RunningCandidate>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeObservation {
    supervisor_alive: bool,
    qemu_alive: bool,
    qmp_machine_name: std::result::Result<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StaleReason {
    Contract(String),
    SupervisorDead(u32),
    QemuDead(u32),
    QmpUnavailable(String),
    MachineMismatch { expected: String, actual: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OrphanCleanup {
    Complete {
        qemu_was_alive: bool,
        machine_name: Option<String>,
        removed: Vec<PathBuf>,
    },
    Incomplete {
        phase: &'static str,
        error: String,
        qemu_was_alive: Option<bool>,
        machine_name: Option<String>,
    },
}

trait OrphanControl {
    fn machine_name(&mut self) -> Result<String>;
    fn quit(&mut self) -> Result<()>;
}

trait OrphanRuntime {
    fn process_identity_matches(&self, pid: u32, identity: &str) -> bool;
    fn connect(&self, run: &LiveRun, timeout: Duration) -> Result<Box<dyn OrphanControl>>;
    fn wait_for_exit(&self, pid: u32, identity: &str, timeout: Duration) -> Result<()>;
    fn wait_for_tree_exit(&self, process_group: u32, timeout: Duration) -> Result<()>;
    fn teardown(&self, run_dir: &Path) -> Result<Vec<PathBuf>>;
}

struct SystemOrphanRuntime;

impl OrphanControl for QmpClient {
    fn machine_name(&mut self) -> Result<String> {
        self.query_machine_name()
    }

    fn quit(&mut self) -> Result<()> {
        self.fire("quit")
    }
}

impl OrphanRuntime for SystemOrphanRuntime {
    fn process_identity_matches(&self, pid: u32, identity: &str) -> bool {
        process_identity_matches(pid, identity)
    }

    fn connect(&self, run: &LiveRun, timeout: Duration) -> Result<Box<dyn OrphanControl>> {
        Ok(Box::new(qmp::connect(run.qmp_port, timeout)?))
    }

    fn wait_for_exit(&self, pid: u32, identity: &str, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !process_identity_matches(pid, identity) {
                return Ok(());
            }
            thread::sleep(ORPHAN_EXIT_POLL_INTERVAL);
        }
        bail!(
            "QEMU PID {pid} remained alive after {} seconds",
            timeout.as_secs()
        )
    }

    fn wait_for_tree_exit(&self, process_group: u32, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !qol_process::is_group_alive(process_group) {
                return Ok(());
            }
            thread::sleep(ORPHAN_EXIT_POLL_INTERVAL);
        }
        bail!(
            "process tree {process_group} remained alive after {} seconds",
            timeout.as_secs()
        )
    }

    fn teardown(&self, run_dir: &Path) -> Result<Vec<PathBuf>> {
        machine::teardown(run_dir)
    }
}

impl StaleReason {
    fn message(&self) -> String {
        match self {
            Self::Contract(reason) => reason.clone(),
            Self::SupervisorDead(pid) => format!("supervisor PID {pid} is not alive"),
            Self::QemuDead(pid) => format!("QEMU PID {pid} is not alive"),
            Self::QmpUnavailable(reason) => format!("QMP identity is unavailable: {reason}"),
            Self::MachineMismatch { expected, actual } => {
                format!("QMP machine identity mismatch: expected `{expected}`, got `{actual}`")
            }
        }
    }
}

pub(crate) fn new_run_id(environment_id: &str) -> Result<String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| anyhow!("system time is before UNIX_EPOCH"))?;
    let sequence = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format_run_id(
        environment_id,
        elapsed.as_nanos(),
        std::process::id(),
        sequence,
    ))
}

pub(crate) fn find(runs_root: &Path, selector: &str) -> Result<VerifiedRun> {
    find_in_roots(std::iter::once(runs_root), selector)
}

pub(crate) fn find_in_roots<'a>(
    runs_roots: impl IntoIterator<Item = &'a Path>,
    selector: &str,
) -> Result<VerifiedRun> {
    let candidates = candidates_in_roots(runs_roots);
    match select_candidates(&candidates, selector)? {
        CandidateSelection::Exact(candidate) => verify_for_control(candidate),
        CandidateSelection::Environment(matches) => verify_environment(matches, selector),
    }
}

fn select_candidates<'a>(
    candidates: &'a [RunningCandidate],
    selector: &str,
) -> Result<CandidateSelection<'a>> {
    let exact = candidates
        .iter()
        .filter(|candidate| candidate.run_id == selector)
        .collect::<Vec<_>>();
    if exact.len() > 1 {
        let paths = exact
            .iter()
            .map(|candidate| candidate.run_dir.display().to_string())
            .collect::<Vec<_>>();
        bail!(
            "duplicate running emu identity `{selector}` in: {}",
            paths.join(", ")
        );
    }
    if let Some(candidate) = exact.first() {
        return Ok(CandidateSelection::Exact(candidate));
    }

    let matches = candidates
        .iter()
        .filter(|candidate| candidate.environment_id.as_deref() == Some(selector))
        .collect::<Vec<_>>();
    Ok(CandidateSelection::Environment(matches))
}

fn verify_environment(matches: Vec<&RunningCandidate>, selector: &str) -> Result<VerifiedRun> {
    let mut verified = matches
        .into_iter()
        .filter_map(|candidate| verify_for_listing(candidate, CONTROL_TIMEOUT))
        .collect::<Vec<_>>();
    if verified.len() == 1 {
        return Ok(verified.remove(0));
    }
    if verified.is_empty() {
        return Err(no_live_run(selector));
    }
    Err(ambiguous_environment(selector, &verified))
}

pub(crate) fn list_in_roots<'a>(runs_roots: impl IntoIterator<Item = &'a Path>) -> Vec<LiveRun> {
    let mut live_runs = candidates_in_roots(runs_roots)
        .iter()
        .filter_map(|candidate| verify_for_listing(candidate, LIST_TIMEOUT))
        .map(|verified| verified.run)
        .collect::<Vec<_>>();
    live_runs.sort_by(|left, right| {
        left.run_id
            .cmp(&right.run_id)
            .then_with(|| left.run_dir.cmp(&right.run_dir))
    });
    live_runs
}

pub(crate) fn reconcile_exact_image_import_vm(
    run_dir: &Path,
    expected_run_id: &str,
    expected_owner_pid: u32,
    expected_owner_identity: &str,
) -> Result<()> {
    ensure_owned_run_directory(run_dir, expected_run_id)?;
    let report_path = run_dir.join("report.json");
    let report = qol_dev_env::read_report_checked(
        &report_path,
        expected_run_id,
        &qol_dev_env::ReportKind::ImageImport,
    )?
    .context("image-import VM report disappeared during reconciliation")?;
    if report.owner.pid != Some(expected_owner_pid)
        || report.owner.process_identity.as_deref() != Some(expected_owner_identity)
    {
        bail!("image-import VM owner identity changed during reconciliation");
    }
    if qol_process::process_identity_matches(expected_owner_pid, expected_owner_identity) {
        bail!("image-import VM owner PID {expected_owner_pid} is still alive");
    }
    let candidate = running_candidate_from_dir(run_dir.to_path_buf())
        .context("image-import VM report has no recoverable runtime identity")?;
    if candidate.supervisor_pid != Some(expected_owner_pid)
        || candidate.supervisor_process_identity.as_deref() != Some(expected_owner_identity)
    {
        bail!("image-import VM supervisor identity does not match its owner journal");
    }
    record_stale_with(
        &candidate,
        &StaleReason::SupervisorDead(expected_owner_pid),
        &ExactOwnerRuntime {
            owner_pid: expected_owner_pid,
            owner_identity: expected_owner_identity,
        },
    )?;
    Ok(())
}

struct ExactOwnerRuntime<'a> {
    owner_pid: u32,
    owner_identity: &'a str,
}

impl OrphanRuntime for ExactOwnerRuntime<'_> {
    fn process_identity_matches(&self, pid: u32, identity: &str) -> bool {
        if pid == self.owner_pid {
            return identity == self.owner_identity
                && process_identity_matches(pid, self.owner_identity);
        }
        SystemOrphanRuntime.process_identity_matches(pid, identity)
    }

    fn connect(&self, run: &LiveRun, timeout: Duration) -> Result<Box<dyn OrphanControl>> {
        SystemOrphanRuntime.connect(run, timeout)
    }

    fn wait_for_exit(&self, pid: u32, identity: &str, timeout: Duration) -> Result<()> {
        SystemOrphanRuntime.wait_for_exit(pid, identity, timeout)
    }

    fn wait_for_tree_exit(&self, process_group: u32, timeout: Duration) -> Result<()> {
        SystemOrphanRuntime.wait_for_tree_exit(process_group, timeout)
    }

    fn teardown(&self, run_dir: &Path) -> Result<Vec<PathBuf>> {
        SystemOrphanRuntime.teardown(run_dir)
    }
}

pub(crate) fn reconcile_owned_terminated(
    run_dir: &Path,
    expected_run_id: &str,
    reason: &str,
    _tree_proof: &qol_process::TerminatedProcessTree,
) -> Result<OwnedRunCleanup> {
    ensure_owned_run_directory(run_dir, expected_run_id)?;
    fs::create_dir_all(run_dir)
        .with_context(|| format!("failed to create {}", run_dir.display()))?;
    let lock_path = run_dir.join("reconcile.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open {}", lock_path.display()))?;
    lock.lock()
        .with_context(|| format!("failed to lock {}", lock_path.display()))?;

    let report_path = run_dir.join("report.json");
    let existing = qol_dev_env::read_report_checked(
        &report_path,
        expected_run_id,
        &qol_dev_env::ReportKind::Flow,
    )?;
    let existing_content = existing
        .as_ref()
        .map(|report| {
            serde_json::to_string_pretty(report.document())
                .map(|content| format!("{content}\n"))
                .context("failed to serialize existing owned run report")
        })
        .transpose()?;
    let existing_report = existing.as_ref().map(|report| report.document().clone());
    let existing_status = existing
        .as_ref()
        .map(|report| report.status.as_str().to_string());
    for (label, pid, identity) in existing_report
        .as_ref()
        .and_then(|report| report.get("runtime"))
        .into_iter()
        .flat_map(|runtime| {
            [
                (
                    "supervisor",
                    runtime.get("supervisor_pid"),
                    runtime.get("supervisor_process_identity"),
                ),
                (
                    "QEMU",
                    runtime.get("qemu_pid"),
                    runtime.get("qemu_process_identity"),
                ),
            ]
        })
        .filter_map(|(label, pid, identity)| {
            pid.and_then(Value::as_u64)
                .and_then(|pid| u32::try_from(pid).ok())
                .filter(|pid| *pid != 0)
                .map(|pid| (label, pid, identity.and_then(Value::as_str)))
        })
    {
        let identity = identity.with_context(|| {
            format!("owned run {label} PID {pid} has no exact process identity")
        })?;
        if process_identity_matches(pid, identity) {
            bail!("owned run {label} PID {pid} is still alive");
        }
    }
    let removed = machine::teardown(run_dir)?;
    let already_terminal = existing
        .as_ref()
        .is_some_and(|report| report.status.is_terminal());
    let already_verified_cleanup = existing
        .as_ref()
        .is_some_and(|report| report.cleanup.is_complete());
    if already_terminal && already_verified_cleanup && removed.is_empty() {
        return Ok(OwnedRunCleanup {
            report_status: existing_status.unwrap_or_default(),
            evidence_path: report_path,
            removed,
        });
    }

    let interrupted_path = run_dir.join("report.interrupted.json");
    if let Some(content) = &existing_content {
        if fs::symlink_metadata(&interrupted_path).is_err() {
            qol_fs::atomic_write(&interrupted_path, content.as_bytes())
                .with_context(|| format!("failed to write {}", interrupted_path.display()))?;
        }
    }
    let observed_at = qol_dev_env::unix_millis()?;
    let marker_path = run_dir.join("owner-cleanup.json");
    let marker = json!({
        "status": "complete",
        "run_id": expected_run_id,
        "observed_at_unix_ms": observed_at,
        "reason": reason,
        "tree_exit_verified": true,
        "removed": &removed,
        "previous_report": existing_content.as_ref().map(|_| &interrupted_path),
    });
    write_json(&marker_path, &marker)?;

    let mut report = existing_report.unwrap_or_else(|| json!({}));
    if report.get("name").is_none() {
        report["name"] = json!("qol-emu-owned-recovery");
    }
    if report.get("kind").is_none() {
        report["kind"] = json!("flow");
    }
    report["run_id"] = json!(expected_run_id);
    report["status"] = json!("abandoned");
    report["finished_at_unix_ms"] = json!(observed_at);
    report["teardown"] = json!({
        "status": "complete",
        "phase": "owner-tree",
        "qemu_exit_verified": true,
        "tree_exit_verified": true,
        "removed": &removed,
    });
    report["reconciliation"] = json!({
        "previous_status": existing_status,
        "reason": reason,
        "observed_at_unix_ms": observed_at,
        "evidence_report": existing_content.as_ref().map(|_| &interrupted_path),
        "owner_cleanup": &marker_path,
    });
    write_json(&report_path, &report)?;
    Ok(OwnedRunCleanup {
        report_status: "abandoned".to_string(),
        evidence_path: marker_path,
        removed,
    })
}

fn ensure_owned_run_directory(run_dir: &Path, expected_run_id: &str) -> Result<()> {
    let directory_id = run_dir
        .file_name()
        .and_then(|name| name.to_str())
        .context("owned run directory has no UTF-8 identity")?;
    if directory_id != expected_run_id {
        bail!(
            "owned run directory identity mismatch: expected `{expected_run_id}`, got `{directory_id}`"
        );
    }
    let metadata = match fs::symlink_metadata(run_dir) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", run_dir.display()))
        }
    };
    if metadata.is_some_and(|metadata| !metadata.file_type().is_dir()) {
        bail!("owned run path is not a directory: {}", run_dir.display());
    }
    Ok(())
}

fn candidates_in_roots<'a>(
    runs_roots: impl IntoIterator<Item = &'a Path>,
) -> Vec<RunningCandidate> {
    let mut candidates = runs_roots
        .into_iter()
        .flat_map(candidates_in_root)
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.run_dir.cmp(&right.run_dir));
    candidates.dedup_by(|left, right| left.run_dir == right.run_dir);
    candidates
}

fn candidates_in_root(runs_root: &Path) -> Vec<RunningCandidate> {
    let Ok(entries) = fs::read_dir(runs_root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| running_candidate_from_dir(entry.path()))
        .collect()
}

fn format_run_id(environment_id: &str, unix_nanos: u128, process_id: u32, sequence: u64) -> String {
    let normalized = super::sanitize_id(environment_id);
    let suffix = format!("{:x}-{process_id:x}-{sequence:x}", unix_nanos as u64);
    let available = MAX_RUN_ID_LEN.saturating_sub(suffix.len() + 1);
    let environment = normalized
        .chars()
        .take(RUN_ID_ENVIRONMENT_CHARS.min(available))
        .collect::<String>()
        .trim_end_matches('-')
        .to_string();
    format!("{environment}-{suffix}")
}

fn running_candidate_from_dir(run_dir: PathBuf) -> Option<RunningCandidate> {
    let report_content = fs::read_to_string(run_dir.join("report.json")).ok()?;
    let report = serde_json::from_str::<Value>(&report_content).ok()?;
    running_candidate_from_report(run_dir, report_content, report)
}

fn running_candidate_from_report(
    run_dir: PathBuf,
    report_content: String,
    report: Value,
) -> Option<RunningCandidate> {
    let status = report.get("status")?.as_str()?.to_string();
    if !matches!(
        status.as_str(),
        "preparing" | "starting" | "running" | "stopping" | "cleanup-incomplete"
    ) {
        return None;
    }
    let directory_id = run_dir.file_name()?.to_string_lossy().into_owned();
    let run_id = report
        .get("run_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .unwrap_or(&directory_id)
        .to_string();
    let environment_id = report
        .get("environment")
        .and_then(|environment| environment.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let qmp_port = report
        .get("qmp")
        .and_then(|qmp| qmp.get("port"))
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok());
    let serial_port = report
        .get("serial")
        .and_then(|serial| serial.get("port"))
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok());
    let supervisor_pid = report
        .get("runtime")
        .and_then(|runtime| runtime.get("supervisor_pid"))
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid != 0);
    let supervisor_process_identity = report
        .get("runtime")
        .and_then(|runtime| runtime.get("supervisor_process_identity"))
        .and_then(Value::as_str)
        .filter(|identity| !identity.is_empty())
        .map(str::to_string);
    let qemu_pid = report
        .get("runtime")
        .and_then(|runtime| runtime.get("qemu_pid"))
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid != 0)
        .or_else(|| qemu_pid_from_pidfile(&run_dir));
    let qemu_process_identity = report
        .get("runtime")
        .and_then(|runtime| runtime.get("qemu_process_identity"))
        .and_then(Value::as_str)
        .filter(|identity| !identity.is_empty())
        .map(str::to_string);
    let spawn_state = report
        .get("spawn")
        .and_then(|spawn| spawn.get("state"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(RunningCandidate {
        run_id,
        directory_id,
        status,
        spawn_state,
        environment_id,
        run_dir,
        report_content,
        report,
        qmp_port,
        serial_port,
        supervisor_pid,
        supervisor_process_identity,
        qemu_pid,
        qemu_process_identity,
    })
}

fn qemu_pid_from_pidfile(run_dir: &Path) -> Option<u32> {
    fs::read_to_string(run_dir.join("qemu.pid"))
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|pid| *pid != 0)
}

fn verify_for_control(candidate: &RunningCandidate) -> Result<VerifiedRun> {
    match verify_candidate(candidate, CONTROL_TIMEOUT) {
        Ok(verified) => Ok(verified),
        Err(reason) => {
            let reconciliation = record_stale(candidate, &reason);
            let evidence = reconciliation
                .map(|path| format!("; evidence: {}", path.display()))
                .unwrap_or_else(|error| format!("; failed to record evidence: {error:#}"));
            bail!(
                "emu run `{}` is not controllable: {}{}",
                candidate.run_id,
                reason.message(),
                evidence
            )
        }
    }
}

fn verify_for_listing(candidate: &RunningCandidate, timeout: Duration) -> Option<VerifiedRun> {
    if candidate.status == "preparing"
        && candidate
            .supervisor_pid
            .zip(candidate.supervisor_process_identity.as_deref())
            .is_some_and(|(pid, identity)| process_identity_matches(pid, identity))
        && candidate.qemu_pid.is_none()
    {
        return None;
    }
    match verify_candidate(candidate, timeout) {
        Ok(verified) => Some(verified),
        Err(reason) => {
            let _ = record_stale(candidate, &reason);
            None
        }
    }
}

fn verify_candidate(
    candidate: &RunningCandidate,
    timeout: Duration,
) -> Result<VerifiedRun, StaleReason> {
    let run = candidate.live_run()?;
    let supervisor_alive =
        process_identity_matches(run.supervisor_pid, &run.supervisor_process_identity);
    let qemu_alive = process_identity_matches(run.qemu_pid, &run.qemu_process_identity);
    let process_observation = RuntimeObservation {
        supervisor_alive,
        qemu_alive,
        qmp_machine_name: Err("not queried".to_string()),
    };
    verify_processes(&run, &process_observation)?;
    let mut client = qmp::connect(run.qmp_port, timeout)
        .map_err(|error| StaleReason::QmpUnavailable(format!("{error:#}")))?;
    let machine_name = client
        .query_machine_name()
        .map_err(|error| format!("{error:#}"));
    let observation = RuntimeObservation {
        supervisor_alive,
        qemu_alive,
        qmp_machine_name: machine_name,
    };
    verify_runtime(&run, &observation)?;
    Ok(VerifiedRun { run, qmp: client })
}

impl RunningCandidate {
    fn live_run(&self) -> Result<LiveRun, StaleReason> {
        if self.run_id != self.directory_id {
            return Err(StaleReason::Contract(format!(
                "report run ID `{}` does not match immutable directory identity `{}`",
                self.run_id, self.directory_id
            )));
        }
        let environment_id = self.environment_id.clone().ok_or_else(|| {
            StaleReason::Contract("report has no environment identity".to_string())
        })?;
        let qmp_port = self
            .qmp_port
            .ok_or_else(|| StaleReason::Contract("report has no valid QMP port".to_string()))?;
        let supervisor_pid = self.supervisor_pid.ok_or_else(|| {
            StaleReason::Contract("report has no valid supervisor PID".to_string())
        })?;
        let supervisor_process_identity =
            self.supervisor_process_identity.clone().ok_or_else(|| {
                StaleReason::Contract("report has no valid supervisor process identity".to_string())
            })?;
        let qemu_pid = self
            .qemu_pid
            .ok_or_else(|| StaleReason::Contract("report has no valid QEMU PID".to_string()))?;
        let qemu_process_identity = self.qemu_process_identity.clone().ok_or_else(|| {
            StaleReason::Contract("report has no valid QEMU process identity".to_string())
        })?;
        let machine_name = format!("qol-emu-{}", self.run_id);
        Ok(LiveRun {
            run_id: self.run_id.clone(),
            environment_id,
            run_dir: self.run_dir.clone(),
            qmp_port,
            serial_port: self.serial_port,
            supervisor_pid,
            supervisor_process_identity,
            qemu_pid,
            qemu_process_identity,
            machine_name,
        })
    }
}

fn verify_processes(run: &LiveRun, observation: &RuntimeObservation) -> Result<(), StaleReason> {
    if !observation.supervisor_alive {
        return Err(StaleReason::SupervisorDead(run.supervisor_pid));
    }
    if !observation.qemu_alive {
        return Err(StaleReason::QemuDead(run.qemu_pid));
    }
    Ok(())
}

fn verify_runtime(run: &LiveRun, observation: &RuntimeObservation) -> Result<(), StaleReason> {
    verify_processes(run, observation)?;
    let machine_name = observation
        .qmp_machine_name
        .as_ref()
        .map_err(|reason| StaleReason::QmpUnavailable(reason.clone()))?;
    if machine_name != &run.machine_name {
        return Err(StaleReason::MachineMismatch {
            expected: run.machine_name.clone(),
            actual: machine_name.clone(),
        });
    }
    Ok(())
}

fn process_identity_matches(pid: u32, identity: &str) -> bool {
    !qol_process::is_pid_zombie(pid) && qol_process::process_identity_matches(pid, identity)
}

fn record_stale(candidate: &RunningCandidate, reason: &StaleReason) -> Result<PathBuf> {
    record_stale_with(candidate, reason, &SystemOrphanRuntime)
}

fn record_stale_with(
    candidate: &RunningCandidate,
    reason: &StaleReason,
    runtime: &impl OrphanRuntime,
) -> Result<PathBuf> {
    let observed_at = qol_dev_env::unix_millis()?;
    let marker_path = candidate.run_dir.join("stale.json");
    let lock_path = candidate.run_dir.join("reconcile.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open {}", lock_path.display()))?;
    lock.lock()
        .with_context(|| format!("failed to lock {}", lock_path.display()))?;
    let supervisor_dead = candidate
        .supervisor_pid
        .zip(candidate.supervisor_process_identity.as_deref())
        .is_some_and(|(pid, identity)| !runtime.process_identity_matches(pid, identity));
    if !supervisor_dead {
        let cleanup = OrphanCleanup::Incomplete {
            phase: "supervisor",
            error: "supervisor death could not be verified".to_string(),
            qemu_was_alive: candidate
                .qemu_pid
                .zip(candidate.qemu_process_identity.as_deref())
                .map(|(pid, identity)| runtime.process_identity_matches(pid, identity)),
            machine_name: None,
        };
        write_stale_marker(candidate, reason, observed_at, &marker_path, &cleanup)?;
        return Ok(marker_path);
    }
    ensure_report_unchanged(candidate)?;
    let evidence_path = preserve_running_report(candidate)?;
    let cleanup = reconcile_orphan(candidate, runtime);
    write_stale_marker(candidate, reason, observed_at, &marker_path, &cleanup)?;
    commit_reconciliation(
        candidate,
        reason,
        observed_at,
        &marker_path,
        &evidence_path,
        &cleanup,
    )?;
    Ok(marker_path)
}

fn reconcile_orphan(candidate: &RunningCandidate, runtime: &impl OrphanRuntime) -> OrphanCleanup {
    if candidate.status == "preparing" && candidate.spawn_state.as_deref() == Some("not-started") {
        if candidate.qemu_pid.is_some() {
            return incomplete_cleanup(
                "spawn-identity",
                anyhow!("pre-spawn journal unexpectedly has a QEMU process identity"),
                true,
                None,
            );
        }
        return match runtime.teardown(&candidate.run_dir) {
            Ok(removed) => OrphanCleanup::Complete {
                qemu_was_alive: false,
                machine_name: None,
                removed,
            },
            Err(error) => incomplete_cleanup("artifacts", error, false, None),
        };
    }
    let run = match candidate.live_run() {
        Ok(run) => run,
        Err(error) => {
            return OrphanCleanup::Incomplete {
                phase: "contract",
                error: error.message(),
                qemu_was_alive: candidate
                    .qemu_pid
                    .zip(candidate.qemu_process_identity.as_deref())
                    .map(|(pid, identity)| runtime.process_identity_matches(pid, identity)),
                machine_name: None,
            }
        }
    };
    if runtime.process_identity_matches(run.supervisor_pid, &run.supervisor_process_identity) {
        return OrphanCleanup::Incomplete {
            phase: "supervisor",
            error: format!("supervisor PID {} became live", run.supervisor_pid),
            qemu_was_alive: Some(
                runtime.process_identity_matches(run.qemu_pid, &run.qemu_process_identity),
            ),
            machine_name: None,
        };
    }
    let qemu_was_alive = runtime.process_identity_matches(run.qemu_pid, &run.qemu_process_identity);
    let mut machine_name = None;
    if qemu_was_alive {
        let mut control = match runtime.connect(&run, CONTROL_TIMEOUT) {
            Ok(control) => control,
            Err(error) => {
                return incomplete_cleanup("identity", error, true, None);
            }
        };
        let actual = match control.machine_name() {
            Ok(actual) => actual,
            Err(error) => {
                return incomplete_cleanup("identity", error, true, None);
            }
        };
        if actual != run.machine_name {
            return incomplete_cleanup(
                "identity",
                anyhow!(
                    "QMP machine identity mismatch: expected `{}`, got `{actual}`",
                    run.machine_name
                ),
                true,
                Some(actual),
            );
        }
        machine_name = Some(actual);
        if runtime.process_identity_matches(run.supervisor_pid, &run.supervisor_process_identity) {
            return incomplete_cleanup(
                "supervisor",
                anyhow!("supervisor PID {} became live", run.supervisor_pid),
                true,
                machine_name,
            );
        }
        if runtime.process_identity_matches(run.qemu_pid, &run.qemu_process_identity) {
            if let Err(error) = control.quit() {
                return incomplete_cleanup("shutdown", error, true, machine_name);
            }
            if let Err(error) = runtime.wait_for_exit(
                run.qemu_pid,
                &run.qemu_process_identity,
                ORPHAN_EXIT_TIMEOUT,
            ) {
                return incomplete_cleanup("exit", error, true, machine_name);
            }
        }
    }
    if runtime.process_identity_matches(run.qemu_pid, &run.qemu_process_identity) {
        return incomplete_cleanup(
            "exit",
            anyhow!("QEMU PID {} exit could not be verified", run.qemu_pid),
            qemu_was_alive,
            machine_name,
        );
    }
    if let Err(error) = runtime.wait_for_tree_exit(run.supervisor_pid, ORPHAN_EXIT_TIMEOUT) {
        return incomplete_cleanup("tree-exit", error, qemu_was_alive, machine_name);
    }
    match runtime.teardown(&candidate.run_dir) {
        Ok(removed) => OrphanCleanup::Complete {
            qemu_was_alive,
            machine_name,
            removed,
        },
        Err(error) => incomplete_cleanup("artifacts", error, qemu_was_alive, machine_name),
    }
}

fn incomplete_cleanup(
    phase: &'static str,
    error: anyhow::Error,
    qemu_was_alive: bool,
    machine_name: Option<String>,
) -> OrphanCleanup {
    OrphanCleanup::Incomplete {
        phase,
        error: format!("{error:#}"),
        qemu_was_alive: Some(qemu_was_alive),
        machine_name,
    }
}

fn ensure_report_unchanged(candidate: &RunningCandidate) -> Result<()> {
    let report_path = candidate.run_dir.join("report.json");
    let current = fs::read_to_string(&report_path)
        .with_context(|| format!("failed to read {}", report_path.display()))?;
    if current != candidate.report_content {
        bail!("report changed while orphan cleanup was being prepared");
    }
    Ok(())
}

fn preserve_running_report(candidate: &RunningCandidate) -> Result<PathBuf> {
    let evidence_path = candidate.run_dir.join("report.running.json");
    if fs::symlink_metadata(&evidence_path).is_err() {
        qol_fs::atomic_write(&evidence_path, candidate.report_content.as_bytes())
            .with_context(|| format!("failed to write {}", evidence_path.display()))?;
    }
    ensure_report_unchanged(candidate)?;
    Ok(evidence_path)
}

fn write_stale_marker(
    candidate: &RunningCandidate,
    reason: &StaleReason,
    observed_at: u64,
    marker_path: &Path,
    cleanup: &OrphanCleanup,
) -> Result<()> {
    let status = match cleanup {
        OrphanCleanup::Complete { .. } => "abandoned",
        OrphanCleanup::Incomplete { .. } => "cleanup-incomplete",
    };
    let marker = json!({
        "status": status,
        "run_id": candidate.run_id,
        "observed_at_unix_ms": observed_at,
        "reason": reason.message(),
        "cleanup": cleanup_json(cleanup),
        "report": candidate.report,
    });
    write_json(marker_path, &marker)
}

fn commit_reconciliation(
    candidate: &RunningCandidate,
    reason: &StaleReason,
    observed_at: u64,
    marker_path: &Path,
    evidence_path: &Path,
    cleanup: &OrphanCleanup,
) -> Result<()> {
    ensure_report_unchanged(candidate)?;
    let mut report = candidate.report.clone();
    let cleanup_json = cleanup_json(cleanup);
    let complete = matches!(cleanup, OrphanCleanup::Complete { .. });
    report["status"] = json!(if complete {
        "abandoned"
    } else {
        "cleanup-incomplete"
    });
    if complete {
        report["finished_at_unix_ms"] = json!(observed_at);
    }
    if !complete {
        report
            .as_object_mut()
            .context("run report must be a JSON object")?
            .remove("finished_at_unix_ms");
    }
    report["teardown"] = cleanup_json.clone();
    report["reconciliation"] = json!({
        "previous_status": candidate.report.get("status").and_then(Value::as_str),
        "reason": reason.message(),
        "observed_at_unix_ms": observed_at,
        "evidence_report": evidence_path,
        "stale_marker": marker_path,
        "cleanup": cleanup_json,
    });
    let report_path = candidate.run_dir.join("report.json");
    write_json(&report_path, &report)
}

fn cleanup_json(cleanup: &OrphanCleanup) -> Value {
    match cleanup {
        OrphanCleanup::Complete {
            qemu_was_alive,
            machine_name,
            removed,
        } => json!({
            "status": "complete",
            "phase": "complete",
            "qemu_was_alive": qemu_was_alive,
            "qemu_exit_verified": true,
            "tree_exit_verified": true,
            "machine_name": machine_name,
            "removed": removed,
        }),
        OrphanCleanup::Incomplete {
            phase,
            error,
            qemu_was_alive,
            machine_name,
        } => json!({
            "status": "incomplete",
            "phase": phase,
            "error": error,
            "qemu_was_alive": qemu_was_alive,
            "qemu_exit_verified": false,
            "tree_exit_verified": false,
            "machine_name": machine_name,
            "removed": [],
        }),
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let content = serde_json::to_string_pretty(value).context("failed to serialize JSON")?;
    qol_fs::atomic_write(path, format!("{content}\n").as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))
}

fn no_live_run(selector: &str) -> anyhow::Error {
    anyhow!("no running emu `{selector}`; start one with `qol emu up {selector}`")
}

fn ambiguous_environment(environment_id: &str, live_runs: &[VerifiedRun]) -> anyhow::Error {
    ambiguity_error(
        environment_id,
        live_runs
            .iter()
            .map(|verified| verified.run.run_id.as_str()),
    )
}

fn ambiguity_error<'a>(
    environment_id: &str,
    run_ids: impl IntoIterator<Item = &'a str>,
) -> anyhow::Error {
    let mut run_ids = run_ids.into_iter().collect::<Vec<_>>();
    run_ids.sort_unstable();
    anyhow!(
        "multiple running emus match environment `{environment_id}`: {}\nrerun the control command with one of these run IDs",
        run_ids.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;
    use std::collections::HashSet;
    use std::process::Command;
    use std::rc::Rc;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use tempfile::TempDir;

    struct FakeControl {
        machine_name: std::result::Result<String, String>,
        quit_error: Option<String>,
        quit_called: Rc<Cell<bool>>,
    }

    impl OrphanControl for FakeControl {
        fn machine_name(&mut self) -> Result<String> {
            self.machine_name.clone().map_err(anyhow::Error::msg)
        }

        fn quit(&mut self) -> Result<()> {
            self.quit_called.set(true);
            if let Some(error) = self.quit_error.clone() {
                bail!(error);
            }
            Ok(())
        }
    }

    struct FakeRuntime {
        supervisor_alive: Cell<bool>,
        qemu_alive: Cell<bool>,
        connect_error: Option<String>,
        machine_name: std::result::Result<String, String>,
        quit_error: Option<String>,
        wait_error: Option<String>,
        teardown_error: Option<String>,
        quit_called: Rc<Cell<bool>>,
        wait_called: Cell<bool>,
        teardown_called: Cell<bool>,
    }

    impl FakeRuntime {
        fn exact(qemu_alive: bool) -> Self {
            Self {
                supervisor_alive: Cell::new(false),
                qemu_alive: Cell::new(qemu_alive),
                connect_error: None,
                machine_name: Ok("qol-emu-mint-a".to_string()),
                quit_error: None,
                wait_error: None,
                teardown_error: None,
                quit_called: Rc::new(Cell::new(false)),
                wait_called: Cell::new(false),
                teardown_called: Cell::new(false),
            }
        }
    }

    impl OrphanRuntime for FakeRuntime {
        fn process_identity_matches(&self, pid: u32, _: &str) -> bool {
            match pid {
                10 => self.supervisor_alive.get(),
                11 => self.qemu_alive.get(),
                _ => false,
            }
        }

        fn connect(&self, _: &LiveRun, _: Duration) -> Result<Box<dyn OrphanControl>> {
            if let Some(error) = self.connect_error.clone() {
                bail!(error);
            }
            Ok(Box::new(FakeControl {
                machine_name: self.machine_name.clone(),
                quit_error: self.quit_error.clone(),
                quit_called: Rc::clone(&self.quit_called),
            }))
        }

        fn wait_for_exit(&self, _: u32, _: &str, _: Duration) -> Result<()> {
            self.wait_called.set(true);
            if let Some(error) = self.wait_error.clone() {
                bail!(error);
            }
            self.qemu_alive.set(false);
            Ok(())
        }

        fn wait_for_tree_exit(&self, _: u32, _: Duration) -> Result<()> {
            if let Some(error) = self.wait_error.clone() {
                bail!(error);
            }
            Ok(())
        }

        fn teardown(&self, run_dir: &Path) -> Result<Vec<PathBuf>> {
            self.teardown_called.set(true);
            if let Some(error) = self.teardown_error.clone() {
                bail!(error);
            }
            machine::teardown(run_dir)
        }
    }

    fn report(
        environment_id: &str,
        run_id: Option<&str>,
        qmp_port: u64,
        supervisor_pid: Option<u64>,
        qemu_pid: Option<u64>,
    ) -> Value {
        let supervisor_process_identity = supervisor_pid.and_then(|pid| {
            u32::try_from(pid)
                .ok()
                .and_then(|pid| qol_process::process_identity(pid).ok())
                .or_else(|| Some(format!("test-supervisor-{pid}")))
        });
        let qemu_process_identity = qemu_pid.and_then(|pid| {
            u32::try_from(pid)
                .ok()
                .and_then(|pid| qol_process::process_identity(pid).ok())
                .or_else(|| Some(format!("test-qemu-{pid}")))
        });
        let mut report = json!({
            "kind": "environment",
            "environment": {"id": environment_id},
            "status": "running",
            "qmp": {"port": qmp_port},
            "serial": {"port": qmp_port + 100},
            "runtime": {
                "supervisor_pid": supervisor_pid,
                "supervisor_process_identity": supervisor_process_identity,
                "qemu_pid": qemu_pid,
                "qemu_process_identity": qemu_process_identity,
            },
        });
        if let Some(run_id) = run_id {
            report["run_id"] = json!(run_id);
        }
        report
    }

    fn candidate(directory: &str, report: Value) -> RunningCandidate {
        let content = format!("{}\n", serde_json::to_string_pretty(&report).unwrap());
        running_candidate_from_report(PathBuf::from(directory), content, report).unwrap()
    }

    fn write_report(root: &Path, directory: &str, report: Value) -> PathBuf {
        let run_dir = root.join(directory);
        fs::create_dir_all(&run_dir).unwrap();
        fs::write(
            run_dir.join("report.json"),
            format!("{}\n", serde_json::to_string_pretty(&report).unwrap()),
        )
        .unwrap();
        run_dir
    }

    fn stored_candidate(root: &Path, directory: &str, report: Value) -> RunningCandidate {
        let run_dir = write_report(root, directory, report);
        running_candidate_from_dir(run_dir).unwrap()
    }

    #[cfg(target_os = "linux")]
    fn terminated_tree_proof() -> (qol_process::TerminatedProcessTree, u32) {
        let process_tree = crate::process_guardian::own_process_tree().unwrap();
        let mut command = Command::new("sleep");
        command.arg("30");
        let prepared = process_tree.prepare_command(command).unwrap();
        let mut child = prepared.spawn().unwrap();
        let pid = child.id();
        qol_process::terminate_owned(&mut child, Duration::from_millis(100)).unwrap();
        let proof = process_tree
            .terminate_and_wait(Duration::from_secs(1))
            .unwrap();
        assert!(!qol_process::is_pid_alive(pid));
        (proof, pid)
    }

    fn live_run() -> LiveRun {
        LiveRun {
            run_id: "mint-a".to_string(),
            environment_id: "mint".to_string(),
            run_dir: PathBuf::from("mint-a"),
            qmp_port: 4400,
            serial_port: Some(4500),
            supervisor_pid: 10,
            supervisor_process_identity: "test-supervisor-10".to_string(),
            qemu_pid: 11,
            qemu_process_identity: "test-qemu-11".to_string(),
            machine_name: "qol-emu-mint-a".to_string(),
        }
    }

    #[test]
    fn run_id_format_normalizes_bounds_and_uses_safe_characters() {
        let cases = [
            ("Mint Cinnamon", "mint-cinnamon-1-2-3"),
            ("!!!", "emu-1-2-3"),
            ("MiNt_24.1/VM", "mint-24-1-vm-1-2-3"),
            (
                "abcdefghijklmnopqrstuvwxyz-abcdefghijklmnopqrstuvwxyz",
                "abcdefghijklmnopqrstuvwxyz-abcdefghijklm-1-2-3",
            ),
        ];
        for (environment, expected) in cases {
            let actual = format_run_id(environment, 1, 2, 3);
            assert_eq!(actual, expected, "environment: {environment}");
            assert!(
                actual
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
                "unsafe run ID: {actual}"
            );
        }

        let maximum = format_run_id(
            "environment-name-that-is-far-too-long-to-fit",
            u128::MAX,
            u32::MAX,
            u64::MAX,
        );
        assert!(maximum.len() <= MAX_RUN_ID_LEN, "run ID: {maximum}");
    }

    #[test]
    fn new_run_id_is_unique_across_concurrent_calls() {
        const THREADS: usize = 64;
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles = (0..THREADS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    new_run_id("mintish").unwrap()
                })
            })
            .collect::<Vec<_>>();
        let ids = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), THREADS, "concurrent IDs collided: {ids:?}");
    }

    #[test]
    fn report_contract_rejects_identity_runtime_and_endpoint_regressions() {
        let pid = u64::from(std::process::id());
        let cases = [
            (
                "mint-a",
                report("mint", Some("mint-a"), 4400, Some(pid), Some(pid)),
                None,
            ),
            (
                "different-dir",
                report("mint", Some("mint-a"), 4400, Some(pid), Some(pid)),
                Some("does not match immutable directory identity"),
            ),
            (
                "mint-a",
                report("", Some("mint-a"), 4400, Some(pid), Some(pid)),
                Some("no environment identity"),
            ),
            (
                "mint-a",
                report("mint", Some("mint-a"), 70_000, Some(pid), Some(pid)),
                Some("no valid QMP port"),
            ),
            (
                "mint-a",
                report("mint", Some("mint-a"), 4400, None, Some(pid)),
                Some("no valid supervisor PID"),
            ),
            (
                "mint-a",
                report("mint", Some("mint-a"), 4400, Some(pid), None),
                Some("no valid QEMU PID"),
            ),
            (
                "mint-a",
                report("mint", Some("mint-a"), 4400, Some(0), Some(pid)),
                Some("no valid supervisor PID"),
            ),
        ];
        for (directory, report, expected_error) in cases {
            let result = candidate(directory, report).live_run();
            let actual = result.err().map(|error| error.message());
            assert_eq!(
                actual.as_deref().map(
                    |message| expected_error.is_some_and(|expected| message.contains(expected))
                ),
                expected_error.map(|_| true),
                "directory: {directory}, error: {actual:?}"
            );
        }
    }

    #[test]
    fn live_control_requires_and_matches_exact_process_identities() {
        let pid = u64::from(std::process::id());
        let mut missing_supervisor = report("mint", Some("mint-a"), 4400, Some(pid), Some(pid));
        missing_supervisor["runtime"]
            .as_object_mut()
            .unwrap()
            .remove("supervisor_process_identity");
        let error = candidate("mint-a", missing_supervisor)
            .live_run()
            .unwrap_err();
        assert!(error
            .message()
            .contains("no valid supervisor process identity"));

        let mut reused = report("mint", Some("mint-a"), 4400, Some(pid), Some(pid));
        reused["runtime"]["supervisor_process_identity"] = json!("stale-supervisor");
        reused["runtime"]["qemu_process_identity"] = json!("stale-qemu");
        let candidate = candidate("mint-a", reused);

        let error = match verify_candidate(&candidate, LIST_TIMEOUT) {
            Ok(_) => panic!("stale process identities were accepted"),
            Err(error) => error,
        };
        assert_eq!(error, StaleReason::SupervisorDead(std::process::id()));
    }

    #[test]
    fn runtime_verification_covers_dead_pids_qmp_failure_and_pid_reuse() {
        let run = live_run();
        let cases = [
            (
                RuntimeObservation {
                    supervisor_alive: false,
                    qemu_alive: true,
                    qmp_machine_name: Ok(run.machine_name.clone()),
                },
                Some(StaleReason::SupervisorDead(10)),
            ),
            (
                RuntimeObservation {
                    supervisor_alive: true,
                    qemu_alive: false,
                    qmp_machine_name: Ok(run.machine_name.clone()),
                },
                Some(StaleReason::QemuDead(11)),
            ),
            (
                RuntimeObservation {
                    supervisor_alive: true,
                    qemu_alive: true,
                    qmp_machine_name: Err("connection refused".to_string()),
                },
                Some(StaleReason::QmpUnavailable(
                    "connection refused".to_string(),
                )),
            ),
            (
                RuntimeObservation {
                    supervisor_alive: true,
                    qemu_alive: true,
                    qmp_machine_name: Ok("qol-emu-another-run".to_string()),
                },
                Some(StaleReason::MachineMismatch {
                    expected: run.machine_name.clone(),
                    actual: "qol-emu-another-run".to_string(),
                }),
            ),
            (
                RuntimeObservation {
                    supervisor_alive: true,
                    qemu_alive: true,
                    qmp_machine_name: Ok(run.machine_name.clone()),
                },
                None,
            ),
        ];
        for (observation, expected) in cases {
            assert_eq!(verify_runtime(&run, &observation).err(), expected);
        }
    }

    #[test]
    fn exact_identity_selection_precedes_environment_fallback() {
        let pid = u64::from(std::process::id());
        let candidates = [
            candidate(
                "foo-a",
                report("foo", Some("foo-a"), 4400, Some(pid), Some(pid)),
            ),
            candidate(
                "foo-b",
                report("foo", Some("foo-b"), 4401, Some(pid), Some(pid)),
            ),
            candidate(
                "foo",
                report("bar", Some("foo"), 4402, Some(pid), Some(pid)),
            ),
        ];
        let selection = select_candidates(&candidates, "foo").unwrap();
        let CandidateSelection::Exact(exact) = selection else {
            panic!("exact run ID did not precede environment fallback");
        };
        assert_eq!(exact.run_id, "foo");
        assert_eq!(exact.environment_id.as_deref(), Some("bar"));
    }

    #[test]
    fn duplicate_exact_identity_is_never_selected_arbitrarily() {
        let pid = u64::from(std::process::id());
        let candidates = [
            candidate(
                "first",
                report("mint", Some("same"), 4400, Some(pid), Some(pid)),
            ),
            candidate(
                "second",
                report("mint", Some("same"), 4401, Some(pid), Some(pid)),
            ),
        ];
        let error = match select_candidates(&candidates, "same") {
            Ok(_) => panic!("duplicate exact identity unexpectedly resolved"),
            Err(error) => error.to_string(),
        };
        assert_eq!(
            error,
            "duplicate running emu identity `same` in: first, second"
        );
    }

    #[test]
    fn ambiguity_message_sorts_actionable_run_ids() {
        let mut first = live_run();
        first.run_id = "mint-z".to_string();
        let mut second = live_run();
        second.run_id = "mint-a".to_string();
        let runs = [first, second];
        let error = ambiguity_error("mint", runs.iter().map(|run| run.run_id.as_str())).to_string();
        assert_eq!(
            error,
            "multiple running emus match environment `mint`: mint-a, mint-z\nrerun the control command with one of these run IDs"
        );
    }

    #[test]
    fn dead_supervisor_with_unverified_live_qemu_stays_cleanup_incomplete() {
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        let root = TempDir::new().unwrap();
        let current_pid = u64::from(std::process::id());
        let run_dir = write_report(
            root.path(),
            "mint-dead",
            report(
                "mint",
                Some("mint-dead"),
                4400,
                Some(u64::from(dead_pid)),
                Some(current_pid),
            ),
        );

        assert!(list_in_roots(std::iter::once(root.path())).is_empty());
        let reconciled: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(reconciled["status"], "cleanup-incomplete");
        assert_eq!(reconciled["teardown"]["status"], "incomplete");
        assert_eq!(reconciled["teardown"]["phase"], "identity");
        assert!(reconciled.get("finished_at_unix_ms").is_none());
        assert!(reconciled["reconciliation"]["reason"]
            .as_str()
            .unwrap()
            .contains("supervisor PID"));
        assert!(run_dir.join("report.running.json").is_file());
        assert!(run_dir.join("stale.json").is_file());
    }

    #[test]
    fn dead_supervisor_and_dead_qemu_are_cleaned_by_system_runtime() {
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        let root = TempDir::new().unwrap();
        let run_dir = write_report(
            root.path(),
            "mint-dead",
            report(
                "mint",
                Some("mint-dead"),
                4400,
                Some(u64::from(dead_pid)),
                Some(u64::from(dead_pid)),
            ),
        );
        fs::write(run_dir.join("overlay.qcow2"), b"disposable").unwrap();

        assert!(list_in_roots(std::iter::once(root.path())).is_empty());

        let reconciled: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(reconciled["status"], "abandoned");
        assert_eq!(reconciled["teardown"]["status"], "complete");
        assert_eq!(reconciled["teardown"]["qemu_was_alive"], false);
        assert_eq!(reconciled["teardown"]["qemu_exit_verified"], true);
        assert!(!run_dir.join("overlay.qcow2").exists());
    }

    #[test]
    fn dead_preparing_supervisor_cleans_proven_not_started_artifacts() {
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        let root = TempDir::new().unwrap();
        let run_dir = write_report(
            root.path(),
            "mint-preparing",
            json!({
                "kind": "environment",
                "run_id": "mint-preparing",
                "status": "preparing",
                "environment": { "id": "mint" },
                "runtime": {
                    "supervisor_pid": dead_pid,
                    "supervisor_process_identity": format!("dead-supervisor-{dead_pid}"),
                },
                "spawn": { "state": "not-started", "pidfile": "qemu.pid" },
            }),
        );
        let artifact = run_dir.join("overlay.qcow2");
        fs::write(&artifact, b"disposable").unwrap();

        assert!(list_in_roots(std::iter::once(root.path())).is_empty());

        let reconciled: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(reconciled["status"], "abandoned");
        assert_eq!(reconciled["teardown"]["tree_exit_verified"], true);
        assert!(!artifact.exists());
    }

    #[test]
    fn live_preparing_supervisor_is_not_recorded_as_stale() {
        let root = TempDir::new().unwrap();
        let run_dir = write_report(
            root.path(),
            "mint-preparing",
            json!({
                "kind": "environment",
                "run_id": "mint-preparing",
                "status": "preparing",
                "environment": { "id": "mint" },
                "runtime": {
                    "supervisor_pid": std::process::id(),
                    "supervisor_process_identity": qol_process::process_identity(std::process::id()).unwrap(),
                },
                "spawn": { "state": "not-started", "pidfile": "qemu.pid" },
            }),
        );
        let before = fs::read(run_dir.join("report.json")).unwrap();

        assert!(list_in_roots(std::iter::once(root.path())).is_empty());

        assert_eq!(fs::read(run_dir.join("report.json")).unwrap(), before);
        assert!(!run_dir.join("stale.json").exists());
    }

    #[test]
    fn launching_report_recovers_qemu_identity_from_canonical_pidfile() {
        let root = TempDir::new().unwrap();
        let current_pid = std::process::id();
        let run_dir = write_report(
            root.path(),
            "mint-launching",
            json!({
                "kind": "environment",
                "run_id": "mint-launching",
                "status": "preparing",
                "environment": { "id": "mint" },
                "qmp": { "port": 4400 },
                "runtime": {
                    "supervisor_pid": current_pid,
                    "supervisor_process_identity": qol_process::process_identity(current_pid).unwrap(),
                },
                "spawn": { "state": "launching", "pidfile": "/untrusted/path" },
            }),
        );
        fs::write(run_dir.join("qemu.pid"), current_pid.to_string()).unwrap();

        let candidate = running_candidate_from_dir(run_dir).unwrap();

        assert_eq!(candidate.qemu_pid, Some(current_pid));
        assert_eq!(candidate.spawn_state.as_deref(), Some("launching"));
    }

    #[test]
    fn dead_launching_supervisor_without_qemu_identity_preserves_artifacts() {
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        let root = TempDir::new().unwrap();
        let run_dir = write_report(
            root.path(),
            "mint-launching",
            json!({
                "kind": "environment",
                "run_id": "mint-launching",
                "status": "preparing",
                "environment": { "id": "mint" },
                "qmp": { "port": 4400 },
                "runtime": {
                    "supervisor_pid": dead_pid,
                    "supervisor_process_identity": format!("dead-supervisor-{dead_pid}"),
                },
                "spawn": { "state": "launching", "pidfile": "qemu.pid" },
            }),
        );
        let artifact = run_dir.join("overlay.qcow2");
        fs::write(&artifact, b"must-remain").unwrap();

        assert!(list_in_roots(std::iter::once(root.path())).is_empty());

        let reconciled: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(reconciled["status"], "cleanup-incomplete");
        assert_eq!(fs::read(&artifact).unwrap(), b"must-remain");
        assert!(reconciled.get("finished_at_unix_ms").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn owned_tree_recovery_finalizes_matching_active_runs() {
        let (proof, dead_pid) = terminated_tree_proof();
        for status in ["starting", "running"] {
            let root = TempDir::new().unwrap();
            let mut active_report = report(
                "debian",
                Some("debian-owned"),
                4400,
                Some(u64::from(dead_pid)),
                Some(u64::from(dead_pid)),
            );
            active_report["kind"] = json!("flow");
            active_report["status"] = json!(status);
            let run_dir = write_report(root.path(), "debian-owned", active_report);
            let artifact = run_dir.join("overlay.qcow2");
            fs::write(&artifact, b"disposable").unwrap();

            let cleanup = reconcile_owned_terminated(
                &run_dir,
                "debian-owned",
                "flow supervisor exited",
                &proof,
            )
            .unwrap();

            assert_eq!(cleanup.report_status, "abandoned", "status: {status}");
            assert_eq!(
                cleanup.evidence_path,
                run_dir.join("owner-cleanup.json"),
                "status: {status}"
            );
            assert_eq!(cleanup.removed, vec![artifact.clone()], "status: {status}");
            assert!(!artifact.exists(), "status: {status}");
            assert!(
                run_dir.join("report.interrupted.json").is_file(),
                "status: {status}"
            );
            let recovered: Value =
                serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                    .unwrap();
            assert_eq!(recovered["run_id"], "debian-owned", "status: {status}");
            assert_eq!(recovered["status"], "abandoned", "status: {status}");
            assert_eq!(
                recovered["teardown"]["status"], "complete",
                "status: {status}"
            );
            assert_eq!(
                recovered["teardown"]["qemu_exit_verified"], true,
                "status: {status}"
            );
            assert_eq!(
                recovered["teardown"]["tree_exit_verified"], true,
                "status: {status}"
            );
            assert_eq!(
                recovered["reconciliation"]["previous_status"], status,
                "status: {status}"
            );
            assert!(
                recovered.get("finished_at_unix_ms").is_some(),
                "status: {status}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn owned_tree_recovery_upgrades_legacy_terminal_cleanup_evidence() {
        let (proof, dead_pid) = terminated_tree_proof();
        let root = TempDir::new().unwrap();
        let mut legacy = report(
            "debian",
            Some("debian-owned"),
            4400,
            Some(u64::from(dead_pid)),
            Some(u64::from(dead_pid)),
        );
        legacy["kind"] = json!("flow");
        legacy["status"] = json!("pass");
        legacy["teardown"] = json!({ "removed": [] });
        let run_dir = write_report(root.path(), "debian-owned", legacy);

        let cleanup =
            reconcile_owned_terminated(&run_dir, "debian-owned", "flow supervisor exited", &proof)
                .unwrap();
        let recovered: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                .unwrap();

        assert_eq!(cleanup.report_status, "abandoned");
        assert_eq!(recovered["status"], "abandoned");
        assert_eq!(recovered["teardown"]["status"], "complete");
        assert_eq!(recovered["teardown"]["qemu_exit_verified"], true);
        assert_eq!(recovered["teardown"]["tree_exit_verified"], true);
        assert!(run_dir.join("report.interrupted.json").is_file());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn owned_tree_recovery_rejects_report_identity_mismatch() {
        let (proof, dead_pid) = terminated_tree_proof();
        let root = TempDir::new().unwrap();
        let mismatched_report = report(
            "debian",
            Some("different-run"),
            4400,
            Some(u64::from(dead_pid)),
            Some(u64::from(dead_pid)),
        );
        let mut mismatched_report = mismatched_report;
        mismatched_report["kind"] = json!("flow");
        let run_dir = write_report(root.path(), "debian-owned", mismatched_report);
        let report_before = fs::read(run_dir.join("report.json")).unwrap();
        let artifact = run_dir.join("overlay.qcow2");
        fs::write(&artifact, b"must-remain").unwrap();

        let error =
            reconcile_owned_terminated(&run_dir, "debian-owned", "flow supervisor exited", &proof)
                .unwrap_err();

        assert!(error
            .to_string()
            .contains("belongs to run `different-run`, expected `debian-owned`"));
        assert_eq!(fs::read(&artifact).unwrap(), b"must-remain");
        assert_eq!(
            fs::read(run_dir.join("report.json")).unwrap(),
            report_before
        );
        assert!(!run_dir.join("report.interrupted.json").exists());
        assert!(!run_dir.join("owner-cleanup.json").exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn owned_tree_recovery_rejects_wrong_kind_and_missing_run_identity() {
        let (proof, dead_pid) = terminated_tree_proof();
        let mut missing_identity = report(
            "debian",
            Some("debian-owned"),
            4400,
            Some(u64::from(dead_pid)),
            Some(u64::from(dead_pid)),
        );
        missing_identity["kind"] = json!("flow");
        missing_identity.as_object_mut().unwrap().remove("run_id");
        let cases = [
            (
                report(
                    "debian",
                    Some("debian-owned"),
                    4400,
                    Some(u64::from(dead_pid)),
                    Some(u64::from(dead_pid)),
                ),
                "expected `flow`",
            ),
            (missing_identity, "has no run_id"),
        ];
        for (report, expected) in cases {
            let root = TempDir::new().unwrap();
            let run_dir = write_report(root.path(), "debian-owned", report);
            let artifact = run_dir.join("overlay.qcow2");
            fs::write(&artifact, b"must-remain").unwrap();

            let error = reconcile_owned_terminated(
                &run_dir,
                "debian-owned",
                "flow supervisor exited",
                &proof,
            )
            .unwrap_err();

            assert!(error.to_string().contains(expected));
            assert_eq!(fs::read(&artifact).unwrap(), b"must-remain");
            assert!(!run_dir.join("report.interrupted.json").exists());
            assert!(!run_dir.join("owner-cleanup.json").exists());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn owned_tree_recovery_synthesizes_missing_report() {
        let (proof, _) = terminated_tree_proof();
        let root = TempDir::new().unwrap();
        let run_dir = root.path().join("debian-owned");
        fs::create_dir_all(&run_dir).unwrap();
        let artifact = run_dir.join("usb-stick.raw");
        fs::write(&artifact, b"disposable").unwrap();

        let cleanup = reconcile_owned_terminated(
            &run_dir,
            "debian-owned",
            "flow supervisor exited before report creation",
            &proof,
        )
        .unwrap();

        assert_eq!(cleanup.report_status, "abandoned");
        assert_eq!(cleanup.evidence_path, run_dir.join("owner-cleanup.json"));
        assert_eq!(cleanup.removed, vec![artifact.clone()]);
        assert!(!artifact.exists());
        assert!(!run_dir.join("report.interrupted.json").exists());
        let recovered: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                .unwrap();
        let evidence: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("owner-cleanup.json")).unwrap())
                .unwrap();
        assert_eq!(recovered["name"], "qol-emu-owned-recovery");
        assert_eq!(recovered["run_id"], "debian-owned");
        assert_eq!(recovered["status"], "abandoned");
        assert_eq!(recovered["teardown"]["status"], "complete");
        assert_eq!(recovered["teardown"]["tree_exit_verified"], true);
        assert_eq!(evidence["status"], "complete");
        assert_eq!(evidence["tree_exit_verified"], true);
        assert!(evidence["previous_report"].is_null());
    }

    #[test]
    fn exact_identity_quit_exit_and_teardown_finalize_abandoned() {
        let root = TempDir::new().unwrap();
        let run_dir = root.path().join("mint-a");
        let candidate = stored_candidate(
            root.path(),
            "mint-a",
            report("mint", Some("mint-a"), 4400, Some(10), Some(11)),
        );
        fs::write(run_dir.join("overlay.qcow2"), b"disposable").unwrap();
        let runtime = FakeRuntime::exact(true);

        record_stale_with(&candidate, &StaleReason::SupervisorDead(10), &runtime).unwrap();

        let reconciled: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(reconciled["status"], "abandoned");
        assert_eq!(reconciled["teardown"]["status"], "complete");
        assert_eq!(reconciled["teardown"]["qemu_exit_verified"], true);
        assert_eq!(reconciled["teardown"]["machine_name"], "qol-emu-mint-a");
        assert!(reconciled.get("finished_at_unix_ms").is_some());
        assert!(runtime.quit_called.get());
        assert!(runtime.wait_called.get());
        assert!(runtime.teardown_called.get());
        assert!(!run_dir.join("overlay.qcow2").exists());
        assert!(run_dir.join("report.running.json").is_file());
    }

    #[test]
    fn dead_qemu_skips_control_then_cleans_and_finalizes() {
        let root = TempDir::new().unwrap();
        let run_dir = root.path().join("mint-a");
        let candidate = stored_candidate(
            root.path(),
            "mint-a",
            report("mint", Some("mint-a"), 4400, Some(10), Some(11)),
        );
        fs::write(run_dir.join("usb-stick.raw"), b"disposable").unwrap();
        let runtime = FakeRuntime::exact(false);

        record_stale_with(&candidate, &StaleReason::SupervisorDead(10), &runtime).unwrap();

        let reconciled: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(reconciled["status"], "abandoned");
        assert_eq!(reconciled["teardown"]["qemu_was_alive"], false);
        assert_eq!(reconciled["teardown"]["qemu_exit_verified"], true);
        assert!(!runtime.quit_called.get());
        assert!(!runtime.wait_called.get());
        assert!(runtime.teardown_called.get());
        assert!(!run_dir.join("usb-stick.raw").exists());
    }

    #[test]
    fn identity_mismatch_never_quits_or_cleans_reused_process() {
        let root = TempDir::new().unwrap();
        let run_dir = root.path().join("mint-a");
        let candidate = stored_candidate(
            root.path(),
            "mint-a",
            report("mint", Some("mint-a"), 4400, Some(10), Some(11)),
        );
        fs::write(run_dir.join("overlay.qcow2"), b"must-remain").unwrap();
        let mut runtime = FakeRuntime::exact(true);
        runtime.machine_name = Ok("qol-emu-reused-pid".to_string());

        record_stale_with(&candidate, &StaleReason::SupervisorDead(10), &runtime).unwrap();

        let reconciled: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(reconciled["status"], "cleanup-incomplete");
        assert_eq!(reconciled["teardown"]["phase"], "identity");
        assert!(reconciled["teardown"]["error"]
            .as_str()
            .unwrap()
            .contains("identity mismatch"));
        assert!(!runtime.quit_called.get());
        assert!(!runtime.wait_called.get());
        assert!(!runtime.teardown_called.get());
        assert_eq!(
            fs::read(run_dir.join("overlay.qcow2")).unwrap(),
            b"must-remain"
        );
        assert!(reconciled.get("finished_at_unix_ms").is_none());
    }

    #[test]
    fn shutdown_exit_and_artifact_faults_remain_cleanup_incomplete() {
        let cases = [
            ("shutdown", Some("quit failed"), None, None),
            ("exit", None, Some("exit timeout"), None),
            ("artifacts", None, None, Some("remove failed")),
        ];
        for (expected_phase, quit_error, wait_error, teardown_error) in cases {
            let root = TempDir::new().unwrap();
            let candidate = stored_candidate(
                root.path(),
                "mint-a",
                report("mint", Some("mint-a"), 4400, Some(10), Some(11)),
            );
            let mut runtime = FakeRuntime::exact(true);
            runtime.quit_error = quit_error.map(str::to_string);
            runtime.wait_error = wait_error.map(str::to_string);
            runtime.teardown_error = teardown_error.map(str::to_string);

            record_stale_with(&candidate, &StaleReason::SupervisorDead(10), &runtime).unwrap();

            let reconciled: Value = serde_json::from_str(
                &fs::read_to_string(root.path().join("mint-a/report.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(reconciled["status"], "cleanup-incomplete");
            assert_eq!(reconciled["teardown"]["phase"], expected_phase);
            assert_eq!(reconciled["teardown"]["qemu_exit_verified"], false);
            assert!(reconciled.get("finished_at_unix_ms").is_none());
        }
    }

    #[test]
    fn legacy_report_without_runtime_is_marked_but_not_overwritten() {
        let root = TempDir::new().unwrap();
        let run_dir = write_report(
            root.path(),
            "mint-legacy",
            json!({
                "environment": {"id": "mint"},
                "status": "running",
                "qmp": {"port": 4400},
            }),
        );

        assert!(list_in_roots(std::iter::once(root.path())).is_empty());
        let unchanged: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("report.json")).unwrap())
                .unwrap();
        let marker: Value =
            serde_json::from_str(&fs::read_to_string(run_dir.join("stale.json")).unwrap()).unwrap();
        assert_eq!(unchanged["status"], "running");
        assert_eq!(marker["status"], "cleanup-incomplete");
        assert_eq!(marker["cleanup"]["phase"], "supervisor");
        assert!(marker["reason"]
            .as_str()
            .unwrap()
            .contains("supervisor PID"));
        assert!(!run_dir.join("report.running.json").exists());
    }

    #[test]
    fn missing_selector_reports_how_to_start_it() {
        let root = TempDir::new().unwrap();
        let error = match find(root.path(), "missing") {
            Ok(_) => panic!("missing selector unexpectedly resolved"),
            Err(error) => error.to_string(),
        };
        assert_eq!(
            error,
            "no running emu `missing`; start one with `qol emu up missing`"
        );
    }
}
