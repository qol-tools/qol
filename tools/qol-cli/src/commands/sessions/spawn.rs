use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use qol_terminal_sessions::cli::{CliSessionInterpreter, CliToolId};
use qol_terminal_sessions::{
    SessionFacts, SessionId, SpawnIdentity, SpawnKey, SpawnRequest, SpawnSurface,
    TerminalSessionService,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub(super) const SURFACE_TAB: &str = "tab";
pub(super) const SURFACE_OS_WINDOW: &str = "os-window";
const READY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const READY_TIMEOUT_MS: u64 = 30_000;

static KEY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize)]
pub(super) struct SpawnOutcome {
    pub(super) session: String,
    pub(super) tool: String,
    pub(super) key: String,
    pub(super) reused: bool,
    pub(super) cwd: String,
    pub(super) surface: String,
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
}

pub(super) fn config_surface() -> Result<Option<SpawnSurface>> {
    let Some(config_dir) = qol_config::config_dir() else {
        return Ok(None);
    };
    config_surface_at(&config_dir.join("sessions.toml"))
}

pub(super) fn config_model() -> Result<Option<String>> {
    let Some(config_dir) = qol_config::config_dir() else {
        return Ok(None);
    };
    config_model_at(&config_dir.join("sessions.toml"))
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

fn config_model_at(path: &Path) -> Result<Option<String>> {
    let encoded = match fs::read_to_string(path) {
        Ok(encoded) => encoded,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to read spawn model config"),
    };
    let config: SpawnConfigFile =
        toml::from_str(&encoded).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(config.spawn_model.filter(|model| !model.trim().is_empty()))
}

pub(super) struct SpawnLocks {
    dir: PathBuf,
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
    let (tool, cwd, key, surface, model_flag) = parse_args(args)?;
    let configured = config_model()?;
    let model = model_flag.or(configured);
    let outcome = spawn_or_reuse(
        &TerminalSessionService::system(),
        &CliSessionInterpreter::system(),
        &tool,
        &cwd,
        key.as_deref(),
        surface.as_deref(),
        config_surface()?,
        model.as_deref(),
        &SpawnLocks::system()?,
    )?;
    println!(
        "{}",
        serde_json::to_string(&outcome).context("failed to serialize spawn outcome")?
    );
    Ok(())
}

fn parse_args(
    args: &[OsString],
) -> Result<(
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
)> {
    let usage =
        "qol sessions spawn --tool TOOL --cwd PATH [--key KEY] [--surface tab|os-window] [--model MODEL]";
    let mut tool = None;
    let mut cwd = None;
    let mut key = None;
    let mut surface = None;
    let mut model = None;
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
            other => bail!("unknown spawn flag `{other}`\nusage: {usage}"),
        }
    }
    let tool = tool.ok_or_else(|| anyhow!("usage: {usage}"))?;
    let cwd = cwd.ok_or_else(|| anyhow!("usage: {usage}"))?;
    Ok((tool, cwd, key, surface, model))
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

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_or_reuse(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    tool: &str,
    cwd: &str,
    key: Option<&str>,
    surface: Option<&str>,
    config: Option<SpawnSurface>,
    model: Option<&str>,
    locks: &SpawnLocks,
) -> Result<SpawnOutcome> {
    let tool_id = CliToolId::new(tool.to_owned())
        .map_err(|error| anyhow!("invalid tool `{tool}`: {error}"))?;
    let mut launch = interpreter.launch_for(&tool_id).ok_or_else(|| {
        anyhow!("no launch strategy for tool `{tool}`; only registered tools with a launch program can spawn")
    })?;
    if let Some(model) = model {
        launch.env.push(("PI_MODEL".to_owned(), model.to_owned()));
        if tool_id.as_str() == "pi" {
            launch.args.push("--model".to_owned());
            launch.args.push(model.to_owned());
        }
    }
    let key = match key {
        Some(key) => SpawnKey::new(key.to_owned())
            .map_err(|error| anyhow!("invalid spawn key `{key}`: {error}"))?,
        None => SpawnKey::new(generate_key())
            .map_err(|error| anyhow!("generated spawn key is invalid: {error}"))?,
    };
    let surface = resolve_surface(surface, config)?;
    let identity = SpawnIdentity {
        key: key.clone(),
        tool: tool_id,
        surface,
    };
    let _lock = locks.acquire(&key)?;
    let snapshot = terminals.snapshot().context("session discovery failed")?;
    match decide(interpreter, snapshot.sessions(), &identity) {
        SpawnDecision::Launch => {
            let request = SpawnRequest {
                identity: identity.clone(),
                launch,
                cwd: canonicalize_cwd(cwd)?,
                title: None,
            };
            launch_ready(terminals, interpreter, &identity, &request)
        }
        SpawnDecision::Reuse(facts) => {
            qol_runtime::probe!(
                "CLI_SESSION_SPAWN",
                "event=reuse key={} tool={}",
                identity.key,
                identity.tool
            );
            outcome_from_facts(&facts, interpreter, true)
        }
        SpawnDecision::Conflict(found) => {
            qol_runtime::probe!(
                "CLI_SESSION_SPAWN",
                "event=conflict key={} requested_tool={} found_tool={}",
                identity.key,
                identity.tool,
                found
            );
            bail!(
                "spawn key `{key}` is already held by tool `{found}`; a key cannot span tools - pick a distinct key"
            )
        }
        SpawnDecision::WrongHarness { described } => bail!(
            "spawn key `{key}` is tagged for `{}` but the live session is described as `{described}`; refusing to reuse it",
            identity.tool
        ),
        SpawnDecision::Ambiguous(count) => {
            qol_runtime::probe!(
                "CLI_SESSION_SPAWN",
                "event=ambiguous key={} matches={}",
                identity.key,
                count
            );
            bail!(
                "spawn key `{key}` matches {count} live sessions; the key is ambiguous - close the duplicates or pick a distinct key"
            )
        }
    }
}

fn launch_ready(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    identity: &SpawnIdentity,
    request: &SpawnRequest,
) -> Result<SpawnOutcome> {
    let started = Instant::now();
    let session_id = terminals
        .spawn_on(qol_terminal_sessions::kitty::backend_id(), request)
        .context("terminal backend refused the spawn request")?;
    qol_runtime::probe!(
        "CLI_SESSION_SPAWN",
        "event=launch key={} tool={} surface={}",
        identity.key,
        identity.tool,
        surface_token(identity.surface)
    );
    let facts = poll_ready(
        terminals,
        interpreter,
        &session_id,
        identity,
        Duration::from_millis(READY_TIMEOUT_MS),
    )?;
    let mut outcome = outcome_from_facts(&facts, interpreter, false)?;
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

fn poll_ready(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    session_id: &SessionId,
    identity: &SpawnIdentity,
    timeout: Duration,
) -> Result<SessionFacts> {
    let started = Instant::now();
    let mut last = ReadinessObservation {
        appeared: false,
        bound: false,
        described: None,
    };
    loop {
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
        std::thread::sleep(READY_POLL_INTERVAL);
    }
}

fn outcome_from_facts(
    facts: &SessionFacts,
    interpreter: &CliSessionInterpreter,
    reused: bool,
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
        elapsed_ms: 0,
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
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::{Arc, Mutex};

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
            }
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
                        facts.push(Self::spawned_facts(&state));
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
        let backend = Arc::new(FakeBackend::new(discoveries));
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

    fn run_spawn(
        terminals: &TerminalSessionService,
        tool: &str,
        cwd: &str,
        key: Option<&str>,
        surface: Option<&str>,
        config: Option<SpawnSurface>,
        locks: &SpawnLocks,
    ) -> Result<SpawnOutcome> {
        spawn_or_reuse(
            terminals,
            &CliSessionInterpreter::system(),
            tool,
            cwd,
            key,
            surface,
            config,
            None,
            locks,
        )
    }

    #[test]
    fn spawn_args_parse_required_and_optional_flags() {
        let (tool, cwd, key, surface, model) = parse_args(&[
            "--tool".into(),
            "codex".into(),
            "--cwd".into(),
            "/work/project".into(),
            "--key".into(),
            "lane-1".into(),
            "--surface".into(),
            "os-window".into(),
            "--model".into(),
            "deepseek-v4-pro".into(),
        ])
        .unwrap();
        assert_eq!(tool, "codex");
        assert_eq!(cwd, "/work/project");
        assert_eq!(key.as_deref(), Some("lane-1"));
        assert_eq!(surface.as_deref(), Some("os-window"));
        assert_eq!(model.as_deref(), Some("deepseek-v4-pro"));

        let (tool, cwd, key, surface, model) =
            parse_args(&["--tool".into(), "pi".into(), "--cwd".into(), "/tmp".into()]).unwrap();
        assert_eq!(tool, "pi");
        assert_eq!(cwd, "/tmp");
        assert_eq!(key, None);
        assert_eq!(surface, None);
        assert_eq!(model, None);
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
                "--title".into(),
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
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 1);

        let expected = identity("lane-1", "codex", SpawnSurface::Tab);
        let request = backend.last_request.lock().unwrap().clone().unwrap();
        assert_eq!(request.identity, expected);
        assert_eq!(request.launch.program, "codex");
        assert!(request.launch.args.is_empty());
        assert_eq!(request.cwd, std::path::PathBuf::from(&cwd));
        assert_eq!(request.title, None);
    }

    #[test]
    fn configured_spawn_model_injects_pi_model_into_the_launch_env() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("sessions.toml");
        fs::write(&path, "spawn_model = \"deepseek-v4-flash\"\n").unwrap();
        assert_eq!(
            config_model_at(&path).unwrap().as_deref(),
            Some("deepseek-v4-flash")
        );

        let (terminals, backend) = harness(vec![vec![]]);
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let outcome = spawn_or_reuse(
            &terminals,
            &CliSessionInterpreter::system(),
            "codex",
            &cwd,
            Some("lane-model"),
            None,
            None,
            Some("deepseek-v4-flash"),
            &locks(&root),
        )
        .unwrap();
        assert!(!outcome.reused);
        let request = backend.last_request.lock().unwrap().clone().unwrap();
        assert_eq!(
            request.launch.env,
            vec![("PI_MODEL".to_owned(), "deepseek-v4-flash".to_owned())]
        );
        assert!(
            request.launch.args.is_empty(),
            "codex keeps its own model flags"
        );
    }

    #[test]
    fn configured_spawn_model_passes_the_pi_model_flag() {
        let root = tempfile::TempDir::new().unwrap();
        let (terminals, backend) = harness(vec![vec![]]);
        let cwd = fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let outcome = spawn_or_reuse(
            &terminals,
            &CliSessionInterpreter::system(),
            "pi",
            &cwd,
            Some("lane-pi"),
            None,
            None,
            Some("deepseek-v4-flash"),
            &locks(&root),
        )
        .unwrap();
        assert!(!outcome.reused);
        let request = backend.last_request.lock().unwrap().clone().unwrap();
        assert_eq!(
            request.launch.args,
            vec!["--model".to_owned(), "deepseek-v4-flash".to_owned()]
        );
    }

    #[test]
    fn missing_config_file_yields_no_spawn_model() {
        let root = tempfile::TempDir::new().unwrap();
        assert_eq!(
            config_model_at(&root.path().join("sessions.toml")).unwrap(),
            None
        );
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
            None,
            &locks(&root),
        )
        .unwrap();
        assert_eq!(outcome.key, "mcp-key");

        let (terminals, _) = harness(vec![vec![]]);
        let outcome = run_spawn(&terminals, "pi", &cwd, None, None, None, &locks(&root)).unwrap();
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
            None,
            &locks,
        )
        .unwrap();
        assert!(!outcome.reused);
        assert_eq!(backend.spawn_count.load(AtomicOrdering::Relaxed), 1);
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
}
