use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use qol_terminal_sessions::{
    DeliveryMode, ScreenReader, SessionBinding, SessionInventory, TerminalSessionService, TextInput,
};
use serde::{Deserialize, Serialize};

use super::bridge::{PendingBridgeStore, PendingRound};
use super::spawn::{SpawnLedger, SpawnLocks};

use qol_terminal_sessions::cli::{CliRuntimeState, CliSessionInterpreter};

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

#[cfg(test)]
impl WatchConfig {
    pub(super) fn fast_for_tests() -> Self {
        Self {
            poll_base: Duration::from_millis(1),
            poll_cap: Duration::from_millis(4),
            stall_after: Duration::from_secs(3600),
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
    marker_seen: bool,
    runtime_working_seen: bool,
    ready_polls: u32,
    external_id_captured: bool,
    autoclose: bool,
    group: Option<String>,
    label: Option<String>,
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
            marker_seen: false,
            runtime_working_seen: false,
            ready_polls: 0,
            external_id_captured: false,
            autoclose: round.autoclose,
            group: round.group,
            label: round.label,
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
    released: bool,
}

impl RoundPoll {
    fn of(keep: bool, changed: bool) -> Self {
        Self {
            keep,
            changed,
            released: false,
        }
    }
}

fn completion_event_base(markerless: bool) -> &'static str {
    if markerless {
        "completed_markerless"
    } else {
        "completed"
    }
}

#[allow(clippy::too_many_arguments)]
fn complete_seen_round(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    pending: &PendingBridgeStore,
    ledger: &SpawnLedger,
    locks: &SpawnLocks,
    round: &mut WatchedRound,
    out: &mut dyn Write,
    trace_dir: &std::path::Path,
    sleep: &mut dyn FnMut(Duration),
    screen: &str,
    wake_msg: &str,
    markerless: bool,
) -> Result<RoundPoll> {
    if !round.external_id_captured {
        round.external_id_captured = super::spawn::capture_lane_external_id(
            terminals,
            interpreter,
            ledger,
            locks,
            &round.binding,
        );
    }
    let base = completion_event_base(markerless);
    let tail = screen_tail(screen);
    if let Some(group) = round.group.as_deref() {
        pending.observe(&round.binding, &round.marker, true)?;
        pending.store_screen(&round.binding, &round.marker, tail)?;
        write_group_fragment(
            trace_dir,
            group,
            &round.session,
            tail,
            round.label.as_deref(),
        )?;
        settle_group_member(
            trace_dir,
            group,
            &round.session,
            round.label.as_deref(),
            GroupOutcome::Completed,
        )?;
        if let Some((combined, delivery)) = maybe_deliver_group_combined(
            pending,
            terminals,
            trace_dir,
            group,
            &round.session,
            &round.driver,
            sleep,
        )? {
            if round.autoclose {
                close_lane_terminal(terminals, &round.binding);
                pending.close_checkpoints_for_session(&round.session)?;
                qol_runtime::probe!(
                    "CLI_SESSION_WATCH",
                    "event=checkpoint_closed session={} autoclose=true",
                    round.session
                );
            }
            qol_runtime::probe!(
                "CLI_SESSION_WATCH",
                "event={}_group session={} group={} reads={} delivered={} autoclose={}",
                base,
                round.session,
                group,
                round.reads,
                delivery.delivered,
                round.autoclose
            );
            emit_completed(
                out,
                &round.session,
                &round.marker,
                &combined,
                round.autoclose,
                &delivery,
                markerless,
            )?;
            return Ok(RoundPoll::of(false, true));
        }
        if round.autoclose {
            close_lane_terminal(terminals, &round.binding);
            pending.close_checkpoints_for_session(&round.session)?;
        }
        qol_runtime::probe!(
            "CLI_SESSION_WATCH",
            "event={}_group_awaiting group={} session={} reads={}",
            base,
            group,
            round.session,
            round.reads
        );
        return Ok(RoundPoll::of(false, true));
    }
    if !pending.claim_wake(&round.binding, "completed")? {
        qol_runtime::probe!(
            "CLI_SESSION_WATCH",
            "event=wake_refused session={} kind=completed",
            round.session
        );
        return Ok(RoundPoll {
            keep: false,
            changed: false,
            released: true,
        });
    }
    let delivery = deliver_wake(
        terminals,
        trace_dir,
        &round.session,
        &round.driver,
        "completed",
        wake_msg,
        sleep,
    )?;
    if round.autoclose && delivery.delivered {
        close_lane_terminal(terminals, &round.binding);
        pending.close_checkpoints_for_session(&round.session)?;
        qol_runtime::probe!(
            "CLI_SESSION_WATCH",
            "event=checkpoint_closed session={} autoclose=true",
            round.session
        );
    }
    pending.observe(&round.binding, &round.marker, true)?;
    pending.store_screen(&round.binding, &round.marker, tail)?;
    qol_runtime::probe!(
        "CLI_SESSION_WATCH",
        "event={} session={} reads={} delivered={} autoclose={}",
        base,
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
        markerless,
    )?;
    Ok(RoundPoll::of(false, true))
}

fn transcript_denies_marker(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
    marker: &str,
) -> bool {
    let Ok(sessions) = terminals.discover() else {
        return false;
    };
    let Some(facts) = sessions
        .into_iter()
        .find(|facts| facts.id == *binding.session_id())
    else {
        return false;
    };
    if interpreter.transcript_completion(&facts, marker) != Some(false) {
        return false;
    }
    qol_runtime::probe!(
        "CLI_SESSION_WATCH",
        "event=marker_denied_by_transcript session={}",
        binding.token()
    );
    true
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
                    let last = round.last_screen.clone().unwrap_or_default();
                    let tail = screen_tail(&last);
                    let full_screen: &str = &last;
                    if let Some(group) = round.group.as_deref() {
                        pending.observe(&round.binding, &round.marker, true)?;
                        pending.store_screen(&round.binding, &round.marker, tail)?;
                        write_group_fragment(
                            trace_dir,
                            group,
                            &round.session,
                            tail,
                            round.label.as_deref(),
                        )?;
                        settle_group_member(
                            trace_dir,
                            group,
                            &round.session,
                            round.label.as_deref(),
                            GroupOutcome::Completed,
                        )?;
                        if let Some((combined, delivery)) = maybe_deliver_group_combined(
                            pending,
                            terminals,
                            trace_dir,
                            group,
                            &round.session,
                            &round.driver,
                            sleep,
                        )? {
                            qol_runtime::probe!(
                                "CLI_SESSION_WATCH",
                                "event=completed_group_after_exit group={} session={} members={} delivered={}",
                                group,
                                round.session,
                                group_roster(pending, trace_dir, group)?.len(),
                                delivery.delivered
                            );
                            emit_completed(
                                out,
                                &round.session,
                                &round.marker,
                                &combined,
                                round.autoclose,
                                &delivery,
                                false,
                            )?;
                            return Ok(RoundPoll::of(false, true));
                        }
                        qol_runtime::probe!(
                            "CLI_SESSION_WATCH",
                            "event=completed_group_awaiting_after_exit group={} session={}",
                            group,
                            round.session
                        );
                        return Ok(RoundPoll::of(false, true));
                    }
                    if !pending.claim_wake(&round.binding, "completed")? {
                        qol_runtime::probe!(
                            "CLI_SESSION_WATCH",
                            "event=wake_refused session={} kind=completed",
                            round.session
                        );
                        return Ok(RoundPoll {
                            keep: false,
                            changed: false,
                            released: true,
                        });
                    }
                    let delivery = deliver_wake(
                        terminals,
                        trace_dir,
                        &round.session,
                        &round.driver,
                        "completed",
                        &wake_message(
                            trace_dir,
                            &round.session,
                            "completed",
                            full_screen,
                            &round.marker,
                            round.autoclose,
                            round.label.as_deref(),
                        ),
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
                        false,
                    )?;
                    return Ok(RoundPoll::of(false, true));
                }
                if let Some(group) = round.group.as_deref() {
                    let last = round.last_screen.clone().unwrap_or_default();
                    write_group_fragment(
                        trace_dir,
                        group,
                        &round.session,
                        &format!(
                            "the lane terminal exited before it reported a completion marker\n\n{}",
                            clean_screen(screen_tail(&last))
                        ),
                        round.label.as_deref(),
                    )?;
                    settle_group_member(
                        trace_dir,
                        group,
                        &round.session,
                        round.label.as_deref(),
                        GroupOutcome::Gone,
                    )?;
                    let combined = maybe_deliver_group_combined(
                        pending,
                        terminals,
                        trace_dir,
                        group,
                        &round.session,
                        &round.driver,
                        sleep,
                    )?;
                    pending.discard(&round.binding)?;
                    match combined {
                        Some((combined, delivery)) => {
                            qol_runtime::probe!(
                                "CLI_SESSION_WATCH",
                                "event=gone_group_completed group={} session={} delivered={}",
                                group,
                                round.session,
                                delivery.delivered
                            );
                            emit_completed(
                                out,
                                &round.session,
                                &round.marker,
                                &combined,
                                round.autoclose,
                                &delivery,
                                true,
                            )?;
                        }
                        None => {
                            qol_runtime::probe!(
                                "CLI_SESSION_WATCH",
                                "event=gone_group_awaiting group={} session={} reads={}",
                                group,
                                round.session,
                                round.reads
                            );
                        }
                    }
                    return Ok(RoundPoll::of(false, true));
                }
                if !pending.claim_wake(&round.binding, "gone")? {
                    return Ok(RoundPoll::of(false, false));
                }
                let delivery = deliver_wake(
                    terminals,
                    trace_dir,
                    &round.session,
                    &round.driver,
                    "gone",
                    &wake_message(
                        trace_dir,
                        &round.session,
                        "gone",
                        "",
                        "",
                        false,
                        round.label.as_deref(),
                    ),
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
                return Ok(RoundPoll::of(false, true));
            }
            return Ok(RoundPoll::of(true, false));
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
    let marker_visible = super::marker_present(&screen, &round.marker)
        && !transcript_denies_marker(terminals, interpreter, &round.binding, &round.marker);
    if marker_visible {
        match pending.pending_round(&round.binding)? {
            None => {
                return Ok(RoundPoll::of(false, false));
            }
            Some(current) if current.completed => {
                qol_runtime::probe!(
                    "CLI_SESSION_WATCH",
                    "event=collected session={} source=checkpoint",
                    round.session
                );
                return Ok(RoundPoll::of(false, false));
            }
            Some(current) if current.completion_marker != round.marker => {
                round.marker_seen = false;
            }
            Some(_) => {
                if round.marker_seen {
                    let full_screen: &str = &screen;
                    let wake_msg = wake_message(
                        trace_dir,
                        &round.session,
                        "completed",
                        full_screen,
                        &round.marker,
                        round.autoclose,
                        round.label.as_deref(),
                    );
                    return complete_seen_round(
                        terminals,
                        interpreter,
                        pending,
                        ledger,
                        locks,
                        round,
                        out,
                        trace_dir,
                        sleep,
                        full_screen,
                        &wake_msg,
                        false,
                    );
                }
                round.marker_seen = true;
                if round.last_screen.as_deref() != Some(screen.as_str()) {
                    round.last_screen = Some(screen.clone());
                    round.last_change = Instant::now();
                }
                return Ok(RoundPoll::of(true, true));
            }
        }
    } else {
        round.marker_seen = false;
    }
    match round_runtime(terminals, interpreter, &round.binding) {
        CliRuntimeState::Working => {
            round.runtime_working_seen = true;
            round.ready_polls = 0;
        }
        CliRuntimeState::Ready if round.runtime_working_seen => {
            round.ready_polls += 1;
        }
        _ => {}
    }
    let mut changed = false;
    if round.last_screen.as_deref() != Some(screen.as_str()) {
        round.last_screen = Some(screen.clone());
        round.last_change = Instant::now();
        changed = true;
    }
    let finished_turn = round.runtime_working_seen && round.ready_polls >= 3;
    let screen_quiet = round.last_change.elapsed() >= config.stall_after;
    if finished_turn || screen_quiet {
        let idle_msg = if finished_turn {
            format!(
                "qol sessions: lane {} finished its turn without printing its completion marker.\nThe attached screen tail is the receipt; review it like a normal report and resubmit if the work is incomplete.\n\n{}",
                round.session,
                clean_screen(screen_tail(&screen))
            )
        } else {
            format!(
                "qol sessions: lane {} went idle for 15 minutes without printing its completion marker.\nThe attached screen tail is the receipt; review it like a normal report and resubmit if the work is incomplete.\n\n{}",
                round.session,
                clean_screen(screen_tail(&screen))
            )
        };
        return complete_seen_round(
            terminals,
            interpreter,
            pending,
            ledger,
            locks,
            round,
            out,
            trace_dir,
            sleep,
            &screen,
            &idle_msg,
            true,
        );
    }
    Ok(RoundPoll {
        keep: true,
        changed,
        released: false,
    })
}

fn reconcile(pending: &PendingBridgeStore, watched: &mut Vec<WatchedRound>) -> Result<()> {
    let mut remaining = Vec::with_capacity(watched.len());
    for mut round in std::mem::take(watched) {
        match pending.pending_round(&round.binding)? {
            None => {
                qol_runtime::probe!(
                    "CLI_SESSION_WATCH",
                    "event=dropped session={} reason=no_open_checkpoint",
                    round.session
                );
            }
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
                        round.marker_seen = false;
                        round.runtime_working_seen = false;
                        round.ready_polls = 0;
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

fn readmit(
    pending: &PendingBridgeStore,
    tokens: &[String],
    watched: Vec<WatchedRound>,
    released: &std::collections::HashSet<String>,
) -> Result<Vec<WatchedRound>> {
    let mut watched = watched;
    for round in load_rounds(pending, tokens)? {
        if round.completed && round.woken {
            continue;
        }
        if released.contains(&round.session) {
            continue;
        }
        if watched
            .iter()
            .any(|current| current.session == round.session)
        {
            continue;
        }
        qol_runtime::probe!(
            "CLI_SESSION_WATCH",
            "event=readmitted session={}",
            round.session
        );
        watched.push(WatchedRound::new(round)?);
    }
    Ok(watched)
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

fn round_runtime(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
) -> CliRuntimeState {
    terminals
        .discover()
        .ok()
        .and_then(|sessions| {
            sessions
                .into_iter()
                .find(|session| session.id == *binding.session_id())
        })
        .map(|session| interpreter.describe(&session).evidence.runtime)
        .unwrap_or_default()
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

fn sanitize_token(token: &str) -> String {
    token.replace([':', '.'], "_")
}

fn group_dir(trace_dir: &std::path::Path, group: &str) -> std::path::PathBuf {
    let safe = group
        .chars()
        .map(|character| {
            if matches!(character, '/' | '\\' | ':' | '.') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    trace_dir.join("groups").join(safe)
}

pub(super) fn label_slug(label: Option<&str>) -> String {
    let Some(label) = label else {
        return String::new();
    };
    let lower = label.to_lowercase();
    let mut slug = String::with_capacity(lower.len());
    let mut separator_pending = false;
    for character in lower.chars() {
        if character.is_ascii_alphanumeric() {
            if separator_pending && !slug.is_empty() {
                slug.push('-');
            }
            separator_pending = false;
            slug.push(character);
        } else {
            separator_pending = true;
        }
    }
    let slug = slug.trim_matches('-');
    slug.chars().take(40).collect()
}

fn fragment_path(
    trace_dir: &std::path::Path,
    group: &str,
    session: &str,
    label: Option<&str>,
) -> std::path::PathBuf {
    let base = format!("{}.txt", sanitize_token(session));
    let slug = label_slug(label);
    let name = if slug.is_empty() {
        base
    } else {
        format!("{slug}_{base}")
    };
    group_dir(trace_dir, group).join(name)
}

fn write_group_fragment(
    trace_dir: &std::path::Path,
    group: &str,
    session: &str,
    tail: &str,
    label: Option<&str>,
) -> Result<()> {
    let path = fragment_path(trace_dir, group, session, label);
    let dir = path.parent().expect("fragment path always has a parent");
    fs::create_dir_all(dir).context("failed to create group fragment directory")?;
    fs::write(&path, tail).context("failed to write group fragment")
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum GroupOutcome {
    Completed,
    Gone,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct GroupMember {
    session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default)]
    outcome: Option<GroupOutcome>,
}

fn member_dir(trace_dir: &std::path::Path, group: &str) -> std::path::PathBuf {
    group_dir(trace_dir, group).join("members")
}

fn member_path(trace_dir: &std::path::Path, group: &str, session: &str) -> std::path::PathBuf {
    member_dir(trace_dir, group).join(format!("{}.json", sanitize_token(session)))
}

fn write_group_member(
    trace_dir: &std::path::Path,
    group: &str,
    member: &GroupMember,
) -> Result<()> {
    let path = member_path(trace_dir, group, &member.session);
    let dir = path.parent().expect("member path always has a parent");
    fs::create_dir_all(dir).context("failed to create group member directory")?;
    let temporary = path.with_extension("tmp");
    let encoded = serde_json::to_string(member)?;
    fs::write(&temporary, encoded).context("failed to write group member record")?;
    fs::rename(&temporary, &path).context("failed to publish group member record")
}

pub(super) fn register_group_member(
    trace_dir: &std::path::Path,
    group: &str,
    session: &str,
    label: Option<&str>,
) -> Result<()> {
    if member_path(trace_dir, group, session).exists() {
        return Ok(());
    }
    write_group_member(
        trace_dir,
        group,
        &GroupMember {
            session: session.to_owned(),
            label: label.map(str::to_owned),
            outcome: None,
        },
    )
}

pub(super) fn settle_group_round(
    trace_dir: &std::path::Path,
    group: &str,
    session: &str,
    label: Option<&str>,
    screen: &str,
) -> Result<()> {
    write_group_fragment(trace_dir, group, session, screen_tail(screen), label)?;
    settle_group_member(trace_dir, group, session, label, GroupOutcome::Completed)
}

fn settle_group_member(
    trace_dir: &std::path::Path,
    group: &str,
    session: &str,
    label: Option<&str>,
    outcome: GroupOutcome,
) -> Result<()> {
    write_group_member(
        trace_dir,
        group,
        &GroupMember {
            session: session.to_owned(),
            label: label.map(str::to_owned),
            outcome: Some(outcome),
        },
    )
}

fn group_roster(
    pending: &PendingBridgeStore,
    trace_dir: &std::path::Path,
    group: &str,
) -> Result<Vec<GroupMember>> {
    let mut members = Vec::new();
    if let Ok(entries) = fs::read_dir(member_dir(trace_dir, group)) {
        for entry in entries {
            let path = entry
                .context("failed to read group member directory")?
                .path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Ok(encoded) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(member) = serde_json::from_str::<GroupMember>(&encoded) else {
                continue;
            };
            members.push(member);
        }
    }
    for round in pending.group_members(group)? {
        if members.iter().any(|member| member.session == round.session) {
            continue;
        }
        members.push(GroupMember {
            session: round.session,
            label: round.label,
            outcome: round.completed.then_some(GroupOutcome::Completed),
        });
    }
    members.sort_by(|left, right| left.session.cmp(&right.session));
    Ok(members)
}

fn claim_group_delivery(dir: &std::path::Path) -> Result<bool> {
    fs::create_dir_all(dir).context("failed to create group directory")?;
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("combined.claim"))
    {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error).context("failed to claim the group combined delivery"),
    }
}

pub(super) fn maybe_deliver_group_combined(
    pending: &PendingBridgeStore,
    terminals: &TerminalSessionService,
    trace_dir: &std::path::Path,
    group: &str,
    session: &str,
    driver: &str,
    sleep: &mut dyn FnMut(Duration),
) -> Result<Option<(String, WakeDelivery)>> {
    let members = group_roster(pending, trace_dir, group)?;
    if members.is_empty() || !members.iter().all(|member| member.outcome.is_some()) {
        return Ok(None);
    }
    let dir = group_dir(trace_dir, group);
    if !claim_group_delivery(&dir)? {
        qol_runtime::probe!(
            "CLI_SESSION_WATCH",
            "event=group_combined_already_claimed group={} session={}",
            group,
            session
        );
        return Ok(None);
    }
    let mut combined = String::new();
    for member in &members {
        combined.push_str(&format!("## {}\n\n", member.session));
        let path = fragment_path(trace_dir, group, &member.session, member.label.as_deref());
        if let Ok(encoded) = fs::read_to_string(&path) {
            combined.push_str(&encoded);
            combined.push('\n');
        }
    }
    let combined_path = dir.join("combined.md");
    fs::write(&combined_path, &combined).context("failed to write group combined report")?;
    let lane_lines: Vec<(String, bool)> = members
        .iter()
        .map(|member| {
            (
                member.session.clone(),
                member.outcome == Some(GroupOutcome::Completed),
            )
        })
        .collect();
    let message = grouped_message(group, &lane_lines, &combined_path);
    let delivery = deliver_wake(
        terminals,
        trace_dir,
        session,
        driver,
        "completed",
        &message,
        sleep,
    )?;
    if !delivery.delivered {
        let _ = fs::remove_file(dir.join("combined.claim"));
    }
    Ok(Some((combined, delivery)))
}

fn grouped_message(
    group: &str,
    lanes: &[(String, bool)],
    combined_path: &std::path::Path,
) -> String {
    let lane_text: String = lanes
        .iter()
        .map(|(name, completed)| {
            let status = if *completed {
                "completed"
            } else {
                "did not complete"
            };
            format!("- {name} ({status})\n")
        })
        .collect();
    format!(
        "qol sessions: grouped research `{group}` complete, all {} lanes finished.\n\nCombined file: {}\n\nLanes:\n{lane_text}",
        lanes.len(),
        combined_path.display()
    )
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
    let mut released = std::collections::HashSet::new();
    loop {
        if !explicit {
            if let Ok(sessions) = terminals.discover() {
                let live_tokens = sessions
                    .into_iter()
                    .filter_map(|session| session.binding().ok())
                    .map(|binding| binding.token())
                    .collect::<std::collections::HashSet<_>>();
                pending.retain_live(&live_tokens)?;
            }
        }
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
            if outcome.released {
                released.insert(round.session.clone());
            }
            if outcome.keep {
                remaining.push(round);
            }
        }
        watched = readmit(pending, &tokens, remaining, &released)?;
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

pub(super) fn close_lane_terminal(terminals: &TerminalSessionService, binding: &SessionBinding) {
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

pub(super) struct WakeDelivery {
    delivered: bool,
    error: Option<String>,
}

fn wake_message(
    trace_dir: &std::path::Path,
    session: &str,
    event: &str,
    screen: &str,
    _marker: &str,
    autoclose: bool,
    label: Option<&str>,
) -> String {
    match event {
        "completed" => completion_message(trace_dir, session, screen, autoclose, label),
        "gone" => format!(
            "qol sessions: lane {session} gone. The lane terminal closed and its round was discarded; start a fresh lane if the work still matters."
        ),
        _ => format!(
            "qol sessions: lane {session} wake {event}."
        ),
    }
}

fn completion_message(
    trace_dir: &std::path::Path,
    session: &str,
    screen: &str,
    autoclose: bool,
    label: Option<&str>,
) -> String {
    let sentence = if autoclose {
        format!(
            "qol sessions: lane {session} completed and the lane terminal closed. Report below."
        )
    } else {
        format!("qol sessions: {session} completed. Report below.")
    };
    let cleaned = clean_screen(screen);
    match write_lane_report(trace_dir, session, &cleaned, label) {
        Ok(path) => format!("{sentence}\n\nReport: {}", path.display()),
        Err(error) => {
            let report = inline_report(&cleaned);
            format!(
                "{sentence}\n\n{report}\n\nWarning: could not write the lane report file ({error}); the full screen is not preserved."
            )
        }
    }
}

fn lane_report_path(
    trace_dir: &std::path::Path,
    session: &str,
    label: Option<&str>,
) -> std::path::PathBuf {
    let base = format!("{}.md", sanitize_token(session));
    let slug = label_slug(label);
    let name = if slug.is_empty() {
        base
    } else {
        format!("{slug}_{base}")
    };
    trace_dir.join("lanes").join(name)
}

fn write_lane_report(
    trace_dir: &std::path::Path,
    session: &str,
    cleaned: &str,
    label: Option<&str>,
) -> Result<std::path::PathBuf> {
    let path = lane_report_path(trace_dir, session, label);
    let dir = path.parent().expect("lane report path always has a parent");
    fs::create_dir_all(dir).context("failed to create lane report directory")?;
    fs::write(&path, cleaned).context("failed to write lane report")?;
    Ok(path)
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

fn inline_report(cleaned: &str) -> String {
    let mut body = cleaned.to_owned();
    if let Some(path) = discover_deliverable(cleaned) {
        body = format!("Deliverable: {path}\n\n{body}");
    }
    report_snippet(&body)
}

fn discover_deliverable(report: &str) -> Option<String> {
    let mut deliverable = None;
    for token in report.split_whitespace() {
        if !token.starts_with('/') {
            continue;
        }
        let candidate = token.trim_end_matches(|character: char| {
            matches!(character, ',' | ':' | ';' | ')' | ']' | '}' | '.')
        });
        if std::path::Path::new(candidate).is_file() {
            deliverable = Some(candidate.to_owned());
        }
    }
    deliverable
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
    markerless: bool,
) -> Result<()> {
    let event_name = if markerless {
        "completed_markerless"
    } else {
        "completed"
    };
    let mut event = serde_json::json!({
        "event": event_name,
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

    use qol_terminal_sessions::cli::{CliTool, CliToolId};
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:8:800",
                false,
                None,
            )
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
    fn grouped_lanes_emit_no_wake_until_all_members_complete_then_one_combined_wake() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let group = "research";
        let lane_a: SessionBinding = "v1:fake:7:100".parse().unwrap();
        let lane_b: SessionBinding = "v1:fake:8:200".parse().unwrap();
        pending
            .start(
                &lane_a,
                "QOL_BRIDGE_DONE_a",
                "v1:fake:7:100",
                false,
                Some(group),
            )
            .unwrap();
        pending
            .start(
                &lane_b,
                "QOL_BRIDGE_DONE_b",
                "v1:fake:8:200",
                false,
                Some(group),
            )
            .unwrap();
        let backend_a = FakeBackend::new(
            facts("7", 100),
            vec![
                "tail a\nQOL_BRIDGE_DONE_a".to_owned(),
                "tail a\nQOL_BRIDGE_DONE_a".to_owned(),
            ],
        );
        let (terminals_a, backend_a) = harness(backend_a);
        let mut round_a =
            WatchedRound::new(pending.pending_round(&lane_a).unwrap().unwrap()).unwrap();
        let mut out_a = Vec::new();
        drive_to_completion(
            &terminals_a,
            &pending,
            &ledger(&root),
            &locks(&root),
            &mut round_a,
            &mut out_a,
            root.path(),
        );
        assert!(
            backend_a.sent.lock().unwrap().is_empty(),
            "the first grouped member must not wake the initiator on its own"
        );
        assert!(
            out_a.is_empty(),
            "the first grouped member must emit no completed event: {out_a:?}"
        );
        let combined_dir = root.path().join("groups").join(group);
        let combined_path = combined_dir.join("combined.md");
        assert!(
            !combined_path.exists(),
            "the combined file must wait until every member completes"
        );

        let backend_b = FakeBackend::new(
            facts("8", 200),
            vec![
                "tail b\nQOL_BRIDGE_DONE_b".to_owned(),
                "tail b\nQOL_BRIDGE_DONE_b".to_owned(),
            ],
        );
        let (terminals_b, backend_b) = harness(backend_b);
        let mut round_b =
            WatchedRound::new(pending.pending_round(&lane_b).unwrap().unwrap()).unwrap();
        let mut out_b = Vec::new();
        drive_to_completion(
            &terminals_b,
            &pending,
            &ledger(&root),
            &locks(&root),
            &mut round_b,
            &mut out_b,
            root.path(),
        );
        let events = lines(&out_b);
        assert_eq!(events.len(), 1, "one combined wake expected: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["delivered"], true);
        assert!(
            combined_path.exists(),
            "the combined file must be written when the group completes"
        );
        let sent = backend_b.sent.lock().unwrap();
        assert_eq!(
            sent.len(),
            1,
            "exactly one grouped wake must reach the initiator after both members finish"
        );
        let (_, text, _) = &sent[0];
        let combined_display = combined_path.display().to_string();
        let combined_pos = text
            .find(&combined_display)
            .expect("grouped wake names the combined file");
        let lanes_pos = text.find("Lanes:").expect("grouped wake lists the lanes");
        assert!(
            combined_pos < lanes_pos,
            "the combined file path must sit near the top, before the per-lane list: {text:?}"
        );
        assert!(
            text.contains(&lane_a.token()) && text.contains(&lane_b.token()),
            "the grouped wake must name every member lane: {text:?}"
        );
        assert!(
            text.contains("(completed)"),
            "the grouped wake must state each lane's completion status: {text:?}"
        );
        assert!(
            !text.contains("tail a") && !text.contains("tail b"),
            "the grouped wake is a pointer, not a transcript, and must not paste lane scrollback: {text:?}"
        );
        assert!(!text.contains("QOL_BRIDGE_DONE"));
        drop(sent);

        let a_fragment =
            combined_dir.join(format!("{}.txt", super::sanitize_token(&lane_a.token())));
        let b_fragment =
            combined_dir.join(format!("{}.txt", super::sanitize_token(&lane_b.token())));
        assert!(
            a_fragment.exists() && b_fragment.exists(),
            "each member lane must leave a fragment file"
        );
        assert_eq!(
            std::fs::read_to_string(&a_fragment).unwrap(),
            "tail a\nQOL_BRIDGE_DONE_a"
        );
        assert_eq!(
            std::fs::read_to_string(&b_fragment).unwrap(),
            "tail b\nQOL_BRIDGE_DONE_b"
        );
        let combined = std::fs::read_to_string(&combined_path).unwrap();
        let a_pos = combined.find(&format!("## {}", lane_a.token())).unwrap();
        let b_pos = combined.find(&format!("## {}", lane_b.token())).unwrap();
        assert!(
            a_pos < b_pos,
            "fragments must concatenate in session-token order"
        );
    }

    #[test]
    fn grouped_message_is_a_pointer_naming_members_without_lane_scrollback() {
        let root = tempfile::TempDir::new().unwrap();
        let combined_path = root.path().join("groups").join("research");
        std::fs::create_dir_all(&combined_path).unwrap();
        let combined_path = combined_path.join("combined.md");
        let lanes = vec![
            ("v1:fake:7:100".to_owned(), true),
            ("v1:fake:8:200".to_owned(), true),
            ("v1:fake:9:300".to_owned(), true),
        ];
        let message = grouped_message("research", &lanes, &combined_path);
        let expected_display = combined_path.display().to_string();
        let combined_pos = message.find(&expected_display).unwrap();
        let lanes_pos = message.find("Lanes:").unwrap();
        assert_eq!(
            message.lines().count(),
            8,
            "header, blank, Combined file, blank, Lanes, then three lanes: {message:?}"
        );
        assert!(
            message.starts_with(
                "qol sessions: grouped research `research` complete, all 3 lanes finished."
            ),
            "the header must name the group and count lanes: {message:?}"
        );
        assert!(
            combined_pos < lanes_pos,
            "the combined path must sit near the top before the lane list: {message:?}"
        );
        for name in ["v1:fake:7:100", "v1:fake:8:200", "v1:fake:9:300"] {
            assert!(
                message.contains(&format!("- {name} (completed)")),
                "every member lane must be named with its status: {message:?}"
            );
        }
        assert_eq!(message.matches("(completed)").count(), 3);
        assert!(
            !message.contains("tail") && !message.contains("QOL_BRIDGE_DONE"),
            "the grouped wake must never paste lane scrollback: {message:?}"
        );
    }

    #[test]
    fn grouped_message_marks_an_unfinished_lane_as_not_completed() {
        let root = tempfile::TempDir::new().unwrap();
        let combined_path = root.path().join("combined.md");
        let lanes = vec![
            ("v1:fake:7:100".to_owned(), true),
            ("v1:fake:8:200".to_owned(), false),
        ];
        let message = grouped_message("research", &lanes, &combined_path);
        assert!(message.contains("- v1:fake:7:100 (completed)"));
        assert!(message.contains("- v1:fake:8:200 (did not complete)"));
    }

    #[test]
    fn groupless_lane_still_wakes_individually() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let lone: SessionBinding = "v1:fake:9:300".parse().unwrap();
        pending
            .start(&lone, "QOL_BRIDGE_DONE_lone", "v1:fake:9:300", false, None)
            .unwrap();
        let backend_lone = FakeBackend::new(
            facts("9", 300),
            vec![
                "tail lone\nQOL_BRIDGE_DONE_lone".to_owned(),
                "tail lone\nQOL_BRIDGE_DONE_lone".to_owned(),
            ],
        );
        let (terminals, backend_lone) = harness(backend_lone);
        let mut round = WatchedRound::new(pending.pending_round(&lone).unwrap().unwrap()).unwrap();
        let mut out = Vec::new();
        drive_to_completion(
            &terminals,
            &pending,
            &ledger(&root),
            &locks(&root),
            &mut round,
            &mut out,
            root.path(),
        );

        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["delivered"], true);
        let sent = backend_lone.sent.lock().unwrap();
        assert_eq!(
            sent.len(),
            1,
            "a groupless lane must still deliver an individual wake"
        );
        let (_, text, _) = &sent[0];
        assert!(text.contains("qol sessions: v1:fake:9:300 completed"));
        drop(sent);
        assert!(
            !root.path().join("groups").exists(),
            "groupless lanes must not write group fragments or combined files"
        );
    }

    fn drive_to_completion(
        terminals: &TerminalSessionService,
        pending: &PendingBridgeStore,
        ledger: &SpawnLedger,
        locks: &SpawnLocks,
        round: &mut WatchedRound,
        out: &mut Vec<u8>,
        trace_dir: &std::path::Path,
    ) {
        let interpreter = CliSessionInterpreter::system();
        let mut attempts = 0;
        loop {
            let result = poll_round(
                terminals,
                &interpreter,
                pending,
                ledger,
                locks,
                round,
                out,
                trace_dir,
                fast_config(Duration::from_secs(3600)),
                &mut |_| {},
            )
            .unwrap();
            attempts += 1;
            if !result.keep {
                return;
            }
            assert!(
                attempts < 10,
                "round did not complete within the poll budget"
            );
        }
    }

    #[test]
    fn completed_without_a_spawn_identity_leaves_no_ledger_record() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let ledger = ledger(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:8:800",
                false,
                None,
            )
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:8:800",
                false,
                None,
            )
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
        let dir = tempfile::TempDir::new().unwrap();
        let closable = wake_message(
            dir.path(),
            "v1:kitty:5:100",
            "completed",
            "done",
            "QOL_BRIDGE_DONE_none",
            true,
            None,
        );
        assert!(
            !closable.contains("session_loop_close") && !closable.contains("Review it"),
            "the closable wake must not instruct the driver: {closable:?}"
        );
        assert!(closable.starts_with(
            "qol sessions: lane v1:kitty:5:100 completed and the lane terminal closed. Report below."
        ));

        let plain = wake_message(
            dir.path(),
            "v1:kitty:5:100",
            "completed",
            "done",
            "QOL_BRIDGE_DONE_none",
            false,
            None,
        );
        assert!(
            !plain.contains("session_loop_close") && !plain.contains("Review it"),
            "the plain wake must not instruct the driver: {plain:?}"
        );
        assert!(plain.starts_with("qol sessions: v1:kitty:5:100 completed. Report below."));
        assert!(plain.contains(&format!(
            "Report: {}",
            dir.path().join("lanes").join("v1_kitty_5_100.md").display()
        )));
    }

    #[test]
    fn pi_style_screen_without_prompt_glyphs_wake_is_a_file_pointer_and_not_the_body() {
        let dir = tempfile::TempDir::new().unwrap();
        let session = "v1:pi:7:100";
        let marker = "QOL_BRIDGE_DONE_round";
        let screen = [
            "pi home \u{1f5a5}\u{fe0f}",
            "thinking about the request",
            "a bit more scrollback before the middle",
            "ACTION_MIDDLE to be asserted later",
            "more scrollback after the middle",
            "and even more scrollback",
            "the final answer is here and it matters",
            "QOL_BRIDGE_DONE_round",
        ]
        .join("\n");
        let wake = wake_message(
            dir.path(),
            session,
            "completed",
            &screen,
            marker,
            false,
            None,
        );
        assert!(
            wake.contains(&format!(
                "Report: {}",
                dir.path()
                    .join("lanes")
                    .join(sanitize_token(session) + ".md")
                    .display()
            )),
            "the wake must carry a pointer to the lane report: {wake:?}"
        );
        assert!(
            !wake.contains("ACTION_MIDDLE"),
            "the wake must not paste the body of the pi screen: {wake:?}"
        );
        assert!(
            !wake.contains("thinking about the request") && !wake.contains(marker),
            "the wake must not carry scrollback or the marker: {wake:?}"
        );
        let written = std::fs::read_to_string(sanitize_lane_path(dir.path(), session)).unwrap();
        assert!(
            written.contains("ACTION_MIDDLE") && written.contains("the final answer is here and it matters"),
            "the written report must contain the full cleaned screen including the middle: {written:?}"
        );
    }

    #[test]
    fn written_lane_report_keeps_content_beyond_the_old_two_kib_truncation() {
        let dir = tempfile::TempDir::new().unwrap();
        let session = "v1:pi:7:100";
        let marker = "QOL_BRIDGE_DONE_round";
        let filler = "Z".repeat(100);
        let head = format!("{filler}\nearly-prefix-keep-me");
        let tail_line = "the report tail stays too";
        let screen = format!(
            "{head}\n{}\n{tail_line}\n{marker}",
            (0..60)
                .map(|_| filler.clone())
                .collect::<Vec<_>>()
                .join("\n")
        );
        let wake = wake_message(
            dir.path(),
            session,
            "completed",
            &screen,
            marker,
            false,
            None,
        );
        assert!(wake.contains("Report:"), "the wake is a pointer: {wake:?}");
        assert!(
            wake.len() < 2048,
            "the wake must stay short even for a huge report: {} bytes",
            wake.len()
        );
        let written = std::fs::read_to_string(sanitize_lane_path(dir.path(), session)).unwrap();
        assert!(
            written.contains("early-prefix-keep-me") && written.contains(tail_line),
            "the file must keep the full cleaned screen far beyond 2 KiB: {}",
            written.len()
        );
        assert!(
            written.len() > 2 * 1024,
            "the written report should comfortably exceed the old cap: {} bytes",
            written.len()
        );
    }

    #[test]
    fn huge_screen_still_produces_a_short_wake() {
        let dir = tempfile::TempDir::new().unwrap();
        let session = "v1:pi:7:100";
        let marker = "QOL_BRIDGE_DONE_round";
        let filler = "X".repeat(80);
        let mut lines = Vec::new();
        for _ in 0..2000 {
            lines.push(filler.clone());
        }
        lines.push("the very last line before the marker".to_owned());
        lines.push(marker.to_owned());
        let screen = lines.join("\n");
        assert!(screen.len() > WAKE_SNIPPET_MAX_BYTES * 4);
        let wake = wake_message(
            dir.path(),
            session,
            "completed",
            &screen,
            marker,
            false,
            None,
        );
        assert!(
            wake.len() < 2048,
            "a huge screen must not bloat the wake pointer: {} bytes",
            wake.len()
        );
        assert!(
            wake.contains("Report:"),
            "the wake stays a pointer: {wake:?}"
        );
    }

    #[test]
    fn unwritable_report_dir_makes_the_wake_name_the_failure_and_keeps_inline_report() {
        let root = tempfile::TempDir::new().unwrap();
        let session = "v1:pi:7:100";
        let blocker = root.path().join("some-file");
        std::fs::write(&blocker, b"").unwrap();
        let trace_dir = blocker.as_path();
        let screen = "line one\nline two\nWrote line three.\nQOL_BRIDGE_DONE_round";
        let wake = wake_message(
            trace_dir,
            session,
            "completed",
            screen,
            "QOL_BRIDGE_DONE_round",
            false,
            None,
        );
        assert!(
            wake.contains("could not write the lane report"),
            "the wake must explicitly say the report file could not be written: {wake:?}"
        );
        assert!(
            wake.contains("line two"),
            "the wake must fall back to inline content: {wake:?}"
        );
    }

    fn sanitize_lane_path(dir: &std::path::Path, session: &str) -> std::path::PathBuf {
        dir.join("lanes")
            .join(format!("{}.md", sanitize_token(session)))
    }

    #[test]
    fn labeled_rounds_slug_prefix_report_and_fragment_filenames() {
        let dir = tempfile::TempDir::new().unwrap();
        let session = "v1:pi:7:100";
        let label = Some("csharp-settings-panel");
        assert_eq!(label_slug(label), "csharp-settings-panel");
        assert_eq!(label_slug(None), "");
        assert_eq!(
            label_slug(Some("  C# Settings //Panel!! ")),
            "c-settings-panel"
        );

        let report_path = write_lane_report(dir.path(), session, "body", label).unwrap();
        assert_eq!(
            report_path,
            dir.path()
                .join("lanes")
                .join("csharp-settings-panel_v1_pi_7_100.md")
        );
        assert_eq!(std::fs::read_to_string(&report_path).unwrap(), "body");

        write_group_fragment(dir.path(), "research", session, "tail", label).unwrap();
        assert!(dir
            .path()
            .join("groups")
            .join("research")
            .join("csharp-settings-panel_v1_pi_7_100.txt")
            .is_file());

        let plain_report_path = write_lane_report(dir.path(), session, "body", None).unwrap();
        assert_eq!(plain_report_path, sanitize_lane_path(dir.path(), session));
        write_group_fragment(dir.path(), "research", session, "tail", None).unwrap();
        assert!(dir
            .path()
            .join("groups")
            .join("research")
            .join("v1_pi_7_100.txt")
            .is_file());
    }

    #[test]
    fn autoclose_round_closes_the_lane_terminal_after_completed_and_plain_rounds_stay_open() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);

        let auto_binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(
                &auto_binding,
                "QOL_BRIDGE_DONE_auto",
                "v1:fake:7:100",
                true,
                None,
            )
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
        assert!(
            text.contains("Report: "),
            "the completed wake must point at the report file: {text:?}"
        );
        assert!(
            !text.contains("QOL_BRIDGE_DONE_auto"),
            "the completed wake must not carry scrollback or the marker: {text:?}"
        );
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
            std::slice::from_ref(&auto_binding),
            "an autoclose round must close the lane terminal after delivery"
        );
        drop(closed);
        assert!(
            pending.pending_round(&auto_binding).unwrap().is_none(),
            "an autoclose round must also close its pending-bridge checkpoint"
        );

        let plain_binding: SessionBinding = "v1:fake:8:200".parse().unwrap();
        pending
            .start(
                &plain_binding,
                "QOL_BRIDGE_DONE_plain",
                "v1:fake:8:200",
                false,
                None,
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:7:100",
                true,
                None,
            )
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:7:100",
                false,
                None,
            )
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:8:200",
                true,
                None,
            )
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:8:800",
                false,
                None,
            )
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:8:800",
                false,
                None,
            )
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
                None,
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
                None,
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
                None,
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
                None,
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
    fn markerless_completion_closes_the_round() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:8:800",
                false,
                None,
            )
            .unwrap();
        let screens = (0..64).map(|_| "idle".to_owned()).collect();
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
        let completed = events
            .iter()
            .filter(|event| event["event"] == "completed_markerless")
            .collect::<Vec<_>>();
        assert_eq!(completed.len(), 1, "events: {events:?}");
        assert_eq!(completed[0]["session"], "v1:fake:7:100");
        assert_eq!(completed[0]["screen"], "idle");
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(round.completed);
        assert_eq!(round.screen.as_deref(), Some("idle"));
    }

    #[test]
    fn no_change_polls_grow_the_sleep_and_a_change_resets_it() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:8:800",
                false,
                None,
            )
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:8:800",
                false,
                None,
            )
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
        assert_eq!(ls, 58);
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:7:100",
                false,
                None,
            )
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
            9,
            "the first marker poll must not emit; the second confirms, then each successful poll's early external-id capture, the transcript gate on each marker sighting, the completion capture and the delivery re-check each add a discovery"
        );
        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(
            sent[0].1.contains("Report: "),
            "the completed wake must point at the report file: {}",
            sent[0].1
        );
        assert!(
            !sent[0].1.contains("QOL_BRIDGE_DONE_round"),
            "the completed wake must not carry scrollback or the marker: {}",
            sent[0].1
        );
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(round.completed);
    }

    #[test]
    fn wake_delivers_immediately_when_the_composer_clears() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:7:100",
                false,
                None,
            )
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
            sent[0].1.contains("Report: "),
            "the completed wake must point at the report file: {}",
            sent[0].1
        );
        assert!(
            !sent[0].1.contains("QOL_BRIDGE_DONE_round"),
            "the completed wake must not carry scrollback or the marker: {}",
            sent[0].1
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
    fn a_dropped_round_is_readmitted_while_its_checkpoint_is_open() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:7:100",
                false,
                None,
            )
            .unwrap();
        let tokens = vec![binding.token().to_owned()];

        let readmitted = readmit(&pending, &tokens, Vec::new(), &Default::default()).unwrap();
        assert_eq!(
            readmitted.len(),
            1,
            "an open uncompleted round must come back into the watch set"
        );

        let kept = readmit(&pending, &tokens, readmitted, &Default::default()).unwrap();
        assert_eq!(kept.len(), 1, "a watched round is never duplicated");

        pending
            .observe(&binding, "QOL_BRIDGE_DONE_round", true)
            .unwrap();
        pending.claim_wake(&binding, "completed").unwrap();
        assert!(
            readmit(&pending, &tokens, Vec::new(), &Default::default())
                .unwrap()
                .is_empty(),
            "a completed and woken round is not watched again"
        );
    }

    #[test]
    fn wake_delivers_when_the_draft_region_stops_changing() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:7:100",
                false,
                None,
            )
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:7:100",
                false,
                None,
            )
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:7:100",
                false,
                None,
            )
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:8:800",
                false,
                None,
            )
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
            .start(
                &binding,
                "QOL_BRIDGE_DONE_round",
                "v1:fake:8:800",
                false,
                None,
            )
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
                None,
            )
            .unwrap();
        pending
            .start(
                &other_binding,
                "QOL_BRIDGE_DONE_other",
                "v1:fake:9:900",
                false,
                None,
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
            .start(&binding, "QOL_BRIDGE_DONE_error", "", false, None)
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
            .start(&binding, "QOL_BRIDGE_DONE_done", "", false, None)
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
    const REASONING_SCREEN: &str = concat!(
        " Also note: \"Final receipt: at most 3 short lines.\" And end with completion\n",
        " fragments joined: QOL_BRIDGE_DONE_round - joined with no spaces or punctuation.\n",
        " So final line: QOL_BRIDGE_DONE_round.\n",
        "\n",
        " \u{2826} Working...\n",
        "0.0%/1.0M (auto)\n"
    );

    const FINAL_SCREEN: &str = concat!(
        " Wrote the report to disk.\n",
        "\n",
        "QOL_BRIDGE_DONE_round\n"
    );

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Transcript {
        Silent,
        Working,
        Finished,
    }

    struct FakeTool {
        tool: CliTool,
        transcript: AtomicU64,
    }

    impl FakeTool {
        fn new(transcript: Transcript) -> Arc<Self> {
            let tool = CliTool::new(
                CliToolId::new("simulated").unwrap(),
                "Simulated",
                qol_terminal_sessions::cli::CliToolColor::new(1, 2, 3),
            );
            let fake = Arc::new(Self {
                tool,
                transcript: AtomicU64::new(0),
            });
            fake.set(transcript);
            fake
        }

        fn set(&self, transcript: Transcript) {
            let encoded = match transcript {
                Transcript::Silent => 0,
                Transcript::Working => 1,
                Transcript::Finished => 2,
            };
            self.transcript.store(encoded, Ordering::Relaxed);
        }
    }

    impl qol_terminal_sessions::cli::CliSessionStrategy for FakeTool {
        fn tool(&self) -> &CliTool {
            &self.tool
        }

        fn priority(&self) -> i32 {
            1_000
        }

        fn matches(&self, _session: &SessionFacts) -> bool {
            true
        }

        fn describe(
            &self,
            _session: &SessionFacts,
        ) -> qol_terminal_sessions::cli::CliSessionDescriptor {
            let runtime = match self.transcript.load(Ordering::Relaxed) {
                1 => CliRuntimeState::Working,
                2 => CliRuntimeState::Ready,
                _ => CliRuntimeState::Unknown,
            };
            qol_terminal_sessions::cli::CliSessionDescriptor {
                tool: self.tool.clone(),
                display_name: None,
                external_id: None,
                has_activity: None,
                evidence: qol_terminal_sessions::cli::CliSessionEvidence {
                    runtime,
                    ..Default::default()
                },
            }
        }

        fn transcript_completion(&self, _session: &SessionFacts, _marker: &str) -> Option<bool> {
            match self.transcript.load(Ordering::Relaxed) {
                1 => Some(false),
                2 => Some(true),
                _ => None,
            }
        }
    }

    struct SessionSim {
        root: tempfile::TempDir,
        pending: PendingBridgeStore,
        ledger: SpawnLedger,
        locks: SpawnLocks,
    }

    impl SessionSim {
        fn new() -> Self {
            let root = tempfile::TempDir::new().unwrap();
            let pending = store(&root);
            let ledger = ledger(&root);
            let locks = locks(&root);
            Self {
                root,
                pending,
                ledger,
                locks,
            }
        }

        fn trace_dir(&self) -> &std::path::Path {
            self.root.path()
        }

        #[allow(clippy::too_many_arguments)]
        fn lane(
            &self,
            native: &str,
            pid: i32,
            marker: &str,
            autoclose: bool,
            group: Option<&str>,
            label: Option<&str>,
            transcript: Transcript,
            screens: Vec<String>,
        ) -> LaneSim {
            let binding: SessionBinding = format!("v1:fake:{native}:{pid}").parse().unwrap();
            let driver_native = format!("d{native}");
            let driver: SessionBinding = format!("v1:fake:{driver_native}:{}", pid + 1000)
                .parse()
                .unwrap();
            self.pending
                .start_with_label(&binding, marker, &driver.token(), autoclose, group, label)
                .unwrap();
            if let Some(group) = group {
                super::register_group_member(self.trace_dir(), group, &binding.token(), label)
                    .unwrap();
            }
            let backend = FakeBackend::new(facts(native, pid), screens)
                .with_driver(facts(&driver_native, pid + 1000));
            let (terminals, backend) = harness(backend);
            let tool = FakeTool::new(transcript);
            let interpreter = CliSessionInterpreter::from_strategies([
                Arc::clone(&tool) as Arc<dyn qol_terminal_sessions::cli::CliSessionStrategy>
            ])
            .unwrap();
            let round =
                WatchedRound::new(self.pending.pending_round(&binding).unwrap().unwrap()).unwrap();
            LaneSim {
                binding,
                terminals,
                backend,
                interpreter,
                tool,
                round,
                out: Vec::new(),
                stall_after: Duration::from_secs(3600),
            }
        }

        fn simple_lane(&self, native: &str, pid: i32, screens: Vec<String>) -> LaneSim {
            self.lane(
                native,
                pid,
                "QOL_BRIDGE_DONE_round",
                true,
                None,
                None,
                Transcript::Silent,
                screens,
            )
        }

        fn combined(&self, group: &str) -> Option<String> {
            std::fs::read_to_string(super::group_dir(self.trace_dir(), group).join("combined.md"))
                .ok()
        }
    }

    struct LaneSim {
        binding: SessionBinding,
        terminals: TerminalSessionService,
        backend: Arc<FakeBackend>,
        interpreter: CliSessionInterpreter,
        tool: Arc<FakeTool>,
        round: WatchedRound,
        out: Vec<u8>,
        stall_after: Duration,
    }

    impl LaneSim {
        fn stall_after(mut self, stall_after: Duration) -> Self {
            self.stall_after = stall_after;
            self
        }

        fn transcript(&self, transcript: Transcript) {
            self.tool.set(transcript);
        }

        fn poll(&mut self, sim: &SessionSim) -> RoundPoll {
            poll_round(
                &self.terminals,
                &self.interpreter,
                &sim.pending,
                &sim.ledger,
                &sim.locks,
                &mut self.round,
                &mut self.out,
                sim.trace_dir(),
                fast_config(self.stall_after),
                &mut |_| {},
            )
            .unwrap()
        }

        fn poll_times(&mut self, sim: &SessionSim, times: usize) {
            for _ in 0..times {
                if !self.poll(sim).keep {
                    return;
                }
            }
        }

        fn run(&mut self, sim: &SessionSim) {
            for _ in 0..32 {
                if !self.poll(sim).keep {
                    return;
                }
            }
            panic!("the simulated lane never settled");
        }

        fn events(&self) -> Vec<serde_json::Value> {
            lines(&self.out)
        }

        fn wakes(&self) -> Vec<String> {
            self.backend
                .sent
                .lock()
                .unwrap()
                .iter()
                .map(|(_, text, _)| text.clone())
                .collect()
        }

        fn closed(&self) -> usize {
            self.backend.closed.lock().unwrap().len()
        }

        fn open_round(&self, sim: &SessionSim) -> Option<PendingRound> {
            sim.pending.pending_round(&self.binding).unwrap()
        }

        fn settled(&self, sim: &SessionSim) -> bool {
            self.open_round(sim).is_none_or(|round| round.completed)
        }
    }

    #[test]
    fn sim_a_lane_that_prints_its_marker_completes_and_closes_its_terminal() {
        let sim = SessionSim::new();
        let mut lane = sim.simple_lane("7", 100, vec![FINAL_SCREEN.to_owned(); 4]);

        lane.run(&sim);

        let events = lane.events();
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(lane.wakes().len(), 1);
        assert_eq!(lane.closed(), 1, "an autoclose lane closes its terminal");
        assert!(lane.settled(&sim));
        assert!(
            lane.open_round(&sim).is_none(),
            "an autoclose lane leaves no open checkpoint"
        );
    }

    #[test]
    fn sim_a_marker_restated_while_the_transcript_still_works_never_completes_the_round() {
        let sim = SessionSim::new();
        let mut lane = sim.lane(
            "7",
            100,
            "QOL_BRIDGE_DONE_round",
            true,
            None,
            None,
            Transcript::Working,
            vec![REASONING_SCREEN.to_owned(); 8],
        );

        lane.poll_times(&sim, 8);

        assert!(
            super::super::marker_present(REASONING_SCREEN, "QOL_BRIDGE_DONE_round"),
            "the reasoning screen really does restate the marker"
        );
        assert!(
            lane.events().is_empty(),
            "a mid-work screen must emit no completion: {:?}",
            lane.events()
        );
        assert!(lane.wakes().is_empty(), "no wake may reach the initiator");
        assert_eq!(lane.closed(), 0, "the lane terminal must stay open");
        assert!(
            !lane.open_round(&sim).unwrap().completed,
            "the checkpoint must still be open and unfinished"
        );
    }

    #[test]
    fn sim_the_round_completes_once_the_transcript_agrees_the_marker_is_final() {
        let sim = SessionSim::new();
        let mut lane = sim.lane(
            "7",
            100,
            "QOL_BRIDGE_DONE_round",
            true,
            None,
            None,
            Transcript::Working,
            vec![REASONING_SCREEN.to_owned(); 4],
        );
        lane.poll_times(&sim, 4);
        assert!(lane.events().is_empty());

        lane.transcript(Transcript::Finished);
        lane.run(&sim);

        let events = lane.events();
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert!(lane.settled(&sim));
    }

    #[test]
    fn sim_a_tool_without_a_transcript_still_completes_on_the_screen_marker_alone() {
        let sim = SessionSim::new();
        let mut lane = sim.lane(
            "7",
            100,
            "QOL_BRIDGE_DONE_round",
            true,
            None,
            None,
            Transcript::Silent,
            vec![REASONING_SCREEN.to_owned(); 4],
        );

        lane.run(&sim);

        let events = lane.events();
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
    }

    #[test]
    fn sim_an_idle_lane_completes_markerless_after_the_stall_window() {
        let sim = SessionSim::new();
        let mut lane = sim
            .simple_lane("7", 100, vec!["idle".to_owned(); 4])
            .stall_after(Duration::from_millis(0));

        lane.run(&sim);

        let events = lane.events();
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed_markerless");
        assert!(lane.settled(&sim));
    }

    #[test]
    fn sim_a_lane_that_finishes_without_the_marker_stalls_with_its_screen_tail() {
        let sim = SessionSim::new();
        let mut lane = sim.lane(
            "7",
            100,
            "QOL_BRIDGE_DONE_round",
            true,
            None,
            None,
            Transcript::Working,
            vec!["the lane finished without a marker".to_owned(); 8],
        );

        lane.poll_times(&sim, 2);
        lane.transcript(Transcript::Finished);
        lane.poll_times(&sim, 3);

        let events = lane.events();
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed_markerless");
        let wakes = lane.wakes();
        assert_eq!(wakes.len(), 1, "wakes: {wakes:?}");
        assert!(
            wakes[0].contains("finished its turn without printing its completion marker"),
            "the wake must name the finished-turn stall: {:?}",
            wakes[0]
        );
        assert!(
            wakes[0].contains("the lane finished without a marker"),
            "the wake must carry the lane screen tail: {:?}",
            wakes[0]
        );
        assert!(lane.settled(&sim));
    }

    #[test]
    fn sim_a_lane_that_never_worked_does_not_stall_on_a_ready_runtime() {
        let sim = SessionSim::new();
        let mut lane = sim.lane(
            "7",
            100,
            "QOL_BRIDGE_DONE_round",
            true,
            None,
            None,
            Transcript::Finished,
            vec!["idle".to_owned(); 8],
        );

        lane.poll_times(&sim, 8);

        assert!(
            lane.events().is_empty(),
            "a lane that was never observed working must not stall: {:?}",
            lane.events()
        );
        assert!(!lane.settled(&sim));
    }

    #[test]
    fn sim_the_marker_preempts_the_runtime_stall() {
        let sim = SessionSim::new();
        let mut lane = sim.lane(
            "7",
            100,
            "QOL_BRIDGE_DONE_round",
            true,
            None,
            None,
            Transcript::Working,
            vec![
                "still working".to_owned(),
                "still working".to_owned(),
                FINAL_SCREEN.to_owned(),
                FINAL_SCREEN.to_owned(),
            ],
        );

        lane.poll_times(&sim, 2);
        lane.transcript(Transcript::Finished);
        lane.run(&sim);

        let events = lane.events();
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
    }

    #[test]
    fn sim_a_grouped_set_aggregates_when_one_member_mangles_its_marker() {
        let sim = SessionSim::new();
        let group = "mangled-marker-group";
        let expected = "QOL_BRIDGE_DONE_4aab033102f7a21f7322";
        let printed = "QOL_BRIDGE_DONE_4aab0331027f21a7322";
        let mut lane_a = sim.lane(
            "7",
            100,
            "QOL_BRIDGE_DONE_a",
            true,
            Some(group),
            Some("panel-a"),
            Transcript::Finished,
            vec!["report a\nQOL_BRIDGE_DONE_a".to_owned(); 4],
        );
        let mut lane_b = sim.lane(
            "8",
            200,
            "QOL_BRIDGE_DONE_b",
            true,
            Some(group),
            Some("panel-b"),
            Transcript::Finished,
            vec!["report b\nQOL_BRIDGE_DONE_b".to_owned(); 4],
        );
        let mut lane_c = sim.lane(
            "9",
            300,
            expected,
            true,
            Some(group),
            Some("panel-audit"),
            Transcript::Finished,
            vec![format!("report c\n{printed}"); 4],
        );

        lane_a.run(&sim);
        lane_b.run(&sim);
        lane_c.run(&sim);

        let events = lane_c.events();
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        let wakes = lane_c.wakes();
        assert_eq!(wakes.len(), 1, "one combined wake: {wakes:?}");
        assert!(
            wakes[0].contains("all 3 lanes finished"),
            "the combined wake must count every member: {:?}",
            wakes[0]
        );
        let combined = sim.combined(group).expect("combined report written");
        assert!(
            combined.contains("report c"),
            "the mangled-marker member must leave its fragment: {combined}"
        );
        assert!(
            combined.contains(printed),
            "the combined report must carry the mangled marker text: {combined}"
        );
    }

    #[test]
    fn sim_a_lane_that_exits_after_showing_its_marker_still_completes() {
        let sim = SessionSim::new();
        let mut lane = sim.simple_lane("7", 100, vec![FINAL_SCREEN.to_owned(); 2]);
        lane.poll(&sim);
        lane.backend.mark_gone();

        lane.run(&sim);

        let events = lane.events();
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert!(lane.settled(&sim));
    }

    #[test]
    fn sim_a_lane_that_exits_without_a_marker_reports_gone() {
        let sim = SessionSim::new();
        let mut lane = sim.simple_lane("7", 100, vec!["idle".to_owned(); 2]);
        lane.poll(&sim);
        lane.backend.mark_gone();

        lane.run(&sim);

        let events = lane.events();
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "gone");
        assert!(
            sim.pending.pending_round(&lane.binding).unwrap().is_none(),
            "a gone lane leaves no open checkpoint behind"
        );
    }

    #[test]
    fn sim_grouped_autoclose_lanes_assemble_every_member_into_one_combined_wake() {
        let sim = SessionSim::new();
        let group = "staleness-research";
        let markers = [
            "QOL_BRIDGE_DONE_a",
            "QOL_BRIDGE_DONE_b",
            "QOL_BRIDGE_DONE_c",
        ];
        let labels = ["research-a", "research-b", "research-c"];
        let mut lanes = Vec::new();
        for (index, (marker, label)) in markers.iter().zip(labels).enumerate() {
            let native = format!("{}", 7 + index);
            let screen = format!("report {label}\n{marker}");
            lanes.push(sim.lane(
                &native,
                100 + index as i32,
                marker,
                true,
                Some(group),
                Some(label),
                Transcript::Finished,
                vec![screen; 4],
            ));
        }

        for lane in lanes.iter_mut().take(2) {
            lane.run(&sim);
            assert!(
                lane.events().is_empty(),
                "an early grouped member must stay silent: {:?}",
                lane.events()
            );
            assert!(lane.wakes().is_empty());
            assert_eq!(lane.closed(), 1, "each finished grouped lane still closes");
            assert!(
                sim.combined(group).is_none(),
                "the combined file waits for every member"
            );
        }

        lanes[2].run(&sim);

        let events = lanes[2].events();
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        let wakes = lanes[2].wakes();
        assert_eq!(wakes.len(), 1, "exactly one combined wake: {wakes:?}");
        assert!(
            wakes[0].contains("all 3 lanes finished"),
            "the combined wake counts every member: {:?}",
            wakes[0]
        );
        let combined = sim.combined(group).expect("combined report written");
        for label in labels {
            assert!(
                combined.contains(&format!("report {label}")),
                "the combined report must carry {label}: {combined}"
            );
        }
        assert_eq!(
            combined.matches("## v1:fake:").count(),
            3,
            "every member gets a section: {combined}"
        );
    }

    #[test]
    fn sim_a_grouped_lane_that_dies_unreported_still_lets_the_group_finish() {
        let sim = SessionSim::new();
        let group = "research";
        let mut alive = sim.lane(
            "7",
            100,
            "QOL_BRIDGE_DONE_a",
            true,
            Some(group),
            Some("research-a"),
            Transcript::Finished,
            vec!["report a\nQOL_BRIDGE_DONE_a".to_owned(); 4],
        );
        let mut dead = sim.lane(
            "8",
            200,
            "QOL_BRIDGE_DONE_b",
            true,
            Some(group),
            Some("research-b"),
            Transcript::Working,
            vec!["still thinking".to_owned(); 4],
        );

        alive.run(&sim);
        assert!(alive.events().is_empty());
        assert!(sim.combined(group).is_none());

        dead.poll(&sim);
        dead.backend.mark_gone();
        dead.run(&sim);

        let events = dead.events();
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(
            events[0]["event"], "completed_markerless",
            "the group closes on a lane that never printed a marker"
        );
        let wakes = dead.wakes();
        assert_eq!(wakes.len(), 1, "one combined wake: {wakes:?}");
        assert!(
            wakes[0].contains("(did not complete)"),
            "the dead lane is named as unfinished: {:?}",
            wakes[0]
        );
        let combined = sim.combined(group).expect("combined report written");
        assert!(combined.contains("report a"));
        assert!(
            combined.contains("exited before it reported a completion marker"),
            "the dead lane leaves a receipt in the combined report: {combined}"
        );
    }

    #[test]
    fn sim_a_grouped_wake_is_delivered_only_once() {
        let sim = SessionSim::new();
        let group = "research";
        let mut lane = sim.lane(
            "7",
            100,
            "QOL_BRIDGE_DONE_a",
            false,
            Some(group),
            Some("research-a"),
            Transcript::Finished,
            vec!["report a\nQOL_BRIDGE_DONE_a".to_owned(); 8],
        );

        lane.run(&sim);
        assert_eq!(lane.wakes().len(), 1);

        let mut again = WatchedRound::new(
            sim.pending
                .pending_round(&lane.binding)
                .unwrap()
                .expect("the round stays open without autoclose"),
        )
        .unwrap();
        again.marker_seen = true;
        let mut out = Vec::new();
        poll_round(
            &lane.terminals,
            &lane.interpreter,
            &sim.pending,
            &sim.ledger,
            &sim.locks,
            &mut again,
            &mut out,
            sim.trace_dir(),
            fast_config(Duration::from_secs(3600)),
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(
            lane.wakes().len(),
            1,
            "a second pass must never repeat the combined wake"
        );
    }
}
