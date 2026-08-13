use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use qol_process::{run_owned_with_output_timeout, BoundedCommandOutput};
use qol_terminal_sessions::bridge::BridgeCheckpoint;
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::{
    DeliveryMode, ScreenReader, SessionBinding, SessionFacts, SessionInventory,
    TerminalSessionService, TextInput,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

const DELIVERY_VERIFY_WINDOW: Duration = Duration::from_secs(15);
const DELIVERY_VERIFY_INTERVAL: Duration = Duration::from_secs(1);
const STALL_PROBE_AFTER: Duration = Duration::from_secs(30);
const TASK_MAX_BYTES: usize = 64 * 1024;
const STALE_TMP_AFTER: Duration = Duration::from_secs(3600);
const GATE_LOCK_MESSAGE: &str = "Blocking waiting for file lock";
const GATE_STEP_TIMEOUT: Duration = Duration::from_secs(600);
const GATE_LOCK_RETRY_BUDGET: Duration = Duration::from_secs(1800);
const GATE_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const GATE_COMMANDS: [&str; 4] = [
    "cargo fmt --check -p qol",
    "cargo clippy -p qol --all-targets -- -D warnings",
    "cargo test -p qol --bin qol",
    "cargo test -p qol-terminal-sessions",
];

pub(super) const TIMEOUT_MIN_MS: u64 = 1_000;
pub(super) const TIMEOUT_DEFAULT_MS: u64 = TIMEOUT_MAX_MS;
pub(super) const TIMEOUT_MAX_MS: u64 = 86_400_000;

static MARKER_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize)]
pub(super) struct BridgeOutcome {
    pub(super) completed: bool,
    pub(super) submitted: bool,
    pub(super) stalled: bool,
    pub(super) session: String,
    pub(super) completion_marker: String,
    pub(super) screen: String,
    pub(super) reads: u64,
    pub(super) elapsed_ms: u128,
    pub(super) next_command: String,
}

#[derive(Debug)]
pub(super) struct PendingRound {
    pub(super) session: String,
    pub(super) completion_marker: String,
    pub(super) completed: bool,
}

struct PendingBridgeLock {
    file: File,
}

impl Drop for PendingBridgeLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Debug)]
pub(super) struct BridgeOwner {
    file: File,
}

impl Drop for BridgeOwner {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(super) struct PendingBridgeStore {
    dir: PathBuf,
}

impl PendingBridgeStore {
    pub(super) fn system() -> Result<Self> {
        let dir = qol_terminal_sessions::bridge::checkpoint_dir()
            .ok_or_else(|| anyhow!("sessions data directory is unavailable"))?;
        Ok(Self { dir })
    }

    #[cfg(test)]
    pub(super) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn load(&self, binding: &SessionBinding) -> Result<Option<BridgeCheckpoint>> {
        let _lock = self.lock(binding)?;
        self.load_unlocked(binding)
    }

    fn load_unlocked(&self, binding: &SessionBinding) -> Result<Option<BridgeCheckpoint>> {
        let file = self.file_for(binding);
        let encoded = match fs::read_to_string(&file) {
            Ok(encoded) => encoded,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("failed to read pending bridge checkpoint"),
        };
        serde_json::from_str(&encoded)
            .map(Some)
            .context("pending bridge checkpoint is invalid")
    }

    pub(super) fn start(&self, binding: &SessionBinding, marker: &str, driver: &str) -> Result<()> {
        let _lock = self.lock(binding)?;
        if self
            .load_unlocked(binding)?
            .is_some_and(|checkpoint| !checkpoint.closed)
        {
            bail!("a bridge is already pending for `{binding}`");
        }
        self.write_unlocked(binding, marker, driver, false)
    }

    pub(super) fn observe(
        &self,
        binding: &SessionBinding,
        marker: &str,
        completed: bool,
    ) -> Result<()> {
        let _lock = self.lock(binding)?;
        let Some(checkpoint) = self.load_unlocked(binding)? else {
            return Ok(());
        };
        if checkpoint.closed || checkpoint.completion_marker != marker {
            return Ok(());
        }
        self.write_unlocked(binding, marker, &checkpoint.driver, completed)
    }

    fn write_unlocked(
        &self,
        binding: &SessionBinding,
        marker: &str,
        driver: &str,
        completed: bool,
    ) -> Result<()> {
        fs::create_dir_all(&self.dir).context("failed to create pending bridge directory")?;
        let file = self.file_for(binding);
        let temporary = file.with_extension("tmp");
        let encoded = serde_json::to_string(&BridgeCheckpoint {
            session: binding.token(),
            driver: driver.to_owned(),
            completion_marker: marker.to_owned(),
            completed,
            closed: false,
        })?;
        fs::write(&temporary, encoded).context("failed to write pending bridge checkpoint")?;
        fs::rename(&temporary, &file).context("failed to publish pending bridge checkpoint")
    }

    pub(super) fn acknowledge(
        &self,
        binding: &SessionBinding,
        marker: &str,
        require_completed: bool,
    ) -> Result<()> {
        let _lock = self.lock(binding)?;
        let checkpoint = self
            .load_unlocked(binding)?
            .ok_or_else(|| anyhow!("no pending bridge exists for `{binding}`"))?;
        if checkpoint.closed {
            bail!("the pending bridge is already closed");
        }
        if checkpoint.completion_marker != marker {
            bail!("bridge acknowledgement does not match the pending round");
        }
        if require_completed && !checkpoint.completed {
            bail!("the pending bridge has not completed");
        }
        fs::remove_file(self.file_for(binding))
            .context("failed to remove pending bridge checkpoint")
    }

    pub(super) fn discard(&self, binding: &SessionBinding) -> Result<BridgeCheckpoint> {
        let _lock = self.lock(binding)?;
        let checkpoint = self
            .load_unlocked(binding)?
            .ok_or_else(|| anyhow!("no pending bridge checkpoint exists for `{binding}`"))?;
        fs::remove_file(self.file_for(binding))
            .context("failed to remove pending bridge checkpoint")?;
        Ok(checkpoint)
    }

    pub(super) fn pending_round(&self, binding: &SessionBinding) -> Result<Option<PendingRound>> {
        Ok(self
            .load(binding)?
            .filter(|checkpoint| !checkpoint.closed)
            .map(|checkpoint| PendingRound {
                session: binding.token(),
                completion_marker: checkpoint.completion_marker,
                completed: checkpoint.completed,
            }))
    }

    pub(super) fn pending_rounds(&self) -> Result<Vec<PendingRound>> {
        let mut rounds = self
            .open_checkpoints()?
            .into_iter()
            .filter(|checkpoint| !checkpoint.session.is_empty())
            .map(|checkpoint| PendingRound {
                session: checkpoint.session,
                completion_marker: checkpoint.completion_marker,
                completed: checkpoint.completed,
            })
            .collect::<Vec<_>>();
        rounds.sort_by(|left, right| left.session.cmp(&right.session));
        Ok(rounds)
    }

    fn open_checkpoints(&self) -> Result<Vec<BridgeCheckpoint>> {
        self.sweep()?;
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("failed to read pending bridge directory"),
        };
        let mut checkpoints = Vec::new();
        for entry in entries {
            let path = entry
                .context("failed to read pending bridge directory")?
                .path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Ok(encoded) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(checkpoint) = serde_json::from_str::<BridgeCheckpoint>(&encoded) else {
                continue;
            };
            if checkpoint.closed {
                continue;
            }
            checkpoints.push(checkpoint);
        }
        Ok(checkpoints)
    }

    fn sweep(&self) -> Result<()> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).context("failed to read pending bridge directory"),
        };
        for entry in entries {
            let path = entry
                .context("failed to read pending bridge directory")?
                .path();
            match path.extension().and_then(|extension| extension.to_str()) {
                Some("tmp") if older_than(&path, STALE_TMP_AFTER) => {
                    let _ = fs::remove_file(&path);
                }
                Some("json") => {
                    let Ok(encoded) = fs::read_to_string(&path) else {
                        continue;
                    };
                    let Ok(checkpoint) = serde_json::from_str::<BridgeCheckpoint>(&encoded) else {
                        continue;
                    };
                    if checkpoint.closed || checkpoint.session.is_empty() {
                        let _ = fs::remove_file(&path);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn acquire_owner(&self, binding: &SessionBinding) -> Result<BridgeOwner> {
        fs::create_dir_all(&self.dir).context("failed to create pending bridge directory")?;
        let path = self.owner_for(binding);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .context("failed to open bridge owner lock")?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let owner = fs::read_to_string(&path).unwrap_or_default();
                let owner = owner.trim();
                let owner = if owner.is_empty() { "unknown" } else { owner };
                qol_runtime::probe!(
                    "CLI_SESSION_BRIDGE",
                    "event=owner_conflict target_backend={}",
                    binding.session_id().backend()
                );
                bail!(
                    "another bridge process (pid {owner}) is already attached to `{binding}`; never start a second one - run `qol sessions next {}` and follow the command it prints",
                    binding.token()
                );
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).context("failed to lock the bridge owner file");
            }
        }
        fs::write(&path, process::id().to_string()).context("failed to record the bridge owner")?;
        Ok(BridgeOwner { file })
    }

    pub(super) fn owner_pid(&self, binding: &SessionBinding) -> Option<String> {
        let path = self.owner_for(binding);
        let file = OpenOptions::new().read(true).write(true).open(&path).ok()?;
        match file.try_lock() {
            Ok(()) => {
                let _ = file.unlock();
                None
            }
            Err(TryLockError::WouldBlock) => {
                Some(fs::read_to_string(&path).ok()?.trim().to_owned())
            }
            Err(TryLockError::Error(_)) => None,
        }
    }

    fn lock(&self, binding: &SessionBinding) -> Result<PendingBridgeLock> {
        fs::create_dir_all(&self.dir).context("failed to create pending bridge directory")?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.lock_for(binding))
            .context("failed to open pending bridge lock")?;
        file.lock()
            .context("failed to lock pending bridge checkpoint")?;
        Ok(PendingBridgeLock { file })
    }

    fn file_for(&self, binding: &SessionBinding) -> PathBuf {
        let digest = Sha256::digest(binding.token().as_bytes());
        self.dir.join(format!("{digest:x}.json"))
    }

    fn lock_for(&self, binding: &SessionBinding) -> PathBuf {
        self.file_for(binding).with_extension("lock")
    }

    fn owner_for(&self, binding: &SessionBinding) -> PathBuf {
        self.file_for(binding).with_extension("owner")
    }
}

fn older_than(path: &Path, age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified.elapsed().is_ok_and(|elapsed| elapsed >= age))
}

struct CompletionMarker {
    token: String,
    left: String,
    right: String,
}

impl CompletionMarker {
    fn generate() -> Self {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = MARKER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut digest = Sha256::new();
        digest.update(process::id().to_le_bytes());
        digest.update(elapsed.to_le_bytes());
        digest.update(sequence.to_le_bytes());
        let digest = digest.finalize();
        let nonce = format!("{digest:x}");
        Self::from_nonce(&nonce[..20])
    }

    fn from_nonce(nonce: &str) -> Self {
        Self {
            token: format!("QOL_BRIDGE_DONE_{nonce}"),
            left: "QOL_BRIDGE_DONE_".to_owned(),
            right: nonce.to_owned(),
        }
    }

    fn from_token(token: &str) -> Result<Self> {
        let nonce = token
            .strip_prefix("QOL_BRIDGE_DONE_")
            .filter(|nonce| !nonce.is_empty())
            .ok_or_else(|| anyhow!("pending bridge checkpoint has an invalid completion marker"))?;
        Ok(Self::from_nonce(nonce))
    }
}

pub(super) fn execute(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
    task: &str,
    timeout: Duration,
    pending: &PendingBridgeStore,
    acknowledge_marker: Option<&str>,
) -> Result<BridgeOutcome> {
    validate_task(task)?;
    let _owner = pending.acquire_owner(binding)?;
    if terminals
        .is_current(binding)
        .context("failed to identify the current terminal session")?
    {
        bail!("cannot bridge to the calling terminal; choose an independent session");
    }
    let target = resolve_target(terminals, binding)?;
    if let Some(marker) = acknowledge_marker {
        pending.acknowledge(binding, marker, true)?;
    } else if let Some(round) = pending.pending_round(binding)? {
        let liveness = session_liveness(terminals, interpreter, binding);
        let pre_screen = terminals
            .read_screen(binding)
            .context("bridge screen read failed")?;
        let alive = round.completed
            || delivery_observed(
                terminals,
                binding,
                "QOL_BRIDGE_DONE_",
                &pre_screen,
                &liveness,
                DELIVERY_VERIFY_WINDOW,
            )?;
        if alive {
            return resume_owned(terminals, interpreter, binding, timeout, pending, false);
        }
        pending.acknowledge(binding, &round.completion_marker, false)?;
        qol_runtime::probe!(
            "CLI_SESSION_BRIDGE",
            "event=superseded_undelivered target_backend={}",
            binding.session_id().backend()
        );
    }

    let (changed_tx, changed_rx) = mpsc::sync_channel(1);
    let subscription = interpreter
        .subscribe(
            &target,
            Arc::new(move || {
                let _ = changed_tx.try_send(());
            }),
        )
        .context("failed to subscribe to implementation-session changes")?;
    let subscribed = subscription.is_some();

    let marker = CompletionMarker::generate();
    let prompt = bridge_prompt(task, &marker);
    pending.start(binding, &marker.token, &driver_token(terminals))?;

    let liveness = session_liveness(terminals, interpreter, binding);
    let pre_screen = terminals
        .read_screen(binding)
        .context("bridge screen read failed")?;
    if let Err(error) = terminals.send_text(binding, &prompt, DeliveryMode::Submit) {
        pending.acknowledge(binding, &marker.token, false)?;
        qol_runtime::probe!(
            "CLI_SESSION_BRIDGE",
            "event=delivery_failed target_backend={}",
            binding.session_id().backend()
        );
        return Err(error).context("bridge task delivery failed");
    }
    if !delivery_observed(
        terminals,
        binding,
        &marker.left,
        &pre_screen,
        &liveness,
        DELIVERY_VERIFY_WINDOW,
    )? {
        qol_runtime::probe!(
            "CLI_SESSION_BRIDGE",
            "event=delivery_retry target_backend={}",
            binding.session_id().backend()
        );
        terminals
            .send_text(binding, &prompt, DeliveryMode::Submit)
            .context("bridge task redelivery failed")?;
        if !delivery_observed(
            terminals,
            binding,
            &marker.left,
            &pre_screen,
            &liveness,
            DELIVERY_VERIFY_WINDOW,
        )? {
            pending.acknowledge(binding, &marker.token, false)?;
            qol_runtime::probe!(
                "CLI_SESSION_BRIDGE",
                "event=delivery_unobserved target_backend={}",
                binding.session_id().backend()
            );
            bail!(
                "the target never showed the submitted task; no round is pending - fix the target session, then resubmit via `qol sessions bridge`"
            );
        }
    }
    qol_runtime::probe!(
        "CLI_SESSION_BRIDGE",
        "event=submitted target_backend={} subscription={}",
        binding.session_id().backend(),
        if subscribed { "active" } else { "fallback" }
    );

    let outcome = wait_for_completion(
        terminals,
        binding,
        &marker.token,
        timeout,
        changed_rx,
        subscribed,
        true,
        &session_liveness(terminals, interpreter, binding),
        STALL_PROBE_AFTER,
    )?;
    pending.observe(binding, &marker.token, outcome.completed)?;
    qol_runtime::probe!(
        "CLI_SESSION_BRIDGE",
        "event={} target_backend={} subscription={} elapsed_ms={} reads={}",
        if outcome.completed {
            "completed"
        } else {
            "timeout"
        },
        binding.session_id().backend(),
        if subscribed { "active" } else { "fallback" },
        outcome.elapsed_ms,
        outcome.reads
    );
    drop(subscription);
    Ok(outcome)
}

pub(super) fn submit(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
    task: &str,
    pending: &PendingBridgeStore,
    acknowledge_marker: Option<&str>,
) -> Result<BridgeOutcome> {
    validate_task(task)?;
    let _owner = pending.acquire_owner(binding)?;
    if terminals
        .is_current(binding)
        .context("failed to identify the current terminal session")?
    {
        bail!("cannot submit to the calling terminal; choose an independent session");
    }
    let _target = resolve_target(terminals, binding)?;
    if let Some(marker) = acknowledge_marker {
        pending.acknowledge(binding, marker, true)?;
    } else if pending.pending_round(binding)?.is_some() {
        bail!(
            "a round is already pending for `{binding}`; wait for it with `qol sessions bridge` (no task) before submitting another"
        );
    }
    let marker = CompletionMarker::generate();
    let prompt = bridge_prompt(task, &marker);
    pending.start(binding, &marker.token, &driver_token(terminals))?;
    let liveness = session_liveness(terminals, interpreter, binding);
    let pre_screen = terminals
        .read_screen(binding)
        .context("submit screen read failed")?;
    if let Err(error) = terminals.send_text(binding, &prompt, DeliveryMode::Submit) {
        pending.acknowledge(binding, &marker.token, false)?;
        qol_runtime::probe!(
            "CLI_SESSION_BRIDGE",
            "event=submit_delivery_failed target_backend={}",
            binding.session_id().backend()
        );
        return Err(error).context("submit task delivery failed");
    }
    if !delivery_observed(
        terminals,
        binding,
        &marker.left,
        &pre_screen,
        &liveness,
        DELIVERY_VERIFY_WINDOW,
    )? {
        qol_runtime::probe!(
            "CLI_SESSION_BRIDGE",
            "event=submit_delivery_retry target_backend={}",
            binding.session_id().backend()
        );
        terminals
            .send_text(binding, &prompt, DeliveryMode::Submit)
            .context("submit task redelivery failed")?;
        if !delivery_observed(
            terminals,
            binding,
            &marker.left,
            &pre_screen,
            &liveness,
            DELIVERY_VERIFY_WINDOW,
        )? {
            pending.acknowledge(binding, &marker.token, false)?;
            qol_runtime::probe!(
                "CLI_SESSION_BRIDGE",
                "event=submit_delivery_unobserved target_backend={}",
                binding.session_id().backend()
            );
            bail!(
                "the target never showed the submitted task; no round is pending - fix the target session, then submit again"
            );
        }
    }
    qol_runtime::probe!(
        "CLI_SESSION_BRIDGE",
        "event=submitted_async target_backend={}",
        binding.session_id().backend()
    );
    let screen = terminals
        .read_screen(binding)
        .context("submit screen read failed")?;
    Ok(outcome(
        false,
        true,
        false,
        binding,
        &marker.token,
        screen,
        1,
        Instant::now(),
    ))
}

pub(super) fn resume(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
    timeout: Duration,
    pending: &PendingBridgeStore,
    kickstart: bool,
) -> Result<BridgeOutcome> {
    let _owner = pending.acquire_owner(binding)?;
    resume_owned(terminals, interpreter, binding, timeout, pending, kickstart)
}

fn resume_owned(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
    timeout: Duration,
    pending: &PendingBridgeStore,
    kickstart: bool,
) -> Result<BridgeOutcome> {
    let round = pending.pending_round(binding)?.ok_or_else(|| {
        anyhow!("no pending bridge exists for `{binding}`; run `qol sessions next`")
    })?;
    if round.completed {
        let started = Instant::now();
        let screen = terminals
            .read_screen(binding)
            .context("bridge screen read failed")?;
        return Ok(outcome(
            true,
            false,
            false,
            binding,
            &round.completion_marker,
            screen,
            1,
            started,
        ));
    }
    let target = resolve_target(terminals, binding).map_err(|error| {
        let gone = terminals
            .discover()
            .map(|facts| {
                !facts
                    .iter()
                    .any(|session| session.id == *binding.session_id())
            })
            .unwrap_or(false);
        if gone {
            anyhow!(
                "{error}; the terminal is gone - recover the orphaned round with `qol sessions discard {}`",
                binding.token()
            )
        } else {
            error
        }
    })?;
    let (changed_tx, changed_rx) = mpsc::sync_channel(1);
    let subscription = interpreter
        .subscribe(
            &target,
            Arc::new(move || {
                let _ = changed_tx.try_send(());
            }),
        )
        .context("failed to subscribe to implementation-session changes")?;
    let subscribed = subscription.is_some();
    if kickstart {
        let marker = CompletionMarker::from_token(&round.completion_marker)?;
        terminals
            .send_text(binding, &kickstart_prompt(&marker), DeliveryMode::Submit)
            .context("bridge kickstart delivery failed")?;
        qol_runtime::probe!(
            "CLI_SESSION_BRIDGE",
            "event=kickstarted target_backend={}",
            binding.session_id().backend()
        );
    } else {
        let liveness = session_liveness(terminals, interpreter, binding);
        let pre_screen = terminals
            .read_screen(binding)
            .context("bridge screen read failed")?;
        if !delivery_observed(
            terminals,
            binding,
            "QOL_BRIDGE_DONE_",
            &pre_screen,
            &liveness,
            DELIVERY_VERIFY_WINDOW,
        )? {
            pending.acknowledge(binding, &round.completion_marker, false)?;
            qol_runtime::probe!(
                "CLI_SESSION_BRIDGE",
                "event=resume_closed_unobserved target_backend={}",
                binding.session_id().backend()
            );
            bail!(
                "the pending round shows no trace on the target and is now closed; resubmit the task via `qol sessions bridge`"
            );
        }
    }
    let outcome = wait_for_completion(
        terminals,
        binding,
        &round.completion_marker,
        timeout,
        changed_rx,
        subscribed,
        false,
        &session_liveness(terminals, interpreter, binding),
        STALL_PROBE_AFTER,
    )?;
    if outcome.completed {
        pending.observe(binding, &round.completion_marker, true)?;
    }
    drop(subscription);
    Ok(outcome)
}

pub(super) fn delivery_observed(
    terminals: &TerminalSessionService,
    binding: &SessionBinding,
    fragment: &str,
    pre_screen: &str,
    liveness: &dyn Fn() -> Option<bool>,
    window: Duration,
) -> Result<bool> {
    let deadline = Instant::now() + window;
    loop {
        let screen = terminals
            .read_screen_relaxed(binding)
            .context("bridge screen read failed")?;
        if screen.contains(fragment) || screen != pre_screen || liveness() == Some(true) {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(DELIVERY_VERIFY_INTERVAL);
    }
}

pub(super) fn session_liveness<'a>(
    terminals: &'a TerminalSessionService,
    interpreter: &'a CliSessionInterpreter,
    binding: &'a SessionBinding,
) -> impl Fn() -> Option<bool> + 'a {
    move || {
        let facts = terminals.discover().ok()?;
        let session = facts
            .into_iter()
            .find(|session| session.id == *binding.session_id())?;
        interpreter.describe(&session).has_activity
    }
}

fn driver_token(terminals: &TerminalSessionService) -> String {
    let Ok(sessions) = terminals.discover() else {
        return String::new();
    };
    sessions
        .into_iter()
        .filter_map(|session| session.binding().ok())
        .find(|binding| terminals.is_current(binding).unwrap_or(false))
        .map(|binding| binding.token())
        .unwrap_or_default()
}

fn resolve_target(
    terminals: &TerminalSessionService,
    binding: &SessionBinding,
) -> Result<SessionFacts> {
    let sessions = terminals.discover().context("session discovery failed")?;
    let target = sessions
        .into_iter()
        .find(|session| session.id == *binding.session_id())
        .ok_or_else(|| anyhow!("bridge target `{binding}` is no longer present"))?;
    let current = target
        .binding()
        .context("bridge target has an invalid live identity")?;
    if current != *binding {
        bail!("bridge target changed identity; re-run `qol sessions list`");
    }
    Ok(target)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn wait_for_completion(
    terminals: &TerminalSessionService,
    binding: &SessionBinding,
    marker: &str,
    timeout: Duration,
    changed: mpsc::Receiver<()>,
    subscribed: bool,
    submitted: bool,
    liveness: &dyn Fn() -> Option<bool>,
    stall_after: Duration,
) -> Result<BridgeOutcome> {
    let outcome = terminals
        .wait_for_completion(
            binding,
            marker,
            timeout,
            changed,
            subscribed,
            submitted,
            liveness,
            stall_after,
        )
        .context("bridge screen read failed")?;
    Ok(BridgeOutcome {
        completed: outcome.completed,
        submitted: outcome.submitted,
        stalled: outcome.stalled,
        session: binding.token(),
        completion_marker: marker.to_owned(),
        screen: outcome.screen,
        reads: outcome.reads,
        elapsed_ms: outcome.elapsed.as_millis(),
        next_command: format!("qol sessions next {}", binding.token()),
    })
}

#[allow(clippy::too_many_arguments)]
fn outcome(
    completed: bool,
    submitted: bool,
    stalled: bool,
    binding: &SessionBinding,
    marker: &str,
    screen: String,
    reads: u64,
    started: Instant,
) -> BridgeOutcome {
    BridgeOutcome {
        completed,
        submitted,
        stalled,
        session: binding.token(),
        completion_marker: marker.to_owned(),
        screen,
        reads,
        elapsed_ms: started.elapsed().as_millis(),
        next_command: format!("qol sessions next {}", binding.token()),
    }
}

fn kickstart_prompt(marker: &CompletionMarker) -> String {
    format!(
        "[qol session bridge]\nThe bounded task previously submitted to this session is still open and its completion signal was never emitted; the session may have been interrupted. If the task is already complete, reply now ending with the completion fragments joined with no spaces or punctuation. Otherwise continue the task to completion and end your final response with them.\n\nCompletion fragments: `{}` and `{}`.",
        marker.left, marker.right
    )
}

fn bridge_prompt(task: &str, marker: &CompletionMarker) -> String {
    format!(
        "[qol session bridge]\nAct as the implementation agent for the bounded task below. Work directly on that task and do not delegate it. When the task is genuinely complete, end your final response with the completion fragments joined with no spaces or punctuation.\n\nTask:\n{task}\n\nCompletion fragments: `{}` and `{}`.",
        marker.left, marker.right
    )
}

fn validate_task(task: &str) -> Result<()> {
    if task.trim().is_empty() {
        bail!("bridge task must not be empty");
    }
    if task.len() > TASK_MAX_BYTES {
        bail!("bridge task exceeds {TASK_MAX_BYTES} bytes; use a file handoff");
    }
    if task
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        bail!("bridge task contains unsupported control characters");
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct GateStepResult {
    pub(super) command: String,
    pub(super) passed: bool,
    pub(super) elapsed: Duration,
    pub(super) reason: Option<String>,
}

#[derive(Debug)]
pub(super) struct GateSummary {
    pub(super) steps: Vec<GateStepResult>,
    pub(super) total: Duration,
    pub(super) skipped_reason: Option<String>,
}

pub(super) fn run_quality_gate(screen: &str, cwd: &Path) -> String {
    if !cwd.join("Cargo.toml").is_file() {
        return append_gate_section(
            screen,
            &GateSummary {
                steps: Vec::new(),
                total: Duration::ZERO,
                skipped_reason: Some(format!(
                    "no Cargo.toml in {}; the gate is skipped",
                    cwd.display()
                )),
            },
        );
    }
    qol_runtime::probe!("CLI_SESSION_BRIDGE", "event=gate_started");
    let started = Instant::now();
    let mut steps = Vec::with_capacity(GATE_COMMANDS.len());
    for command in GATE_COMMANDS {
        steps.push(run_gate_step(command, cwd));
    }
    let summary = GateSummary {
        steps,
        total: started.elapsed(),
        skipped_reason: None,
    };
    let verdict = if summary.steps.iter().all(|step| step.passed) {
        "GREEN"
    } else {
        "RED"
    };
    qol_runtime::probe!(
        "CLI_SESSION_BRIDGE",
        "event=gate_finished verdict={} total_ms={}",
        verdict,
        summary.total.as_millis()
    );
    append_gate_section(screen, &summary)
}

fn append_gate_section(screen: &str, summary: &GateSummary) -> String {
    format!("{screen}\n\n{}", format_gate_summary(summary))
}

pub(super) fn format_gate_summary(summary: &GateSummary) -> String {
    if let Some(reason) = &summary.skipped_reason {
        return format!("--- GATE ---\nskipped: {reason}");
    }
    let mut lines = vec!["--- GATE ---".to_owned()];
    let count = summary.steps.len();
    for (index, step) in summary.steps.iter().enumerate() {
        let status = if step.passed { "PASS" } else { "FAIL" };
        let mut line = format!(
            "[{}/{}] {status} {} ({:.1}s)",
            index + 1,
            count,
            step.command,
            step.elapsed.as_secs_f64()
        );
        if let Some(reason) = &step.reason {
            line.push_str(&format!(" - {reason}"));
        }
        lines.push(line);
    }
    lines.push(format!("total: {:.1}s", summary.total.as_secs_f64()));
    lines.push(format!(
        "verdict: {}",
        if summary.steps.iter().all(|step| step.passed) {
            "GREEN"
        } else {
            "RED"
        }
    ));
    lines.join("\n")
}

fn run_gate_step(command: &str, cwd: &Path) -> GateStepResult {
    let started = Instant::now();
    let mut parts = command.split_whitespace();
    let program = parts.next().unwrap_or("cargo");
    let args = parts.collect::<Vec<_>>();
    let lock_budget_started = Instant::now();
    loop {
        let mut child = std::process::Command::new(program);
        child.args(&args).current_dir(cwd);
        match run_owned_with_output_timeout(child, GATE_STEP_TIMEOUT, GATE_OUTPUT_LIMIT) {
            Ok(BoundedCommandOutput::Completed(output)) => {
                let passed = output.status.success();
                return GateStepResult {
                    command: command.to_owned(),
                    passed,
                    elapsed: started.elapsed(),
                    reason: (!passed).then(|| exit_status_reason(&output.status)),
                };
            }
            Ok(BoundedCommandOutput::TimedOut { stdout, stderr }) => {
                let mut output = String::from_utf8_lossy(stdout.as_bytes()).into_owned();
                output.push_str(&String::from_utf8_lossy(stderr.as_bytes()));
                let lock_held = output.contains(GATE_LOCK_MESSAGE);
                if lock_held && lock_budget_started.elapsed() < GATE_LOCK_RETRY_BUDGET {
                    qol_runtime::probe!(
                        "CLI_SESSION_BRIDGE",
                        "event=gate_lock_retry command={}",
                        command
                    );
                    continue;
                }
                let reason = if lock_held {
                    format!(
                        "timed out after {}s waiting for the cargo file lock",
                        GATE_STEP_TIMEOUT.as_secs()
                    )
                } else {
                    format!("timed out after {}s", GATE_STEP_TIMEOUT.as_secs())
                };
                return GateStepResult {
                    command: command.to_owned(),
                    passed: false,
                    elapsed: started.elapsed(),
                    reason: Some(reason),
                };
            }
            Err(error) => {
                return GateStepResult {
                    command: command.to_owned(),
                    passed: false,
                    elapsed: started.elapsed(),
                    reason: Some(format!("failed to run: {error}")),
                };
            }
        }
    }
}

fn exit_status_reason(status: &std::process::ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "terminated by signal".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use std::thread;

    use qol_terminal_sessions::cli::{
        CliSessionChangeHandler, CliSessionDescriptor, CliSessionEvidence, CliSessionStrategy,
        CliSessionSubscription, CliSessionSubscriptionError, CliTool, CliToolColor, CliToolId,
    };
    use qol_terminal_sessions::{
        BackendId, SessionCapabilities, SessionFocus, SessionId, TerminalBackend, TerminalError,
        TerminalSnapshot,
    };

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FakeCall {
        Ls,
        GetText,
        GetTextMatch,
        SendText,
    }

    struct FakeBackend {
        id: BackendId,
        facts: SessionFacts,
        screens: Mutex<VecDeque<String>>,
        sent: Mutex<Vec<(SessionBinding, String, DeliveryMode)>>,
        calls: Mutex<Vec<FakeCall>>,
    }

    impl FakeBackend {
        fn new(facts: SessionFacts, screens: Vec<String>) -> Arc<Self> {
            Arc::new(Self {
                id: BackendId::new("fake").unwrap(),
                facts,
                screens: Mutex::new(screens.into()),
                sent: Mutex::new(Vec::new()),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn next_screen(&self) -> String {
            let mut screens = self.screens.lock().unwrap();
            if let Some(screen) = screens.pop_front() {
                return screen;
            }
            self.generated_completion()
                .unwrap_or_else(|| ">>> ready".to_owned())
        }

        fn generated_completion(&self) -> Option<String> {
            let sent = self.sent.lock().unwrap();
            let prompt = &sent.last()?.1;
            let fragments = prompt
                .split('`')
                .enumerate()
                .filter_map(|(index, part)| (index % 2 == 1).then_some(part))
                .collect::<Vec<_>>();
            let right = fragments.last()?;
            let left = fragments.get(fragments.len().checked_sub(2)?)?;
            Some(format!("implementation complete\n{left}{right}"))
        }
    }

    impl SessionInventory for FakeBackend {
        fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
            self.calls.lock().unwrap().push(FakeCall::Ls);
            Ok(vec![self.facts.clone()])
        }
    }

    impl ScreenReader for FakeBackend {
        fn read_screen(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            self.calls.lock().unwrap().push(FakeCall::Ls);
            self.calls.lock().unwrap().push(FakeCall::GetText);
            Ok(self.next_screen())
        }

        fn read_screen_relaxed(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            self.calls.lock().unwrap().push(FakeCall::GetText);
            Ok(self.next_screen())
        }

        fn read_screen_matching(
            &self,
            _target: &SessionBinding,
            pattern: &str,
        ) -> Result<String, TerminalError> {
            self.calls.lock().unwrap().push(FakeCall::GetTextMatch);
            let screen = self.next_screen();
            Ok(if screen.contains(pattern) {
                format!("1: {screen}")
            } else {
                String::new()
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
            target: &SessionBinding,
            text: &str,
            mode: DeliveryMode,
        ) -> Result<(), TerminalError> {
            self.calls.lock().unwrap().push(FakeCall::SendText);
            self.sent
                .lock()
                .unwrap()
                .push((target.clone(), text.to_owned(), mode));
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
            _target: &SessionBinding,
        ) -> Result<String, TerminalError> {
            Ok(">>> ready".to_owned())
        }

        fn id(&self) -> &BackendId {
            &self.id
        }
    }

    struct StopSignal(Arc<AtomicBool>);

    impl Drop for StopSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    struct TickerStrategy;

    impl TickerStrategy {
        fn tool() -> CliTool {
            CliTool::new(
                CliToolId::new("ticker").unwrap(),
                "Ticker",
                CliToolColor::new(0, 0, 0),
            )
        }
    }

    impl CliSessionStrategy for TickerStrategy {
        fn tool(&self) -> &CliTool {
            static TOOL: std::sync::OnceLock<CliTool> = std::sync::OnceLock::new();
            TOOL.get_or_init(TickerStrategy::tool)
        }

        fn matches(&self, _session: &SessionFacts) -> bool {
            true
        }

        fn describe(&self, _session: &SessionFacts) -> CliSessionDescriptor {
            CliSessionDescriptor {
                tool: self.tool().clone(),
                display_name: None,
                external_id: None,
                has_activity: None,
                evidence: CliSessionEvidence::default(),
            }
        }

        fn subscribe(
            &self,
            _session: &SessionFacts,
            on_change: CliSessionChangeHandler,
        ) -> Result<Option<CliSessionSubscription>, CliSessionSubscriptionError> {
            let stop = Arc::new(AtomicBool::new(false));
            let tick_stop = Arc::clone(&stop);
            thread::spawn(move || {
                while !tick_stop.load(Ordering::Relaxed) {
                    on_change();
                    thread::sleep(Duration::from_millis(1));
                }
            });
            Ok(Some(CliSessionSubscription::from_guard(StopSignal(stop))))
        }
    }

    #[test]
    fn hot_loop_polls_marker_matches_and_reads_full_screens_only_every_tenth() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = PendingBridgeStore::with_dir(root.path().to_path_buf());
        let binding = SessionBinding::new(
            SessionId::new(BackendId::new("fake").unwrap(), "7").unwrap(),
            123,
        )
        .unwrap();
        let facts = SessionFacts {
            id: binding.session_id().clone(),
            root_pid: binding.root_pid(),
            cwd: "/work/demo".to_owned(),
            title: "Demo REPL".to_owned(),
            at_prompt: true,
            reported_cmd: Some("agent".to_owned()),
            foreground_basenames: Vec::new(),
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: None,
        };
        let mut screens = vec![">>> ready".to_owned()];
        screens.push(
            ">>> [qol session bridge]\nCompletion fragments: `QOL_BRIDGE_DONE_` and `abc123`."
                .to_owned(),
        );
        screens.extend((0..12).map(|index| format!("building step {index}")));
        let backend = FakeBackend::new(facts, screens);
        let terminals = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();
        let interpreter = CliSessionInterpreter::from_strategies([
            Arc::new(TickerStrategy) as Arc<dyn CliSessionStrategy>
        ])
        .unwrap();

        let outcome = execute(
            &terminals,
            &interpreter,
            &binding,
            "implement the bounded change",
            Duration::from_secs(10),
            &pending,
            None,
        )
        .unwrap();

        assert!(outcome.completed);
        assert_eq!(outcome.reads, 14);
        let calls = backend.calls.lock().unwrap();
        let ls = calls.iter().filter(|call| **call == FakeCall::Ls).count() as u64;
        let get_text = calls
            .iter()
            .filter(|call| **call == FakeCall::GetText)
            .count() as u64;
        let get_text_match = calls
            .iter()
            .filter(|call| **call == FakeCall::GetTextMatch)
            .count() as u64;
        assert_eq!(
            calls
                .iter()
                .filter(|call| **call == FakeCall::SendText)
                .count(),
            1
        );
        assert_eq!(ls, outcome.reads / 10 + 3);
        assert_eq!(get_text_match, outcome.reads - 2);
        assert_eq!(get_text, 5);
        assert!(
            get_text < outcome.reads,
            "full reads must drop below the poll count"
        );
        assert!(ls - 3 <= outcome.reads.div_ceil(10) + 1);
    }

    #[test]
    fn prompt_never_contains_the_joined_completion_marker() {
        let marker = CompletionMarker::from_nonce("abc123");
        let prompt = bridge_prompt("implement the bounded change", &marker);

        assert!(prompt.contains("QOL_BRIDGE_DONE_"));
        assert!(prompt.contains("abc123"));
        assert!(!prompt.contains(&marker.token));
    }

    #[test]
    fn kickstart_prompt_reuses_the_checkpoint_marker_fragments() {
        let marker = CompletionMarker::from_token("QOL_BRIDGE_DONE_abc123").unwrap();
        let prompt = kickstart_prompt(&marker);

        assert!(prompt.contains("QOL_BRIDGE_DONE_"));
        assert!(prompt.contains("abc123"));
        assert!(!prompt.contains(&marker.token));
        assert!(CompletionMarker::from_token("unrelated").is_err());
        assert!(CompletionMarker::from_token("QOL_BRIDGE_DONE_").is_err());
    }

    #[test]
    fn task_validation_accepts_prose_and_rejects_terminal_controls() {
        validate_task("line one\nline two\tvalue").unwrap();
        assert!(validate_task("\u{1b}[31munsafe").is_err());
        assert!(validate_task("\0").is_err());
        assert!(validate_task("   ").is_err());
    }

    #[test]
    fn pending_rounds_expose_open_checkpoints_with_their_phase() {
        let root = tempfile::TempDir::new().unwrap();
        let store = PendingBridgeStore::with_dir(root.path().to_path_buf());
        let waiting = SessionBinding::from_str("v1:fake:1:100").unwrap();
        let review = SessionBinding::from_str("v1:fake:2:200").unwrap();
        let closed = SessionBinding::from_str("v1:fake:3:300").unwrap();
        store
            .start(&waiting, "QOL_BRIDGE_DONE_wait", "v1:fake:8:800")
            .unwrap();
        store
            .start(&review, "QOL_BRIDGE_DONE_review", "v1:fake:8:800")
            .unwrap();
        store
            .observe(&review, "QOL_BRIDGE_DONE_review", true)
            .unwrap();
        store.start(&closed, "QOL_BRIDGE_DONE_closed", "").unwrap();
        store
            .acknowledge(&closed, "QOL_BRIDGE_DONE_closed", false)
            .unwrap();

        let rounds = store.pending_rounds().unwrap();
        assert_eq!(rounds.len(), 2);
        let by_session = |token: &str| rounds.iter().find(|round| round.session == token).unwrap();
        assert!(!by_session("v1:fake:1:100").completed);
        assert!(by_session("v1:fake:2:200").completed);
        assert_eq!(
            by_session("v1:fake:2:200").completion_marker,
            "QOL_BRIDGE_DONE_review"
        );

        assert!(store.pending_round(&closed).unwrap().is_none());
        assert!(store.pending_round(&waiting).unwrap().is_some());
    }

    #[test]
    fn only_one_process_can_own_a_session_bridge_at_a_time() {
        let root = tempfile::TempDir::new().unwrap();
        let store = PendingBridgeStore::with_dir(root.path().to_path_buf());
        let binding = SessionBinding::from_str("v1:fake:9:900").unwrap();
        let other = SessionBinding::from_str("v1:fake:9:901").unwrap();

        assert!(store.owner_pid(&binding).is_none());
        let owner = store.acquire_owner(&binding).unwrap();
        assert_eq!(
            store.owner_pid(&binding),
            Some(process::id().to_string()),
            "a held bridge must report its owning process"
        );
        let conflict = store.acquire_owner(&binding).unwrap_err().to_string();
        assert!(conflict.contains("already attached"), "{conflict}");
        assert!(conflict.contains("qol sessions next"), "{conflict}");
        store.acquire_owner(&other).unwrap();

        drop(owner);
        assert!(store.owner_pid(&binding).is_none());
        store.acquire_owner(&binding).unwrap();
    }

    #[test]
    fn acknowledged_checkpoint_is_deleted_and_cannot_be_resurrected_by_a_late_bridge() {
        let root = tempfile::TempDir::new().unwrap();
        let store = PendingBridgeStore::with_dir(root.path().to_path_buf());
        let binding = SessionBinding::from_str("v1:fake:7:123").unwrap();
        store
            .start(&binding, "QOL_BRIDGE_DONE_old", "v1:fake:8:800")
            .unwrap();
        store
            .acknowledge(&binding, "QOL_BRIDGE_DONE_old", false)
            .unwrap();
        assert!(
            store.load(&binding).unwrap().is_none(),
            "acknowledging a round must remove its checkpoint file"
        );
        store
            .observe(&binding, "QOL_BRIDGE_DONE_old", true)
            .unwrap();
        assert!(
            store.load(&binding).unwrap().is_none(),
            "a late observe must not resurrect an acknowledged round"
        );

        store
            .start(&binding, "QOL_BRIDGE_DONE_new", "v1:fake:8:800")
            .unwrap();
        store
            .observe(&binding, "QOL_BRIDGE_DONE_old", true)
            .unwrap();
        let current = store.load(&binding).unwrap().unwrap();
        assert_eq!(current.completion_marker, "QOL_BRIDGE_DONE_new");
        assert!(!current.completed);
        assert!(!current.closed);
    }

    #[test]
    fn sweep_removes_stale_tmp_legacy_and_closed_checkpoints() {
        let root = tempfile::TempDir::new().unwrap();
        let store = PendingBridgeStore::with_dir(root.path().to_path_buf());
        let open = SessionBinding::from_str("v1:fake:1:100").unwrap();
        store
            .start(&open, "QOL_BRIDGE_DONE_open", "v1:fake:8:800")
            .unwrap();

        let stale_tmp = root.path().join("stale.tmp");
        fs::write(&stale_tmp, b"partial").unwrap();
        age_file(&stale_tmp, Duration::from_secs(7200));
        let fresh_tmp = root.path().join("fresh.tmp");
        fs::write(&fresh_tmp, b"partial").unwrap();
        let legacy = root.path().join("legacy.json");
        fs::write(
            &legacy,
            serde_json::to_string(&BridgeCheckpoint {
                session: String::new(),
                driver: String::new(),
                completion_marker: "QOL_BRIDGE_DONE_legacy".to_owned(),
                completed: false,
                closed: false,
            })
            .unwrap(),
        )
        .unwrap();
        let closed = root.path().join("closed.json");
        fs::write(
            &closed,
            serde_json::to_string(&BridgeCheckpoint {
                session: "v1:fake:2:200".to_owned(),
                driver: String::new(),
                completion_marker: "QOL_BRIDGE_DONE_closed".to_owned(),
                completed: true,
                closed: true,
            })
            .unwrap(),
        )
        .unwrap();

        let rounds = store.pending_rounds().unwrap();
        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].session, "v1:fake:1:100");
        assert!(!stale_tmp.exists());
        assert!(fresh_tmp.exists());
        assert!(!legacy.exists());
        assert!(!closed.exists());
        assert!(store.file_for(&open).exists());
    }

    fn age_file(path: &std::path::Path, age: Duration) {
        let file = fs::File::open(path).unwrap();
        let times = std::fs::FileTimes::new().set_modified(SystemTime::now() - age);
        file.set_times(times).unwrap();
    }

    #[test]
    fn gate_summary_formatter_lists_step_results_total_and_verdict() {
        let summary = GateSummary {
            steps: vec![
                GateStepResult {
                    command: "cargo fmt --check -p qol".to_owned(),
                    passed: true,
                    elapsed: Duration::from_millis(1200),
                    reason: None,
                },
                GateStepResult {
                    command: "cargo clippy -p qol --all-targets -- -D warnings".to_owned(),
                    passed: false,
                    elapsed: Duration::from_secs(38),
                    reason: Some("exit code 101".to_owned()),
                },
            ],
            total: Duration::from_secs(40),
            skipped_reason: None,
        };

        let text = format_gate_summary(&summary);
        assert!(text.starts_with("--- GATE ---\n"));
        assert!(text.contains("[1/2] PASS cargo fmt --check -p qol (1.2s)"));
        assert!(text.contains(
            "[2/2] FAIL cargo clippy -p qol --all-targets -- -D warnings (38.0s) - exit code 101"
        ));
        assert!(text.contains("total: 40.0s"));
        assert!(text.contains("verdict: RED"));
    }

    #[test]
    fn gate_summary_formatter_verdict_is_green_only_when_every_step_passes() {
        let summary = GateSummary {
            steps: vec![GateStepResult {
                command: "cargo test -p qol --bin qol".to_owned(),
                passed: true,
                elapsed: Duration::from_secs(5),
                reason: None,
            }],
            total: Duration::from_secs(5),
            skipped_reason: None,
        };
        assert!(format_gate_summary(&summary).contains("verdict: GREEN"));
    }

    #[test]
    fn gate_summary_formatter_renders_the_skip_note_without_a_verdict() {
        let summary = GateSummary {
            steps: Vec::new(),
            total: Duration::ZERO,
            skipped_reason: Some("no Cargo.toml in /tmp/none; the gate is skipped".to_owned()),
        };
        let text = format_gate_summary(&summary);
        assert!(text.starts_with("--- GATE ---\nskipped: "));
        assert!(text.contains("no Cargo.toml in /tmp/none; the gate is skipped"));
        assert!(!text.contains("verdict:"));
    }

    #[test]
    fn quality_gate_skips_a_directory_without_a_cargo_manifest_and_preserves_the_screen() {
        let root = tempfile::TempDir::new().unwrap();
        let screen = "implementation complete\nQOL_BRIDGE_DONE_abc123";

        let text = run_quality_gate(screen, root.path());

        assert!(text.starts_with(screen));
        assert!(text.contains("--- GATE ---"));
        assert!(text.contains("skipped: no Cargo.toml in"));
    }

    #[cfg(unix)]
    #[test]
    fn gate_step_reports_pass_and_fail_from_the_exit_status() {
        let cwd = std::env::temp_dir();
        let pass = run_gate_step("/bin/true", &cwd);
        assert!(
            pass.passed,
            "{}",
            pass.reason.as_deref().unwrap_or_default()
        );
        assert!(pass.reason.is_none());

        let fail = run_gate_step("/bin/false", &cwd);
        assert!(!fail.passed);
        assert_eq!(fail.reason.as_deref(), Some("exit code 1"));
        assert!(fail.elapsed < Duration::from_secs(30));
    }
}
