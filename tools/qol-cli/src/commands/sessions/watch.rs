use std::ffi::OsString;
use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use qol_terminal_sessions::{
    DeliveryMode, ScreenReader, SessionBinding, SessionInventory, TerminalSessionService, TextInput,
};

use super::bridge::{PendingBridgeStore, PendingRound};
use super::spawn::{SpawnLedger, SpawnLocks};

use qol_terminal_sessions::cli::CliSessionInterpreter;

const POLL_BASE: Duration = Duration::from_secs(3);
const POLL_CAP: Duration = Duration::from_secs(30);
const STALL_AFTER: Duration = Duration::from_secs(15 * 60);
const SCREEN_SNAPSHOT_MAX_BYTES: usize = 64 * 1024;
const WAKE_SNIPPET_MAX_BYTES: usize = 8 * 1024;
const WAKE_COMPOSER_POLL_INTERVAL: Duration = Duration::from_secs(1);
const WAKE_COMPOSER_STATIC_POLLS: usize = 30;
const WAKE_COMPOSER_MAX_ATTEMPTS: usize = 300;
const WATCH_ALL_KEY: &str = "watch-all";

#[derive(Clone, Copy)]
pub(super) struct WatchConfig {
    poll_base: Duration,
    poll_cap: Duration,
    stall_after: Duration,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            poll_base: POLL_BASE,
            poll_cap: POLL_CAP,
            stall_after: STALL_AFTER,
        }
    }
}

struct WatchedRound {
    session: String,
    binding: SessionBinding,
    driver: String,
    marker: String,
    reads: u64,
    last_screen: Option<String>,
    last_change: Instant,
    stalled_reported: bool,
    marker_seen: bool,
    external_id_captured: bool,
    autoclose: bool,
}

impl WatchedRound {
    fn new(round: PendingRound) -> Result<Self> {
        let binding = round
            .session
            .parse()
            .map_err(|_| anyhow!("pending checkpoint carries an invalid session token"))?;
        Ok(Self {
            session: round.session,
            binding,
            driver: round.driver,
            marker: round.completion_marker,
            reads: 0,
            last_screen: None,
            last_change: Instant::now(),
            stalled_reported: false,
            marker_seen: false,
            external_id_captured: false,
            autoclose: round.autoclose,
        })
    }
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    let tokens = args
        .iter()
        .map(|argument| {
            let token = argument
                .to_str()
                .ok_or_else(|| anyhow!("watch tokens must be valid UTF-8"))?
                .to_owned();
            token
                .parse::<SessionBinding>()
                .map_err(|_| anyhow!("invalid session token `{token}`"))?;
            Ok(token)
        })
        .collect::<Result<Vec<_>>>()?;
    let trace_dir = qol_config::data_subdir("sessions").unwrap_or_else(|| ".".into());
    watch(
        &TerminalSessionService::system(),
        &CliSessionInterpreter::system(),
        &PendingBridgeStore::system()?,
        &SpawnLedger::system()?,
        &SpawnLocks::system()?,
        &tokens,
        &mut std::io::stdout(),
        &trace_dir,
        WatchConfig::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn watch(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    pending: &PendingBridgeStore,
    ledger: &SpawnLedger,
    locks: &SpawnLocks,
    tokens: &[String],
    out: &mut dyn Write,
    trace_dir: &std::path::Path,
    config: WatchConfig,
) -> Result<()> {
    let watch_lock = if tokens.is_empty() {
        let key = qol_terminal_sessions::SpawnKey::new(WATCH_ALL_KEY)
            .context("watch lock key is invalid")?;
        let guard = locks
            .acquire(&key)
            .context("another qol sessions watch is already running")?;
        Some((key, guard))
    } else {
        None
    };
    let result = watch_loop(
        terminals,
        interpreter,
        pending,
        ledger,
        locks,
        tokens,
        out,
        trace_dir,
        config,
        &mut |duration| std::thread::sleep(duration),
    );
    if let Some((key, guard)) = watch_lock {
        drop(guard);
        locks.remove(&key);
    }
    result
}

struct RoundPoll {
    keep: bool,
    changed: bool,
}

#[allow(clippy::too_many_arguments)]
fn poll_round(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    pending: &PendingBridgeStore,
    ledger: &SpawnLedger,
    locks: &SpawnLocks,
    round: &mut WatchedRound,
    out: &mut dyn Write,
    trace_dir: &std::path::Path,
    config: WatchConfig,
    sleep: &mut dyn FnMut(Duration),
) -> Result<RoundPoll> {
    round.reads += 1;
    let screen = if round.reads.is_multiple_of(10) {
        terminals.read_screen(&round.binding)
    } else {
        terminals.read_screen_relaxed(&round.binding)
    };
    let screen = match screen {
        Ok(screen) => screen,
        Err(_) => {
            if session_gone(terminals, &round.binding) {
                if round.marker_seen {
                    if !pending.claim_wake(&round.binding, "completed")? {
                        return Ok(RoundPoll {
                            keep: false,
                            changed: false,
                        });
                    }
                    let last = round.last_screen.clone().unwrap_or_default();
                    let tail = screen_tail(&last);
                    let delivery = deliver_wake(
                        terminals,
                        trace_dir,
                        &round.session,
                        &round.driver,
                        "completed",
                        &wake_message(&round.session, "completed", tail, round.autoclose),
                        sleep,
                    )?;
                    pending.observe(&round.binding, &round.marker, true)?;
                    pending.store_screen(&round.binding, &round.marker, tail)?;
                    qol_runtime::probe!(
                        "CLI_SESSION_WATCH",
                        "event=completed_after_exit session={} reads={} delivered={} autoclose={}",
                        round.session,
                        round.reads,
                        delivery.delivered,
                        round.autoclose
                    );
                    emit_completed(
                        out,
                        &round.session,
                        &round.marker,
                        tail,
                        round.autoclose,
                        &delivery,
                    )?;
                    return Ok(RoundPoll {
                        keep: false,
                        changed: true,
                    });
                }
                if !pending.claim_wake(&round.binding, "gone")? {
                    return Ok(RoundPoll {
                        keep: false,
                        changed: false,
                    });
                }
                let delivery = deliver_wake(
                    terminals,
                    trace_dir,
                    &round.session,
                    &round.driver,
                    "gone",
                    &wake_message(&round.session, "gone", "", false),
                    sleep,
                )?;
                emit_gone(out, &round.session, &delivery)?;
                pending.discard(&round.binding)?;
                qol_runtime::probe!(
                    "CLI_SESSION_WATCH",
                    "event=gone session={} reads={} delivered={}",
                    round.session,
                    round.reads,
                    delivery.delivered
                );
                return Ok(RoundPoll {
                    keep: false,
                    changed: true,
                });
            }
            return Ok(RoundPoll {
                keep: true,
                changed: false,
            });
        }
    };
    if !round.external_id_captured {
        round.external_id_captured = super::spawn::capture_lane_external_id(
            terminals,
            interpreter,
            ledger,
            locks,
            &round.binding,
        );
    }
    if screen.contains(&round.marker) {
        match pending.pending_round(&round.binding)? {
            None => {
                return Ok(RoundPoll {
                    keep: false,
                    changed: false,
                });
            }
            Some(current) if current.completed => {
                qol_runtime::probe!(
                    "CLI_SESSION_WATCH",
                    "event=collected session={} source=checkpoint",
                    round.session
                );
                return Ok(RoundPoll {
                    keep: false,
                    changed: false,
                });
            }
            Some(current) if current.completion_marker != round.marker => {
                round.marker_seen = false;
            }
            Some(_) => {
                if round.marker_seen {
                    if !pending.claim_wake(&round.binding, "completed")? {
                        return Ok(RoundPoll {
                            keep: false,
                            changed: false,
                        });
                    }
                    if !round.external_id_captured {
                        round.external_id_captured = super::spawn::capture_lane_external_id(
                            terminals,
                            interpreter,
                            ledger,
                            locks,
                            &round.binding,
                        );
                    }
                    let tail = screen_tail(&screen);
                    let delivery = deliver_wake(
                        terminals,
                        trace_dir,
                        &round.session,
                        &round.driver,
                        "completed",
                        &wake_message(&round.session, "completed", tail, round.autoclose),
                        sleep,
                    )?;
                    if round.autoclose && delivery.delivered {
                        close_lane_terminal(terminals, &round.binding);
                    }
                    pending.observe(&round.binding, &round.marker, true)?;
                    pending.store_screen(&round.binding, &round.marker, tail)?;
                    qol_runtime::probe!(
                        "CLI_SESSION_WATCH",
                        "event=completed session={} reads={} delivered={} autoclose={}",
                        round.session,
                        round.reads,
                        delivery.delivered,
                        round.autoclose
                    );
                    emit_completed(
                        out,
                        &round.session,
                        &round.marker,
                        tail,
                        round.autoclose,
                        &delivery,
                    )?;
                    return Ok(RoundPoll {
                        keep: false,
                        changed: true,
                    });
                }
                round.marker_seen = true;
                if round.last_screen.as_deref() != Some(screen.as_str()) {
                    round.last_screen = Some(screen.clone());
                    round.last_change = Instant::now();
                }
                return Ok(RoundPoll {
                    keep: true,
                    changed: true,
                });
            }
        }
    } else {
        round.marker_seen = false;
    }
    let mut changed = false;
    if round.last_screen.as_deref() != Some(screen.as_str()) {
        round.last_screen = Some(screen.clone());
        round.last_change = Instant::now();
        changed = true;
    }
    if !round.stalled_reported && round.last_change.elapsed() >= config.stall_after {
        round.stalled_reported = true;
        if pending.claim_wake(&round.binding, "stalled")? {
            let delivery = deliver_wake(
                terminals,
                trace_dir,
                &round.session,
                &round.driver,
                "stalled",
                &wake_message(&round.session, "stalled", "", false),
                sleep,
            )?;
            emit_stalled(out, &round.session, &delivery)?;
            qol_runtime::probe!(
                "CLI_SESSION_WATCH",
                "event=stalled session={} reads={} delivered={}",
                round.session,
                round.reads,
                delivery.delivered
            );
        }
        changed = true;
    }
    Ok(RoundPoll {
        keep: true,
        changed,
    })
}

fn reconcile(pending: &PendingBridgeStore, watched: &mut Vec<WatchedRound>) -> Result<()> {
    let mut remaining = Vec::with_capacity(watched.len());
    for mut round in std::mem::take(watched) {
        match pending.pending_round(&round.binding)? {
            None => {}
            Some(current) => {
                if current.completed {
                    qol_runtime::probe!(
                        "CLI_SESSION_WATCH",
                        "event=collected session={} source=checkpoint",
                        round.session
                    );
                } else {
                    if current.completion_marker != round.marker {
                        round.marker = current.completion_marker;
                        round.reads = 0;
                        round.last_screen = None;
                        round.last_change = Instant::now();
                        round.stalled_reported = false;
                        round.marker_seen = false;
                    }
                    remaining.push(round);
                }
            }
        }
    }
    *watched = remaining;
    Ok(())
}

fn prune_stale_tokens(
    terminals: &TerminalSessionService,
    pending: &PendingBridgeStore,
    tokens: &[String],
) -> Result<Vec<String>> {
    if tokens.is_empty() {
        return Ok(tokens.to_vec());
    }
    let mut kept = Vec::with_capacity(tokens.len());
    for token in tokens {
        let binding: SessionBinding = token.parse().context("invalid session token")?;
        let round_open = pending
            .pending_round(&binding)?
            .is_some_and(|round| !round.completed);
        if round_open || !session_gone(terminals, &binding) {
            kept.push(token.clone());
        } else {
            qol_runtime::probe!("CLI_SESSION_WATCH", "event=pruned_stale session={}", token);
        }
    }
    Ok(kept)
}

fn load_rounds(pending: &PendingBridgeStore, tokens: &[String]) -> Result<Vec<PendingRound>> {
    if tokens.is_empty() {
        return pending.pending_rounds();
    }
    let mut rounds = Vec::new();
    for token in tokens {
        let binding: SessionBinding = token.parse().context("invalid session token")?;
        if let Some(round) = pending.pending_round(&binding)? {
            rounds.push(round);
        }
    }
    rounds.sort_by(|left, right| left.session.cmp(&right.session));
    Ok(rounds)
}

fn session_gone(terminals: &TerminalSessionService, binding: &SessionBinding) -> bool {
    terminals
        .discover()
        .map(|facts| {
            !facts
                .iter()
                .any(|session| session.id == *binding.session_id())
        })
        .unwrap_or(false)
}

fn screen_tail(screen: &str) -> &str {
    if screen.len() <= SCREEN_SNAPSHOT_MAX_BYTES {
        return screen;
    }
    let mut start = screen.len() - SCREEN_SNAPSHOT_MAX_BYTES;
    while !screen.is_char_boundary(start) {
        start += 1;
    }
    &screen[start..]
}

#[allow(clippy::too_many_arguments)]
fn watch_loop(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    pending: &PendingBridgeStore,
    ledger: &SpawnLedger,
    locks: &SpawnLocks,
    tokens: &[String],
    out: &mut dyn Write,
    trace_dir: &std::path::Path,
    config: WatchConfig,
    sleep: &mut dyn FnMut(Duration),
) -> Result<()> {
    let explicit = !tokens.is_empty();
    let tokens = prune_stale_tokens(terminals, pending, tokens)?;
    if explicit && tokens.is_empty() {
        return Ok(());
    }
    let mut watched = load_rounds(pending, &tokens)?
        .into_iter()
        .map(WatchedRound::new)
        .collect::<Result<Vec<_>>>()?;
    let mut poll_interval = config.poll_base;
    loop {
        if watched.is_empty() {
            return Ok(());
        }
        reconcile(pending, &mut watched)?;
        if watched.is_empty() {
            return Ok(());
        }
        let mut changed = false;
        let mut remaining = Vec::with_capacity(watched.len());
        for mut round in watched {
            let outcome = poll_round(
                terminals,
                interpreter,
                pending,
                ledger,
                locks,
                &mut round,
                out,
                trace_dir,
                config,
                sleep,
            )?;
            changed |= outcome.changed;
            if outcome.keep {
                remaining.push(round);
            }
        }
        watched = remaining;
        let sleep_for = if changed {
            config.poll_base
        } else {
            poll_interval
        };
        poll_interval = if changed {
            config.poll_base
        } else {
            next_poll_interval(poll_interval, config.poll_cap)
        };
        sleep(sleep_for);
    }
}

fn next_poll_interval(current: Duration, cap: Duration) -> Duration {
    current.saturating_mul(2).min(cap)
}

fn close_lane_terminal(terminals: &TerminalSessionService, binding: &SessionBinding) {
    if session_gone(terminals, binding) {
        qol_runtime::probe!(
            "CLI_SESSION_WATCH",
            "event=autoclose_skipped session={} reason=already_gone",
            binding.token()
        );
        return;
    }
    if terminals.is_current(binding).unwrap_or(false) {
        qol_runtime::probe!(
            "CLI_SESSION_WATCH",
            "event=autoclose_skipped session={} reason=calling_terminal",
            binding.token()
        );
        return;
    }
    match terminals.close(binding) {
        Ok(()) => {
            qol_runtime::probe!(
                "CLI_SESSION_WATCH",
                "event=autoclosed session={}",
                binding.token()
            );
        }
        Err(error) => {
            qol_runtime::probe!(
                "CLI_SESSION_WATCH",
                "event=autoclose_failed session={} error={}",
                binding.token(),
                error
            );
        }
    }
}

struct WakeDelivery {
    delivered: bool,
    error: Option<String>,
}

fn wake_message(session: &str, event: &str, screen: &str, autoclose: bool) -> String {
    match event {
        "completed" => {
            let report = report_snippet(screen);
            if autoclose {
                format!(
                    "qol sessions: lane {session} completed and the lane terminal closed. Report below.\n\n{report}"
                )
            } else {
                format!("qol sessions: {session} completed. Report below.\n\n{report}")
            }
        }
        "gone" => format!(
            "qol sessions: lane {session} gone. The lane terminal closed and its round was discarded; start a fresh lane if the work still matters."
        ),
        _ => format!(
            "qol sessions: lane {session} stalled. The lane produced no output for 15 minutes; nudge it with qol sessions resume --kickstart, or collect with session_bridge."
        ),
    }
}

fn clean_screen(text: &str) -> String {
    let mut cleaned = Vec::new();
    let mut blank_run = 0usize;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run == 1 && !cleaned.is_empty() {
                cleaned.push(String::new());
            }
        } else {
            blank_run = 0;
            cleaned.push(line.to_owned());
        }
    }
    while cleaned.last().is_some_and(|line| line.is_empty()) {
        cleaned.pop();
    }
    cleaned.join("\n")
}

fn report_snippet(screen: &str) -> String {
    let cleaned = clean_screen(screen);
    if cleaned.len() <= WAKE_SNIPPET_MAX_BYTES {
        return cleaned;
    }
    let mut start = cleaned.len() - WAKE_SNIPPET_MAX_BYTES;
    while !cleaned.is_char_boundary(start) {
        start += 1;
    }
    format!(
        "(report tail; full screen via session_bridge)\n{}",
        &cleaned[start..]
    )
}

fn wake_failure_path(trace_dir: &std::path::Path, session: &str) -> std::path::PathBuf {
    let key = session.replace([':', '.'], "_");
    trace_dir.join(format!("wake-failed-{key}.json"))
}

fn record_wake_failure(
    trace_dir: &std::path::Path,
    session: &str,
    driver: &str,
    event: &str,
    error: &str,
) {
    let path = wake_failure_path(trace_dir, session);
    let record = serde_json::json!({
        "session": session,
        "driver": driver,
        "event": event,
        "error": error,
        "at": chrono::Utc::now().to_rfc3339(),
    });
    let _ = std::fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new(".")));
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&record).unwrap_or_default(),
    );
}

fn composer_busy(screen: &str) -> bool {
    let mut composer = None;
    for line in screen.lines() {
        let line = line.trim();
        if let Some(rest) = line
            .strip_prefix("> ")
            .or_else(|| line.strip_prefix("❯ "))
            .or_else(|| (line == ">" || line == "❯").then_some(""))
        {
            composer = Some(rest);
        }
    }
    composer.is_some_and(|rest| rest.chars().any(|character| !character.is_whitespace()))
}

fn composer_draft_region(screen: &str) -> Option<String> {
    let mut region_start = None;
    for (index, line) in screen.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed
            .strip_prefix("> ")
            .or_else(|| trimmed.strip_prefix("❯ "))
            .or_else(|| (trimmed == ">" || trimmed == "❯").then_some(""))
            .is_some()
        {
            region_start = Some(index);
        }
    }
    let start = region_start?;
    Some(
        screen
            .lines()
            .skip(start)
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn deliver_wake(
    terminals: &TerminalSessionService,
    trace_dir: &std::path::Path,
    session: &str,
    driver: &str,
    event: &str,
    message: &str,
    sleep: &mut dyn FnMut(Duration),
) -> Result<WakeDelivery> {
    let outcome = if driver.is_empty() {
        WakeDelivery {
            delivered: false,
            error: Some(
                "the initiator terminal is unknown; the spawn ran outside a terminal".to_owned(),
            ),
        }
    } else {
        match driver.parse::<SessionBinding>() {
            Ok(binding) => {
                if session_gone(terminals, &binding) {
                    WakeDelivery {
                        delivered: false,
                        error: Some("the initiator terminal is no longer live".to_owned()),
                    }
                } else {
                    let mut deferrals = 0usize;
                    let mut static_polls = 0usize;
                    let mut busy_cleared = false;
                    if let Ok(mut screen) = terminals.read_screen_relaxed(&binding) {
                        let mut previous_region = composer_draft_region(&screen);
                        while composer_busy(&screen)
                            && deferrals < WAKE_COMPOSER_MAX_ATTEMPTS
                            && static_polls < WAKE_COMPOSER_STATIC_POLLS
                        {
                            sleep(WAKE_COMPOSER_POLL_INTERVAL);
                            deferrals += 1;
                            match terminals.read_screen_relaxed(&binding) {
                                Ok(next) => {
                                    let region = composer_draft_region(&next);
                                    if region == previous_region {
                                        static_polls += 1;
                                    } else {
                                        static_polls = 0;
                                        previous_region = region;
                                    }
                                    screen = next;
                                }
                                Err(_) => break,
                            }
                        }
                        busy_cleared = !composer_busy(&screen);
                    }
                    if deferrals > 0 {
                        qol_runtime::probe!(
                            "CLI_SESSION_WATCH",
                            "event=wake_deferred_composer_busy driver={} waited_s={}",
                            driver,
                            (deferrals as u64) * WAKE_COMPOSER_POLL_INTERVAL.as_secs()
                        );
                        let sanitized = driver.replace([':', '.'], "_");
                        let _ = std::fs::OpenOptions::new()
                            .append(true)
                            .create(true)
                            .open(trace_dir.join(format!("wake-debug-{sanitized}.log")))
                            .and_then(|mut file| {
                                file.write_all(
                                    format!(
                                        "{} wake deferred driver={} polls={} static={} busy_cleared={}\n",
                                        chrono::Utc::now().to_rfc3339(),
                                        driver,
                                        deferrals,
                                        static_polls,
                                        busy_cleared
                                    )
                                    .as_bytes(),
                                )
                            });
                    }
                    match terminals.send_text(&binding, message, DeliveryMode::Submit) {
                        Ok(()) => WakeDelivery {
                            delivered: true,
                            error: None,
                        },
                        Err(error) => WakeDelivery {
                            delivered: false,
                            error: Some(error.to_string()),
                        },
                    }
                }
            }
            Err(error) => WakeDelivery {
                delivered: false,
                error: Some(format!("the initiator token is invalid: {error}")),
            },
        }
    };
    if !outcome.delivered {
        record_wake_failure(
            trace_dir,
            session,
            driver,
            event,
            outcome.error.as_deref().unwrap_or("delivery failed"),
        );
    }
    Ok(outcome)
}

fn emit_completed(
    out: &mut dyn Write,
    session: &str,
    marker: &str,
    screen: &str,
    autoclose: bool,
    delivery: &WakeDelivery,
) -> Result<()> {
    let mut event = serde_json::json!({
        "event": "completed",
        "session": session,
        "marker": marker,
        "screen": screen,
        "autoclose": autoclose,
        "delivered": delivery.delivered,
    });
    if let Some(error) = &delivery.error {
        event["wake_error"] = serde_json::json!(error);
    }
    emit(out, event)
}

fn emit_gone(out: &mut dyn Write, session: &str, delivery: &WakeDelivery) -> Result<()> {
    let mut event = serde_json::json!({
        "event": "gone",
        "session": session,
        "delivered": delivery.delivered,
    });
    if let Some(error) = &delivery.error {
        event["wake_error"] = serde_json::json!(error);
    }
    emit(out, event)
}

fn emit_stalled(out: &mut dyn Write, session: &str, delivery: &WakeDelivery) -> Result<()> {
    let mut event = serde_json::json!({
        "event": "stalled",
        "session": session,
        "delivered": delivery.delivered,
    });
    if let Some(error) = &delivery.error {
        event["wake_error"] = serde_json::json!(error);
    }
    emit(out, event)
}

fn emit(out: &mut dyn Write, line: serde_json::Value) -> Result<()> {
    writeln!(out, "{line}").context("failed to write watch event")?;
    out.flush().context("failed to flush watch event")
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use qol_terminal_sessions::cli::CliToolId;
    use qol_terminal_sessions::{
        BackendId, DeliveryMode, SessionCapabilities, SessionFacts, SessionFocus, SessionId,
        SpawnIdentity, SpawnKey, SpawnSurface, TerminalBackend, TerminalError, TerminalSnapshot,
        TextInput,
    };
    use sha2::{Digest, Sha256};

    use super::super::bridge::PendingBridgeStore;
    use super::super::spawn::{SpawnLedger, SpawnLocks, SpawnRecord};
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CallKind {
        Ls,
        Full,
    }

    struct FakeBackend {
        id: BackendId,
        facts: SessionFacts,
        driver_facts: Mutex<Option<SessionFacts>>,
        gone: AtomicBool,
        fail_send: AtomicBool,
        die_after: AtomicU64,
        read_count: AtomicU64,
        screens: Mutex<VecDeque<String>>,
        last: Mutex<Option<String>>,
        calls: Mutex<Vec<(CallKind, Instant)>>,
        sent: Mutex<Vec<(SessionBinding, String, DeliveryMode)>>,
        closed: Mutex<Vec<SessionBinding>>,
    }

    impl FakeBackend {
        fn new(facts: SessionFacts, screens: Vec<String>) -> Arc<Self> {
            Arc::new(Self {
                id: BackendId::new("fake").unwrap(),
                facts,
                driver_facts: Mutex::new(None),
                gone: AtomicBool::new(false),
                fail_send: AtomicBool::new(false),
                die_after: AtomicU64::new(u64::MAX),
                read_count: AtomicU64::new(0),
                screens: Mutex::new(screens.into()),
                last: Mutex::new(None),
                calls: Mutex::new(Vec::new()),
                sent: Mutex::new(Vec::new()),
                closed: Mutex::new(Vec::new()),
            })
        }

        fn mark_gone(&self) {
            self.gone.store(true, Ordering::Relaxed);
        }

        fn fail_sending(&self) {
            self.fail_send.store(true, Ordering::Relaxed);
        }

        fn with_driver(self: Arc<Self>, driver: SessionFacts) -> Arc<Self> {
            *self.driver_facts.lock().unwrap() = Some(driver);
            self
        }

        fn die_after_reads(self: Arc<Self>, reads: u64) -> Arc<Self> {
            self.die_after.store(reads, Ordering::Relaxed);
            self
        }

        fn lane_alive(&self) -> bool {
            self.read_count.load(Ordering::Relaxed) <= self.die_after.load(Ordering::Relaxed)
        }

        fn record(&self, kind: CallKind) {
            self.calls.lock().unwrap().push((kind, Instant::now()));
        }

        fn next_screen(&self) -> String {
            let mut screens = self.screens.lock().unwrap();
            if let Some(screen) = screens.pop_front() {
                *self.last.lock().unwrap() = Some(screen.clone());
                return screen;
            }
            self.last.lock().unwrap().clone().unwrap_or_default()
        }
    }

    impl SessionInventory for FakeBackend {
        fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
            self.record(CallKind::Ls);
            if self.gone.load(Ordering::Relaxed) || !self.lane_alive() {
                return Ok(self
                    .driver_facts
                    .lock()
                    .unwrap()
                    .clone()
                    .into_iter()
                    .collect());
            }
            let mut facts = vec![self.facts.clone()];
            if let Some(driver) = self.driver_facts.lock().unwrap().clone() {
                facts.push(driver);
            }
            Ok(facts)
        }
    }

    impl ScreenReader for FakeBackend {
        fn read_screen(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            self.record(CallKind::Ls);
            self.record(CallKind::Full);
            self.read_count.fetch_add(1, Ordering::Relaxed);
            if self.gone.load(Ordering::Relaxed) || !self.lane_alive() {
                return Err(TerminalError::TargetMissing(_target.clone()));
            }
            Ok(self.next_screen())
        }

        fn read_screen_relaxed(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            self.record(CallKind::Full);
            self.read_count.fetch_add(1, Ordering::Relaxed);
            if self.gone.load(Ordering::Relaxed) || !self.lane_alive() {
                return Err(TerminalError::TargetMissing(_target.clone()));
            }
            Ok(self.next_screen())
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
            self.sent
                .lock()
                .unwrap()
                .push((target.clone(), text.to_owned(), mode));
            if self.fail_send.load(Ordering::Relaxed) {
                return Err(TerminalError::TargetMissing(target.clone()));
            }
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
            Ok(self.next_screen())
        }

        fn id(&self) -> &BackendId {
            &self.id
        }

        fn closer(&self) -> Option<&dyn qol_terminal_sessions::SessionCloser> {
            Some(self)
        }
    }

    impl qol_terminal_sessions::SessionCloser for FakeBackend {
        fn close(&self, target: &SessionBinding) -> Result<(), TerminalError> {
            self.closed.lock().unwrap().push(target.clone());
            Ok(())
        }
    }

    fn facts(native: &str, root_pid: i32) -> SessionFacts {
        SessionFacts {
            id: SessionId::new(BackendId::new("fake").unwrap(), native).unwrap(),
            root_pid,
            cwd: "/work".to_owned(),
            title: "Terminal".to_owned(),
            at_prompt: true,
            reported_cmd: None,
            foreground_basenames: Vec::new(),
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: None,
        }
    }

    fn harness(backend: Arc<FakeBackend>) -> (TerminalSessionService, Arc<FakeBackend>) {
        let terminals = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();
        (terminals, backend)
    }

    fn store(root: &tempfile::TempDir) -> PendingBridgeStore {
        PendingBridgeStore::with_dir(root.path().to_path_buf())
    }

    fn locks(root: &tempfile::TempDir) -> SpawnLocks {
        SpawnLocks::with_dir(root.path().join("spawn-locks"))
    }

    fn ledger(root: &tempfile::TempDir) -> SpawnLedger {
        SpawnLedger::with_dir(root.path().join("spawn-records"))
    }

    fn fast_config(stall_after: Duration) -> WatchConfig {
        WatchConfig {
            poll_base: Duration::from_millis(1),
            poll_cap: Duration::from_millis(4),
            stall_after,
        }
    }

    fn lines(out: &[u8]) -> Vec<serde_json::Value> {
        String::from_utf8_lossy(out)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn completed_event_fires_and_the_checkpoint_completes() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        let backend = FakeBackend::new(
            facts("7", 100),
            vec![
                "idle".to_owned(),
                "idle".to_owned(),
                "done\nQOL_BRIDGE_DONE_round".to_owned(),
            ],
        );
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["session"], "v1:fake:7:100");
        assert_eq!(events[0]["marker"], "QOL_BRIDGE_DONE_round");
        assert_eq!(events[0]["screen"], "done\nQOL_BRIDGE_DONE_round");
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(round.completed);
    }

    #[test]
    fn completed_without_a_spawn_identity_leaves_no_ledger_record() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let ledger = ledger(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        let backend = FakeBackend::new(
            facts("7", 100),
            vec![
                "idle".to_owned(),
                "idle".to_owned(),
                "done\nQOL_BRIDGE_DONE_round".to_owned(),
            ],
        );
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger,
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "completed");
        let ledger_dir = root.path().join("spawn-records");
        assert!(
            !ledger_dir.exists() || std::fs::read_dir(&ledger_dir).unwrap().next().is_none(),
            "a completed lane without a spawn identity must not write a spawn record"
        );
    }

    #[test]
    fn lane_gone_before_completion_still_records_the_external_id() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let ledger = ledger(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        let mut facts = facts("7", 100);
        facts.spawn_identity = Some(SpawnIdentity {
            key: SpawnKey::new("lane-gone-early").unwrap(),
            tool: CliToolId::new("pi").unwrap(),
            surface: SpawnSurface::Tab,
        });
        facts.foreground_basenames = vec!["pi".to_owned()];
        facts.foreground_pids = vec![424242];
        let backend =
            FakeBackend::new(facts, vec!["idle".to_owned(), "idle".to_owned()]).die_after_reads(2);
        let (terminals, _) = harness(backend);
        let session_dir = tempfile::TempDir::new().unwrap();
        let encoded_dir = session_dir.path().join("--work--");
        std::fs::create_dir_all(&encoded_dir).unwrap();
        std::fs::write(
            encoded_dir.join("2026-08-16T10-00-00-000Z_0000cafe-cafe-cafe-cafe-cafecafecafe.jsonl"),
            "",
        )
        .unwrap();
        let previous = std::env::var_os("PI_CODING_AGENT_SESSION_DIR");
        std::env::set_var("PI_CODING_AGENT_SESSION_DIR", session_dir.path());
        let mut out = Vec::new();
        let result = watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger,
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        );
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_SESSION_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_SESSION_DIR"),
        }
        result.unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "gone");
        let record_dir = root.path().join("spawn-records");
        let record_paths = std::fs::read_dir(&record_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(record_paths.len(), 1);
        let record: SpawnRecord =
            serde_json::from_str(&std::fs::read_to_string(&record_paths[0]).unwrap()).unwrap();
        assert_eq!(
            record.external_id.as_deref(),
            Some("0000cafe-cafe-cafe-cafe-cafecafecafe")
        );
    }

    #[test]
    fn completed_wake_text_never_instructs_the_driver() {
        let closable = wake_message("v1:kitty:5:100", "completed", "done", true);
        assert!(
            !closable.contains("session_loop_close") && !closable.contains("Review it"),
            "the closable wake must not instruct the driver: {closable:?}"
        );
        assert!(closable.starts_with(
            "qol sessions: lane v1:kitty:5:100 completed and the lane terminal closed. Report below."
        ));
        assert!(closable.ends_with("\n\ndone"));

        let plain = wake_message("v1:kitty:5:100", "completed", "done", false);
        assert!(
            !plain.contains("session_loop_close") && !plain.contains("Review it"),
            "the plain wake must not instruct the driver: {plain:?}"
        );
        assert!(plain.starts_with("qol sessions: v1:kitty:5:100 completed. Report below."));
        assert!(plain.ends_with("\n\ndone"));
    }

    #[test]
    fn autoclose_round_closes_the_lane_terminal_after_completed_and_plain_rounds_stay_open() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);

        let auto_binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&auto_binding, "QOL_BRIDGE_DONE_auto", "v1:fake:7:100", true)
            .unwrap();
        let auto_backend = FakeBackend::new(
            facts("7", 100),
            vec!["done   \n\n\nQOL_BRIDGE_DONE_auto  ".to_owned(); 2],
        );
        let (terminals, auto_backend) = harness(auto_backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["autoclose"], true);
        assert_eq!(events[0]["delivered"], true);
        assert_eq!(
            events[0]["screen"], "done   \n\n\nQOL_BRIDGE_DONE_auto  ",
            "the event screen must stay raw"
        );
        let sent = auto_backend.sent.lock().unwrap();
        assert_eq!(
            sent.len(),
            1,
            "the wake must be typed into the initiator terminal"
        );
        let (driver, text, mode) = &sent[0];
        assert_eq!(driver.token(), auto_binding.token());
        assert_eq!(*mode, DeliveryMode::Submit);
        assert!(text.contains("qol sessions: lane v1:fake:7:100 completed and the lane terminal closed. Report below."));
        assert!(text.contains("QOL_BRIDGE_DONE_auto"));
        assert!(
            !text.contains("session_loop_close") && !text.contains("Review it"),
            "the completed wake must not instruct the driver: {text:?}"
        );
        assert!(
            text.lines().all(|line| line == line.trim_end()),
            "the wake text must not carry trailing padding: {text:?}"
        );
        let text_lines = text.lines().collect::<Vec<_>>();
        assert!(
            !text_lines
                .windows(2)
                .any(|pair| pair[0].trim().is_empty() && pair[1].trim().is_empty()),
            "the wake text must not contain consecutive blank lines: {text:?}"
        );
        drop(sent);
        let closed = auto_backend.closed.lock().unwrap();
        assert_eq!(
            closed.as_slice(),
            &[auto_binding],
            "an autoclose round must close the lane terminal after delivery"
        );
        drop(closed);

        let plain_binding: SessionBinding = "v1:fake:8:200".parse().unwrap();
        pending
            .start(
                &plain_binding,
                "QOL_BRIDGE_DONE_plain",
                "v1:fake:8:200",
                false,
            )
            .unwrap();
        let plain_backend = FakeBackend::new(
            facts("8", 200),
            vec!["done\nQOL_BRIDGE_DONE_plain".to_owned(); 2],
        );
        let (terminals, plain_backend) = harness(plain_backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:8:200".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["autoclose"], false);
        assert_eq!(events[0]["delivered"], true);
        assert!(
            plain_backend.closed.lock().unwrap().is_empty(),
            "a plain round must never close its terminal"
        );
    }

    #[test]
    fn autoclose_is_skipped_when_the_wake_cannot_be_delivered_and_a_trace_is_left() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:7:100", true)
            .unwrap();
        let backend = FakeBackend::new(
            facts("7", 100),
            vec!["done\nQOL_BRIDGE_DONE_round".to_owned(); 2],
        );
        backend.fail_sending();
        let (terminals, backend) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["delivered"], false);
        assert!(
            events[0]["wake_error"].as_str().is_some(),
            "an undeliverable wake must name its error in the event"
        );
        assert!(
            backend.closed.lock().unwrap().is_empty(),
            "the lane must stay open when the wake could not be delivered"
        );
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(round.completed, "the round still completed");
        let trace = root.path().join("wake-failed-v1_fake_7_100.json");
        assert!(trace.exists(), "a failed wake must leave a durable trace");
        let record: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(trace).unwrap()).unwrap();
        assert_eq!(record["event"], "completed");
        assert_eq!(record["session"], "v1:fake:7:100");
    }

    #[test]
    fn a_claimed_wake_keeps_a_second_watcher_silent() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:7:100", false)
            .unwrap();
        assert!(pending.claim_wake(&binding, "completed").unwrap());
        let backend = FakeBackend::new(
            facts("7", 100),
            vec!["done\nQOL_BRIDGE_DONE_round".to_owned(); 2],
        );
        let (terminals, backend) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        assert!(
            out.is_empty(),
            "a watcher whose wake was already claimed must stay silent: {out:?}"
        );
        assert!(
            backend.sent.lock().unwrap().is_empty(),
            "no second delivery may happen"
        );
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(
            !round.completed,
            "the claiming watcher owns the observation"
        );
    }

    #[test]
    fn lane_exiting_after_the_marker_completes_with_the_last_screen_instead_of_going_gone() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:200", true)
            .unwrap();
        let final_screen = "done\nQOL_BRIDGE_DONE_round".to_owned();
        let backend = FakeBackend::new(facts("7", 100), vec![final_screen.clone()])
            .with_driver(facts("8", 200))
            .die_after_reads(1);
        let (terminals, backend) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(
            events[0]["event"], "completed",
            "a marker seen before the lane window closed must not be reported as gone"
        );
        assert_eq!(events[0]["screen"], final_screen);
        assert_eq!(events[0]["delivered"], true);
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(round.completed, "the checkpoint must stay collectable");
        assert!(
            backend.closed.lock().unwrap().is_empty(),
            "an already-closed lane needs no autoclose"
        );
    }

    #[test]
    fn completion_stores_the_screen_tail_and_keeps_the_checkpoint() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        let final_screen = format!("{}\ndone\nQOL_BRIDGE_DONE_round", "x".repeat(70 * 1024));
        let backend = FakeBackend::new(
            facts("7", 100),
            vec!["idle".to_owned(), final_screen.clone()],
        );
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["screen"], screen_tail(&final_screen));
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(
            round.completed,
            "the completed checkpoint must stay collectable"
        );
        let stored = round.screen.unwrap();
        assert_eq!(stored, screen_tail(&final_screen));
        assert_eq!(stored.len(), 64 * 1024);
        assert!(stored.ends_with("QOL_BRIDGE_DONE_round"));
    }

    #[test]
    fn clean_screen_strips_padding_and_collapses_blank_runs() {
        assert_eq!(
            clean_screen("\n\n   line one   \n\n\n\n   line two  \n\n   \n\n"),
            "   line one\n\n   line two"
        );
        assert_eq!(clean_screen("no padding"), "no padding");
        assert_eq!(clean_screen("a\n\n\n\nb"), "a\n\nb");
        assert_eq!(clean_screen(""), "");
        assert_eq!(clean_screen("   \n\n"), "");
    }

    #[test]
    fn composer_busy_detects_an_in_progress_prompt_and_ignores_bare_prompts() {
        let cases: &[(&str, bool)] = &[
            ("> fixing the parser", true),
            ("> fixing the parser\nidle output", true),
            ("> draft\nstatus: running", true),
            ("❯ fixing the parser", true),
            ("   > fixing the parser", true),
            ("> old message\n> ", false),
            ("> old message\n> draft", true),
            ("❯ old message\n❯ ", false),
            (">", false),
            ("> ", false),
            (">  ", false),
            ("❯", false),
            ("❯ ", false),
            ("just output, no prompt", false),
            ("", false),
        ];
        for (screen, expected) in cases {
            assert_eq!(composer_busy(screen), *expected, "screen: {screen:?}");
        }
    }

    #[test]
    fn composer_draft_region_spans_from_the_prompt_line_to_the_end() {
        assert_eq!(
            composer_draft_region("> old message\n❯ draft\nstatus: running  ").as_deref(),
            Some("❯ draft\nstatus: running")
        );
        assert_eq!(composer_draft_region("just output, no prompt"), None);
    }

    #[test]
    fn screen_tail_caps_at_64_kib_and_keeps_the_char_boundary() {
        let short = "a short screen";
        assert_eq!(screen_tail(short), short);

        let screen = format!("{}abc", "€".repeat(30_000));
        let trimmed = screen_tail(&screen);
        assert!(trimmed.len() <= 64 * 1024);
        assert!(trimmed.len() < screen.len());
        assert!(trimmed.starts_with('€'));
        assert!(screen.ends_with(trimmed));
        assert!(trimmed.ends_with("abc"));
    }

    #[test]
    fn gone_event_fires_and_the_checkpoint_is_discarded() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        let backend = FakeBackend::new(facts("7", 100), Vec::new());
        backend.mark_gone();
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();

        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "gone");
        assert_eq!(events[0]["session"], "v1:fake:7:100");
        assert!(pending.pending_round(&binding).unwrap().is_none());
    }

    #[test]
    fn stale_tokens_with_gone_terminals_and_no_open_round_are_pruned_without_events() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let absent_binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        let collected_binding: SessionBinding = "v1:fake:8:200".parse().unwrap();
        pending
            .start(
                &collected_binding,
                "QOL_BRIDGE_DONE_collected",
                "v1:fake:9:900",
                false,
            )
            .unwrap();
        pending
            .observe(&collected_binding, "QOL_BRIDGE_DONE_collected", true)
            .unwrap();
        let backend = FakeBackend::new(facts("9", 900), Vec::new());
        backend.mark_gone();
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &[
                absent_binding.token().to_owned(),
                collected_binding.token().to_owned(),
            ],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        assert!(
            out.is_empty(),
            "a stale token must not wake its owner: {out:?}"
        );
        let round = pending.pending_round(&collected_binding).unwrap().unwrap();
        assert!(
            round.completed,
            "the collected checkpoint must not be discarded by the prune"
        );
    }

    #[test]
    fn stale_token_is_pruned_while_an_open_round_still_wakes() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let stale_binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(
                &stale_binding,
                "QOL_BRIDGE_DONE_stale",
                "v1:fake:8:800",
                false,
            )
            .unwrap();
        pending
            .observe(&stale_binding, "QOL_BRIDGE_DONE_stale", true)
            .unwrap();
        let live_binding: SessionBinding = "v1:fake:8:200".parse().unwrap();
        pending
            .start(
                &live_binding,
                "QOL_BRIDGE_DONE_live",
                "v1:fake:8:800",
                false,
            )
            .unwrap();
        let backend = FakeBackend::new(
            facts("8", 200),
            vec!["idle".to_owned(), "done\nQOL_BRIDGE_DONE_live".to_owned()],
        );
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned(), "v1:fake:8:200".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(
            events.len(),
            1,
            "only the live open round may wake: {events:?}"
        );
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["session"], "v1:fake:8:200");
    }

    #[test]
    fn pruning_everything_stays_in_explicit_mode_and_ignores_foreign_rounds() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let stale_binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        let foreign_binding: SessionBinding = "v1:fake:8:200".parse().unwrap();
        pending
            .start(
                &foreign_binding,
                "QOL_BRIDGE_DONE_foreign",
                "v1:fake:9:900",
                false,
            )
            .unwrap();
        let backend = FakeBackend::new(facts("9", 900), Vec::new());
        backend.mark_gone();
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &[stale_binding.token().to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        assert!(
            out.is_empty(),
            "a pruned explicit watch must not flip to all-rounds: {out:?}"
        );
        let round = pending.pending_round(&foreign_binding).unwrap().unwrap();
        assert!(
            !round.completed,
            "the foreign round must stay untouched by an explicit watch"
        );
    }

    #[test]
    fn live_terminal_without_an_open_round_stays_unwatched() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        let backend = FakeBackend::new(facts("7", 100), vec!["idle".to_owned()]);
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &[binding.token().to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        assert!(
            out.is_empty(),
            "a token without an open round must never be watched: {out:?}"
        );
    }

    #[test]
    fn stalled_fires_exactly_once_per_round() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        let screens = (0..64)
            .map(|_| "idle".to_owned())
            .chain(["done\nQOL_BRIDGE_DONE_round".to_owned()])
            .collect();
        let backend = FakeBackend::new(facts("7", 100), screens);
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_millis(5)),
        )
        .unwrap();

        let events = lines(&out);
        let stalled = events
            .iter()
            .filter(|event| event["event"] == "stalled")
            .collect::<Vec<_>>();
        assert_eq!(stalled.len(), 1, "events: {events:?}");
        assert_eq!(stalled[0]["session"], "v1:fake:7:100");
        let completed = events
            .iter()
            .filter(|event| event["event"] == "completed")
            .collect::<Vec<_>>();
        assert_eq!(completed.len(), 1, "events: {events:?}");
    }

    #[test]
    fn no_change_polls_grow_the_sleep_and_a_change_resets_it() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        let screens = [
            "idle".to_owned(),
            "idle".to_owned(),
            "idle".to_owned(),
            "idle".to_owned(),
            "idle".to_owned(),
            "changed".to_owned(),
            "done\nQOL_BRIDGE_DONE_round".to_owned(),
        ]
        .into_iter()
        .collect();
        let backend = FakeBackend::new(facts("7", 100), screens);
        let (terminals, _backend) = harness(backend);
        let mut out = Vec::new();

        let mut requested = Vec::new();
        watch_loop(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            WatchConfig {
                poll_base: Duration::from_millis(30),
                poll_cap: Duration::from_millis(120),
                stall_after: Duration::from_secs(3600),
            },
            &mut |duration| requested.push(duration),
        )
        .unwrap();

        assert_eq!(
            requested,
            vec![
                Duration::from_millis(30),
                Duration::from_millis(30),
                Duration::from_millis(60),
                Duration::from_millis(120),
                Duration::from_millis(120),
                Duration::from_millis(30),
                Duration::from_millis(30),
                Duration::from_millis(30),
            ]
        );
    }

    #[test]
    fn watch_exits_zero_when_nothing_is_pending() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let (terminals, _) = harness(FakeBackend::new(facts("7", 100), Vec::new()));
        let mut out = Vec::new();

        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &[],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        assert!(out.is_empty());

        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn watch_cycle_reads_relaxed_with_a_strict_read_every_tenth_poll() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        let screens = (0..25)
            .map(|_| "idle".to_owned())
            .chain(["done\nQOL_BRIDGE_DONE_round".to_owned()])
            .collect();
        let backend = FakeBackend::new(facts("7", 100), screens);
        let (terminals, backend) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let calls = backend.calls.lock().unwrap();
        let full = calls
            .iter()
            .filter(|(kind, _)| *kind == CallKind::Full)
            .count();
        let ls = calls
            .iter()
            .filter(|(kind, _)| *kind == CallKind::Ls)
            .count();
        assert_eq!(full, 27);
        assert_eq!(ls, 31);
        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "completed");
    }

    #[test]
    fn completed_fires_on_the_second_consecutive_marker_poll() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:7:100", false)
            .unwrap();
        let backend = FakeBackend::new(
            facts("7", 100),
            vec![
                "done\nQOL_BRIDGE_DONE_round".to_owned(),
                "done\nQOL_BRIDGE_DONE_round".to_owned(),
            ],
        );
        let (terminals, backend) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["session"], "v1:fake:7:100");
        assert_eq!(events[0]["marker"], "QOL_BRIDGE_DONE_round");
        assert_eq!(events[0]["screen"], "done\nQOL_BRIDGE_DONE_round");
        let calls = backend.calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            7,
            "the first marker poll must not emit; the second confirms, then each successful poll's early external-id capture, the completion capture and the delivery re-check each add a discovery"
        );
        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].1.contains("QOL_BRIDGE_DONE_round"));
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(round.completed);
    }

    #[test]
    fn wake_delivers_immediately_when_the_composer_clears() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:7:100", false)
            .unwrap();
        let backend = FakeBackend::new(
            facts("7", 100),
            vec![
                "done\nQOL_BRIDGE_DONE_round".to_owned(),
                "done\nQOL_BRIDGE_DONE_round".to_owned(),
                "> fixing the parser".to_owned(),
                "> fixing the parser".to_owned(),
                "idle".to_owned(),
            ],
        );
        let (terminals, backend) = harness(backend);
        let mut out = Vec::new();
        let mut sleeps = Vec::new();
        watch_loop(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &[binding.token().to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
            &mut |duration| sleeps.push(duration),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["delivered"], true);
        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "the wake must be sent exactly once");
        assert!(
            sent[0].1.contains("QOL_BRIDGE_DONE_round"),
            "the wake must carry the report tail"
        );
        drop(sent);
        let deferrals = sleeps
            .iter()
            .filter(|duration| **duration == WAKE_COMPOSER_POLL_INTERVAL)
            .count();
        assert_eq!(
            deferrals, 2,
            "two busy reads must defer the wake twice: {sleeps:?}"
        );
    }

    #[test]
    fn wake_delivers_when_the_draft_region_stops_changing() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:7:100", false)
            .unwrap();
        let backend = FakeBackend::new(
            facts("7", 100),
            vec![
                "done\nQOL_BRIDGE_DONE_round".to_owned(),
                "done\nQOL_BRIDGE_DONE_round".to_owned(),
                "> draft".to_owned(),
            ],
        );
        let (terminals, backend) = harness(backend);
        let mut out = Vec::new();
        let mut sleeps = Vec::new();
        watch_loop(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &[binding.token().to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
            &mut |duration| sleeps.push(duration),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["delivered"], true);
        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "the wake must be sent exactly once");
        drop(sent);
        let deferrals = sleeps
            .iter()
            .filter(|duration| **duration == WAKE_COMPOSER_POLL_INTERVAL)
            .count();
        assert_eq!(
            deferrals, WAKE_COMPOSER_STATIC_POLLS,
            "an unchanged draft region must stop deferring after the static budget: {sleeps:?}"
        );
    }

    #[test]
    fn wake_defers_while_the_draft_region_keeps_changing() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:7:100", false)
            .unwrap();
        let mut screens = vec![
            "done\nQOL_BRIDGE_DONE_round".to_owned(),
            "done\nQOL_BRIDGE_DONE_round".to_owned(),
        ];
        screens
            .extend((1..=WAKE_COMPOSER_MAX_ATTEMPTS + 1).map(|i| format!("> {}", "a".repeat(i))));
        let backend = FakeBackend::new(facts("7", 100), screens);
        let (terminals, backend) = harness(backend);
        let mut out = Vec::new();
        let mut sleeps = Vec::new();
        watch_loop(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &[binding.token().to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
            &mut |duration| sleeps.push(duration),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["delivered"], true);
        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "the wake must be sent exactly once");
        drop(sent);
        let deferrals = sleeps
            .iter()
            .filter(|duration| **duration == WAKE_COMPOSER_POLL_INTERVAL)
            .count();
        assert_eq!(
            deferrals, WAKE_COMPOSER_MAX_ATTEMPTS,
            "a draft region that keeps changing must exhaust the attempt budget: {sleeps:?}"
        );
    }

    #[test]
    fn wake_static_counter_resets_when_typing_resumes() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:7:100", false)
            .unwrap();
        let identical_before_change = 10usize;
        let mut screens = vec![
            "done\nQOL_BRIDGE_DONE_round".to_owned(),
            "done\nQOL_BRIDGE_DONE_round".to_owned(),
        ];
        screens.extend((0..=identical_before_change).map(|_| "> draft".to_owned()));
        screens.push("> draft edited".to_owned());
        let backend = FakeBackend::new(facts("7", 100), screens);
        let (terminals, backend) = harness(backend);
        let mut out = Vec::new();
        let mut sleeps = Vec::new();
        watch_loop(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &[binding.token().to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
            &mut |duration| sleeps.push(duration),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["delivered"], true);
        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "the wake must be sent exactly once");
        drop(sent);
        let deferrals = sleeps
            .iter()
            .filter(|duration| **duration == WAKE_COMPOSER_POLL_INTERVAL)
            .count();
        assert_eq!(
            deferrals,
            identical_before_change + 1 + WAKE_COMPOSER_STATIC_POLLS,
            "resumed typing must restart the static budget from zero: {sleeps:?}"
        );
    }

    #[test]
    fn completed_checkpoint_at_detection_stays_silent() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        pending
            .observe(&binding, "QOL_BRIDGE_DONE_round", true)
            .unwrap();
        let backend = FakeBackend::new(
            facts("7", 100),
            vec!["done\nQOL_BRIDGE_DONE_round".to_owned()],
        );
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        assert!(out.is_empty(), "a collected round must not be re-announced");
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(round.completed);
    }

    #[test]
    fn bridge_completion_between_polls_stays_silent() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = Arc::new(store(&root));
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        let backend = FakeBackend::new(
            facts("7", 100),
            vec!["done\nQOL_BRIDGE_DONE_round".to_owned()],
        );
        let (terminals, backend) = harness(backend);
        let out = Arc::new(Mutex::new(Vec::new()));
        let thread_pending = Arc::clone(&pending);
        let thread_terminals = terminals;
        let thread_out = Arc::clone(&out);
        let handle = std::thread::spawn(move || {
            let thread_locks = SpawnLocks::with_dir(root.path().join("spawn-locks"));
            let thread_ledger = SpawnLedger::with_dir(root.path().join("spawn-records"));
            let thread_interpreter = CliSessionInterpreter::system();
            let thread_trace = root.path().to_path_buf();
            watch(
                &thread_terminals,
                &thread_interpreter,
                &thread_pending,
                &thread_ledger,
                &thread_locks,
                &["v1:fake:7:100".to_owned()],
                &mut *thread_out.lock().unwrap(),
                &thread_trace,
                WatchConfig {
                    poll_base: Duration::from_millis(50),
                    poll_cap: Duration::from_millis(50),
                    stall_after: Duration::from_secs(3600),
                },
            )
            .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if !backend.calls.lock().unwrap().is_empty() {
                break;
            }
            assert!(Instant::now() < deadline, "watcher never polled");
            std::thread::sleep(Duration::from_millis(2));
        }
        pending
            .observe(&binding, "QOL_BRIDGE_DONE_round", true)
            .unwrap();
        handle.join().unwrap();

        let events = lines(&out.lock().unwrap());
        assert!(
            events.is_empty(),
            "a bridge that collected the round must win: {events:?}"
        );
    }

    #[test]
    fn explicit_tokens_limit_the_watched_scope() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let watched_binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        let other_binding: SessionBinding = "v1:fake:8:200".parse().unwrap();
        pending
            .start(
                &watched_binding,
                "QOL_BRIDGE_DONE_watched",
                "v1:fake:9:900",
                false,
            )
            .unwrap();
        pending
            .start(
                &other_binding,
                "QOL_BRIDGE_DONE_other",
                "v1:fake:9:900",
                false,
            )
            .unwrap();
        let backend = FakeBackend::new(
            facts("7", 100),
            vec!["done\nQOL_BRIDGE_DONE_watched".to_owned()],
        );
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["session"], "v1:fake:7:100");
        assert!(
            pending.pending_round(&other_binding).unwrap().is_some(),
            "untouched checkpoints stay pending"
        );
    }

    #[test]
    fn all_rounds_mode_takes_the_watch_lock_so_one_watcher_polls() {
        let root = tempfile::TempDir::new().unwrap();
        let locks = locks(&root);
        let key = qol_terminal_sessions::SpawnKey::new(WATCH_ALL_KEY).unwrap();
        let guard = locks.acquire(&key).unwrap();

        let error = watch(
            &TerminalSessionService::system(),
            &CliSessionInterpreter::system(),
            &store(&root),
            &ledger(&root),
            &locks,
            &[],
            &mut Vec::new(),
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("another qol sessions watch is already running"),
            "{error}"
        );
        drop(guard);
    }

    #[test]
    fn all_rounds_mode_removes_the_watch_lock_on_normal_and_error_exit() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let locks = locks(&root);
        let key = qol_terminal_sessions::SpawnKey::new(WATCH_ALL_KEY).unwrap();
        let digest = Sha256::digest(key.as_str().as_bytes());
        let lock_path = root
            .path()
            .join("spawn-locks")
            .join(format!("{digest:x}.lock"));
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();

        pending
            .start(&binding, "QOL_BRIDGE_DONE_error", "", false)
            .unwrap();
        let (terminals, _) = harness(FakeBackend::new(
            facts("7", 100),
            vec!["done\nQOL_BRIDGE_DONE_error".to_owned()],
        ));
        let error = watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks,
            &[],
            &mut FailingWriter,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("failed to write"), "{error}");
        assert!(
            !lock_path.exists(),
            "an erroring watch must leave no lock file behind"
        );

        pending.discard(&binding).unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_done", "", false)
            .unwrap();
        let (terminals, _) = harness(FakeBackend::new(
            facts("7", 100),
            vec!["done\nQOL_BRIDGE_DONE_done".to_owned()],
        ));
        let mut out = Vec::new();
        watch(
            &terminals,
            &CliSessionInterpreter::system(),
            &pending,
            &ledger(&root),
            &locks,
            &[],
            &mut out,
            root.path(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();
        assert!(
            !lock_path.exists(),
            "a completed watch must leave no lock file behind"
        );
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("watch write failed"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
