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
use serde::Serialize;
use sha2::{Digest, Sha256};

const COMPLETION_SETTLE_INTERVAL: Duration = Duration::from_millis(250);
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(1);
const SUBSCRIPTION_HEARTBEAT: Duration = Duration::from_secs(30);
const TASK_MAX_BYTES: usize = 64 * 1024;

pub(super) const TIMEOUT_MIN_MS: u64 = 1_000;
pub(super) const TIMEOUT_DEFAULT_MS: u64 = 3_600_000;
pub(super) const TIMEOUT_MAX_MS: u64 = 86_400_000;

static MARKER_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize)]
pub(super) struct BridgeOutcome {
    pub(super) completed: bool,
    pub(super) session: String,
    pub(super) completion_marker: String,
    pub(super) screen: String,
    pub(super) reads: u64,
    pub(super) elapsed_ms: u128,
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
}

pub(super) fn execute(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
    task: &str,
    timeout: Duration,
) -> Result<BridgeOutcome> {
    validate_task(task)?;
    if terminals
        .is_current(binding)
        .context("failed to identify the current terminal session")?
    {
        bail!("cannot bridge to the calling terminal; choose an independent session");
    }
    let target = resolve_target(terminals, binding)?;
    let marker = CompletionMarker::generate();
    let prompt = bridge_prompt(task, &marker);
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

    if let Err(error) = terminals.send_text(binding, &prompt, DeliveryMode::Submit) {
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
    )?;
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

fn wait_for_completion(
    terminals: &TerminalSessionService,
    binding: &SessionBinding,
    marker: &str,
    timeout: Duration,
    changed: mpsc::Receiver<()>,
    subscribed: bool,
) -> Result<BridgeOutcome> {
    let started = Instant::now();
    let mut previous = None;
    let mut reads = 0;
    loop {
        let screen = terminals
            .read_screen(binding)
            .context("bridge screen read failed")?;
        reads += 1;
        let matched = screen.contains(marker);
        if matched && previous.as_deref() == Some(screen.as_str()) {
            return Ok(outcome(true, binding, marker, screen, reads, started));
        }
        previous = Some(screen.clone());
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return Ok(outcome(false, binding, marker, screen, reads, started));
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

fn outcome(
    completed: bool,
    binding: &SessionBinding,
    marker: &str,
    screen: String,
    reads: u64,
    started: Instant,
) -> BridgeOutcome {
    BridgeOutcome {
        completed,
        session: binding.token(),
        completion_marker: marker.to_owned(),
        screen,
        reads,
        elapsed_ms: started.elapsed().as_millis(),
    }
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

    #[test]
    fn prompt_never_contains_the_joined_completion_marker() {
        let marker = CompletionMarker::from_nonce("abc123");
        let prompt = bridge_prompt("implement the bounded change", &marker);

        assert!(prompt.contains("QOL_BRIDGE_DONE_"));
        assert!(prompt.contains("abc123"));
        assert!(!prompt.contains(&marker.token));
    }

    #[test]
    fn task_validation_accepts_prose_and_rejects_terminal_controls() {
        validate_task("line one\nline two\tvalue").unwrap();
        assert!(validate_task("\u{1b}[31munsafe").is_err());
        assert!(validate_task("\0").is_err());
        assert!(validate_task("   ").is_err());
    }
}
