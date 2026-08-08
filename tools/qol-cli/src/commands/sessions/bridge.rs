use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::{
    DeliveryMode, ScreenReader, SessionBinding, SessionFacts, SessionInventory,
    TerminalSessionService, TextInput,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const COMPLETION_SETTLE_INTERVAL: Duration = Duration::from_millis(250);
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(1);
const SUBSCRIPTION_HEARTBEAT: Duration = Duration::from_secs(30);
const STALL_PROBE_AFTER: Duration = Duration::from_secs(30);
const TASK_MAX_BYTES: usize = 64 * 1024;

pub(super) const TIMEOUT_MIN_MS: u64 = 1_000;
pub(super) const TIMEOUT_DEFAULT_MS: u64 = 3_600_000;
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
}

#[derive(Debug, Deserialize, Serialize)]
struct PendingBridge {
    #[serde(default)]
    session: String,
    completion_marker: String,
    completed: bool,
    closed: bool,
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

pub(super) struct PendingBridgeStore {
    dir: PathBuf,
}

impl PendingBridgeStore {
    pub(super) fn system() -> Result<Self> {
        let dir = qol_config::data_subdir("sessions")
            .ok_or_else(|| anyhow!("sessions data directory is unavailable"))?
            .join("pending-bridge");
        Ok(Self { dir })
    }

    #[cfg(test)]
    pub(super) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn load(&self, binding: &SessionBinding) -> Result<Option<PendingBridge>> {
        let _lock = self.lock(binding)?;
        self.load_unlocked(binding)
    }

    fn load_unlocked(&self, binding: &SessionBinding) -> Result<Option<PendingBridge>> {
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

    pub(super) fn start(&self, binding: &SessionBinding, marker: &str) -> Result<()> {
        let _lock = self.lock(binding)?;
        if self
            .load_unlocked(binding)?
            .is_some_and(|checkpoint| !checkpoint.closed)
        {
            bail!("a bridge is already pending for `{binding}`");
        }
        self.write_unlocked(binding, marker, false, false)
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
        self.write_unlocked(binding, marker, completed, false)
    }

    fn write_unlocked(
        &self,
        binding: &SessionBinding,
        marker: &str,
        completed: bool,
        closed: bool,
    ) -> Result<()> {
        fs::create_dir_all(&self.dir).context("failed to create pending bridge directory")?;
        let file = self.file_for(binding);
        let temporary = file.with_extension("tmp");
        let encoded = serde_json::to_string(&PendingBridge {
            session: binding.token(),
            completion_marker: marker.to_owned(),
            completed,
            closed,
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
        self.write_unlocked(binding, marker, checkpoint.completed, true)
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
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("failed to read pending bridge directory"),
        };
        let mut rounds = Vec::new();
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
            let Ok(checkpoint) = serde_json::from_str::<PendingBridge>(&encoded) else {
                continue;
            };
            if checkpoint.closed || checkpoint.session.is_empty() {
                continue;
            }
            rounds.push(PendingRound {
                session: checkpoint.session,
                completion_marker: checkpoint.completion_marker,
                completed: checkpoint.completed,
            });
        }
        rounds.sort_by(|left, right| left.session.cmp(&right.session));
        Ok(rounds)
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
    if terminals
        .is_current(binding)
        .context("failed to identify the current terminal session")?
    {
        bail!("cannot bridge to the calling terminal; choose an independent session");
    }
    let target = resolve_target(terminals, binding)?;
    if let Some(marker) = acknowledge_marker {
        pending.acknowledge(binding, marker, true)?;
    } else if pending
        .load(binding)?
        .is_some_and(|checkpoint| !checkpoint.closed)
    {
        return resume(terminals, interpreter, binding, timeout, pending, false);
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
    pending.start(binding, &marker.token)?;

    if let Err(error) = terminals.send_text(binding, &prompt, DeliveryMode::Submit) {
        pending.acknowledge(binding, &marker.token, false)?;
        qol_runtime::probe!(
            "CLI_SESSION_BRIDGE",
            "event=delivery_failed target_backend={}",
            binding.session_id().backend()
        );
        return Err(error).context("bridge task delivery failed");
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

pub(super) fn resume(
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
    let target = resolve_target(terminals, binding)?;
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
    let started = Instant::now();
    let mut previous = None;
    let mut last_change = Instant::now();
    let mut last_probe: Option<Instant> = None;
    let mut reads = 0;
    loop {
        let screen = terminals
            .read_screen(binding)
            .context("bridge screen read failed")?;
        reads += 1;
        let matched = screen.contains(marker);
        if matched && previous.as_deref() == Some(screen.as_str()) {
            return Ok(outcome(
                true, submitted, false, binding, marker, screen, reads, started,
            ));
        }
        if previous.as_deref() != Some(screen.as_str()) {
            last_change = Instant::now();
        }
        previous = Some(screen.clone());
        if !matched
            && last_change.elapsed() >= stall_after
            && last_probe.is_none_or(|probed| probed.elapsed() >= stall_after)
        {
            last_probe = Some(Instant::now());
            if liveness() == Some(false) {
                return Ok(outcome(
                    false, submitted, true, binding, marker, screen, reads, started,
                ));
            }
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return Ok(outcome(
                false, submitted, false, binding, marker, screen, reads, started,
            ));
        }
        let remaining = timeout.saturating_sub(elapsed);
        let interval = if matched {
            COMPLETION_SETTLE_INTERVAL
        } else if !subscribed {
            FALLBACK_POLL_INTERVAL
        } else {
            SUBSCRIPTION_HEARTBEAT
        }
        .min(remaining);
        if subscribed {
            match changed.recv_timeout(interval) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(anyhow!("implementation-session change stream disconnected"));
                }
            }
        } else {
            std::thread::sleep(interval);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

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
        store.start(&waiting, "QOL_BRIDGE_DONE_wait").unwrap();
        store.start(&review, "QOL_BRIDGE_DONE_review").unwrap();
        store
            .observe(&review, "QOL_BRIDGE_DONE_review", true)
            .unwrap();
        store.start(&closed, "QOL_BRIDGE_DONE_closed").unwrap();
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
    fn closed_checkpoint_cannot_be_resurrected_by_a_late_bridge() {
        let root = tempfile::TempDir::new().unwrap();
        let store = PendingBridgeStore::with_dir(root.path().to_path_buf());
        let binding = SessionBinding::from_str("v1:fake:7:123").unwrap();
        store.start(&binding, "QOL_BRIDGE_DONE_old").unwrap();
        store
            .acknowledge(&binding, "QOL_BRIDGE_DONE_old", false)
            .unwrap();
        store
            .observe(&binding, "QOL_BRIDGE_DONE_old", true)
            .unwrap();
        let closed = store.load(&binding).unwrap().unwrap();
        assert!(closed.closed);

        store.start(&binding, "QOL_BRIDGE_DONE_new").unwrap();
        store
            .observe(&binding, "QOL_BRIDGE_DONE_old", true)
            .unwrap();
        let current = store.load(&binding).unwrap().unwrap();
        assert_eq!(current.completion_marker, "QOL_BRIDGE_DONE_new");
        assert!(!current.completed);
        assert!(!current.closed);
    }
}
