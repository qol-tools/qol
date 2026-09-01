use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use qol_terminal_sessions::cli::{CliLaunchProgram, CliSessionInterpreter, CliToolId};
use qol_terminal_sessions::{
    ScreenReader, SessionBinding, SessionFacts, SessionId, SessionInventory, SpawnIdentity,
    SpawnKey, SpawnRequest, SpawnSurface, TerminalSessionService,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const SURFACE_TAB: &str = "tab";
pub(super) const SURFACE_OS_WINDOW: &str = "os-window";
const READY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const READY_TIMEOUT_MS: u64 = 30_000;
const READY_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(250);
const SPAWN_TASK_READY_TIMEOUT: Duration = Duration::from_secs(60);
const SCOPE_SLICE: &str = "qol-agents.slice";
const SCOPE_WEIGHT_MIN: u32 = 1;
const SCOPE_WEIGHT_MAX: u32 = 10_000;
const SPAWN_CAP_DEFAULT_CPU_WEIGHT: u32 = 40;
const SPAWN_CAP_DEFAULT_IO_WEIGHT: u32 = 40;
const SCOPE_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

static KEY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize)]
pub(super) struct SpawnOutcome {
    pub(super) session: String,
    pub(super) tool: String,
    pub(super) key: String,
    pub(super) reused: bool,
    pub(super) cwd: String,
    pub(super) surface: String,
    pub(super) model: Option<String>,
    pub(super) title: String,
    pub(super) task_submitted: Option<bool>,
    pub(super) completion_marker: Option<String>,
    pub(super) screen: Option<String>,
    pub(super) next_command: Option<String>,
    pub(super) elapsed_ms: u128,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) background: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) autoclose: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) resume: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) resume_detail: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct LaneSpec {
    pub(super) key: String,
    pub(super) task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) silent_wake: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(super) struct LaneSetOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) group: Option<String>,
    pub(super) combined_report: bool,
    pub(super) lanes: Vec<SpawnOutcome>,
    pub(super) elapsed_ms: u128,
}

#[derive(Debug, PartialEq, Eq)]
enum SpawnDecision {
    Launch,
    Reuse(Box<SessionFacts>),
    Conflict(CliToolId),
    WrongHarness { described: CliToolId },
    Ambiguous(usize),
}

pub(super) fn surface_token(surface: SpawnSurface) -> &'static str {
    match surface {
        SpawnSurface::Tab => SURFACE_TAB,
        SpawnSurface::OsWindow => SURFACE_OS_WINDOW,
    }
}

fn parse_surface(token: &str) -> Option<SpawnSurface> {
    match token {
        SURFACE_TAB => Some(SpawnSurface::Tab),
        SURFACE_OS_WINDOW => Some(SpawnSurface::OsWindow),
        _ => None,
    }
}

fn resolve_surface(flag: Option<&str>, config: Option<SpawnSurface>) -> Result<SpawnSurface> {
    match flag {
        Some(token) => parse_surface(token).ok_or_else(|| {
            anyhow!("invalid surface `{token}`; expected `{SURFACE_TAB}` or `{SURFACE_OS_WINDOW}`")
        }),
        None => Ok(config.unwrap_or(SpawnSurface::Tab)),
    }
}

#[derive(serde::Deserialize)]
struct SpawnConfigFile {
    spawn_surface: Option<String>,
    spawn_model: Option<String>,
    allowed_models: Option<Vec<String>>,
    spawn_cap: Option<bool>,
    spawn_cpu_weight: Option<u32>,
    spawn_io_weight: Option<u32>,
    spawn_cpu_quota: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SpawnCapConfig {
    pub(super) enabled: bool,
    pub(super) cpu_weight: u32,
    pub(super) io_weight: u32,
    pub(super) cpu_quota: Option<String>,
}

impl Default for SpawnCapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_weight: SPAWN_CAP_DEFAULT_CPU_WEIGHT,
            io_weight: SPAWN_CAP_DEFAULT_IO_WEIGHT,
            cpu_quota: None,
        }
    }
}

pub(super) fn config_spawn_cap() -> Result<SpawnCapConfig> {
    let Some(config_dir) = qol_config::config_dir() else {
        return Ok(SpawnCapConfig::default());
    };
    config_spawn_cap_at(&config_dir.join("sessions.toml"))
}

fn config_spawn_cap_at(path: &Path) -> Result<SpawnCapConfig> {
    let mut cap = SpawnCapConfig::default();
    let encoded = match fs::read_to_string(path) {
        Ok(encoded) => encoded,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(cap),
        Err(error) => return Err(error).context("failed to read spawn cap config"),
    };
    let config: SpawnConfigFile =
        toml::from_str(&encoded).with_context(|| format!("failed to parse {}", path.display()))?;
    if let Some(enabled) = config.spawn_cap {
        cap.enabled = enabled;
    }
    if let Some(weight) = config.spawn_cpu_weight {
        validate_scope_weight(weight, "spawn_cpu_weight", path)?;
        cap.cpu_weight = weight;
    }
    if let Some(weight) = config.spawn_io_weight {
        validate_scope_weight(weight, "spawn_io_weight", path)?;
        cap.io_weight = weight;
    }
    if let Some(quota) = config.spawn_cpu_quota {
        if quota.trim().is_empty() {
            bail!(
                "spawn_cpu_quota must be a non-empty value such as `600%` in {}",
                path.display()
            );
        }
        cap.cpu_quota = Some(quota);
    }
    Ok(cap)
}

fn validate_scope_weight(weight: u32, key: &str, path: &Path) -> Result<()> {
    if (SCOPE_WEIGHT_MIN..=SCOPE_WEIGHT_MAX).contains(&weight) {
        return Ok(());
    }
    bail!(
        "{key} must be between {SCOPE_WEIGHT_MIN} and {SCOPE_WEIGHT_MAX} in {}",
        path.display()
    )
}

pub(super) fn wrap_launch(
    launch: &CliLaunchProgram,
    cap: Option<&SpawnCapConfig>,
) -> CliLaunchProgram {
    let Some(cap) = cap else {
        return launch.clone();
    };
    if !cap.enabled {
        return launch.clone();
    }
    let mut args = scope_property_args(cap, true);
    args.push("--".to_owned());
    args.push(launch.program.clone());
    args.extend(launch.args.iter().cloned());
    CliLaunchProgram {
        program: "systemd-run".to_owned(),
        args,
    }
}

fn scope_property_args(cap: &SpawnCapConfig, with_quota: bool) -> Vec<String> {
    let mut args = vec![
        "--user".to_owned(),
        "--scope".to_owned(),
        "--quiet".to_owned(),
        format!("--slice={SCOPE_SLICE}"),
        "-p".to_owned(),
        format!("CPUWeight={}", cap.cpu_weight),
        "-p".to_owned(),
        format!("IOWeight={}", cap.io_weight),
    ];
    if with_quota {
        if let Some(quota) = &cap.cpu_quota {
            args.push("-p".to_owned());
            args.push(format!("CPUQuota={quota}"));
        }
    }
    args
}

pub(super) fn resolve_spawn_cap(config: SpawnCapConfig) -> Option<SpawnCapConfig> {
    if !config.enabled {
        qol_runtime::probe!("CLI_SESSION_SPAWN", "event=cap_disabled reason=config");
        return None;
    }
    if probe_scope(&scope_property_args(&config, true)) {
        return Some(config);
    }
    if config.cpu_quota.is_some() && probe_scope(&scope_property_args(&config, false)) {
        let mut weight_only = config.clone();
        weight_only.cpu_quota = None;
        qol_runtime::probe!(
            "CLI_SESSION_SPAWN",
            "event=cap_quota_dropped reason=systemd_rejected_cpu_quota"
        );
        return Some(weight_only);
    }
    qol_runtime::probe!(
        "CLI_SESSION_SPAWN",
        "event=cap_disabled reason=systemd_scope_unavailable"
    );
    None
}

fn probe_scope(args: &[String]) -> bool {
    let mut command = process::Command::new("systemd-run");
    command
        .args(args)
        .arg("--")
        .arg("true")
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return false,
    };
    let deadline = Instant::now() + SCOPE_PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(READY_POLL_INTERVAL),
            Err(_) => return false,
        }
    }
}

fn apply_slice_properties(cap: Option<&SpawnCapConfig>) {
    let Some(cap) = cap else {
        return;
    };
    for slice in ["qol.slice", "qol-agents.slice"] {
        let mut command = process::Command::new("systemctl");
        let quota = cap.cpu_quota.as_deref().unwrap_or("");
        command
            .arg("--user")
            .arg("set-property")
            .arg(slice)
            .arg(format!("CPUWeight={}", cap.cpu_weight))
            .arg(format!("IOWeight={}", cap.io_weight))
            .arg(format!("CPUQuota={quota}"));
        let _ = command
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .status();
    }
}

pub(super) fn config_surface() -> Result<Option<SpawnSurface>> {
    let Some(config_dir) = qol_config::config_dir() else {
        return Ok(None);
    };
    config_surface_at(&config_dir.join("sessions.toml"))
}

fn config_surface_at(path: &Path) -> Result<Option<SpawnSurface>> {
    let encoded = match fs::read_to_string(path) {
        Ok(encoded) => encoded,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to read spawn surface config"),
    };
    let config: SpawnConfigFile =
        toml::from_str(&encoded).with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(token) = config.spawn_surface else {
        return Ok(None);
    };
    let surface = parse_surface(&token).ok_or_else(|| {
        anyhow!(
            "invalid spawn_surface `{token}` in {}; expected `{SURFACE_TAB}` or `{SURFACE_OS_WINDOW}`",
            path.display()
        )
    })?;
    Ok(Some(surface))
}

pub(super) fn sessions_config_path() -> Option<std::path::PathBuf> {
    sessions_config_candidates()
        .into_iter()
        .find(|path| path.exists())
}

fn sessions_config_candidates() -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Some(config_dir) = qol_config::config_dir() {
        candidates.push(config_dir.join("sessions.toml"));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(
            home.join(".config")
                .join(qol_config::NAMESPACE)
                .join("sessions.toml"),
        );
    }
    candidates
}

pub(super) fn config_spawn_model() -> Result<Option<String>> {
    let Some(path) = sessions_config_path() else {
        return Ok(None);
    };
    config_spawn_model_at(&path)
}

fn config_spawn_model_at(path: &Path) -> Result<Option<String>> {
    let encoded = match fs::read_to_string(path) {
        Ok(encoded) => encoded,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to read spawn model config"),
    };
    let config: SpawnConfigFile =
        toml::from_str(&encoded).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(config.spawn_model)
}

pub(super) fn config_allowed_models() -> Result<Vec<String>> {
    let Some(path) = sessions_config_path() else {
        return Ok(Vec::new());
    };
    config_allowed_models_at(&path)
}

fn config_allowed_models_at(path: &Path) -> Result<Vec<String>> {
    let encoded = match fs::read_to_string(path) {
        Ok(encoded) => encoded,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).context("failed to read allowed model config"),
    };
    let config: SpawnConfigFile =
        toml::from_str(&encoded).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(allowed_models_from(
        config.allowed_models,
        config.spawn_model,
    ))
}

fn allowed_models_from(allowed: Option<Vec<String>>, spawn_model: Option<String>) -> Vec<String> {
    match allowed {
        Some(allowed) if !allowed.is_empty() => allowed,
        _ => spawn_model.into_iter().collect(),
    }
}

pub(super) fn enforce_allowed_model_with(model: &str, allowed: &[String]) -> Result<()> {
    if allowed.is_empty() || allowed.iter().any(|entry| entry == model) {
        return Ok(());
    }
    bail!(
        "model {model} is not one this host may launch. sessions.toml allows: {}. Model tiers are billed per token, so only the person paying picks one: ask them to widen allowed_models rather than passing another tier",
        allowed.join(", ")
    )
}

pub(super) fn enforce_allowed_model(model: &str) -> Result<()> {
    enforce_allowed_model_with(model, &config_allowed_models()?)
}

pub(super) fn resolve_model_with(
    flag: Option<&str>,
    config: Option<String>,
) -> Result<Option<String>> {
    Ok(match flag {
        Some(model) => Some(model.to_owned()),
        None => config,
    })
}

pub(super) fn resolve_allowed_model_with(
    flag: Option<&str>,
    config: Option<String>,
    allowed: &[String],
) -> Result<Option<String>> {
    let model = resolve_model_with(flag, config)?;
    if let Some(model) = model.as_deref() {
        enforce_allowed_model_with(model, allowed)?;
    }
    Ok(model)
}

pub(super) fn resolve_model(flag: Option<&str>) -> Result<Option<String>> {
    resolve_model_with(flag, config_spawn_model()?)
}

pub(super) fn require_model_for_launch(model: Option<&str>) -> Result<()> {
    match model.map(str::trim) {
        Some(model) if !model.is_empty() => Ok(()),
        _ => bail!(
            "spawning a new session requires an explicit model so the lane launches at the intended tier; pass --model MODEL or set spawn_model in sessions.toml. The reuse path needs no model."
        ),
    }
}

fn model_args(tool: &CliToolId, model: &str) -> Result<Vec<String>> {
    let flag = match tool.as_str() {
        "pi" | "codex" | "claude" | "kimi" => "--model",
        other => bail!(
            "tool `{other}` has no model override flag; launch it directly with the model instead"
        ),
    };
    Ok(vec![flag.to_owned(), model.to_owned()])
}

pub(super) struct SpawnLocks {
    dir: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct SpawnRecord {
    pub(super) key: String,
    pub(super) tool: String,
    pub(super) surface: String,
    pub(super) cwd: String,
    pub(super) model: Option<String>,
    pub(super) external_id: Option<String>,
    pub(super) created_at: u64,
    pub(super) last_seen: u64,
}

pub(super) struct SpawnLedger {
    dir: PathBuf,
}

impl SpawnLedger {
    pub(super) fn system() -> Result<Self> {
        let dir = qol_config::data_subdir("sessions")
            .ok_or_else(|| anyhow!("sessions data directory is unavailable"))?
            .join("spawn-records");
        Ok(Self { dir })
    }

    #[cfg(test)]
    pub(super) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub(super) fn record(
        &self,
        key: &SpawnKey,
        tool: &CliToolId,
        surface: SpawnSurface,
        cwd: &str,
        model: Option<&str>,
        external_id: Option<&str>,
    ) -> Result<()> {
        fs::create_dir_all(&self.dir).context("failed to create spawn record directory")?;
        let path = self.file_for(key, cwd);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let previous = fs::read_to_string(&path)
            .ok()
            .and_then(|encoded| serde_json::from_str::<SpawnRecord>(&encoded).ok());
        let record = SpawnRecord {
            key: key.to_string(),
            tool: tool.to_string(),
            surface: surface_token(surface).to_owned(),
            cwd: cwd.to_owned(),
            model: model
                .map(str::to_owned)
                .or_else(|| previous.as_ref().and_then(|record| record.model.clone())),
            external_id: external_id.map(str::to_owned).or_else(|| {
                previous
                    .as_ref()
                    .and_then(|record| record.external_id.clone())
            }),
            created_at: previous
                .as_ref()
                .map(|record| record.created_at)
                .unwrap_or(now),
            last_seen: now,
        };
        let temporary = path.with_extension("tmp");
        let encoded = serde_json::to_string(&record)?;
        fs::write(&temporary, encoded).context("failed to write spawn record")?;
        fs::rename(&temporary, &path).context("failed to publish spawn record")
    }

    pub(super) fn load(&self, key: &SpawnKey, cwd: &str) -> Result<Option<SpawnRecord>> {
        let encoded = match fs::read_to_string(self.file_for(key, cwd)) {
            Ok(encoded) => encoded,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("failed to read spawn record"),
        };
        serde_json::from_str(&encoded)
            .map(Some)
            .context("spawn record is invalid")
    }

    fn file_for(&self, key: &SpawnKey, cwd: &str) -> PathBuf {
        let digest = Sha256::digest(format!("{}\u{0}{}", key.as_str(), cwd).as_bytes());
        self.dir.join(format!("{digest:x}.json"))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ResumeDecision {
    Apply {
        external_id: String,
        args: Vec<String>,
    },
    Skip {
        reason: &'static str,
    },
}

impl ResumeDecision {
    fn status(&self) -> &'static str {
        match self {
            ResumeDecision::Apply { .. } => "applied",
            ResumeDecision::Skip { .. } => "skipped",
        }
    }

    fn detail(&self) -> String {
        match self {
            ResumeDecision::Apply { external_id, .. } => external_id.clone(),
            ResumeDecision::Skip { reason } => (*reason).to_owned(),
        }
    }
}

fn decide_resume(
    requested: Option<bool>,
    record: Option<&SpawnRecord>,
    tool: &CliToolId,
    cwd: &str,
    interpreter: &CliSessionInterpreter,
) -> ResumeDecision {
    if requested == Some(false) {
        return ResumeDecision::Skip {
            reason: "opted_out",
        };
    }
    let Some(record) = record else {
        return ResumeDecision::Skip {
            reason: "no_prior_record",
        };
    };
    if record.tool != tool.to_string() {
        return ResumeDecision::Skip {
            reason: "tool_mismatch",
        };
    }
    if record.cwd != cwd {
        return ResumeDecision::Skip {
            reason: "cwd_mismatch",
        };
    }
    let Some(external_id) = record.external_id.as_deref().filter(|id| !id.is_empty()) else {
        return ResumeDecision::Skip {
            reason: "no_external_id",
        };
    };
    let Some(args) = interpreter.resume_args_for(tool, external_id) else {
        return ResumeDecision::Skip {
            reason: "tool_has_no_resume",
        };
    };
    ResumeDecision::Apply {
        external_id: external_id.to_owned(),
        args,
    }
}

pub(super) struct SpawnLockGuard {
    file: File,
}

impl std::fmt::Debug for SpawnLockGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SpawnLockGuard")
    }
}

impl Drop for SpawnLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl SpawnLocks {
    pub(super) fn system() -> Result<Self> {
        let dir = qol_config::data_subdir("sessions")
            .ok_or_else(|| anyhow!("sessions data directory is unavailable"))?
            .join("spawn-locks");
        Ok(Self { dir })
    }

    #[cfg(test)]
    pub(super) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub(super) fn acquire(&self, key: &SpawnKey) -> Result<SpawnLockGuard> {
        fs::create_dir_all(&self.dir).context("failed to create spawn lock directory")?;
        let path = self.lock_for(key);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .context("failed to open spawn key lock")?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let owner = fs::read_to_string(&path).unwrap_or_default();
                let owner = if owner.trim().is_empty() {
                    "unknown".to_owned()
                } else {
                    owner.trim().to_owned()
                };
                bail!(
                    "another spawn process (pid {owner}) is already handling spawn key `{key}`; wait for it to finish, then retry with the same key"
                );
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).context("failed to lock the spawn key lock");
            }
        }
        fs::write(&path, process::id().to_string())
            .context("failed to record the spawn lock owner")?;
        Ok(SpawnLockGuard { file })
    }

    pub(super) fn remove(&self, key: &SpawnKey) {
        let _ = fs::remove_file(self.lock_for(key));
    }

    fn lock_for(&self, key: &SpawnKey) -> PathBuf {
        let digest = Sha256::digest(key.as_str().as_bytes());
        self.dir.join(format!("{digest:x}.lock"))
    }
}

pub(super) fn generate_key() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = KEY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut digest = Sha256::new();
    digest.update(process::id().to_le_bytes());
    digest.update(elapsed.to_le_bytes());
    digest.update(sequence.to_le_bytes());
    let digest = format!("{:x}", digest.finalize());
    digest[..20].to_owned()
}

fn generated_group(lanes: &[LaneSpec]) -> String {
    let mut digest = Sha256::new();
    for lane in lanes {
        digest.update(lane.key.as_bytes());
        digest.update([0]);
    }
    let digest = format!("{:x}", digest.finalize());
    format!("set-{}-{}", lanes.len(), &digest[..8])
}

fn canonicalize_cwd(requested: &str) -> Result<PathBuf> {
    let process_cwd =
        std::env::current_dir().context("cannot resolve the current working directory")?;
    canonicalize_cwd_at(&process_cwd, requested)
}

fn canonicalize_cwd_at(base: &Path, requested: &str) -> Result<PathBuf> {
    let requested = Path::new(requested);
    let candidate = if requested.is_absolute() {
        requested.to_owned()
    } else {
        base.join(requested)
    };
    let canonical = fs::canonicalize(&candidate).with_context(|| {
        format!(
            "cwd `{}` does not exist or cannot be resolved",
            candidate.display()
        )
    })?;
    if !canonical.is_dir() {
        bail!("cwd `{}` is not a directory", canonical.display());
    }
    Ok(canonical)
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    let parsed = parse_args(args)?;
    let model = resolve_model(parsed.model.as_deref())?;
    if let Some(model) = model.as_deref() {
        enforce_allowed_model(model)?;
    }
    let cap = resolve_spawn_cap(config_spawn_cap()?);
    if let Some(cap) = &cap {
        qol_runtime::probe!(
            "CLI_SESSION_SPAWN",
            "event=cap_enabled weight={} io={} quota={}",
            cap.cpu_weight,
            cap.io_weight,
            cap.cpu_quota.as_deref().unwrap_or("-")
        );
    }
    let terminals = TerminalSessionService::system();
    let surface = config_surface()?;
    let locks = SpawnLocks::system()?;
    let encoded = if parsed.lanes.is_empty() {
        let outcome = run_with(&terminals, parsed, model, surface, &locks, cap)?;
        serde_json::to_string(&outcome).context("failed to serialize spawn outcome")?
    } else {
        let outcome = run_lanes_with(&terminals, parsed, model, surface, &locks, cap)?;
        serde_json::to_string(&outcome).context("failed to serialize lane set outcome")?
    };
    println!("{encoded}");
    Ok(())
}

fn run_lanes_with(
    terminals: &TerminalSessionService,
    parsed: SpawnArgs,
    model: Option<String>,
    config: Option<SpawnSurface>,
    locks: &SpawnLocks,
    cap: Option<SpawnCapConfig>,
) -> Result<LaneSetOutcome> {
    let outcome = spawn_lanes(
        terminals,
        &CliSessionInterpreter::system(),
        &parsed.tool,
        &parsed.cwd,
        &parsed.lanes,
        parsed.surface.as_deref(),
        model.as_deref(),
        config,
        cap.as_ref(),
        locks,
        &SpawnLedger::system()?,
        parsed.resume,
        parsed.group.as_deref(),
        &super::bridge::PendingBridgeStore::system()?,
        &super::bridge::trace_dir(),
        None,
    )?;
    for (lane, spawned) in parsed.lanes.iter().zip(&outcome.lanes) {
        if lane.silent_wake.unwrap_or(false) {
            spawn_silent_wake_watcher(&spawned.session)?;
        }
    }
    Ok(outcome)
}

fn run_with(
    terminals: &TerminalSessionService,
    parsed: SpawnArgs,
    model: Option<String>,
    config: Option<SpawnSurface>,
    locks: &SpawnLocks,
    cap: Option<SpawnCapConfig>,
) -> Result<SpawnOutcome> {
    let outcome = spawn_or_reuse(
        terminals,
        &CliSessionInterpreter::system(),
        &parsed.tool,
        &parsed.cwd,
        parsed.key.as_deref(),
        parsed.surface.as_deref(),
        model.as_deref(),
        parsed.title.as_deref(),
        config,
        cap.as_ref(),
        locks,
        &SpawnLedger::system()?,
        parsed.background,
        true,
        parsed.silent_wake,
        parsed.resume,
        parsed.task.as_deref(),
        parsed.group.as_deref(),
        &super::bridge::PendingBridgeStore::system()?,
        &super::bridge::trace_dir(),
        None,
    )?;
    if parsed.silent_wake {
        spawn_silent_wake_watcher(&outcome.session)?;
    }
    Ok(outcome)
}

fn spawn_silent_wake_watcher(session: &str) -> Result<()> {
    let executable = std::env::current_exe()
        .context("cannot resolve the current executable for the silent-wake watcher")?;
    process::Command::new(executable)
        .args(["sessions", "watch", session])
        .stdin(process::Stdio::null())
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .spawn()
        .context("failed to launch the detached silent-wake watcher")?;
    Ok(())
}

pub(super) fn deliver_task(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    mut outcome: SpawnOutcome,
    task: &str,
    pending: &super::bridge::PendingBridgeStore,
    resumed: bool,
    cancel: Option<&AtomicBool>,
) -> Result<SpawnOutcome> {
    let binding = outcome
        .session
        .parse()
        .context("spawned session token cannot be resolved for task delivery")?;
    wait_until_live(terminals, interpreter, &binding, cancel)?;
    let submitted = super::bridge::submit(
        terminals,
        interpreter,
        &binding,
        task,
        pending,
        None,
        resumed,
    )?;
    outcome.task_submitted = Some(true);
    outcome.completion_marker = Some(submitted.completion_marker);
    outcome.screen = Some(submitted.screen);
    outcome.next_command = Some(submitted.next_command);
    pending.set_label(&binding, Some(&outcome.key))?;
    Ok(outcome)
}

fn wait_until_live(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    let started = Instant::now();
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            bail!("spawn wait cancelled; the lane was not launched and the round was not opened");
        }
        let facts = terminals
            .discover()
            .context("target discovery failed while waiting for its live UI")?
            .into_iter()
            .find(|facts| facts.id == *binding.session_id())
            .ok_or_else(|| anyhow!("spawned target disappeared while waiting for its live UI"))?;
        let screen = terminals
            .read_screen(binding)
            .context("target screen read failed while waiting for its live UI")?;
        let evidence = interpreter.classify_screen(&facts, &screen);
        if interpreter.ui_rendered(&facts, &screen)
            || evidence.viewport == qol_terminal_sessions::cli::CliViewportState::Live
            || evidence.runtime != qol_terminal_sessions::cli::CliRuntimeState::Unknown
        {
            return Ok(());
        }
        if started.elapsed() >= SPAWN_TASK_READY_TIMEOUT {
            bail!(
                "the spawned session's UI did not become live within {}s; the task was not delivered - check the session or spawn again without --task",
                SPAWN_TASK_READY_TIMEOUT.as_secs()
            );
        }
        if cancel.is_some() {
            std::thread::sleep(READY_CANCEL_POLL_INTERVAL);
        } else {
            std::thread::sleep(READY_POLL_INTERVAL);
        }
    }
}

struct SpawnArgs {
    tool: String,
    cwd: String,
    key: Option<String>,
    surface: Option<String>,
    model: Option<String>,
    title: Option<String>,
    task: Option<String>,
    background: bool,
    silent_wake: bool,
    resume: Option<bool>,
    group: Option<String>,
    lanes: Vec<LaneSpec>,
}

fn parse_args(args: &[OsString]) -> Result<SpawnArgs> {
    let usage = "qol sessions spawn --tool TOOL --cwd PATH [--key KEY] [--surface tab|os-window] --model MODEL [--title TITLE] [--task TASK] [--background] [--resume] [--no-resume] [--group GROUP] [--lanes JSON]\n--model is required when launching a new session; the reuse path needs no model. --background embeds the task in the launch and queues the round without waiting for the live UI; it requires --task. --silent-wake requires --background, skips the parent wake message, still writes the lane report plus a receipt json, and still closes the lane terminal. A fresh lane closes its terminal when the watcher confirms the round's completion; a reused session is only closed when it carries a spawn identity. --resume forces a resume; resume is otherwise automatic when the spawn ledger holds a session id for the key (same tool and cwd); --no-resume opts out; the spawn JSON reports resume and resume_detail. --group registers the lane as a member of a grouped-research set so its completed rounds aggregate into a single combined wake under the sessions data dir. --lanes takes a JSON array of {key, task, title?} objects and launches the whole set in one call; it replaces --key, --task and --title, and two or more lanes are grouped automatically so the set delivers one combined report instead of one wake per lane.";
    let mut tool = None;
    let mut cwd = None;
    let mut key = None;
    let mut surface = None;
    let mut model = None;
    let mut title = None;
    let mut task = None;
    let mut background = false;
    let mut silent_wake = false;
    let mut resume = None;
    let mut group = None;
    let mut lanes = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = args[index]
            .to_str()
            .ok_or_else(|| anyhow!("spawn arguments must be valid UTF-8"))?;
        match argument {
            "--tool" => {
                tool = Some(flag_value(args, index, "--tool", usage)?);
                index += 2;
            }
            "--cwd" => {
                cwd = Some(flag_value(args, index, "--cwd", usage)?);
                index += 2;
            }
            "--key" => {
                key = Some(flag_value(args, index, "--key", usage)?);
                index += 2;
            }
            "--surface" => {
                surface = Some(flag_value(args, index, "--surface", usage)?);
                index += 2;
            }
            "--model" => {
                model = Some(flag_value(args, index, "--model", usage)?);
                index += 2;
            }
            "--title" => {
                title = Some(flag_value(args, index, "--title", usage)?);
                index += 2;
            }
            "--task" => {
                task = Some(flag_value(args, index, "--task", usage)?);
                index += 2;
            }
            "--background" => {
                background = true;
                index += 1;
            }
            "--silent-wake" => {
                silent_wake = true;
                index += 1;
            }
            "--resume" => {
                resume = Some(true);
                index += 1;
            }
            "--no-resume" => {
                resume = Some(false);
                index += 1;
            }
            "--group" => {
                group = Some(flag_value(args, index, "--group", usage)?);
                index += 2;
            }
            "--lanes" => {
                let value = flag_value(args, index, "--lanes", usage)?;
                lanes = serde_json::from_str(&value).with_context(|| {
                    format!("--lanes expects a JSON array of {{key, task, title?}} objects, got `{value}`")
                })?;
                index += 2;
            }
            other => bail!("unknown spawn flag `{other}`\nusage: {usage}"),
        }
    }
    let tool = tool.ok_or_else(|| anyhow!("usage: {usage}"))?;
    let cwd = cwd.ok_or_else(|| anyhow!("usage: {usage}"))?;
    if !lanes.is_empty() && (key.is_some() || task.is_some() || title.is_some()) {
        bail!("--lanes carries every lane's key, task and title, so --key, --task and --title cannot be combined with it\nusage: {usage}");
    }
    if silent_wake && !background {
        bail!("--silent-wake requires --background\nusage: {usage}");
    }
    Ok(SpawnArgs {
        tool,
        cwd,
        key,
        surface,
        model,
        title,
        task,
        background,
        silent_wake,
        resume,
        group,
        lanes,
    })
}

fn flag_value(args: &[OsString], index: usize, flag: &str, usage: &str) -> Result<String> {
    let value = args
        .get(index + 1)
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("usage: {usage}"))?;
    if value.starts_with("--") {
        bail!("{flag} requires a value\nusage: {usage}");
    }
    Ok(value.to_owned())
}

struct SpawnContext {
    tool_id: CliToolId,
    launch: CliLaunchProgram,
    key: SpawnKey,
    identity: SpawnIdentity,
    title: String,
}

fn prepare_spawn(
    interpreter: &CliSessionInterpreter,
    tool: &str,
    key: Option<&str>,
    surface: Option<&str>,
    title: Option<&str>,
    config: Option<SpawnSurface>,
) -> Result<SpawnContext> {
    let tool_id = CliToolId::new(tool.to_owned())
        .map_err(|error| anyhow!("invalid tool `{tool}`: {error}"))?;
    let launch = interpreter.launch_for(&tool_id).ok_or_else(|| {
        anyhow!("no launch strategy for tool `{tool}`; only registered tools with a launch program can spawn")
    })?;
    let key = match key {
        Some(key) => SpawnKey::new(key.to_owned())
            .map_err(|error| anyhow!("invalid spawn key `{key}`: {error}"))?,
        None => SpawnKey::new(generate_key())
            .map_err(|error| anyhow!("generated spawn key is invalid: {error}"))?,
    };
    let surface = resolve_surface(surface, config)?;
    let title = title.unwrap_or(&key.to_string()).to_owned();
    let identity = SpawnIdentity {
        key: key.clone(),
        tool: tool_id.clone(),
        surface,
    };
    Ok(SpawnContext {
        tool_id,
        launch,
        key,
        identity,
        title,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_lanes(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    tool: &str,
    cwd: &str,
    lanes: &[LaneSpec],
    surface: Option<&str>,
    model: Option<&str>,
    config: Option<SpawnSurface>,
    cap: Option<&SpawnCapConfig>,
    locks: &SpawnLocks,
    ledger: &SpawnLedger,
    resume: Option<bool>,
    group: Option<&str>,
    pending: &super::bridge::PendingBridgeStore,
    trace_dir: &std::path::Path,
    cancel: Option<&AtomicBool>,
) -> Result<LaneSetOutcome> {
    if lanes.is_empty() {
        bail!("`lanes` is empty; give one entry per lane, each with its own key and bounded task");
    }
    let mut seen = std::collections::HashSet::new();
    for lane in lanes {
        if lane.key.trim().is_empty() {
            bail!("every lane needs a non-empty key so its round stays idempotent");
        }
        if !seen.insert(lane.key.as_str()) {
            bail!(
                "lane key `{}` appears twice; every lane in a set needs its own key",
                lane.key
            );
        }
    }
    let started = Instant::now();
    let group = match group {
        Some(group) => Some(group.to_owned()),
        None if lanes.len() > 1 => Some(generated_group(lanes)),
        None => None,
    };
    let mut outcomes = Vec::with_capacity(lanes.len());
    for lane in lanes {
        let outcome = spawn_or_reuse(
            terminals,
            interpreter,
            tool,
            cwd,
            Some(lane.key.as_str()),
            surface,
            model,
            lane.title.as_deref(),
            config,
            cap,
            locks,
            ledger,
            true,
            true,
            lane.silent_wake.unwrap_or(false),
            resume,
            Some(lane.task.as_str()),
            group.as_deref(),
            pending,
            trace_dir,
            cancel,
        )
        .with_context(|| {
            format!(
                "lane `{}` failed to spawn; {} lane(s) of this set are already live and keep running",
                lane.key,
                outcomes.len()
            )
        })?;
        outcomes.push(outcome);
    }
    qol_runtime::probe!(
        "CLI_SESSION_SPAWN",
        "event=lane_set lanes={} group={}",
        outcomes.len(),
        group.as_deref().unwrap_or("-")
    );
    Ok(LaneSetOutcome {
        combined_report: group.is_some(),
        group,
        lanes: outcomes,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn refuse_second_ungrouped_lane(
    pending: &super::bridge::PendingBridgeStore,
    driver: &str,
    live: &std::collections::HashSet<String>,
) -> Result<()> {
    for round in pending.pending_rounds()? {
        if round.completed || round.group.is_some() || !live.contains(&round.session) {
            continue;
        }
        if !driver.is_empty() && round.driver != driver {
            continue;
        }
        let label = round.label.unwrap_or_else(|| round.session.clone());
        bail!(
            "lane `{label}` is still running an open round on `{}`, and a second ungrouped lane would wake you once per lane. Spawn the whole set in one call by passing `lanes`, one entry per lane, so they aggregate into a single combined report, or pass `group` to join them yourself. If that lane is hung rather than working, clear it in one call with `qol sessions close {}`",
            round.session,
            round.session
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_or_reuse(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    tool: &str,
    cwd: &str,
    key: Option<&str>,
    surface: Option<&str>,
    model: Option<&str>,
    title: Option<&str>,
    config: Option<SpawnSurface>,
    cap: Option<&SpawnCapConfig>,
    locks: &SpawnLocks,
    ledger: &SpawnLedger,
    background: bool,
    autoclose: bool,
    silent_wake: bool,
    resume: Option<bool>,
    task: Option<&str>,
    group: Option<&str>,
    pending: &super::bridge::PendingBridgeStore,
    trace_dir: &std::path::Path,
    cancel: Option<&AtomicBool>,
) -> Result<SpawnOutcome> {
    if background && task.is_none() {
        bail!(
            "background spawn requires --task so the first round is queued at launch; the reuse path takes no --background"
        );
    }
    let prepared = prepare_spawn(interpreter, tool, key, surface, title, config)?;
    let _guard = locks.acquire(&prepared.key)?;
    let result = (|| {
        let snapshot = terminals.snapshot().context("session discovery failed")?;
        match decide(interpreter, snapshot.sessions(), &prepared.identity) {
            SpawnDecision::Launch => {
                if background && group.is_none() {
                    let live = snapshot
                        .sessions()
                        .iter()
                        .filter_map(|facts| facts.binding().ok().map(|binding| binding.token()))
                        .collect::<std::collections::HashSet<_>>();
                    refuse_second_ungrouped_lane(
                        pending,
                        &super::bridge::driver_token(terminals),
                        &live,
                    )?;
                }
                require_model_for_launch(model)?;
                let mut launch = wrap_launch(&prepared.launch, cap);
                let requested_cwd = canonicalize_cwd(cwd)?;
                let resume_decision = decide_resume(
                    resume,
                    ledger.load(&prepared.key, &requested_cwd.to_string_lossy())?.as_ref(),
                    &prepared.tool_id,
                    &requested_cwd.to_string_lossy(),
                    interpreter,
                );
                match &resume_decision {
                    ResumeDecision::Apply { external_id, args } => {
                        launch.args.extend(args.iter().cloned());
                        qol_runtime::probe!(
                            "CLI_SESSION_SPAWN",
                            "event=resume_applied key={} tool={} id={}",
                            prepared.identity.key,
                            prepared.identity.tool,
                            external_id
                        );
                    }
                    ResumeDecision::Skip { reason } => {
                        qol_runtime::probe!(
                            "CLI_SESSION_SPAWN",
                            "event=resume_skipped key={} tool={} reason={}",
                            prepared.identity.key,
                            prepared.identity.tool,
                            reason
                        );
                    }
                }
                if let Some(model) = model {
                    launch.args.extend(model_args(&prepared.tool_id, model)?);
                }
                let resumed = key.is_some()
                    && task.is_some()
                    && pending.has_key_history(prepared.key.as_str())?;
                if background {
                    let round_task =
                        task.expect("the background guard above guarantees a task");
                    super::bridge::validate_task(round_task)?;
                    let marker = super::bridge::CompletionMarker::generate();
                    let joined = prepared.identity.tool.as_str()
                        == qol_terminal_sessions::cli::PI_TOOL_ID;
                    let prompt = if resumed {
                        super::bridge::resume_lane_prompt(round_task, &marker, joined)
                    } else {
                        super::bridge::bridge_prompt(
                            round_task,
                            &marker,
                            super::bridge::Role::Lane,
                            joined,
                        )
                    };
                    launch.args.push(prompt);
                    let request = SpawnRequest {
                        identity: prepared.identity.clone(),
                        launch,
                        cwd: requested_cwd.clone(),
                        title: Some(prepared.title.clone()),
                    };
                    let mut outcome = launch_background(
                        terminals,
                        interpreter,
                        pending,
                        ledger,
                        &prepared.identity,
                        &request,
                        &marker.token,
                        model,
                        &prepared.title,
                        autoclose,
                        silent_wake,
                        group,
                        trace_dir,
                        cancel,
                    )?;
                    outcome.resume = Some(resume_decision.status());
                    outcome.resume_detail = Some(resume_decision.detail());
                    apply_slice_properties(cap);
                    Ok(outcome)
                } else {
                    let request = SpawnRequest {
                        identity: prepared.identity.clone(),
                        launch,
                        cwd: requested_cwd,
                        title: Some(prepared.title.clone()),
                    };
                    let mut outcome = launch_ready(
                        terminals,
                        interpreter,
                        pending,
                        ledger,
                        &prepared.identity,
                        &request,
                        model,
                        &prepared.title,
                        autoclose,
                        cancel,
                    )?;
                    outcome.resume = Some(resume_decision.status());
                    outcome.resume_detail = Some(resume_decision.detail());
                    apply_slice_properties(cap);
                    match task {
                        Some(round_task) => deliver_task(
                            terminals,
                            interpreter,
                            outcome,
                            round_task,
                            pending,
                            resumed,
                            cancel,
                        ),
                        None => Ok(outcome),
                    }
                }
            }
            SpawnDecision::Reuse(facts) => {
                qol_runtime::probe!(
                    "CLI_SESSION_SPAWN",
                    "event=reuse key={} tool={}",
                    prepared.identity.key,
                    prepared.identity.tool
                );
                let outcome = outcome_from_facts(
                    &facts,
                    interpreter,
                    true,
                    model.map(str::to_owned),
                    &prepared.title,
                )?;
                let binding = facts
                    .binding()
                    .context("spawned session cannot bind to a stable token")?;
                pending.set_role(&binding, super::bridge::Role::Lane)?;
                let descriptor = interpreter.describe(&facts);
                ledger.record(
                    &prepared.key,
                    &prepared.tool_id,
                    prepared.identity.surface,
                    &facts.cwd,
                    model,
                    descriptor.external_id.as_deref(),
                )?;
                match task {
                    Some(round_task) => deliver_task(
                        terminals,
                        interpreter,
                        outcome,
                        round_task,
                        pending,
                        false,
                        cancel,
                    ),
                    None => Ok(outcome),
                }
            }
            SpawnDecision::Conflict(found) => {
                qol_runtime::probe!(
                    "CLI_SESSION_SPAWN",
                    "event=conflict key={} requested_tool={} found_tool={}",
                    prepared.identity.key,
                    prepared.identity.tool,
                    found
                );
                bail!(
                    "spawn key `{}` is already held by tool `{found}`; a key cannot span tools - pick a distinct key",
                    prepared.key
                )
            }
            SpawnDecision::WrongHarness { described } => bail!(
                "spawn key `{}` is tagged for `{}` but the live session is described as `{described}`; refusing to reuse it",
                prepared.key, prepared.identity.tool
            ),
            SpawnDecision::Ambiguous(count) => {
                qol_runtime::probe!(
                    "CLI_SESSION_SPAWN",
                    "event=ambiguous key={} matches={}",
                    prepared.identity.key,
                    count
                );
                bail!(
                    "spawn key `{}` matches {count} live sessions; the key is ambiguous - close the duplicates or pick a distinct key",
                    prepared.key
                )
            }
        }
    })();
    drop(_guard);
    locks.remove(&prepared.key);
    result
}

pub(super) enum ExternalIdCapture {
    Authoritative,
    Heuristic,
    Unresolved,
}

pub(super) fn capture_lane_external_id(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    ledger: &SpawnLedger,
    locks: &SpawnLocks,
    binding: &SessionBinding,
    marker: &str,
    since: Option<std::time::SystemTime>,
) -> ExternalIdCapture {
    let Ok(facts) = terminals.discover() else {
        return ExternalIdCapture::Unresolved;
    };
    let Some(facts) = facts
        .into_iter()
        .find(|facts| facts.id == *binding.session_id())
    else {
        return ExternalIdCapture::Unresolved;
    };
    let Some(identity) = facts.spawn_identity.as_ref() else {
        return ExternalIdCapture::Unresolved;
    };
    let marker_id = (!marker.is_empty())
        .then(|| {
            qol_terminal_sessions::cli::session_file_containing_marker(
                &facts.cwd,
                marker,
                since.unwrap_or(std::time::UNIX_EPOCH),
            )
        })
        .flatten()
        .and_then(|path| external_id_from_transcript_path(&path));
    let (external_id, authoritative) = match marker_id {
        Some(external_id) => {
            qol_runtime::probe!(
                "CLI_SESSION_SPAWN",
                "event=external_id_captured_from_transcript key={} tool={} id={}",
                identity.key,
                identity.tool,
                external_id
            );
            (external_id, true)
        }
        None => {
            let Some((external_id, authoritative)) =
                resolve_lane_external_id(interpreter, ledger, identity, &facts)
            else {
                return ExternalIdCapture::Unresolved;
            };
            (external_id, authoritative)
        }
    };
    if !authoritative {
        qol_runtime::probe!(
            "CLI_SESSION_SPAWN",
            "event=external_id_provisional key={} tool={} id={}",
            identity.key,
            identity.tool,
            external_id
        );
        return ExternalIdCapture::Heuristic;
    }
    let Ok(_guard) = locks.acquire(&identity.key) else {
        qol_runtime::probe!(
            "CLI_SESSION_SPAWN",
            "event=external_id_capture_skipped key={} reason=spawn_in_flight",
            identity.key
        );
        return ExternalIdCapture::Unresolved;
    };
    match ledger.record(
        &identity.key,
        &identity.tool,
        identity.surface,
        &facts.cwd,
        None,
        Some(&external_id),
    ) {
        Ok(()) => {
            qol_runtime::probe!(
                "CLI_SESSION_SPAWN",
                "event=external_id_captured key={} tool={} id={}",
                identity.key,
                identity.tool,
                external_id
            );
            ExternalIdCapture::Authoritative
        }
        Err(error) => {
            qol_runtime::probe!(
                "CLI_SESSION_SPAWN",
                "event=external_id_capture_failed key={} error={}",
                identity.key,
                error
            );
            ExternalIdCapture::Unresolved
        }
    }
}

fn external_id_from_transcript_path(path: &std::path::Path) -> Option<String> {
    let stem = path.file_name()?.to_str()?.strip_suffix(".jsonl")?;
    let (_, id) = stem.rsplit_once('_')?;
    (!id.is_empty()).then(|| id.to_owned())
}

fn resolve_lane_external_id(
    interpreter: &CliSessionInterpreter,
    ledger: &SpawnLedger,
    identity: &SpawnIdentity,
    facts: &SessionFacts,
) -> Option<(String, bool)> {
    let descriptor = interpreter.describe(facts);
    let described = descriptor.external_id.filter(|id| !id.is_empty());
    let authoritative = descriptor.external_id_authoritative;
    let recorded = ledger
        .load(&identity.key, &facts.cwd)
        .ok()
        .flatten()
        .and_then(|record| record.external_id)
        .filter(|id| !id.is_empty());
    match (described, recorded, authoritative) {
        (Some(resolved), Some(recorded), true) if resolved != recorded => {
            qol_runtime::probe!(
                "CLI_SESSION_SPAWN",
                "event=external_id_corrected_from_env key={} tool={} recorded={} resolved={}",
                identity.key,
                identity.tool,
                recorded,
                resolved
            );
            Some((resolved, true))
        }
        (Some(resolved), Some(recorded), false) if resolved != recorded => {
            qol_runtime::probe!(
                "CLI_SESSION_SPAWN",
                "event=external_id_conflict_kept_ledger key={} tool={} recorded={} resolved={}",
                identity.key,
                identity.tool,
                recorded,
                resolved
            );
            Some((recorded, false))
        }
        (Some(resolved), _, authoritative) => Some((resolved, authoritative)),
        (None, Some(recorded), _) => {
            qol_runtime::probe!(
                "CLI_SESSION_SPAWN",
                "event=external_id_resumed_from_ledger key={} tool={} id={}",
                identity.key,
                identity.tool,
                recorded
            );
            Some((recorded, false))
        }
        (None, None, _) => {
            qol_runtime::probe!(
                "CLI_SESSION_SPAWN",
                "event=external_id_unresolved key={} tool={}",
                identity.key,
                identity.tool
            );
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn launch_background(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    pending: &super::bridge::PendingBridgeStore,
    ledger: &SpawnLedger,
    identity: &SpawnIdentity,
    request: &SpawnRequest,
    marker: &str,
    model: Option<&str>,
    title: &str,
    autoclose: bool,
    silent_wake: bool,
    group: Option<&str>,
    trace_dir: &std::path::Path,
    cancel: Option<&AtomicBool>,
) -> Result<SpawnOutcome> {
    let started = Instant::now();
    let session_id = terminals
        .spawn_on(qol_terminal_sessions::kitty::backend_id(), request)
        .context("terminal backend refused the spawn request")?;
    qol_runtime::probe!(
        "CLI_SESSION_SPAWN",
        "event=background key={} tool={} surface={} model={}",
        identity.key,
        identity.tool,
        surface_token(identity.surface),
        model.unwrap_or("default")
    );
    let facts = poll_ready(
        terminals,
        interpreter,
        &session_id,
        identity,
        Duration::from_millis(READY_TIMEOUT_MS),
        cancel,
    )?;
    let descriptor = interpreter.describe(&facts);
    ledger.record(
        &identity.key,
        &identity.tool,
        identity.surface,
        &facts.cwd,
        model,
        descriptor.external_id.as_deref(),
    )?;
    let binding = facts
        .binding()
        .context("spawned session cannot bind to a stable token")?;
    pending.set_role(&binding, super::bridge::Role::Lane)?;
    pending.start_with_label(
        &binding,
        marker,
        &super::bridge::driver_token(terminals),
        autoclose,
        group,
        Some(identity.key.as_str()),
        silent_wake,
    )?;
    pending.record_transcript_paths(&binding, &interpreter.transcript_paths(&facts))?;
    if let Some(group) = group {
        super::watch::register_group_member(
            trace_dir,
            group,
            &binding.token(),
            Some(identity.key.as_str()),
        )?;
    }
    let mut outcome =
        outcome_from_facts(&facts, interpreter, false, model.map(str::to_owned), title)?;
    outcome.background = true;
    outcome.autoclose = autoclose;
    outcome.task_submitted = Some(true);
    outcome.completion_marker = Some(marker.to_owned());
    outcome.next_command = Some(format!("qol sessions next {}", binding.token()));
    outcome.elapsed_ms = started.elapsed().as_millis();
    qol_runtime::probe!(
        "CLI_SESSION_SPAWN",
        "event=queued key={} tool={} elapsed_ms={}",
        identity.key,
        identity.tool,
        outcome.elapsed_ms
    );
    Ok(outcome)
}

#[allow(clippy::too_many_arguments)]
fn launch_ready(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    pending: &super::bridge::PendingBridgeStore,
    ledger: &SpawnLedger,
    identity: &SpawnIdentity,
    request: &SpawnRequest,
    model: Option<&str>,
    title: &str,
    autoclose: bool,
    cancel: Option<&AtomicBool>,
) -> Result<SpawnOutcome> {
    let started = Instant::now();
    let session_id = terminals
        .spawn_on(qol_terminal_sessions::kitty::backend_id(), request)
        .context("terminal backend refused the spawn request")?;
    qol_runtime::probe!(
        "CLI_SESSION_SPAWN",
        "event=launch key={} tool={} surface={} model={}",
        identity.key,
        identity.tool,
        surface_token(identity.surface),
        model.unwrap_or("default")
    );
    let facts = poll_ready(
        terminals,
        interpreter,
        &session_id,
        identity,
        Duration::from_millis(READY_TIMEOUT_MS),
        cancel,
    )?;
    let descriptor = interpreter.describe(&facts);
    ledger.record(
        &identity.key,
        &identity.tool,
        identity.surface,
        &facts.cwd,
        model,
        descriptor.external_id.as_deref(),
    )?;
    let binding = facts
        .binding()
        .context("spawned session cannot bind to a stable token")?;
    pending.set_role(&binding, super::bridge::Role::Lane)?;
    let mut outcome =
        outcome_from_facts(&facts, interpreter, false, model.map(str::to_owned), title)?;
    outcome.autoclose = autoclose;
    outcome.elapsed_ms = started.elapsed().as_millis();
    qol_runtime::probe!(
        "CLI_SESSION_SPAWN",
        "event=ready key={} tool={} elapsed_ms={}",
        identity.key,
        identity.tool,
        outcome.elapsed_ms
    );
    Ok(outcome)
}

struct ReadinessObservation {
    appeared: bool,
    bound: bool,
    described: Option<CliToolId>,
}

pub(super) struct DetachedLaunch {
    pub(super) session: String,
    pub(super) cwd: String,
    pub(super) surface: &'static str,
    pub(super) title: String,
    pub(super) elapsed_ms: u128,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_detached(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    ledger: &SpawnLedger,
    locks: &SpawnLocks,
    tool: &str,
    cwd: &str,
    key: &str,
    surface: Option<&str>,
    model: Option<&str>,
    extra_args: &[String],
    title: Option<&str>,
    config: Option<SpawnSurface>,
    cap: Option<&SpawnCapConfig>,
    prompt: &str,
) -> Result<DetachedLaunch> {
    require_model_for_launch(model)?;
    let prepared = prepare_spawn(interpreter, tool, Some(key), surface, title, config)?;
    let _guard = locks.acquire(&prepared.key)?;
    let started = Instant::now();
    let snapshot = terminals.snapshot().context("session discovery failed")?;
    if !matches!(
        decide(interpreter, snapshot.sessions(), &prepared.identity),
        SpawnDecision::Launch
    ) {
        bail!(
            "spawn key `{key}` is already held by a live session; a detached fork always starts a fresh tree, so give it an unused key"
        );
    }
    let mut launch = wrap_launch(&prepared.launch, cap);
    if let Some(model) = model {
        launch.args.extend(model_args(&prepared.tool_id, model)?);
    }
    launch.args.extend(extra_args.iter().cloned());
    launch.args.push(prompt.to_owned());
    let request = SpawnRequest {
        identity: prepared.identity.clone(),
        launch,
        cwd: canonicalize_cwd(cwd)?,
        title: Some(prepared.title.clone()),
    };
    let session_id = terminals
        .spawn_on(qol_terminal_sessions::kitty::backend_id(), &request)
        .context("terminal backend refused the spawn request")?;
    let facts = poll_ready(
        terminals,
        interpreter,
        &session_id,
        &prepared.identity,
        Duration::from_millis(READY_TIMEOUT_MS),
        None,
    )?;
    let descriptor = interpreter.describe(&facts);
    ledger.record(
        &prepared.key,
        &prepared.tool_id,
        prepared.identity.surface,
        &facts.cwd,
        model,
        descriptor.external_id.as_deref(),
    )?;
    let binding = facts
        .binding()
        .context("spawned session cannot bind to a stable token")?;
    Ok(DetachedLaunch {
        session: binding.token(),
        cwd: facts.cwd.clone(),
        surface: surface_token(prepared.identity.surface),
        title: prepared.title,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn poll_ready(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    session_id: &SessionId,
    identity: &SpawnIdentity,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<SessionFacts> {
    let started = Instant::now();
    let mut last = ReadinessObservation {
        appeared: false,
        bound: false,
        described: None,
    };
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            bail!("spawn wait cancelled; the lane was not launched and the round was not opened");
        }
        let snapshot = terminals
            .snapshot()
            .context("spawn readiness discovery failed")?;
        if let Some(facts) = snapshot
            .sessions()
            .iter()
            .find(|facts| facts.id == *session_id)
        {
            last.appeared = true;
            last.bound = facts.binding().is_ok();
            last.described = Some(interpreter.describe(facts).tool.id);
            if facts.spawn_identity.as_ref() != Some(identity) {
                bail!(
                    "spawned session `{session_id}` appeared with a different spawn identity than requested (key `{}`, tool `{}`); refusing to return it as bridgeable",
                    identity.key, identity.tool
                );
            }
            if last.bound && last.described.as_ref() == Some(&identity.tool) {
                return Ok(facts.clone());
            }
        } else if last.appeared {
            bail!(
                "spawned session `{session_id}` exited before `{}` was running in it; the launch arguments are the first thing to check",
                identity.tool
            );
        }
        if started.elapsed() >= timeout {
            let observed = if last.appeared {
                format!(
                    "appeared=true bound={} described={}",
                    last.bound,
                    last.described
                        .map(|tool| tool.to_string())
                        .unwrap_or_default()
                )
            } else {
                "never appeared".to_owned()
            };
            bail!(
                "spawned session `{session_id}` was not live and bridgeable within {}ms (last observed state: {observed}); it may have exited before tagging - rerun with the same key",
                timeout.as_millis()
            );
        }
        if cancel.is_some() {
            std::thread::sleep(READY_CANCEL_POLL_INTERVAL);
        } else {
            std::thread::sleep(READY_POLL_INTERVAL);
        }
    }
}

fn outcome_from_facts(
    facts: &SessionFacts,
    interpreter: &CliSessionInterpreter,
    reused: bool,
    model: Option<String>,
    title: &str,
) -> Result<SpawnOutcome> {
    let binding = facts
        .binding()
        .context("spawned session cannot bind to a stable token")?;
    let identity = facts
        .spawn_identity
        .as_ref()
        .ok_or_else(|| anyhow!("spawned session carries no spawn identity"))?;
    Ok(SpawnOutcome {
        session: binding.token(),
        tool: interpreter.describe(facts).tool.id.to_string(),
        key: identity.key.to_string(),
        reused,
        cwd: facts.cwd.clone(),
        surface: surface_token(identity.surface).to_owned(),
        model,
        title: title.to_owned(),
        task_submitted: None,
        completion_marker: None,
        screen: None,
        next_command: None,
        elapsed_ms: 0,
        background: false,
        autoclose: false,
        resume: None,
        resume_detail: None,
    })
}

fn decide(
    interpreter: &CliSessionInterpreter,
    sessions: &[SessionFacts],
    identity: &SpawnIdentity,
) -> SpawnDecision {
    let matches = sessions
        .iter()
        .filter(|facts| {
            facts.spawn_identity.as_ref().map(|tagged| &tagged.key) == Some(&identity.key)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => SpawnDecision::Launch,
        [facts] => {
            let tagged = facts
                .spawn_identity
                .as_ref()
                .expect("the key filter guarantees a tagged identity");
            if tagged.tool != identity.tool {
                return SpawnDecision::Conflict(tagged.tool.clone());
            }
            let described = interpreter.describe(facts).tool.id;
            if described == identity.tool {
                SpawnDecision::Reuse(Box::new((*facts).clone()))
            } else {
                SpawnDecision::WrongHarness { described }
            }
        }
        matches => SpawnDecision::Ambiguous(matches.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::{Arc, Mutex};

    use qol_terminal_sessions::cli::{
        CliSessionDescriptor, CliSessionEvidence, CliSessionStrategy, CliTool, CliToolColor,
        CliToolId,
    };
    use qol_terminal_sessions::{
        BackendId, DeliveryMode, ScreenReader, SessionBinding, SessionCapabilities, SessionFocus,
        SessionInventory, SessionSpawner, TerminalBackend, TerminalError, TerminalSnapshot,
        TextInput,
    };

    #[derive(Clone)]
    struct SpawnedState {
        id: SessionId,
        identity: SpawnIdentity,
        cwd: String,
    }

    struct FakeBackend {
        id: BackendId,
        discoveries: Mutex<VecDeque<Vec<SessionFacts>>>,
        last: Mutex<Vec<SessionFacts>>,
        spawn_count: AtomicUsize,
        refuse_spawn: bool,
        supported: Vec<SpawnSurface>,
        last_request: Mutex<Option<SpawnRequest>>,
        spawned: Mutex<Option<SpawnedState>>,
        reveal_spawned: bool,
        dying_spawn: bool,
        spawn_reveals: AtomicUsize,
    }

    impl FakeBackend {
        fn new(discoveries: Vec<Vec<SessionFacts>>) -> Self {
            Self {
                id: BackendId::new("kitty").unwrap(),
                discoveries: Mutex::new(discoveries.into()),
                last: Mutex::new(Vec::new()),
                spawn_count: AtomicUsize::new(0),
                refuse_spawn: false,
                supported: vec![SpawnSurface::Tab],
                last_request: Mutex::new(None),
                spawned: Mutex::new(None),
                reveal_spawned: true,
                dying_spawn: false,
                spawn_reveals: AtomicUsize::new(0),
            }
        }

        fn dying_spawn(mut self) -> Self {
            self.dying_spawn = true;
            self
        }

        fn facts(id: &str, key: &str, tool: &str, cwd: &str) -> SessionFacts {
            SessionFacts {
                id: SessionId::new(BackendId::new("kitty").unwrap(), id).unwrap(),
                root_pid: 10,
                cwd: cwd.to_owned(),
                title: "Terminal".to_owned(),
                at_prompt: true,
                reported_cmd: None,
                foreground_basenames: vec![tool.to_owned()],
                foreground_pids: Vec::new(),
                capabilities: SessionCapabilities::ALL,
                spawn_identity: Some(SpawnIdentity {
                    key: SpawnKey::new(key).unwrap(),
                    tool: CliToolId::new(tool).unwrap(),
                    surface: SpawnSurface::Tab,
                }),
            }
        }

        fn shell_only_facts(state: &SpawnedState) -> SessionFacts {
            SessionFacts {
                foreground_basenames: vec!["zsh".to_owned()],
                ..Self::spawned_facts(state)
            }
        }

        fn spawned_facts(state: &SpawnedState) -> SessionFacts {
            SessionFacts {
                id: state.id.clone(),
                root_pid: 10,
                cwd: state.cwd.clone(),
                title: "Terminal".to_owned(),
                at_prompt: true,
                reported_cmd: None,
                foreground_basenames: vec![state.identity.tool.to_string()],
                foreground_pids: Vec::new(),
                capabilities: SessionCapabilities::ALL,
                spawn_identity: Some(state.identity.clone()),
            }
        }
    }

    impl SessionInventory for FakeBackend {
        fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
            let mut facts = match self.discoveries.lock().unwrap().pop_front() {
                Some(discovered) => {
                    *self.last.lock().unwrap() = discovered.clone();
                    discovered
                }
                None => self.last.lock().unwrap().clone(),
            };
            if self.reveal_spawned {
                if let Some(state) = self.spawned.lock().unwrap().clone() {
                    if !facts.iter().any(|facts| facts.id == state.id) {
                        match self.dying_spawn {
                            false => facts.push(Self::spawned_facts(&state)),
                            true if self.spawn_reveals.fetch_add(1, Ordering::SeqCst) == 0 => {
                                facts.push(Self::shell_only_facts(&state));
                            }
                            true => {}
                        }
                    }
                }
            }
            Ok(facts)
        }
    }

    impl ScreenReader for FakeBackend {
        fn read_screen(&self, target: &SessionBinding) -> Result<String, TerminalError> {
            Err(TerminalError::Unsupported {
                target: target.session_id().clone(),
                capability: "screen reading",
            })
        }
    }

    impl SessionFocus for FakeBackend {
        fn focus(&self, _target: &SessionBinding) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl TextInput for FakeBackend {
        fn send_text(
            &self,
            _target: &SessionBinding,
            _text: &str,
            _mode: DeliveryMode,
        ) -> Result<(), TerminalError> {
            Ok(())
        }

        fn send_key(&self, _target: &SessionBinding, _key: &str) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl TerminalBackend for FakeBackend {
        fn read_screen_from_snapshot(
            &self,
            _snapshot: &TerminalSnapshot,
            target: &SessionBinding,
        ) -> Result<String, TerminalError> {
            Err(TerminalError::Unsupported {
                target: target.session_id().clone(),
                capability: "screen reading",
            })
        }

        fn id(&self) -> &BackendId {
            &self.id
        }

        fn spawner(&self) -> Option<&dyn SessionSpawner> {
            Some(self)
        }
    }

    impl SessionSpawner for FakeBackend {
        fn supports(&self, surface: SpawnSurface) -> bool {
            self.supported.contains(&surface)
        }

        fn spawn(&self, request: &SpawnRequest) -> Result<SessionId, TerminalError> {
            self.spawn_count.fetch_add(1, AtomicOrdering::Relaxed);
            *self.last_request.lock().unwrap() = Some(request.clone());
            if self.refuse_spawn {
                return Err(TerminalError::SpawnFailed {
                    backend: self.id.clone(),
                    message: "refused by fake".to_owned(),
                });
            }
            let id = SessionId::new(self.id.clone(), format!("spawn-{}", request.identity.key))
                .map_err(|error| TerminalError::SpawnFailed {
                    backend: self.id.clone(),
                    message: error.to_string(),
                })?;
            *self.spawned.lock().unwrap() = Some(SpawnedState {
                id: id.clone(),
                identity: request.identity.clone(),
                cwd: request.cwd.to_string_lossy().into_owned(),
            });
            Ok(id)
        }
    }

    fn harness(discoveries: Vec<Vec<SessionFacts>>) -> (TerminalSessionService, Arc<FakeBackend>) {
        harness_with(FakeBackend::new(discoveries))
    }

    fn harness_with(backend: FakeBackend) -> (TerminalSessionService, Arc<FakeBackend>) {
        let backend = Arc::new(backend);
        let terminals = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();
        (terminals, backend)
    }

    fn locks(root: &tempfile::TempDir) -> SpawnLocks {
        SpawnLocks::with_dir(root.path().join("locks"))
    }

    fn identity(key: &str, tool: &str, surface: SpawnSurface) -> SpawnIdentity {
        SpawnIdentity {
            key: SpawnKey::new(key).unwrap(),
            tool: CliToolId::new(tool).unwrap(),
            surface,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_spawn(
        terminals: &TerminalSessionService,
        tool: &str,
        cwd: &str,
        key: Option<&str>,
        surface: Option<&str>,
        model: Option<&str>,
        title: Option<&str>,
        config: Option<SpawnSurface>,
        locks: &SpawnLocks,
    ) -> Result<SpawnOutcome> {
        let root = tempfile::TempDir::new().unwrap();
        let pending = super::super::bridge::PendingBridgeStore::with_dir(root.path().to_path_buf());
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        run_spawn_with(
            terminals, tool, cwd, key, surface, model, title, config, locks, &ledger, false, false,
            None, None, None, &pending,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_spawn_with(
        terminals: &TerminalSessionService,
        tool: &str,
        cwd: &str,
        key: Option<&str>,
        surface: Option<&str>,
        model: Option<&str>,
        title: Option<&str>,
        config: Option<SpawnSurface>,
        locks: &SpawnLocks,
        ledger: &SpawnLedger,
        background: bool,
        autoclose: bool,
        resume: Option<bool>,
        task: Option<&str>,
        cap: Option<SpawnCapConfig>,
        pending: &super::super::bridge::PendingBridgeStore,
    ) -> Result<SpawnOutcome> {
        spawn_or_reuse(
            terminals,
            &CliSessionInterpreter::system(),
            tool,
            cwd,
            key,
            surface,
            model,
            title,
            config,
            cap.as_ref(),
            locks,
            ledger,
            background,
            autoclose,
            false,
            resume,
            task,
            None,
            pending,
            &std::path::PathBuf::from("."),
            None,
        )
    }

    #[test]
    fn spawn_args_parse_required_and_optional_flags() {
        let parsed = parse_args(&[
            "--tool".into(),
            "codex".into(),
            "--cwd".into(),
            "/work/project".into(),
            "--key".into(),
            "lane-1".into(),
            "--surface".into(),
            "os-window".into(),
            "--model".into(),
            "flash-x".into(),
            "--title".into(),
            "Lane One".into(),
            "--task".into(),
            "implement the fix".into(),
            "--background".into(),
            "--resume".into(),
            "--group".into(),
            "research".into(),
        ])
        .unwrap();
        assert_eq!(parsed.tool, "codex");
        assert_eq!(parsed.cwd, "/work/project");
        assert_eq!(parsed.key.as_deref(), Some("lane-1"));
        assert_eq!(parsed.surface.as_deref(), Some("os-window"));
        assert_eq!(parsed.model.as_deref(), Some("flash-x"));
        assert_eq!(parsed.title.as_deref(), Some("Lane One"));
        assert_eq!(parsed.task.as_deref(), Some("implement the fix"));
        assert!(parsed.background);
        assert_eq!(parsed.resume, Some(true));
        assert_eq!(parsed.group.as_deref(), Some("research"));

        let parsed =
            parse_args(&["--tool".into(), "pi".into(), "--cwd".into(), "/tmp".into()]).unwrap();
        assert_eq!(parsed.tool, "pi");
        assert_eq!(parsed.cwd, "/tmp");
        assert_eq!(parsed.key, None);
        assert_eq!(parsed.surface, None);
        assert_eq!(parsed.model, None);
        assert_eq!(parsed.title, None);
        assert_eq!(parsed.task, None);
        assert!(!parsed.background);
        assert_eq!(parsed.resume, None);
        assert_eq!(parsed.group, None);

        let parsed = parse_args(&[
            "--tool".into(),
            "pi".into(),
            "--cwd".into(),
            "/tmp".into(),
            "--no-resume".into(),
        ])
        .unwrap();
        assert_eq!(parsed.resume, Some(false));
    }

    #[test]
    fn parse_args_reads_a_lane_set() {
        let parsed = parse_args(&[
            "--tool".into(),
            "pi".into(),
            "--cwd".into(),
            ".".into(),
            "--lanes".into(),
            r#"[{"key":"a","task":"first"},{"key":"b","task":"second","title":"B"}]"#.into(),
        ])
        .unwrap();
        assert_eq!(parsed.lanes.len(), 2);
        assert_eq!(parsed.lanes[0].key, "a");
        assert_eq!(parsed.lanes[0].task, "first");
        assert_eq!(parsed.lanes[1].title.as_deref(), Some("B"));
        assert!(parsed.key.is_none());
    }

    #[test]
    fn parse_args_refuses_a_lane_set_beside_a_single_lane() {
        let error = parse_args(&[
            "--tool".into(),
            "pi".into(),
            "--cwd".into(),
            ".".into(),
            "--task".into(),
            "do it".into(),
            "--lanes".into(),
            r#"[{"key":"a","task":"first"}]"#.into(),
        ])
        .err()
        .expect("a lane set beside a single lane is refused")
        .to_string();
        assert!(error.contains("--lanes carries every lane"), "{error}");
    }

    #[test]
    fn a_generated_group_is_stable_per_lane_set() {
        let lane = |key: &str| LaneSpec {
            key: key.to_owned(),
            task: "work".to_owned(),
            title: None,
            silent_wake: None,
        };
        let set = [lane("a"), lane("b")];
        assert_eq!(generated_group(&set), generated_group(&set));
        assert_ne!(generated_group(&set), generated_group(&[lane("a")]));
        assert!(generated_group(&set).starts_with("set-2-"));
    }

    #[test]
    fn spawn_args_reject_the_removed_autoclose_flags() {
        for flag in ["--auto-close", "--no-auto-close"] {
            let error = match parse_args(&[
                "--tool".into(),
                "pi".into(),
                "--cwd".into(),
                "/tmp".into(),
                flag.into(),
            ]) {
                Ok(_) => panic!("{flag} must be rejected as an unknown flag"),
                Err(error) => error.to_string(),
            };
            assert!(error.contains("unknown spawn flag"), "{error}");
            assert!(error.contains(flag), "{error}");
        }
    }

    #[test]
    fn spawn_ledger_records_roundtrip_and_are_keyed_by_the_spawn_key_and_cwd() {
        let root = tempfile::TempDir::new().unwrap();
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let key = SpawnKey::new("lane-ledger").unwrap();

        assert!(ledger.load(&key, "/work").unwrap().is_none());
        ledger
            .record(
                &key,
                &CliToolId::new("pi").unwrap(),
                SpawnSurface::Tab,
                "/work",
                Some("flash-x"),
                Some("01a00a6b"),
            )
            .unwrap();
        let record = ledger.load(&key, "/work").unwrap().unwrap();
        assert_eq!(record.key, "lane-ledger");
        assert_eq!(record.tool, "pi");
        assert_eq!(record.surface, "tab");
        assert_eq!(record.cwd, "/work");
        assert_eq!(record.model.as_deref(), Some("flash-x"));
        assert_eq!(record.external_id.as_deref(), Some("01a00a6b"));
        assert_eq!(record.created_at, record.last_seen);

        ledger
            .record(
                &key,
                &CliToolId::new("pi").unwrap(),
                SpawnSurface::Tab,
                "/work",
                None,
                None,
            )
            .unwrap();
        let record = ledger.load(&key, "/work").unwrap().unwrap();
        assert_eq!(
            record.external_id.as_deref(),
            Some("01a00a6b"),
            "a refresh without an id must keep the last captured external id"
        );
        assert_eq!(
            record.model.as_deref(),
            Some("flash-x"),
            "a refresh without a model keeps the previous model"
        );
        assert_eq!(
            record.created_at, record.last_seen,
            "the first record pins created_at and the refresh bumps last_seen"
        );
    }

    #[test]
    fn a_same_key_spawn_in_another_cwd_does_not_reuse_the_stale_external_id() {
        let root = tempfile::TempDir::new().unwrap();
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let key = SpawnKey::new("lane-ledger").unwrap();

        ledger
            .record(
                &key,
                &CliToolId::new("pi").unwrap(),
                SpawnSurface::Tab,
                "/worktree-a",
                None,
                Some("01a00a6b"),
            )
            .unwrap();
        assert_eq!(
            ledger
                .load(&key, "/worktree-a")
                .unwrap()
                .unwrap()
                .external_id
                .as_deref(),
            Some("01a00a6b"),
            "the record for the recorded cwd stays visible"
        );
        assert!(
            ledger.load(&key, "/worktree-b").unwrap().is_none(),
            "the same key in a different cwd must not see the stale record"
        );

        ledger
            .record(
                &key,
                &CliToolId::new("pi").unwrap(),
                SpawnSurface::Tab,
                "/worktree-b",
                None,
                Some("01a00a6c"),
            )
            .unwrap();
        assert_eq!(
            ledger
                .load(&key, "/worktree-a")
                .unwrap()
                .unwrap()
                .external_id
                .as_deref(),
            Some("01a00a6b"),
            "the worktree-a record survives the worktree-b spawn"
        );
        assert_eq!(
            ledger
                .load(&key, "/worktree-b")
                .unwrap()
                .unwrap()
                .external_id
                .as_deref(),
            Some("01a00a6c"),
            "the worktree-b record holds its own id"
        );
    }

    #[test]
    fn decide_resume_covers_every_skip_reason_and_applies_when_the_data_allows() {
        let interpreter = CliSessionInterpreter::system();
        let pi = CliToolId::new("pi").unwrap();
        let claude = CliToolId::new("claude").unwrap();
        let record = |tool: &str, cwd: &str, external_id: Option<&str>| SpawnRecord {
            key: "lane-1".to_owned(),
            tool: tool.to_owned(),
            surface: "tab".to_owned(),
            cwd: cwd.to_owned(),
            model: Some("flash-x".to_owned()),
            external_id: external_id.map(str::to_owned),
            created_at: 1,
            last_seen: 1,
        };
        let cases = [
            (
                Some(false),
                Some(record("pi", "/work", Some("01a00a6b"))),
                &pi,
                "/work",
                "opted_out",
            ),
            (None, None, &pi, "/work", "no_prior_record"),
            (
                None,
                Some(record("claude", "/work", Some("01a00a6b"))),
                &pi,
                "/work",
                "tool_mismatch",
            ),
            (
                None,
                Some(record("pi", "/elsewhere", Some("01a00a6b"))),
                &pi,
                "/work",
                "cwd_mismatch",
            ),
            (
                None,
                Some(record("pi", "/work", None)),
                &pi,
                "/work",
                "no_external_id",
            ),
            (
                None,
                Some(record("pi", "/work", Some(""))),
                &pi,
                "/work",
                "no_external_id",
            ),
            (
                None,
                Some(record("kimi", "/work", Some("01a00a6b"))),
                &CliToolId::new("kimi").unwrap(),
                "/work",
                "tool_has_no_resume",
            ),
        ];
        for (requested, record, tool, cwd, expected) in cases {
            assert_eq!(
                decide_resume(requested, record.as_ref(), tool, cwd, &interpreter),
                ResumeDecision::Skip { reason: expected },
                "requested: {requested:?} record: {record:?} tool: {tool}"
            );
        }

        let applied = decide_resume(
            None,
            Some(&record("pi", "/work", Some("01a00a6b"))),
            &pi,
            "/work",
            &interpreter,
        );
        assert_eq!(
            applied,
            ResumeDecision::Apply {
                external_id: "01a00a6b".to_owned(),
                args: vec!["--session".to_owned(), "01a00a6b".to_owned()],
            }
        );
        assert_eq!(
            decide_resume(
                Some(true),
                Some(&record("pi", "/work", Some("01a00a6b"))),
                &pi,
                "/work",
                &interpreter,
            ),
            applied,
            "an explicit --resume and the automatic default behave identically"
        );
        assert_eq!(
            decide_resume(
                None,
                Some(&record("claude", "/work", Some("01a00a6b"))),
                &claude,
                "/work",
                &interpreter,
            ),
            ResumeDecision::Apply {
                external_id: "01a00a6b".to_owned(),
                args: vec!["--resume".to_owned(), "01a00a6b".to_owned()],
            }
        );
    }

    #[test]
    fn ledger_record_keeps_the_stored_model_when_only_the_external_id_refreshes() {
        let root = tempfile::TempDir::new().unwrap();
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let key = SpawnKey::new("lane-stomp").unwrap();
        ledger
            .record(
                &key,
                &CliToolId::new("pi").unwrap(),
                SpawnSurface::Tab,
                "/work",
                Some("flash-x"),
                Some("01a00a6b"),
            )
            .unwrap();
        ledger
            .record(
                &key,
                &CliToolId::new("pi").unwrap(),
                SpawnSurface::Tab,
                "/work",
                None,
                Some("01a00a6c"),
            )
            .unwrap();
        let record = ledger.load(&key, "/work").unwrap().unwrap();
        assert_eq!(
            record.model.as_deref(),
            Some("flash-x"),
            "a refresh that passes None as the model must keep the stored model"
        );
        assert_eq!(
            record.external_id.as_deref(),
            Some("01a00a6c"),
            "the refresh still updates the external id"
        );
    }

    #[test]
    fn a_tab_still_sitting_at_a_shell_prompt_never_counts_as_ready() {
        let identity = identity("lane-shell", "pi", SpawnSurface::Tab);
        let shell = SessionFacts {
            foreground_basenames: vec!["zsh".to_owned()],
            ..FakeBackend::facts("9", "lane-shell", "pi", "/work")
        };
        let session_id = shell.id.clone();
        let (terminals, _backend) = harness(vec![vec![shell]]);

        let error = poll_ready(
            &terminals,
            &CliSessionInterpreter::system(),
            &session_id,
            &identity,
            Duration::from_millis(300),
            None,
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("was not live and bridgeable"),
            "a tab whose tool never started must never be handed back as a lane: {error}"
        );
        assert!(
            error.contains("described=generic"),
            "the error has to name what the tab is actually running: {error}"
        );
    }

    #[test]
    fn a_background_lane_whose_tool_never_started_is_an_error_not_a_queued_round() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = super::super::bridge::PendingBridgeStore::with_dir(root.path().to_path_buf());
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, _backend) = harness_with(FakeBackend::new(vec![vec![]]).dying_spawn());
        let locks = locks(&root);

        let error = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-dead-tool"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
            &ledger,
            true,
            false,
            None,
            Some("implement the fix"),
            None,
            &pending,
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("exited before `pi` was running in it"),
            "a tab that never ran the requested tool must not be reported as a live lane: {error}"
        );
        assert!(
            ledger
                .load(&SpawnKey::new("lane-dead-tool").unwrap(), "/work")
                .unwrap()
                .is_none(),
            "a lane that never came up must not leave a spawn record behind"
        );
    }

    #[test]
    fn only_this_drivers_open_ungrouped_round_blocks_a_second_spawn() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = super::super::bridge::PendingBridgeStore::with_dir(root.path().to_path_buf());
        let foreign = SessionBinding::from_str("v1:kitty:foreign-lane:100").unwrap();
        let own = SessionBinding::from_str("v1:kitty:own-lane:200").unwrap();
        pending
            .start_with_label(
                &foreign,
                "QOL_BRIDGE_DONE_foreign",
                "v1:kitty:some-other-terminal:900",
                false,
                None,
                Some("lane-foreign"),
                false,
            )
            .unwrap();
        let live = [foreign.token(), own.token()]
            .into_iter()
            .collect::<std::collections::HashSet<_>>();

        refuse_second_ungrouped_lane(&pending, "v1:kitty:this-terminal:42", &live)
            .expect("another terminal's open ungrouped round must not block this spawn");

        pending
            .start_with_label(
                &own,
                "QOL_BRIDGE_DONE_own",
                "v1:kitty:this-terminal:42",
                false,
                None,
                Some("lane-own"),
                false,
            )
            .unwrap();
        let error = refuse_second_ungrouped_lane(&pending, "v1:kitty:this-terminal:42", &live)
            .expect_err("this driver's own open ungrouped round still blocks a second lane");
        assert!(
            error.to_string().contains("lane-own"),
            "the refusal must name this caller's own open round: {error}"
        );
    }

    #[test]
    fn keyed_launch_after_a_dead_lane_embeds_the_resume_prompt() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = super::super::bridge::PendingBridgeStore::with_dir(root.path().to_path_buf());
        let dead = SessionBinding::from_str("v1:kitty:spawn-lane-rs:200").unwrap();
        pending
            .start(&dead, "QOL_BRIDGE_DONE_dead", "v1:kitty:8:800", true, None)
            .unwrap();
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, backend) = harness(vec![vec![]]);
        let locks = locks(&root);

        let outcome = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-rs"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
            &ledger,
            true,
            false,
            None,
            Some("implement the fix"),
            None,
            &pending,
        )
        .unwrap();
        assert!(outcome.background);
        assert!(!outcome.reused);
        let request = backend.last_request.lock().unwrap().clone().unwrap();
        let prompt = request.launch.args.last().unwrap();
        assert!(prompt.contains("resuming"), "{prompt}");
        assert!(prompt.contains("persisted session"), "{prompt}");
        assert!(prompt.contains("implement the fix"));
        assert!(
            !prompt.contains("Act as the implementation agent"),
            "a keyed respawn of a dead lane must not reuse the plain Lane prompt"
        );
        pending
            .close_checkpoints_for_session(&outcome.session)
            .unwrap();

        let outcome = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("fresh-lane"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
            &ledger,
            true,
            false,
            None,
            Some("another fix"),
            None,
            &pending,
        )
        .unwrap();
        assert!(outcome.background);
        let request = backend.last_request.lock().unwrap().clone().unwrap();
        let prompt = request.launch.args.last().unwrap();
        assert!(
            prompt.contains("Act as the implementation agent"),
            "a fresh key in the same store still gets the plain Lane prompt"
        );
        assert!(!prompt.contains("resuming"));
    }

    #[test]
    fn spawn_model_config_parses_and_missing_file_stays_absent() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("sessions.toml");
        assert_eq!(config_spawn_model_at(&path).unwrap(), None);

        fs::write(&path, "spawn_model = \"flash-x\"\n").unwrap();
        assert_eq!(
            config_spawn_model_at(&path).unwrap().as_deref(),
            Some("flash-x")
        );

        fs::write(
            &path,
            "spawn_surface = \"tab\"\nspawn_model = \"flash-y\"\n",
        )
        .unwrap();
        assert_eq!(
            config_spawn_model_at(&path).unwrap().as_deref(),
            Some("flash-y")
        );
    }

    #[test]
    fn wrap_launch_runs_inside_a_systemd_scope_when_capping_is_resolved() {
        let launch = CliLaunchProgram {
            program: "pi".to_owned(),
            args: vec!["--model".to_owned(), "flash-x".to_owned()],
        };

        let unwrapped = wrap_launch(&launch, None);
        assert_eq!(unwrapped.program, "pi");
        assert_eq!(
            unwrapped.args,
            vec!["--model".to_owned(), "flash-x".to_owned()]
        );

        let wrapped = wrap_launch(&launch, Some(&SpawnCapConfig::default()));
        assert_eq!(wrapped.program, "systemd-run");
        assert_eq!(
            wrapped.args,
            vec![
                "--user".to_owned(),
                "--scope".to_owned(),
                "--quiet".to_owned(),
                "--slice=qol-agents.slice".to_owned(),
                "-p".to_owned(),
                "CPUWeight=40".to_owned(),
                "-p".to_owned(),
                "IOWeight=40".to_owned(),
                "--".to_owned(),
                "pi".to_owned(),
                "--model".to_owned(),
                "flash-x".to_owned(),
            ]
        );
    }

    #[test]
    fn wrap_launch_adds_the_quota_property_only_when_configured() {
        let launch = CliLaunchProgram {
            program: "codex".to_owned(),
            args: Vec::new(),
        };
        let cap = SpawnCapConfig {
            enabled: true,
            cpu_weight: 25,
            io_weight: 20,
            cpu_quota: Some("600%".to_owned()),
        };
        let wrapped = wrap_launch(&launch, Some(&cap));
        assert_eq!(wrapped.program, "systemd-run");
        assert_eq!(
            wrapped.args,
            vec![
                "--user".to_owned(),
                "--scope".to_owned(),
                "--quiet".to_owned(),
                "--slice=qol-agents.slice".to_owned(),
                "-p".to_owned(),
                "CPUWeight=25".to_owned(),
                "-p".to_owned(),
                "IOWeight=20".to_owned(),
                "-p".to_owned(),
                "CPUQuota=600%".to_owned(),
                "--".to_owned(),
                "codex".to_owned(),
            ]
        );

        let disabled = SpawnCapConfig {
            cpu_quota: Some("600%".to_owned()),
            ..cap
        };
        let wrapped = wrap_launch(
            &launch,
            Some(&SpawnCapConfig {
                enabled: false,
                ..disabled
            }),
        );
        assert_eq!(wrapped.program, "codex");
        assert!(wrapped.args.is_empty());
    }

    #[test]
    fn spawn_cap_config_parses_keys_and_defaults_to_weight_based_capping() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("sessions.toml");
        assert_eq!(
            config_spawn_cap_at(&path).unwrap(),
            SpawnCapConfig::default()
        );

        fs::write(
            &path,
            "spawn_cpu_weight = 25\nspawn_io_weight = 20\nspawn_cpu_quota = \"600%\"\n",
        )
        .unwrap();
        assert_eq!(
            config_spawn_cap_at(&path).unwrap(),
            SpawnCapConfig {
                enabled: true,
                cpu_weight: 25,
                io_weight: 20,
                cpu_quota: Some("600%".to_owned()),
            }
        );

        fs::write(&path, "spawn_cap = false\nspawn_cpu_quota = \"300%\"\n").unwrap();
        assert_eq!(
            config_spawn_cap_at(&path).unwrap(),
            SpawnCapConfig {
                enabled: false,
                cpu_quota: Some("300%".to_owned()),
                ..SpawnCapConfig::default()
            }
        );

        fs::write(&path, "spawn_cpu_weight = 0\n").unwrap();
        let error = config_spawn_cap_at(&path).unwrap_err().to_string();
        assert!(error.contains("spawn_cpu_weight"), "{error}");
        assert!(error.contains("10000"), "{error}");

        fs::write(&path, "spawn_io_weight = 10001\n").unwrap();
        let error = config_spawn_cap_at(&path).unwrap_err().to_string();
        assert!(error.contains("spawn_io_weight"), "{error}");

        fs::write(&path, "spawn_cpu_quota = \"  \"\n").unwrap();
        let error = config_spawn_cap_at(&path).unwrap_err().to_string();
        assert!(error.contains("spawn_cpu_quota"), "{error}");
    }

    #[test]
    fn capped_launch_carries_the_scope_wrapper_into_the_spawn_request() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = super::super::bridge::PendingBridgeStore::with_dir(root.path().to_path_buf());
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, backend) = harness(vec![vec![]]);
        let locks = locks(&root);

        let outcome = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-cap"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
            &ledger,
            false,
            false,
            None,
            None,
            Some(SpawnCapConfig::default()),
            &pending,
        )
        .unwrap();
        assert!(!outcome.reused);
        assert_eq!(outcome.model.as_deref(), Some("flash-x"));

        let request = backend.last_request.lock().unwrap().clone().unwrap();
        assert_eq!(request.launch.program, "systemd-run");
        assert_eq!(
            request.launch.args,
            vec![
                "--user".to_owned(),
                "--scope".to_owned(),
                "--quiet".to_owned(),
                "--slice=qol-agents.slice".to_owned(),
                "-p".to_owned(),
                "CPUWeight=40".to_owned(),
                "-p".to_owned(),
                "IOWeight=40".to_owned(),
                "--".to_owned(),
                "pi".to_owned(),
                "--model".to_owned(),
                "flash-x".to_owned(),
            ]
        );
        assert_eq!(request.identity.key.to_string(), "lane-cap");
        assert_eq!(request.cwd, std::path::PathBuf::from(&cwd));
    }

    #[test]
    fn capped_resume_launch_keeps_the_resume_args_after_the_scope_wrapper() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = super::super::bridge::PendingBridgeStore::with_dir(root.path().to_path_buf());
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let key = SpawnKey::new("lane-cap-resume").unwrap();
        ledger
            .record(
                &key,
                &CliToolId::new("pi").unwrap(),
                SpawnSurface::Tab,
                &cwd,
                Some("flash-x"),
                Some("01a00a6b"),
            )
            .unwrap();
        let (terminals, backend) = harness(vec![vec![]]);
        let locks = locks(&root);

        let outcome = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-cap-resume"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
            &ledger,
            false,
            false,
            None,
            None,
            Some(SpawnCapConfig::default()),
            &pending,
        )
        .unwrap();
        assert_eq!(outcome.resume, Some("applied"));
        assert_eq!(outcome.resume_detail.as_deref(), Some("01a00a6b"));

        let request = backend.last_request.lock().unwrap().clone().unwrap();
        assert_eq!(request.launch.program, "systemd-run");
        assert_eq!(
            request.launch.args,
            vec![
                "--user".to_owned(),
                "--scope".to_owned(),
                "--quiet".to_owned(),
                "--slice=qol-agents.slice".to_owned(),
                "-p".to_owned(),
                "CPUWeight=40".to_owned(),
                "-p".to_owned(),
                "IOWeight=40".to_owned(),
                "--".to_owned(),
                "pi".to_owned(),
                "--session".to_owned(),
                "01a00a6b".to_owned(),
                "--model".to_owned(),
                "flash-x".to_owned(),
            ]
        );
    }

    #[test]
    fn model_resolution_prefers_the_explicit_override_over_config() {
        assert_eq!(
            resolve_model_with(Some("flash-x"), Some("flash-y".to_owned())).unwrap(),
            Some("flash-x".to_owned())
        );
        assert_eq!(
            resolve_model_with(None, Some("flash-y".to_owned())).unwrap(),
            Some("flash-y".to_owned())
        );
        assert_eq!(resolve_model_with(None, None).unwrap(), None);
    }

    #[test]
    fn a_model_outside_the_allowed_list_is_refused_by_name() {
        let allowed = vec!["deepseek-v4-flash".to_owned()];
        let error = enforce_allowed_model_with("deepseek-v4-pro", &allowed)
            .expect_err("a tier the host does not allow must not launch");
        let message = error.to_string();
        assert!(message.contains("deepseek-v4-pro"), "{message}");
        assert!(message.contains("deepseek-v4-flash"), "{message}");
    }

    #[test]
    fn an_allowed_model_launches_and_an_empty_list_restricts_nothing() {
        let allowed = vec!["deepseek-v4-flash".to_owned(), "kimi-k2".to_owned()];
        assert!(enforce_allowed_model_with("deepseek-v4-flash", &allowed).is_ok());
        assert!(enforce_allowed_model_with("kimi-k2", &allowed).is_ok());
        assert!(enforce_allowed_model_with("deepseek-v4-pro", &[]).is_ok());
    }

    #[test]
    fn a_configured_spawn_model_alone_becomes_the_whole_allowed_list() {
        assert_eq!(
            allowed_models_from(None, Some("deepseek-v4-flash".to_owned())),
            vec!["deepseek-v4-flash".to_owned()]
        );
        assert_eq!(
            allowed_models_from(Some(Vec::new()), Some("deepseek-v4-flash".to_owned())),
            vec!["deepseek-v4-flash".to_owned()]
        );
        assert!(allowed_models_from(None, None).is_empty());
        assert_eq!(
            allowed_models_from(Some(vec!["kimi-k2".to_owned()]), Some("flash".to_owned())),
            vec!["kimi-k2".to_owned()]
        );
    }

    #[test]
    fn model_args_map_registered_tools_and_reject_unknown_tools() {
        for tool in ["pi", "codex", "claude", "kimi"] {
            let args = model_args(&CliToolId::new(tool).unwrap(), "flash-x").unwrap();
            assert_eq!(
                args,
                vec!["--model".to_owned(), "flash-x".to_owned()],
                "tool: {tool}"
            );
        }
        let error = model_args(&CliToolId::new("generic").unwrap(), "flash-x")
            .unwrap_err()
            .to_string();
        assert!(error.contains("no model override flag"), "{error}");
    }

    #[test]
    fn launch_requires_a_non_empty_model_but_reuse_stays_exempt() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, backend) = harness(vec![vec![]]);
        let locks = locks(&root);

        for model in [None, Some(""), Some("   ")] {
            let error = run_spawn(
                &terminals,
                "codex",
                &cwd,
                Some("lane-1"),
                None,
                model,
                None,
                None,
                &locks,
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("--model"), "model: {model:?} error: {error}");
        }
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 0);

        let outcome = run_spawn(
            &terminals,
            "codex",
            &cwd,
            Some("lane-1"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
        )
        .unwrap();
        assert!(!outcome.reused);
        assert_eq!(outcome.model.as_deref(), Some("flash-x"));
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 1);

        let reused = run_spawn(
            &terminals,
            "codex",
            &cwd,
            Some("lane-1"),
            None,
            None,
            None,
            None,
            &locks,
        )
        .unwrap();
        assert!(reused.reused);
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 1);
    }

    #[test]
    fn background_launch_embeds_the_task_queues_the_round_and_skips_the_liveness_wait() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = super::super::bridge::PendingBridgeStore::with_dir(root.path().to_path_buf());
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, backend) = harness(vec![vec![]]);
        let locks = locks(&root);

        let outcome = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-bg"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
            &ledger,
            true,
            false,
            None,
            Some("implement the fix"),
            None,
            &pending,
        )
        .unwrap();
        assert!(outcome.background);
        assert!(!outcome.reused);
        assert_eq!(outcome.session, "v1:kitty:spawn-lane-bg:10");
        assert_eq!(outcome.task_submitted, Some(true));
        assert_eq!(outcome.screen, None);
        let marker = outcome.completion_marker.as_deref().unwrap();
        assert!(marker.starts_with("QOL_BRIDGE_DONE_"));
        assert_eq!(
            outcome.next_command.as_deref(),
            Some("qol sessions next v1:kitty:spawn-lane-bg:10")
        );

        let request = backend.last_request.lock().unwrap().clone().unwrap();
        assert_eq!(request.launch.program, "pi");
        assert_eq!(
            request.launch.args[..2],
            ["--model".to_owned(), "flash-x".to_owned()]
        );
        let prompt = &request.launch.args[2];
        assert!(prompt.contains("[qol session bridge]"));
        assert!(prompt.contains("implement the fix"));
        assert!(prompt.contains("QOL_BRIDGE_DONE_"));
        assert!(
            prompt.ends_with(&format!("Completion token:\n{marker}")),
            "the pi prompt prints the joined token once, alone, at the end: {prompt}"
        );
        assert!(!prompt.contains("fragments"), "{prompt}");

        let binding: SessionBinding = outcome.session.parse().unwrap();
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert_eq!(round.completion_marker, marker);
        assert!(!round.completed);
        assert_eq!(
            pending.role(&binding).unwrap(),
            super::super::bridge::Role::Lane,
            "a background launch writes the lane role marker"
        );

        let foreground = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-bg"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
            &ledger,
            false,
            false,
            None,
            Some("implement the fix"),
            None,
            &pending,
        )
        .unwrap_err()
        .to_string();
        assert!(
            foreground.contains("screen read failed while waiting for its live UI"),
            "foreground still waits for the live UI: {foreground}"
        );
    }

    #[test]
    fn launch_and_reuse_write_the_lane_role_marker() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = super::super::bridge::PendingBridgeStore::with_dir(root.path().to_path_buf());
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, backend) = harness(vec![vec![]]);
        let locks = locks(&root);

        let outcome = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-role"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
            &ledger,
            false,
            false,
            None,
            None,
            None,
            &pending,
        )
        .unwrap();
        assert!(!outcome.reused);
        let binding: SessionBinding = outcome.session.parse().unwrap();
        assert_eq!(
            pending.role(&binding).unwrap(),
            super::super::bridge::Role::Lane,
            "a fresh launch writes the lane role marker"
        );

        let reused = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-role"),
            None,
            None,
            None,
            None,
            &locks,
            &ledger,
            false,
            false,
            None,
            None,
            None,
            &pending,
        )
        .unwrap();
        assert!(reused.reused);
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 1);
        let binding: SessionBinding = reused.session.parse().unwrap();
        assert_eq!(
            pending.role(&binding).unwrap(),
            super::super::bridge::Role::Lane,
            "the keyed reuse path writes the lane role marker idempotently"
        );

        let reused = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-role"),
            None,
            None,
            None,
            None,
            &locks,
            &ledger,
            false,
            false,
            None,
            None,
            None,
            &pending,
        )
        .unwrap();
        assert!(reused.reused);
        let binding: SessionBinding = reused.session.parse().unwrap();
        assert_eq!(
            pending.role(&binding).unwrap(),
            super::super::bridge::Role::Lane,
            "set_role stays idempotent across repeated reuse"
        );
    }

    #[test]
    fn background_requires_a_task_and_never_bypasses_model_enforcement() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = super::super::bridge::PendingBridgeStore::with_dir(root.path().to_path_buf());
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, backend) = harness(vec![vec![]]);
        let locks = locks(&root);

        let no_task = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-bg"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
            &ledger,
            true,
            false,
            None,
            None,
            None,
            &pending,
        )
        .unwrap_err()
        .to_string();
        assert!(no_task.contains("--task"), "{no_task}");

        let no_model = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-bg"),
            None,
            None,
            None,
            None,
            &locks,
            &ledger,
            true,
            false,
            None,
            Some("implement the fix"),
            None,
            &pending,
        )
        .unwrap_err()
        .to_string();
        assert!(no_model.contains("--model"), "{no_model}");
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn autoclose_marks_new_spawns_and_reuse_stays_allowed() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = super::super::bridge::PendingBridgeStore::with_dir(root.path().to_path_buf());
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, backend) = harness(vec![vec![]]);
        let locks = locks(&root);

        let outcome = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-auto"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
            &ledger,
            true,
            true,
            None,
            Some("implement the fix"),
            None,
            &pending,
        )
        .unwrap();
        assert!(outcome.autoclose);
        assert!(outcome.background);
        assert!(!outcome.reused);
        let binding: SessionBinding = outcome.session.parse().unwrap();
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(
            round.autoclose,
            "the queued round must carry the autoclose flag for the watcher"
        );

        let reused = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-auto"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
            &ledger,
            false,
            true,
            None,
            None,
            None,
            &pending,
        )
        .unwrap();
        assert!(
            reused.reused,
            "reuse stays allowed with autoclose; closing is decided by the spawn identity"
        );
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 1);
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(
            round.autoclose,
            "the reused lane keeps its spawn identity, so its round stays closable"
        );

        let plain = run_spawn_with(
            &terminals,
            "pi",
            &cwd,
            Some("lane-auto"),
            None,
            None,
            None,
            None,
            &locks,
            &ledger,
            false,
            false,
            None,
            None,
            None,
            &pending,
        )
        .unwrap();
        assert!(plain.reused, "plain reuse without autoclose stays allowed");
        assert!(!plain.autoclose);
    }

    #[test]
    fn cli_run_refuses_a_new_lane_without_a_model_and_allows_reuse_without_one() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, backend) = harness(vec![vec![]]);
        let locks = locks(&root);

        let parsed = parse_args(&[
            "--tool".into(),
            "codex".into(),
            "--cwd".into(),
            cwd.clone().into(),
            "--key".into(),
            "lane-1".into(),
        ])
        .unwrap();
        let error = run_with(&terminals, parsed, None, None, &locks, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("--model"), "{error}");
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 0);

        let parsed = parse_args(&[
            "--tool".into(),
            "codex".into(),
            "--cwd".into(),
            cwd.clone().into(),
            "--key".into(),
            "lane-1".into(),
            "--model".into(),
            "flash-x".into(),
        ])
        .unwrap();
        let outcome = run_with(
            &terminals,
            parsed,
            Some("flash-x".to_owned()),
            None,
            &locks,
            None,
        )
        .unwrap();
        assert!(!outcome.reused);
        assert_eq!(outcome.model.as_deref(), Some("flash-x"));
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 1);

        let parsed = parse_args(&[
            "--tool".into(),
            "codex".into(),
            "--cwd".into(),
            "ignored-cwd".into(),
            "--key".into(),
            "lane-1".into(),
        ])
        .unwrap();
        let outcome = run_with(&terminals, parsed, None, None, &locks, None).unwrap();
        assert!(outcome.reused);
        assert_eq!(outcome.cwd, cwd);
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 1);
    }

    #[test]
    fn spawn_args_reject_missing_required_flags_and_unknown_flags() {
        for args in [
            vec!["--cwd".into(), "/tmp".into()],
            vec!["--tool".into(), "pi".into()],
            vec!["--tool".into(), "pi".into(), "--cwd".into()],
            vec![
                "--tool".into(),
                "pi".into(),
                "--cwd".into(),
                "/tmp".into(),
                "--bogus".into(),
                "x".into(),
            ],
        ] {
            assert!(parse_args(&args).is_err(), "args: {args:?}");
        }
    }

    #[test]
    fn surface_resolution_prefers_flag_over_config_over_tab() {
        let cases = [
            (Some("tab"), Some(SpawnSurface::OsWindow), SpawnSurface::Tab),
            (
                Some("os-window"),
                Some(SpawnSurface::Tab),
                SpawnSurface::OsWindow,
            ),
            (None, Some(SpawnSurface::OsWindow), SpawnSurface::OsWindow),
            (None, None, SpawnSurface::Tab),
        ];
        for (flag, config, expected) in cases {
            assert_eq!(
                resolve_surface(flag, config).unwrap(),
                expected,
                "flag: {flag:?} config: {config:?}"
            );
        }
        assert!(resolve_surface(Some("floating"), None).is_err());
    }

    #[test]
    fn config_surface_parses_tokens_and_rejects_unknown_values() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("sessions.toml");
        assert_eq!(config_surface_at(&path).unwrap(), None);

        fs::write(&path, "spawn_surface = \"tab\"\n").unwrap();
        assert_eq!(config_surface_at(&path).unwrap(), Some(SpawnSurface::Tab));

        fs::write(&path, "spawn_surface = \"os-window\"\n").unwrap();
        assert_eq!(
            config_surface_at(&path).unwrap(),
            Some(SpawnSurface::OsWindow)
        );

        fs::write(&path, "spawn_surface = \"floating\"\n").unwrap();
        let error = config_surface_at(&path).unwrap_err().to_string();
        assert!(
            error.contains("invalid spawn_surface `floating`"),
            "{error}"
        );
        assert!(error.contains("tab"), "{error}");

        fs::write(&path, "spawn_surface =\n").unwrap();
        assert!(config_surface_at(&path).is_err());
    }

    #[test]
    fn generated_keys_are_collision_resistant_and_valid() {
        let first = generate_key();
        let second = generate_key();
        assert_ne!(first, second);
        assert_eq!(first.len(), 20);
        assert!(SpawnKey::new(&first).is_ok());
        assert!(SpawnKey::new(&second).is_ok());
    }

    #[test]
    fn key_lock_serializes_the_same_key_across_processes() {
        let root = tempfile::TempDir::new().unwrap();
        let locks = locks(&root);
        let key = SpawnKey::new("lane-1").unwrap();
        let guard = locks.acquire(&key).unwrap();
        let error = locks.acquire(&key).unwrap_err().to_string();
        assert!(
            error.contains("already handling spawn key `lane-1`"),
            "{error}"
        );
        drop(guard);
        locks.acquire(&key).unwrap();

        let other = SpawnKey::new("lane-2").unwrap();
        locks.acquire(&other).unwrap();
    }

    #[test]
    fn generic_and_unknown_tools_have_no_launch_strategy() {
        let root = tempfile::TempDir::new().unwrap();
        let (terminals, backend) = harness(vec![vec![]]);
        for tool in ["generic", "future-tool"] {
            let error = run_spawn(
                &terminals,
                tool,
                "/work/project",
                Some("lane-1"),
                None,
                None,
                None,
                None,
                &locks(&root),
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("no launch strategy for tool"),
                "tool: {tool} error: {error}"
            );
        }
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn fresh_spawn_sends_the_exact_typed_request_and_waits_for_readiness() {
        let (terminals, backend) = harness(vec![vec![]]);
        let root = tempfile::TempDir::new().unwrap();
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let outcome = run_spawn(
            &terminals,
            "codex",
            &cwd,
            Some("lane-1"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks(&root),
        )
        .unwrap();

        assert!(!outcome.reused);
        assert_eq!(outcome.session, "v1:kitty:spawn-lane-1:10");
        assert_eq!(outcome.tool, "codex");
        assert_eq!(outcome.key, "lane-1");
        assert_eq!(outcome.cwd, cwd);
        assert_eq!(outcome.surface, "tab");
        assert_eq!(outcome.model.as_deref(), Some("flash-x"));
        assert_eq!(outcome.title, "lane-1");
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 1);

        let expected = identity("lane-1", "codex", SpawnSurface::Tab);
        let request = backend.last_request.lock().unwrap().clone().unwrap();
        assert_eq!(request.identity, expected);
        assert_eq!(request.launch.program, "codex");
        assert_eq!(
            request.launch.args,
            vec!["--model".to_owned(), "flash-x".to_owned()]
        );
        assert_eq!(request.title.as_deref(), Some("lane-1"));
        assert_eq!(request.cwd, std::path::PathBuf::from(&cwd));
    }

    #[test]
    fn fresh_spawn_carries_an_explicit_title_into_the_launch() {
        let (terminals, backend) = harness(vec![vec![]]);
        let root = tempfile::TempDir::new().unwrap();
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let outcome = run_spawn(
            &terminals,
            "pi",
            &cwd,
            Some("lane-2"),
            None,
            Some("flash-x"),
            Some("Lane Two"),
            None,
            &locks(&root),
        )
        .unwrap();

        assert_eq!(outcome.title, "Lane Two");
        let request = backend.last_request.lock().unwrap().clone().unwrap();
        assert_eq!(request.title.as_deref(), Some("Lane Two"));
    }

    #[test]
    fn cli_key_omission_generates_a_key_while_mcp_style_keys_are_honored() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, _) = harness(vec![vec![]]);
        let outcome = run_spawn(
            &terminals,
            "pi",
            &cwd,
            Some("mcp-key"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks(&root),
        )
        .unwrap();
        assert_eq!(outcome.key, "mcp-key");

        let (terminals, _) = harness(vec![vec![]]);
        let outcome = run_spawn(
            &terminals,
            "pi",
            &cwd,
            None,
            None,
            Some("flash-x"),
            None,
            None,
            &locks(&root),
        )
        .unwrap();
        assert_eq!(outcome.key.len(), 20);
        assert!(SpawnKey::new(&outcome.key).is_ok());
    }

    #[test]
    fn reuse_returns_actual_facts_and_ignores_the_requested_cwd_and_surface() {
        let (terminals, backend) = harness(vec![vec![FakeBackend::facts(
            "7",
            "lane-1",
            "codex",
            "/actual/dir",
        )]]);
        let root = tempfile::TempDir::new().unwrap();
        let outcome = run_spawn(
            &terminals,
            "codex",
            "missing-requested-dir",
            Some("lane-1"),
            Some("os-window"),
            None,
            None,
            None,
            &locks(&root),
        )
        .unwrap();

        assert!(outcome.reused);
        assert_eq!(outcome.session, "v1:kitty:7:10");
        assert_eq!(outcome.tool, "codex");
        assert_eq!(outcome.key, "lane-1");
        assert_eq!(outcome.cwd, "/actual/dir");
        assert_eq!(outcome.surface, "tab");
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn same_key_with_a_different_tool_conflicts() {
        let (terminals, backend) = harness(vec![vec![FakeBackend::facts(
            "7", "lane-1", "claude", "/work",
        )]]);
        let root = tempfile::TempDir::new().unwrap();
        let error = run_spawn(
            &terminals,
            "codex",
            "/work",
            Some("lane-1"),
            None,
            None,
            None,
            None,
            &locks(&root),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("already held by tool `claude`"), "{error}");
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn multiple_matches_for_one_key_are_ambiguous() {
        let first = FakeBackend::facts("7", "lane-1", "codex", "/work");
        let second = FakeBackend::facts("8", "lane-1", "codex", "/work");
        let (terminals, backend) = harness(vec![vec![first, second]]);
        let root = tempfile::TempDir::new().unwrap();
        let error = run_spawn(
            &terminals,
            "codex",
            "/work",
            Some("lane-1"),
            None,
            None,
            None,
            None,
            &locks(&root),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("matches 2 live sessions"), "{error}");
        assert!(error.contains("ambiguous"), "{error}");
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn a_tagged_match_described_as_another_tool_is_never_reused() {
        let mut facts = FakeBackend::facts("7", "lane-1", "codex", "/work");
        facts.foreground_basenames = vec!["claude".to_owned()];
        let (terminals, backend) = harness(vec![vec![facts]]);
        let root = tempfile::TempDir::new().unwrap();
        let error = run_spawn(
            &terminals,
            "codex",
            "/work",
            Some("lane-1"),
            None,
            None,
            None,
            None,
            &locks(&root),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("described as `claude`; refusing to reuse"),
            "{error}"
        );
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn spawn_timeout_reports_actionable_context_without_a_token() {
        let (terminals, _) = harness(vec![vec![]]);
        let identity = identity("lane-1", "codex", SpawnSurface::Tab);
        let session_id = SessionId::new(BackendId::new("kitty").unwrap(), "spawn-lane-1").unwrap();
        let error = poll_ready(
            &terminals,
            &CliSessionInterpreter::system(),
            &session_id,
            &identity,
            Duration::from_millis(200),
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("last observed state: never appeared"),
            "{error}"
        );
        assert!(error.contains("rerun with the same key"), "{error}");
    }

    #[test]
    fn readiness_fails_closed_on_identity_mismatch_but_keeps_polling_transient_states() {
        let mut untagged = FakeBackend::facts("spawn-lane-1", "lane-1", "codex", "/work");
        untagged.spawn_identity = None;
        let (terminals, _) = harness(vec![vec![untagged]]);
        let identity = identity("lane-1", "codex", SpawnSurface::Tab);
        let session_id = SessionId::new(BackendId::new("kitty").unwrap(), "spawn-lane-1").unwrap();
        let error = poll_ready(
            &terminals,
            &CliSessionInterpreter::system(),
            &session_id,
            &identity,
            Duration::from_secs(1),
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("with a different spawn identity than requested"),
            "{error}"
        );

        let mut misdescribed = FakeBackend::spawned_facts(&SpawnedState {
            id: SessionId::new(BackendId::new("kitty").unwrap(), "spawn-lane-1").unwrap(),
            identity: identity.clone(),
            cwd: "/work".to_owned(),
        });
        misdescribed.foreground_basenames = vec!["claude".to_owned()];
        let (terminals, _) = harness(vec![vec![misdescribed]]);
        let error = poll_ready(
            &terminals,
            &CliSessionInterpreter::system(),
            &session_id,
            &identity,
            Duration::from_millis(200),
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("last observed state: appeared=true bound=true described=claude"),
            "{error}"
        );

        let mut unbound = FakeBackend::spawned_facts(&SpawnedState {
            id: SessionId::new(BackendId::new("kitty").unwrap(), "spawn-lane-1").unwrap(),
            identity: identity.clone(),
            cwd: "/work".to_owned(),
        });
        unbound.root_pid = 0;
        let (terminals, _) = harness(vec![vec![unbound]]);
        let error = poll_ready(
            &terminals,
            &CliSessionInterpreter::system(),
            &session_id,
            &identity,
            Duration::from_millis(200),
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("last observed state: appeared=true bound=false described=codex"),
            "{error}"
        );
    }

    #[test]
    fn readiness_waits_for_staged_binding_and_classification() {
        let session_id = SessionId::new(BackendId::new("kitty").unwrap(), "spawn-lane-1").unwrap();
        let identity = identity("lane-1", "codex", SpawnSurface::Tab);
        let ready = FakeBackend::spawned_facts(&SpawnedState {
            id: session_id.clone(),
            identity: identity.clone(),
            cwd: "/work".to_owned(),
        });
        let mut unbound = ready.clone();
        unbound.root_pid = 0;
        let mut generic = ready.clone();
        generic.foreground_basenames = Vec::new();
        let mut misclassified = ready.clone();
        misclassified.foreground_basenames = vec!["claude".to_owned()];
        let interpreter = CliSessionInterpreter::system();

        let (terminals, _) = harness(vec![vec![unbound.clone()], vec![ready.clone()]]);
        let facts = poll_ready(
            &terminals,
            &interpreter,
            &session_id,
            &identity,
            Duration::from_secs(2),
            None,
        )
        .unwrap();
        assert_eq!(facts.binding().unwrap().root_pid(), 10);

        let (terminals, _) = harness(vec![vec![generic.clone()], vec![ready.clone()]]);
        let facts = poll_ready(
            &terminals,
            &interpreter,
            &session_id,
            &identity,
            Duration::from_secs(2),
            None,
        )
        .unwrap();
        assert_eq!(interpreter.describe(&facts).tool.id.to_string(), "codex");

        let (terminals, _) = harness(vec![vec![misclassified.clone()], vec![ready.clone()]]);
        let facts = poll_ready(
            &terminals,
            &interpreter,
            &session_id,
            &identity,
            Duration::from_secs(2),
            None,
        )
        .unwrap();
        assert_eq!(facts.binding().unwrap().token(), "v1:kitty:spawn-lane-1:10");
    }

    #[test]
    fn identity_mismatch_fails_closed_even_when_later_stages_look_correct() {
        let session_id = SessionId::new(BackendId::new("kitty").unwrap(), "spawn-lane-1").unwrap();
        let identity = identity("lane-1", "codex", SpawnSurface::Tab);
        let ready = FakeBackend::spawned_facts(&SpawnedState {
            id: session_id.clone(),
            identity: identity.clone(),
            cwd: "/work".to_owned(),
        });
        let mut untagged = ready.clone();
        untagged.spawn_identity = None;
        let (terminals, _) = harness(vec![vec![untagged], vec![ready]]);
        let error = poll_ready(
            &terminals,
            &CliSessionInterpreter::system(),
            &session_id,
            &identity,
            Duration::from_secs(1),
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("with a different spawn identity than requested"),
            "{error}"
        );
    }

    #[test]
    fn cwd_is_canonicalized_and_validated_before_launch() {
        let root = tempfile::TempDir::new().unwrap();
        fs::create_dir(root.path().join("proj")).unwrap();
        fs::write(root.path().join("file"), b"x").unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let proj = canonical_root.join("proj");

        assert_eq!(canonicalize_cwd_at(&canonical_root, "proj").unwrap(), proj);
        assert_eq!(
            canonicalize_cwd_at(&canonical_root, proj.to_str().unwrap()).unwrap(),
            proj
        );
        let missing = canonicalize_cwd_at(&canonical_root, "nope")
            .unwrap_err()
            .to_string();
        assert!(missing.contains("does not exist"), "{missing}");
        let file = canonicalize_cwd_at(&canonical_root, "file")
            .unwrap_err()
            .to_string();
        assert!(file.contains("not a directory"), "{file}");
    }

    #[test]
    fn missing_and_non_directory_cwd_reject_launch_without_spawning() {
        let root = tempfile::TempDir::new().unwrap();
        fs::write(root.path().join("file"), b"x").unwrap();
        let cwd = fs::canonicalize(root.path()).unwrap();
        let (terminals, backend) = harness(vec![vec![]]);
        let missing = run_spawn(
            &terminals,
            "codex",
            cwd.join("nope").to_str().unwrap(),
            Some("lane-1"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks(&root),
        )
        .unwrap_err()
        .to_string();
        assert!(missing.contains("does not exist"), "{missing}");
        let file_error = run_spawn(
            &terminals,
            "codex",
            cwd.join("file").to_str().unwrap(),
            Some("lane-2"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks(&root),
        )
        .unwrap_err()
        .to_string();
        assert!(file_error.contains("not a directory"), "{file_error}");
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn unsupported_surfaces_fail_cleanly_before_readiness() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, backend) = harness(vec![vec![]]);
        let error = run_spawn(
            &terminals,
            "codex",
            &cwd,
            Some("lane-1"),
            Some("os-window"),
            Some("flash-x"),
            None,
            None,
            &locks(&root),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("refused the spawn request"), "{error}");
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn lock_race_prevents_a_double_launch() {
        let (terminals, backend) = harness(vec![vec![]]);
        let root = tempfile::TempDir::new().unwrap();
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let locks = locks(&root);
        let key = SpawnKey::new("lane-1").unwrap();
        let guard = locks.acquire(&key).unwrap();
        let error = run_spawn(
            &terminals,
            "codex",
            &cwd,
            Some("lane-1"),
            None,
            None,
            None,
            None,
            &locks,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("already handling spawn key `lane-1`"),
            "{error}"
        );
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 0);
        drop(guard);

        let outcome = run_spawn(
            &terminals,
            "codex",
            &cwd,
            Some("lane-1"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
        )
        .unwrap();
        assert!(!outcome.reused);
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 1);
    }

    #[test]
    fn the_spawn_lock_file_is_removed_after_spawn_or_reuse_returns() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, _) = harness(vec![vec![]]);
        let locks = locks(&root);
        let key = SpawnKey::new("lane-1").unwrap();
        let path = locks.lock_for(&key);

        let outcome = run_spawn(
            &terminals,
            "codex",
            &cwd,
            Some("lane-1"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
        )
        .unwrap();
        assert!(!outcome.reused);
        assert!(
            !path.exists(),
            "a completed spawn must leave no lock file behind"
        );

        let outcome = run_spawn(
            &terminals,
            "codex",
            &cwd,
            Some("lane-1"),
            None,
            None,
            None,
            None,
            &locks,
        )
        .unwrap();
        assert!(outcome.reused);
        assert!(!path.exists());
    }

    #[test]
    fn the_spawn_lock_file_is_removed_on_error_paths_too() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, _) = harness(vec![vec![]]);
        let locks = locks(&root);
        let key = SpawnKey::new("lane-1").unwrap();
        let path = locks.lock_for(&key);

        let error = run_spawn(
            &terminals,
            "codex",
            &cwd,
            Some("lane-1"),
            Some("os-window"),
            Some("flash-x"),
            None,
            None,
            &locks,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("refused the spawn request"), "{error}");
        assert!(
            !path.exists(),
            "a failed spawn must leave no lock file behind"
        );
    }

    #[test]
    fn a_leftover_lock_file_from_a_crashed_spawn_does_not_block_a_fresh_one() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (terminals, backend) = harness(vec![vec![]]);
        let locks = locks(&root);
        let key = SpawnKey::new("lane-1").unwrap();
        let path = locks.lock_for(&key);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "99999").unwrap();

        let outcome = run_spawn(
            &terminals,
            "codex",
            &cwd,
            Some("lane-1"),
            None,
            Some("flash-x"),
            None,
            None,
            &locks,
        )
        .unwrap();
        assert!(!outcome.reused);
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 1);
        assert!(!path.exists());
    }

    #[test]
    fn decide_classifies_matches_by_key_tool_and_description() {
        let interpreter = CliSessionInterpreter::system();
        let codex = identity("lane-1", "codex", SpawnSurface::Tab);

        assert_eq!(decide(&interpreter, &[], &codex), SpawnDecision::Launch);

        let reuse = FakeBackend::facts("7", "lane-1", "codex", "/work");
        assert_eq!(
            decide(&interpreter, std::slice::from_ref(&reuse), &codex),
            SpawnDecision::Reuse(Box::new(reuse))
        );

        let conflict = FakeBackend::facts("7", "lane-1", "claude", "/work");
        assert_eq!(
            decide(&interpreter, &[conflict], &codex),
            SpawnDecision::Conflict(CliToolId::new("claude").unwrap())
        );

        let mut misdescribed = FakeBackend::facts("7", "lane-1", "codex", "/work");
        misdescribed.foreground_basenames = vec!["claude".to_owned()];
        assert_eq!(
            decide(&interpreter, &[misdescribed], &codex),
            SpawnDecision::WrongHarness {
                described: CliToolId::new("claude").unwrap()
            }
        );

        let first = FakeBackend::facts("7", "lane-1", "codex", "/work");
        let second = FakeBackend::facts("8", "lane-1", "codex", "/work");
        assert_eq!(
            decide(&interpreter, &[first, second], &codex),
            SpawnDecision::Ambiguous(2)
        );
    }

    fn unresolved_facts(key: &str, cwd: &str) -> SessionFacts {
        SessionFacts {
            id: SessionId::new(BackendId::new("kitty").unwrap(), format!("spawn-{key}")).unwrap(),
            root_pid: 200,
            cwd: cwd.to_owned(),
            title: "Spawned".to_owned(),
            at_prompt: true,
            reported_cmd: None,
            foreground_basenames: vec!["zed".to_owned()],
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: Some(identity(key, "pi", SpawnSurface::Tab)),
        }
    }

    struct FixedExternalIdStrategy {
        external_id: Option<String>,
        authoritative: bool,
    }

    impl CliSessionStrategy for FixedExternalIdStrategy {
        fn tool(&self) -> &CliTool {
            static TOOL: std::sync::OnceLock<CliTool> = std::sync::OnceLock::new();
            TOOL.get_or_init(|| {
                CliTool::new(
                    CliToolId::new("pi").unwrap(),
                    "Pi",
                    CliToolColor::new(1, 2, 3),
                )
            })
        }

        fn matches(&self, _session: &SessionFacts) -> bool {
            true
        }

        fn describe(&self, _session: &SessionFacts) -> CliSessionDescriptor {
            CliSessionDescriptor {
                tool: self.tool().clone(),
                display_name: None,
                external_id: self.external_id.clone(),
                external_id_authoritative: self.authoritative,
                has_activity: None,
                evidence: CliSessionEvidence::default(),
            }
        }
    }

    #[test]
    fn a_described_id_that_conflicts_with_the_ledger_record_keeps_the_ledger_id() {
        let root = tempfile::TempDir::new().unwrap();
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let key = SpawnKey::new("lane-poisoned").unwrap();
        ledger
            .record(
                &key,
                &CliToolId::new("pi").unwrap(),
                SpawnSurface::Tab,
                "/work",
                None,
                Some("id-recorded-A"),
            )
            .unwrap();
        let facts = unresolved_facts("lane-poisoned", "/work");
        let binding = facts.binding().unwrap();
        let (terminals, _) = harness(vec![vec![facts]]);
        let interpreter =
            CliSessionInterpreter::from_strategies([Arc::new(FixedExternalIdStrategy {
                external_id: Some("id-resolved-B".to_owned()),
                authoritative: false,
            }) as Arc<dyn CliSessionStrategy>])
            .unwrap();

        let captured = capture_lane_external_id(
            &terminals,
            &interpreter,
            &ledger,
            &locks(&root),
            &binding,
            "",
            None,
        );
        assert!(
            matches!(captured, ExternalIdCapture::Heuristic),
            "a heuristic resolution is provisional, never final"
        );
        assert_eq!(
            ledger
                .load(&key, "/work")
                .unwrap()
                .unwrap()
                .external_id
                .as_deref(),
            Some("id-recorded-A"),
            "the ledger id must never be replaced by an unrelated resolution"
        );
    }

    #[test]
    fn an_authoritative_env_resolution_corrects_a_poisoned_ledger_record() {
        let root = tempfile::TempDir::new().unwrap();
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let key = SpawnKey::new("lane-poisoned").unwrap();
        ledger
            .record(
                &key,
                &CliToolId::new("pi").unwrap(),
                SpawnSurface::Tab,
                "/work",
                None,
                Some("id-recorded-A"),
            )
            .unwrap();
        let facts = unresolved_facts("lane-poisoned", "/work");
        let binding = facts.binding().unwrap();
        let (terminals, _) = harness(vec![vec![facts]]);
        let interpreter =
            CliSessionInterpreter::from_strategies([Arc::new(FixedExternalIdStrategy {
                external_id: Some("id-resolved-B".to_owned()),
                authoritative: true,
            }) as Arc<dyn CliSessionStrategy>])
            .unwrap();

        let captured = capture_lane_external_id(
            &terminals,
            &interpreter,
            &ledger,
            &locks(&root),
            &binding,
            "",
            None,
        );
        assert!(
            matches!(captured, ExternalIdCapture::Authoritative),
            "an authoritative resolution finalizes the capture"
        );
        assert_eq!(
            ledger
                .load(&key, "/work")
                .unwrap()
                .unwrap()
                .external_id
                .as_deref(),
            Some("id-resolved-B"),
            "the process env is the truth: it must repair a poisoned spawn-time guess"
        );
    }

    #[test]
    fn a_resumed_lane_captures_its_external_id_from_the_ledger_when_description_cannot_resolve() {
        let root = tempfile::TempDir::new().unwrap();
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let key = SpawnKey::new("lane-resumed").unwrap();
        ledger
            .record(
                &key,
                &CliToolId::new("pi").unwrap(),
                SpawnSurface::Tab,
                "/work",
                None,
                Some("019feb29-6312d38b"),
            )
            .unwrap();
        let facts = unresolved_facts("lane-resumed", "/work");
        let binding = facts.binding().unwrap();
        let (terminals, _) = harness(vec![vec![facts]]);

        let captured = capture_lane_external_id(
            &terminals,
            &CliSessionInterpreter::system(),
            &ledger,
            &locks(&root),
            &binding,
            "",
            None,
        );
        assert!(
            matches!(captured, ExternalIdCapture::Heuristic),
            "the ledger fallback is provisional until the env can confirm"
        );
        assert_eq!(
            ledger
                .load(&key, "/work")
                .unwrap()
                .unwrap()
                .external_id
                .as_deref(),
            Some("019feb29-6312d38b"),
            "the resumed id must stay recorded"
        );
    }

    #[test]
    fn the_round_marker_pins_the_lane_to_its_own_transcript() {
        let root = tempfile::TempDir::new().unwrap();
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let key = SpawnKey::new("lane-marked").unwrap();
        let facts = unresolved_facts("lane-marked", "/work");
        let binding = facts.binding().unwrap();
        let (terminals, _) = harness(vec![vec![facts]]);
        let session_dir = tempfile::TempDir::new().unwrap();
        let encoded_dir = session_dir.path().join("--work--");
        std::fs::create_dir_all(&encoded_dir).unwrap();
        let sibling =
            encoded_dir.join("2026-08-24T19-22-22-314Z_01a03539-4aea-7cfa-98f8-16378157ce45.jsonl");
        let own =
            encoded_dir.join("2026-08-24T19-22-27-530Z_01a03539-5f4a-70fb-a215-36e00f9f3323.jsonl");
        std::fs::write(&sibling, "no marker here").unwrap();
        std::fs::write(
            &own,
            r#"{"type":"message","timestamp":"2026-08-24T19:22:27.530Z","message":{"role":"user","content":[{"type":"text","text":"task QOL_BRIDGE_DONE_marker-abc123"}]}}"#,
        )
        .unwrap();
        let previous = std::env::var_os("PI_CODING_AGENT_SESSION_DIR");
        std::env::set_var("PI_CODING_AGENT_SESSION_DIR", session_dir.path());
        let captured = capture_lane_external_id(
            &terminals,
            &CliSessionInterpreter::system(),
            &ledger,
            &locks(&root),
            &binding,
            "QOL_BRIDGE_DONE_marker-abc123",
            Some(std::time::UNIX_EPOCH),
        );
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_SESSION_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_SESSION_DIR"),
        }
        assert!(
            matches!(captured, ExternalIdCapture::Authoritative),
            "the transcript containing the round marker is the authoritative identity"
        );
        assert_eq!(
            ledger
                .load(&key, "/work")
                .unwrap()
                .unwrap()
                .external_id
                .as_deref(),
            Some("01a03539-5f4a-70fb-a215-36e00f9f3323"),
            "the marker pins the lane to its own file, not a sibling's"
        );
    }

    #[test]
    fn a_fresh_lane_without_a_ledger_record_stays_unresolved() {
        let root = tempfile::TempDir::new().unwrap();
        let ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
        let facts = unresolved_facts("lane-fresh", "/work");
        let binding = facts.binding().unwrap();
        let (terminals, _) = harness(vec![vec![facts]]);

        let captured = capture_lane_external_id(
            &terminals,
            &CliSessionInterpreter::system(),
            &ledger,
            &locks(&root),
            &binding,
            "",
            None,
        );
        assert!(
            matches!(captured, ExternalIdCapture::Unresolved),
            "without a ledger record the unresolved description must fall through"
        );
        assert!(ledger
            .load(&SpawnKey::new("lane-fresh").unwrap(), "/work")
            .unwrap()
            .is_none());
    }
}
