use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::Value;

const LEGACY_CLEANUP_BACKUP: &str = "report.legacy-cleanup.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportKind {
    Environment,
    Flow,
    ImageImport,
    EnvironmentBatch,
    FlowFanout,
    Unknown(String),
}

impl ReportKind {
    pub fn parse(value: &str) -> Self {
        match value {
            "environment" => Self::Environment,
            "flow" => Self::Flow,
            "image-import" => Self::ImageImport,
            "environment-batch" => Self::EnvironmentBatch,
            "flow-fanout" => Self::FlowFanout,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Environment => "environment",
            Self::Flow => "flow",
            Self::ImageImport => "image-import",
            Self::EnvironmentBatch => "environment-batch",
            Self::FlowFanout => "flow-fanout",
            Self::Unknown(value) => value,
        }
    }

    pub fn is_session(&self) -> bool {
        matches!(self, Self::EnvironmentBatch | Self::FlowFanout)
    }

    pub fn is_lane(&self) -> bool {
        matches!(self, Self::Environment | Self::Flow | Self::ImageImport)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportStatus {
    Preparing,
    Starting,
    Running,
    Stopping,
    Recovering,
    Cancelling,
    Pass,
    Failed,
    Skipped,
    Cancelled,
    Stopped,
    Abandoned,
    CleanupIncomplete,
    RollbackIncomplete,
    CancellationCleanupIncomplete,
    Unknown(String),
}

impl ReportStatus {
    pub fn parse(value: &str) -> Self {
        match value {
            "preparing" => Self::Preparing,
            "starting" => Self::Starting,
            "running" => Self::Running,
            "stopping" => Self::Stopping,
            "recovering" => Self::Recovering,
            "cancelling" => Self::Cancelling,
            "pass" => Self::Pass,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            "cancelled" => Self::Cancelled,
            "stopped" => Self::Stopped,
            "abandoned" => Self::Abandoned,
            "cleanup-incomplete" => Self::CleanupIncomplete,
            "rollback-incomplete" => Self::RollbackIncomplete,
            "cancellation-cleanup-incomplete" => Self::CancellationCleanupIncomplete,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Preparing => "preparing",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Recovering => "recovering",
            Self::Cancelling => "cancelling",
            Self::Pass => "pass",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
            Self::Stopped => "stopped",
            Self::Abandoned => "abandoned",
            Self::CleanupIncomplete => "cleanup-incomplete",
            Self::RollbackIncomplete => "rollback-incomplete",
            Self::CancellationCleanupIncomplete => "cancellation-cleanup-incomplete",
            Self::Unknown(value) => value,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Preparing
                | Self::Starting
                | Self::Running
                | Self::Stopping
                | Self::Recovering
                | Self::Cancelling
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Pass
                | Self::Failed
                | Self::Skipped
                | Self::Cancelled
                | Self::Stopped
                | Self::Abandoned
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CleanupState {
    Pending,
    Complete,
    Incomplete(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunConcern {
    HistoricalFailure,
    UnresolvedCleanup,
}

impl RunConcern {
    pub fn requires_attention(self) -> bool {
        matches!(self, Self::UnresolvedCleanup)
    }
}

impl CleanupState {
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOwner {
    pub pid: Option<u32>,
    pub process_identity: Option<String>,
    pub state: Option<String>,
    pub worktree: Option<PathBuf>,
    pub task: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RunReport {
    pub report_path: PathBuf,
    pub run_id: String,
    pub kind: ReportKind,
    pub status: ReportStatus,
    pub environment_id: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub finished_at_unix_ms: Option<u64>,
    pub owner: RunOwner,
    pub cleanup: CleanupState,
    pub requested_lanes: Option<u64>,
    pub reported_lanes: Option<u64>,
    pub log_path: Option<PathBuf>,
    pub error: Option<String>,
    document: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSummary {
    pub report_path: PathBuf,
    pub run_id: String,
    pub kind: ReportKind,
    pub status: ReportStatus,
    pub environment_id: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub finished_at_unix_ms: Option<u64>,
    pub owner: RunOwner,
    pub cleanup: CleanupState,
    pub requested_lanes: Option<u64>,
    pub reported_lanes: Option<u64>,
    pub log_path: Option<PathBuf>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LegacyCleanupRepair {
    NotApplicable,
    Repaired { backup_path: PathBuf },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LegacyCleanupFormat {
    Lifecycle,
    OrphanReconciliation,
}

impl LegacyCleanupFormat {
    fn source(self) -> &'static str {
        match self {
            Self::Lifecycle => "qol-emu-legacy-lifecycle-v1",
            Self::OrphanReconciliation => "qol-emu-legacy-orphan-reconciliation-v1",
        }
    }
}

impl RunReport {
    pub fn summary(&self) -> RunSummary {
        RunSummary {
            report_path: self.report_path.clone(),
            run_id: self.run_id.clone(),
            kind: self.kind.clone(),
            status: self.status.clone(),
            environment_id: self.environment_id.clone(),
            started_at_unix_ms: self.started_at_unix_ms,
            finished_at_unix_ms: self.finished_at_unix_ms,
            owner: self.owner.clone(),
            cleanup: self.cleanup.clone(),
            requested_lanes: self.requested_lanes,
            reported_lanes: self.reported_lanes,
            log_path: self.log_path.clone(),
            error: self.error.clone(),
        }
    }

    pub fn document(&self) -> &Value {
        &self.document
    }

    pub fn owned_lane_run_ids(&self) -> Result<Vec<String>> {
        session_lane_run_ids(&self.document, &self.kind)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("invalid lane ownership in {}", self.report_path.display()))
    }
}

impl RunSummary {
    pub fn observed_at_unix_ms(&self) -> u64 {
        self.finished_at_unix_ms
            .or(self.started_at_unix_ms)
            .unwrap_or_default()
    }

    pub fn concern(&self) -> Option<RunConcern> {
        if matches!(self.cleanup, CleanupState::Incomplete(_)) {
            return Some(RunConcern::UnresolvedCleanup);
        }
        if matches!(self.cleanup, CleanupState::Pending) && !self.status.is_active() {
            return Some(RunConcern::UnresolvedCleanup);
        }
        if matches!(self.status, ReportStatus::Failed | ReportStatus::Abandoned) {
            return Some(RunConcern::HistoricalFailure);
        }
        None
    }

    pub fn needs_attention(&self) -> bool {
        self.concern().is_some_and(RunConcern::requires_attention)
    }
}

pub fn read_report(path: &Path) -> Result<Option<RunReport>> {
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };
    parse_report(path, &content).map(Some)
}

pub fn repair_legacy_cleanup_report(path: &Path) -> Result<LegacyCleanupRepair> {
    let initial = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(LegacyCleanupRepair::NotApplicable)
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };
    let initial_document: Value = serde_json::from_slice(&initial)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if legacy_cleanup_format(path, &initial_document)?.is_none() {
        return Ok(LegacyCleanupRepair::NotApplicable);
    }
    let run_dir = path
        .parent()
        .with_context(|| format!("report has no run directory: {}", path.display()))?;
    let _lock = crate::run_dir::lock_run_directory(run_dir, "cleanup-repair.lock")?;
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(LegacyCleanupRepair::NotApplicable)
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };
    let mut document: Value = serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(format) = legacy_cleanup_format(path, &document)? else {
        return Ok(LegacyCleanupRepair::NotApplicable);
    };
    let backup_path = run_dir.join(LEGACY_CLEANUP_BACKUP);
    if !backup_path.exists() {
        qol_fs::atomic_write(&backup_path, &content)
            .with_context(|| format!("failed to write {}", backup_path.display()))?;
    }
    let upgraded_at_unix_ms = crate::unix_millis()?;
    let teardown = document
        .get_mut("teardown")
        .and_then(Value::as_object_mut)
        .context("legacy teardown must be an object")?;
    teardown.insert("status".to_string(), Value::String("complete".to_string()));
    teardown.insert("qemu_exit_verified".to_string(), Value::Bool(true));
    teardown.insert("tree_exit_verified".to_string(), Value::Bool(true));
    document["cleanup_proof_upgrade"] = serde_json::json!({
        "source": format.source(),
        "upgraded_at_unix_ms": upgraded_at_unix_ms,
        "evidence_report": backup_path,
    });
    let upgraded = serde_json::to_vec_pretty(&document).context("failed to serialize report")?;
    let parsed = parse_report(path, &upgraded)?;
    if parsed.cleanup != CleanupState::Complete {
        bail!("legacy cleanup upgrade did not produce typed cleanup proof");
    }
    qol_fs::atomic_write(path, &[upgraded.as_slice(), b"\n"].concat())
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(LegacyCleanupRepair::Repaired { backup_path })
}

fn legacy_cleanup_format(path: &Path, document: &Value) -> Result<Option<LegacyCleanupFormat>> {
    if path.file_name().and_then(|name| name.to_str()) != Some("report.json") {
        return Ok(None);
    }
    let run_dir = path
        .parent()
        .with_context(|| format!("report has no run directory: {}", path.display()))?;
    let directory_id = run_dir.file_name().and_then(|name| name.to_str());
    let run_id = document.get("run_id").and_then(Value::as_str);
    if run_id.is_none() || run_id != directory_id {
        return Ok(None);
    }
    let kind = document.get("kind").and_then(Value::as_str);
    if !matches!(kind, Some("environment" | "flow")) {
        return Ok(None);
    }
    let status = document
        .get("status")
        .and_then(Value::as_str)
        .map(ReportStatus::parse);
    if !status.is_some_and(|status| status.is_terminal()) {
        return Ok(None);
    }
    if document
        .get("name")
        .and_then(Value::as_str)
        .is_none_or(|name| !name.starts_with("qol-emu-"))
    {
        return Ok(None);
    }
    if document
        .get("finished_at_unix_ms")
        .and_then(Value::as_u64)
        .is_none()
    {
        return Ok(None);
    }
    let Some(teardown) = document.get("teardown").and_then(Value::as_object) else {
        return Ok(None);
    };
    let Some(removed) = teardown.get("removed").and_then(Value::as_array) else {
        return Ok(None);
    };
    for value in removed {
        let Some(removed_path) = value.as_str().map(Path::new) else {
            return Ok(None);
        };
        let Some(name) = removed_path.file_name().and_then(|name| name.to_str()) else {
            return Ok(None);
        };
        let generated = name == "overlay.qcow2"
            || name == "usb-stick.raw"
            || (name.starts_with("overlay-snap-") && name.ends_with(".qcow2"));
        if !generated || removed_path != run_dir.join(name) || removed_path.exists() {
            return Ok(None);
        }
    }
    let lifecycle = !teardown.contains_key("status")
        && teardown
            .keys()
            .all(|key| matches!(key.as_str(), "exit" | "removed"))
        && teardown
            .get("exit")
            .and_then(Value::as_str)
            .is_some_and(|exit| exit.starts_with("exit status:"));
    if lifecycle {
        return Ok(Some(LegacyCleanupFormat::Lifecycle));
    }
    if legacy_orphan_reconciliation_is_complete(run_dir, document, teardown) {
        return Ok(Some(LegacyCleanupFormat::OrphanReconciliation));
    }
    Ok(None)
}

fn legacy_orphan_reconciliation_is_complete(
    run_dir: &Path,
    document: &Value,
    teardown: &serde_json::Map<String, Value>,
) -> bool {
    let expected_keys = [
        "machine_name",
        "phase",
        "qemu_exit_verified",
        "qemu_was_alive",
        "removed",
        "status",
    ];
    if teardown.len() != expected_keys.len()
        || !expected_keys.iter().all(|key| teardown.contains_key(*key))
        || teardown.get("status").and_then(Value::as_str) != Some("complete")
        || teardown.get("phase").and_then(Value::as_str) != Some("complete")
        || teardown.get("qemu_exit_verified").and_then(Value::as_bool) != Some(true)
        || teardown.contains_key("tree_exit_verified")
    {
        return false;
    }
    let Some(reconciliation) = document.get("reconciliation") else {
        return false;
    };
    if reconciliation.get("cleanup") != document.get("teardown")
        || reconciliation
            .get("previous_status")
            .and_then(Value::as_str)
            .is_none_or(|status| !ReportStatus::parse(status).is_active())
    {
        return false;
    }
    [
        ("evidence_report", "report.running.json"),
        ("stale_marker", "stale.json"),
    ]
    .into_iter()
    .all(|(field, name)| {
        reconciliation
            .get(field)
            .and_then(Value::as_str)
            .map(Path::new)
            .is_some_and(|path| path == run_dir.join(name) && path.is_file())
    })
}

pub fn parse_report(path: &Path, content: &[u8]) -> Result<RunReport> {
    let document: Value = serde_json::from_slice(content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    normalize_report(path, document)
}

pub fn read_report_checked(
    path: &Path,
    expected_run_id: &str,
    expected_kind: &ReportKind,
) -> Result<Option<RunReport>> {
    let Some(report) = read_report(path)? else {
        return Ok(None);
    };
    if report.run_id != expected_run_id {
        bail!(
            "report {} belongs to run `{}`, expected `{expected_run_id}`",
            path.display(),
            report.run_id
        );
    }
    if report.kind != *expected_kind {
        bail!(
            "report {} has kind `{}`, expected `{}`",
            path.display(),
            report.kind.as_str(),
            expected_kind.as_str()
        );
    }
    Ok(Some(report))
}

fn normalize_report(path: &Path, document: Value) -> Result<RunReport> {
    let run_id = required_string(&document, "run_id", path)?;
    let kind = ReportKind::parse(&required_string(&document, "kind", path)?);
    let status = ReportStatus::parse(&required_string(&document, "status", path)?);
    let cleanup = cleanup_state(&document, &kind, &status);
    let requested_lanes = requested_lanes(&document, &kind);
    let reported_lanes = reported_lanes(&document, &kind);
    Ok(RunReport {
        report_path: path.to_path_buf(),
        run_id,
        kind,
        status,
        environment_id: nested_string(&document, &["environment", "id"]),
        started_at_unix_ms: document.get("started_at_unix_ms").and_then(Value::as_u64),
        finished_at_unix_ms: document.get("finished_at_unix_ms").and_then(Value::as_u64),
        owner: owner(&document),
        cleanup,
        requested_lanes,
        reported_lanes,
        log_path: log_path(&document),
        error: document
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string),
        document,
    })
}

fn required_string(document: &Value, key: &str, path: &Path) -> Result<String> {
    document
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("report {} has no {key}", path.display()))
}

fn nested_string(document: &Value, path: &[&str]) -> Option<String> {
    let mut value = document;
    for key in path {
        value = value.get(key)?;
    }
    value.as_str().map(str::to_string)
}

fn owner(document: &Value) -> RunOwner {
    let owner = document.get("owner");
    RunOwner {
        pid: owner
            .and_then(|value| value.get("pid"))
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok()),
        process_identity: owner
            .and_then(|value| value.get("process_identity"))
            .and_then(Value::as_str)
            .map(str::to_string),
        state: owner
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str)
            .map(str::to_string),
        worktree: owner
            .and_then(|value| value.get("worktree"))
            .and_then(Value::as_str)
            .map(PathBuf::from),
        task: owner
            .and_then(|value| value.get("task"))
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn requested_lanes(document: &Value, kind: &ReportKind) -> Option<u64> {
    match kind {
        ReportKind::EnvironmentBatch => document
            .get("launch")
            .and_then(|value| value.get("count"))
            .and_then(Value::as_u64),
        ReportKind::FlowFanout => document
            .get("workflow")
            .and_then(|value| value.get("repeat"))
            .and_then(Value::as_u64),
        ReportKind::Environment
        | ReportKind::Flow
        | ReportKind::ImageImport
        | ReportKind::Unknown(_) => Some(1),
    }
}

fn reported_lanes(document: &Value, kind: &ReportKind) -> Option<u64> {
    let field = match kind {
        ReportKind::EnvironmentBatch => "runs",
        ReportKind::FlowFanout => "lanes",
        ReportKind::Environment
        | ReportKind::Flow
        | ReportKind::ImageImport
        | ReportKind::Unknown(_) => return Some(1),
    };
    document
        .get(field)
        .and_then(Value::as_array)
        .and_then(|lanes| u64::try_from(lanes.len()).ok())
}

fn session_lane_run_ids(
    document: &Value,
    kind: &ReportKind,
) -> std::result::Result<Vec<String>, String> {
    let field = match kind {
        ReportKind::EnvironmentBatch => "runs",
        ReportKind::FlowFanout => "lanes",
        ReportKind::Environment
        | ReportKind::Flow
        | ReportKind::ImageImport
        | ReportKind::Unknown(_) => {
            return Err(format!(
                "report kind `{}` does not own lanes",
                kind.as_str()
            ))
        }
    };
    let requested = requested_lanes(document, kind)
        .ok_or_else(|| "report has no independent requested lane count".to_string())?;
    if requested == 0 {
        return Err("report requested zero lanes".to_string());
    }
    let lanes = document
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("report has no {field}"))?;
    if Some(requested) != u64::try_from(lanes.len()).ok() {
        return Err("report does not contain every requested lane".to_string());
    }
    validated_lane_run_ids(lanes)
}

fn validated_lane_run_ids(lanes: &[Value]) -> std::result::Result<Vec<String>, String> {
    let mut seen = BTreeSet::new();
    let mut run_ids = Vec::with_capacity(lanes.len());
    for (index, lane) in lanes.iter().enumerate() {
        let run_id = lane
            .get("run_id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("lane {} has no run_id", index + 1))?;
        crate::validate_run_id(run_id)
            .map_err(|_| format!("lane {} has an invalid run_id", index + 1))?;
        if !seen.insert(run_id) {
            return Err(format!("report contains duplicate lane `{run_id}`"));
        }
        run_ids.push(run_id.to_string());
    }
    Ok(run_ids)
}

fn cleanup_state(document: &Value, kind: &ReportKind, status: &ReportStatus) -> CleanupState {
    if matches!(
        kind,
        ReportKind::Environment | ReportKind::Flow | ReportKind::ImageImport
    ) {
        return child_cleanup(document, status);
    }
    if status.is_active() || matches!(status, ReportStatus::Unknown(_)) {
        return CleanupState::Pending;
    }
    if matches!(
        status,
        ReportStatus::CleanupIncomplete
            | ReportStatus::RollbackIncomplete
            | ReportStatus::CancellationCleanupIncomplete
    ) {
        return CleanupState::Incomplete(cleanup_error(document));
    }
    match kind {
        ReportKind::EnvironmentBatch => batch_cleanup(document),
        ReportKind::FlowFanout => fanout_cleanup(document),
        ReportKind::Environment | ReportKind::Flow | ReportKind::ImageImport => unreachable!(),
        ReportKind::Unknown(_) => CleanupState::Pending,
    }
}

fn batch_cleanup(document: &Value) -> CleanupState {
    let owned = match session_lane_run_ids(document, &ReportKind::EnvironmentBatch) {
        Ok(owned) => owned,
        Err(error) => return CleanupState::Incomplete(error),
    };
    let Some(teardown) = document.get("teardown") else {
        return CleanupState::Incomplete("terminal environment report has no teardown".to_string());
    };
    if teardown.get("status").and_then(Value::as_str) != Some("complete") {
        return CleanupState::Incomplete(cleanup_error(document));
    }
    let Some(lanes) = teardown.get("lanes").and_then(Value::as_array) else {
        return CleanupState::Incomplete("terminal environment teardown has no lanes".to_string());
    };
    let teardown_ids = match validated_lane_run_ids(lanes) {
        Ok(ids) => ids,
        Err(error) => return CleanupState::Incomplete(error),
    };
    if BTreeSet::from_iter(owned) != BTreeSet::from_iter(teardown_ids) {
        return CleanupState::Incomplete(
            "environment teardown does not cover every owned lane".to_string(),
        );
    }
    if !lanes.iter().all(environment_teardown_lane_verified) {
        return CleanupState::Incomplete(
            "one or more environment lanes lack verified cleanup".to_string(),
        );
    }
    let Some(payload) = document.get("payload").filter(|payload| !payload.is_null()) else {
        return CleanupState::Complete;
    };
    if payload
        .get("cleanup")
        .and_then(|cleanup| cleanup.get("complete"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return CleanupState::Complete;
    }
    CleanupState::Incomplete("environment payload cleanup is incomplete".to_string())
}

fn fanout_cleanup(document: &Value) -> CleanupState {
    if let Err(error) = session_lane_run_ids(document, &ReportKind::FlowFanout) {
        return CleanupState::Incomplete(error);
    }
    let Some(lanes) = document.get("lanes").and_then(Value::as_array) else {
        return CleanupState::Incomplete("terminal flow report has no lanes".to_string());
    };
    let complete = lanes.iter().all(|lane| {
        lane.get("cleanup")
            .and_then(|value| value.get("complete"))
            .and_then(Value::as_bool)
            == Some(true)
    });
    if !complete {
        return CleanupState::Incomplete(
            "one or more flow lanes lack verified cleanup".to_string(),
        );
    }
    if let Some(preparation) = document.get("preparation") {
        let complete = ["build", "iso"].into_iter().all(|phase| {
            preparation
                .get(phase)
                .and_then(|phase| phase.get("cleanup"))
                .and_then(|cleanup| cleanup.get("complete"))
                .and_then(Value::as_bool)
                == Some(true)
        });
        if !complete {
            return CleanupState::Incomplete(
                "flow preparation process cleanup is incomplete".to_string(),
            );
        }
    }
    let Some(payload) = document.get("payload").filter(|payload| !payload.is_null()) else {
        return CleanupState::Complete;
    };
    if payload
        .get("cleanup")
        .and_then(|cleanup| cleanup.get("complete"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return CleanupState::Complete;
    }
    CleanupState::Incomplete("flow payload cleanup is incomplete".to_string())
}

fn environment_teardown_lane_verified(lane: &Value) -> bool {
    if lane.get("status").and_then(Value::as_str) != Some("pass")
        || lane.get("stop_error").is_some_and(|error| !error.is_null())
    {
        return false;
    }
    match lane.get("verification").and_then(Value::as_str) {
        Some("not-started") => lane
            .get("report_status")
            .is_none_or(|status| status.is_null()),
        Some("verified-cleanup") => lane
            .get("report_status")
            .and_then(Value::as_str)
            .map(ReportStatus::parse)
            .is_some_and(|status| status.is_terminal()),
        _ => false,
    }
}

pub fn child_cleanup(document: &Value, status: &ReportStatus) -> CleanupState {
    if status.is_active() || matches!(status, ReportStatus::Unknown(_)) {
        return CleanupState::Pending;
    }
    if matches!(
        status,
        ReportStatus::CleanupIncomplete
            | ReportStatus::RollbackIncomplete
            | ReportStatus::CancellationCleanupIncomplete
    ) {
        return CleanupState::Incomplete(cleanup_error(document));
    }
    let Some(teardown) = document.get("teardown").filter(|value| !value.is_null()) else {
        return CleanupState::Incomplete(
            "terminal child report has no teardown evidence".to_string(),
        );
    };
    let complete = teardown.get("status").and_then(Value::as_str) == Some("complete");
    let qemu_exit = teardown.get("qemu_exit_verified").and_then(Value::as_bool) == Some(true);
    let tree_exit = teardown.get("tree_exit_verified").and_then(Value::as_bool) == Some(true);
    let staging_removed = document.get("kind").and_then(Value::as_str) != Some("image-import")
        || teardown.get("staging_removed").and_then(Value::as_bool) == Some(true);
    if complete && qemu_exit && tree_exit && staging_removed {
        return CleanupState::Complete;
    }
    CleanupState::Incomplete(
        "terminal child lacks verified process-tree exit or artifact cleanup".to_string(),
    )
}

fn cleanup_error(document: &Value) -> String {
    document
        .get("teardown")
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
        .or_else(|| document.get("error").and_then(Value::as_str))
        .unwrap_or("cleanup is incomplete")
        .to_string()
}

fn log_path(document: &Value) -> Option<PathBuf> {
    document
        .get("artifacts")
        .and_then(|value| value.get("run_log"))
        .and_then(Value::as_str)
        .or_else(|| {
            document
                .get("artifacts")
                .and_then(|value| value.get("qemu_log"))
                .and_then(Value::as_str)
        })
        .or_else(|| document.get("log").and_then(Value::as_str))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn parse(value: Value) -> RunReport {
        normalize_report(Path::new("/runs/report.json"), value).unwrap()
    }

    fn clean_environment_batch(run_id: &str, status: &str) -> Value {
        json!({
            "kind": "environment-batch",
            "run_id": run_id,
            "status": status,
            "launch": { "count": 1 },
            "runs": [{ "run_id": "lane-a" }],
            "teardown": {
                "status": "complete",
                "lanes": [{
                    "run_id": "lane-a",
                    "status": "pass",
                    "verification": "verified-cleanup",
                    "report_status": "stopped",
                    "stop_error": null
                }]
            }
        })
    }

    #[test]
    fn status_classification_keeps_activity_separate_from_terminal_history() {
        let cases = [
            ("running", true, false),
            ("recovering", true, false),
            ("pass", false, true),
            ("failed", false, true),
            ("cleanup-incomplete", false, false),
            ("future-status", false, false),
        ];
        for (value, active, terminal) in cases {
            let status = ReportStatus::parse(value);
            assert_eq!(status.is_active(), active, "status: {value}");
            assert_eq!(status.is_terminal(), terminal, "status: {value}");
        }
    }

    #[test]
    fn image_import_is_a_child_lane_not_a_session_owner() {
        assert!(ReportKind::ImageImport.is_lane());
        assert!(!ReportKind::ImageImport.is_session());
        assert_eq!(
            requested_lanes(&json!({}), &ReportKind::ImageImport),
            Some(1)
        );
        assert_eq!(
            reported_lanes(&json!({}), &ReportKind::ImageImport),
            Some(1)
        );
        assert!(session_lane_run_ids(&json!({}), &ReportKind::ImageImport).is_err());
    }

    #[test]
    fn run_concerns_separate_historical_failures_from_unresolved_cleanup() {
        let cases = [
            (
                clean_environment_batch("failed-clean", "failed"),
                Some(RunConcern::HistoricalFailure),
                false,
            ),
            (
                json!({
                    "kind": "environment",
                    "run_id": "abandoned-clean",
                    "status": "abandoned",
                    "teardown": {
                        "status": "complete",
                        "qemu_exit_verified": true,
                        "tree_exit_verified": true
                    }
                }),
                Some(RunConcern::HistoricalFailure),
                false,
            ),
            (
                json!({
                    "kind": "environment",
                    "run_id": "failed-dirty",
                    "status": "failed"
                }),
                Some(RunConcern::UnresolvedCleanup),
                true,
            ),
            (
                json!({
                    "kind": "environment",
                    "run_id": "pass-dirty",
                    "status": "pass"
                }),
                Some(RunConcern::UnresolvedCleanup),
                true,
            ),
            (
                json!({
                    "kind": "environment",
                    "run_id": "explicit-dirty",
                    "status": "cleanup-incomplete",
                    "teardown": { "error": "qemu still alive" }
                }),
                Some(RunConcern::UnresolvedCleanup),
                true,
            ),
            (
                json!({
                    "kind": "environment",
                    "run_id": "future",
                    "status": "future-status"
                }),
                Some(RunConcern::UnresolvedCleanup),
                true,
            ),
            (
                json!({
                    "kind": "environment",
                    "run_id": "running",
                    "status": "running"
                }),
                None,
                false,
            ),
            (
                json!({
                    "kind": "environment",
                    "run_id": "pass-clean",
                    "status": "pass",
                    "teardown": {
                        "status": "complete",
                        "qemu_exit_verified": true,
                        "tree_exit_verified": true
                    }
                }),
                None,
                false,
            ),
        ];
        for (document, concern, attention) in cases {
            let summary = parse(document).summary();
            assert_eq!(summary.concern(), concern, "run: {}", summary.run_id);
            assert_eq!(
                summary.needs_attention(),
                attention,
                "run: {}",
                summary.run_id
            );
        }
    }

    #[test]
    fn normalizes_cleanup_across_existing_report_shapes() {
        let cases = [
            (
                clean_environment_batch("env", "stopped"),
                CleanupState::Complete,
            ),
            (
                {
                    let mut report = clean_environment_batch("env-payload-clean", "stopped");
                    report["payload"] = json!({ "cleanup": { "complete": true } });
                    report
                },
                CleanupState::Complete,
            ),
            (
                {
                    let mut report = clean_environment_batch("env-payload-dirty", "stopped");
                    report["payload"] = json!({ "cleanup": { "complete": false } });
                    report
                },
                CleanupState::Incomplete("environment payload cleanup is incomplete".to_string()),
            ),
            (
                json!({
                    "kind": "flow-fanout",
                    "run_id": "flow",
                    "status": "pass",
                    "workflow": { "repeat": 2 },
                    "lanes": [
                        { "run_id": "lane-a", "cleanup": { "complete": true } },
                        { "run_id": "lane-b", "cleanup": { "complete": true } }
                    ]
                }),
                CleanupState::Complete,
            ),
            (
                json!({
                    "kind": "flow",
                    "run_id": "child",
                    "status": "pass",
                    "teardown": {
                        "status": "complete",
                        "qemu_exit_verified": true,
                        "tree_exit_verified": true,
                        "removed": []
                    }
                }),
                CleanupState::Complete,
            ),
            (
                json!({
                    "kind": "flow-fanout",
                    "run_id": "payload-clean",
                    "status": "failed",
                    "workflow": { "repeat": 1 },
                    "lanes": [{ "run_id": "lane-a", "cleanup": { "complete": true } }],
                    "payload": { "cleanup": { "complete": true } }
                }),
                CleanupState::Complete,
            ),
            (
                json!({
                    "kind": "flow-fanout",
                    "run_id": "payload-dirty",
                    "status": "failed",
                    "workflow": { "repeat": 1 },
                    "lanes": [{ "run_id": "lane-a", "cleanup": { "complete": true } }],
                    "payload": { "cleanup": { "complete": false } }
                }),
                CleanupState::Incomplete("flow payload cleanup is incomplete".to_string()),
            ),
            (
                json!({
                    "kind": "flow-fanout",
                    "run_id": "preparation-dirty",
                    "status": "failed",
                    "workflow": { "repeat": 1 },
                    "lanes": [{ "run_id": "lane-a", "cleanup": { "complete": true } }],
                    "preparation": {
                        "build": { "cleanup": { "complete": true } },
                        "iso": { "cleanup": { "complete": false } }
                    }
                }),
                CleanupState::Incomplete(
                    "flow preparation process cleanup is incomplete".to_string(),
                ),
            ),
            (
                json!({
                    "kind": "environment",
                    "run_id": "unknown",
                    "status": "new-status",
                    "teardown": { "removed": [] }
                }),
                CleanupState::Pending,
            ),
        ];
        for (document, expected) in cases {
            assert_eq!(parse(document).cleanup, expected);
        }
    }

    #[test]
    fn terminal_child_cleanup_requires_every_explicit_proof_field() {
        let cases = [
            ("missing", json!(null)),
            ("legacy", json!({ "removed": [] })),
            ("status-only", json!({ "status": "complete" })),
            (
                "missing-tree-proof",
                json!({
                    "status": "complete",
                    "qemu_exit_verified": true
                }),
            ),
            (
                "qemu-not-exited",
                json!({
                    "status": "complete",
                    "qemu_exit_verified": false,
                    "tree_exit_verified": true
                }),
            ),
            (
                "tree-not-exited",
                json!({
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": false
                }),
            ),
        ];
        for kind in ["environment", "flow"] {
            for status in [
                "pass",
                "failed",
                "skipped",
                "cancelled",
                "stopped",
                "abandoned",
            ] {
                for (case, teardown) in &cases {
                    let report = parse(json!({
                        "kind": kind,
                        "run_id": format!("{kind}-{status}-{case}"),
                        "status": status,
                        "teardown": teardown,
                    }));
                    assert!(
                        matches!(report.cleanup, CleanupState::Incomplete(_)),
                        "kind={kind} status={status} case={case}"
                    );
                }
                let report = parse(json!({
                    "kind": kind,
                    "run_id": format!("{kind}-{status}-verified"),
                    "status": status,
                    "teardown": {
                        "status": "complete",
                        "qemu_exit_verified": true,
                        "tree_exit_verified": true
                    }
                }));
                assert_eq!(
                    report.cleanup,
                    CleanupState::Complete,
                    "kind={kind} status={status}"
                );
            }
        }
    }

    #[test]
    fn doctor_repairs_the_known_legacy_lifecycle_contract_and_preserves_evidence() {
        let root = tempdir().unwrap();
        let run_dir = root.path().join("cases/lane-a");
        fs::create_dir_all(&run_dir).unwrap();
        let report_path = run_dir.join("report.json");
        let legacy = json!({
            "name": "qol-emu-run",
            "kind": "flow",
            "run_id": "lane-a",
            "status": "pass",
            "finished_at_unix_ms": 1,
            "runtime": {
                "supervisor_pid": 10,
                "qemu_pid": 11
            },
            "teardown": {
                "exit": "exit status: 0",
                "removed": [
                    run_dir.join("overlay.qcow2"),
                    run_dir.join("usb-stick.raw")
                ]
            }
        });
        let original = format!("{}\n", serde_json::to_string_pretty(&legacy).unwrap());
        fs::write(&report_path, &original).unwrap();

        let result = repair_legacy_cleanup_report(&report_path).unwrap();
        let LegacyCleanupRepair::Repaired { backup_path } = result else {
            panic!("legacy report was not repaired");
        };
        assert_eq!(fs::read_to_string(backup_path).unwrap(), original);
        let repaired = read_report(&report_path).unwrap().unwrap();
        assert_eq!(repaired.cleanup, CleanupState::Complete);
        assert_eq!(
            repaired.document()["cleanup_proof_upgrade"]["source"],
            "qol-emu-legacy-lifecycle-v1"
        );
        assert_eq!(
            repair_legacy_cleanup_report(&report_path).unwrap(),
            LegacyCleanupRepair::NotApplicable
        );
    }

    #[test]
    fn doctor_repairs_legacy_orphan_reconciliation_with_durable_evidence() {
        let root = tempdir().unwrap();
        let run_dir = root.path().join("cases/lane-a");
        fs::create_dir_all(&run_dir).unwrap();
        let report_path = run_dir.join("report.json");
        let evidence_path = run_dir.join("report.running.json");
        let stale_path = run_dir.join("stale.json");
        fs::write(&evidence_path, b"{}\n").unwrap();
        fs::write(&stale_path, b"{}\n").unwrap();
        let teardown = json!({
            "status": "complete",
            "phase": "complete",
            "qemu_exit_verified": true,
            "qemu_was_alive": false,
            "machine_name": null,
            "removed": []
        });
        let legacy = json!({
            "name": "qol-emu-run",
            "kind": "flow",
            "run_id": "lane-a",
            "status": "abandoned",
            "finished_at_unix_ms": 1,
            "teardown": teardown,
            "reconciliation": {
                "cleanup": teardown,
                "previous_status": "running",
                "evidence_report": evidence_path,
                "stale_marker": stale_path
            }
        });
        fs::write(
            &report_path,
            format!("{}\n", serde_json::to_string_pretty(&legacy).unwrap()),
        )
        .unwrap();

        let result = repair_legacy_cleanup_report(&report_path).unwrap();

        assert!(matches!(result, LegacyCleanupRepair::Repaired { .. }));
        let repaired = read_report(&report_path).unwrap().unwrap();
        assert_eq!(repaired.cleanup, CleanupState::Complete);
        assert_eq!(
            repaired.document()["cleanup_proof_upgrade"]["source"],
            "qol-emu-legacy-orphan-reconciliation-v1"
        );
    }

    #[test]
    fn doctor_refuses_legacy_cleanup_without_the_old_lifecycle_invariants() {
        let root = tempdir().unwrap();
        let run_dir = root.path().join("cases/lane-a");
        fs::create_dir_all(&run_dir).unwrap();
        let report_path = run_dir.join("report.json");
        let cases = [
            json!({
                "name": "qol-emu-run",
                "kind": "flow",
                "run_id": "lane-a",
                "status": "pass",
                "finished_at_unix_ms": 1,
                "teardown": { "removed": [] }
            }),
            json!({
                "name": "qol-emu-run",
                "kind": "flow",
                "run_id": "lane-a",
                "status": "pass",
                "finished_at_unix_ms": 1,
                "teardown": {
                    "exit": "exit status: 0",
                    "removed": [root.path().join("outside.qcow2")]
                }
            }),
        ];
        for document in cases {
            fs::write(
                &report_path,
                format!("{}\n", serde_json::to_string_pretty(&document).unwrap()),
            )
            .unwrap();
            assert_eq!(
                repair_legacy_cleanup_report(&report_path).unwrap(),
                LegacyCleanupRepair::NotApplicable
            );
        }
        assert!(!run_dir.join(LEGACY_CLEANUP_BACKUP).exists());
        assert!(!run_dir.join("cleanup-repair.lock").exists());
    }

    #[test]
    fn terminal_image_import_cleanup_requires_staging_removal_proof() {
        let mut report = json!({
            "kind": "image-import",
            "run_id": "image-import-a",
            "status": "abandoned",
            "teardown": {
                "status": "complete",
                "qemu_exit_verified": true,
                "tree_exit_verified": true
            }
        });

        assert!(matches!(
            parse(report.clone()).cleanup,
            CleanupState::Incomplete(_)
        ));
        report["teardown"]["staging_removed"] = json!(true);
        assert_eq!(parse(report).cleanup, CleanupState::Complete);
    }

    #[test]
    fn checked_read_rejects_identity_and_kind_mismatches() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("report.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "kind": "environment-batch",
                "run_id": "owned",
                "status": "running"
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(read_report_checked(&path, "other", &ReportKind::EnvironmentBatch).is_err());
        assert!(read_report_checked(&path, "owned", &ReportKind::FlowFanout).is_err());
        assert!(
            read_report_checked(&path, "owned", &ReportKind::EnvironmentBatch)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn owned_lane_ids_require_complete_unique_valid_session_membership() {
        let cases = [
            (
                "valid-flow",
                json!({
                    "kind": "flow-fanout",
                    "run_id": "flow",
                    "status": "running",
                    "workflow": { "repeat": 2 },
                    "lanes": [{ "run_id": "lane-a" }, { "run_id": "lane-b" }]
                }),
                true,
            ),
            (
                "valid-environment",
                json!({
                    "kind": "environment-batch",
                    "run_id": "environment",
                    "status": "running",
                    "launch": { "count": 2 },
                    "runs": [{ "run_id": "lane-a" }, { "run_id": "lane-b" }]
                }),
                true,
            ),
            (
                "environment-without-independent-launch-count",
                json!({
                    "kind": "environment-batch",
                    "run_id": "environment",
                    "status": "running",
                    "resources": { "requested_lanes": 2 },
                    "runs": [{ "run_id": "lane-a" }, { "run_id": "lane-b" }]
                }),
                false,
            ),
            (
                "missing-lane",
                json!({
                    "kind": "flow-fanout",
                    "run_id": "flow",
                    "status": "running",
                    "workflow": { "repeat": 2 },
                    "lanes": [{ "run_id": "lane-a" }]
                }),
                false,
            ),
            (
                "duplicate-lane",
                json!({
                    "kind": "flow-fanout",
                    "run_id": "flow",
                    "status": "running",
                    "workflow": { "repeat": 2 },
                    "lanes": [{ "run_id": "lane-a" }, { "run_id": "lane-a" }]
                }),
                false,
            ),
            (
                "unsafe-lane",
                json!({
                    "kind": "flow-fanout",
                    "run_id": "flow",
                    "status": "running",
                    "workflow": { "repeat": 1 },
                    "lanes": [{ "run_id": "../lane-a" }]
                }),
                false,
            ),
        ];
        for (label, document, valid) in cases {
            let report = parse(document);
            assert_eq!(report.owned_lane_run_ids().is_ok(), valid, "{label}");
        }
    }

    #[test]
    fn terminal_environment_batch_cleanup_requires_complete_matching_lane_evidence() {
        let valid = clean_environment_batch("environment", "stopped");
        assert_eq!(parse(valid.clone()).cleanup, CleanupState::Complete);

        let mut truncated_runs = valid.clone();
        truncated_runs["launch"]["count"] = json!(2);
        assert!(matches!(
            parse(truncated_runs).cleanup,
            CleanupState::Incomplete(_)
        ));

        let mut missing_teardown_lane = valid.clone();
        missing_teardown_lane["teardown"]["lanes"] = json!([]);
        assert!(matches!(
            parse(missing_teardown_lane).cleanup,
            CleanupState::Incomplete(_)
        ));

        let mut duplicate_teardown_lane = valid.clone();
        duplicate_teardown_lane["launch"]["count"] = json!(2);
        duplicate_teardown_lane["runs"] = json!([{ "run_id": "lane-a" }, { "run_id": "lane-b" }]);
        let teardown_lane = duplicate_teardown_lane["teardown"]["lanes"][0].clone();
        duplicate_teardown_lane["teardown"]["lanes"] =
            json!([teardown_lane.clone(), teardown_lane]);
        assert!(matches!(
            parse(duplicate_teardown_lane).cleanup,
            CleanupState::Incomplete(_)
        ));

        let mut unverified_lane = valid;
        unverified_lane["teardown"]["lanes"][0]["verification"] = json!("timeout");
        assert!(matches!(
            parse(unverified_lane).cleanup,
            CleanupState::Incomplete(_)
        ));
    }

    #[test]
    fn terminal_fanout_cleanup_rejects_untrusted_lane_identity() {
        let cases = [
            (
                "missing-id",
                json!([
                    { "cleanup": { "complete": true } },
                    { "run_id": "lane-b", "cleanup": { "complete": true } }
                ]),
            ),
            (
                "duplicate-id",
                json!([
                    { "run_id": "lane-a", "cleanup": { "complete": true } },
                    { "run_id": "lane-a", "cleanup": { "complete": true } }
                ]),
            ),
            (
                "unsafe-id",
                json!([
                    { "run_id": "lane-a", "cleanup": { "complete": true } },
                    { "run_id": "../lane-b", "cleanup": { "complete": true } }
                ]),
            ),
        ];
        for (label, lanes) in cases {
            let report = parse(json!({
                "kind": "flow-fanout",
                "run_id": "flow",
                "status": "pass",
                "workflow": { "repeat": 2 },
                "lanes": lanes
            }));
            assert!(
                matches!(report.cleanup, CleanupState::Incomplete(_)),
                "{label}"
            );
        }
    }

    #[test]
    fn missing_reports_are_not_parse_failures() {
        assert!(read_report(Path::new("/missing/qol-report.json"))
            .unwrap()
            .is_none());
    }
}
