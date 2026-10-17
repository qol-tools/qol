use std::fs::File;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use qol_sessions_engine::knocks::KNOCK_HEADER;
use qol_sessions_engine::FORK_HEADER;
use qol_terminal_sessions::cli::{
    composer_holds_draft, ChatRole, ChatTurn, CliRuntimeState, CliSessionInterpreter,
    COMPACTED_SUMMARY_HEADER,
};
use qol_terminal_sessions::park::{ParkRecord, ParkState};
use qol_terminal_sessions::parking::ParkingStore;
use qol_terminal_sessions::{
    DeliveryMode, SessionBinding, SessionFacts, TerminalError, TerminalSessionService,
    TerminalSnapshot, TextInput,
};

use super::prompt::{read_tail, wake_prompt, wake_section, CLOSE_NOTE, WAKE_HEADER};
use super::resume::resume;
use super::store::ParkStore;

const POLL: Duration = Duration::from_secs(2);
const READY_POLLS_BEFORE_CLOSE: u32 = 2;
const WOKEN_UNREAD_LIMIT: u32 = 30;
const UNKNOWN_POLLS_BEFORE_RESUME: u32 = 30;
const BUSY_POLLS_BEFORE_GIVING_UP: u32 = 1800;

#[derive(Debug, PartialEq, Eq)]
pub enum CallerState {
    Gone,
    Ready,
    Busy,
}

pub enum Target {
    Live(Box<SessionFacts>),
    Gone,
    Unknown,
}

pub fn target(terminals: &TerminalSessionService, binding: &SessionBinding) -> Target {
    match terminals.snapshot() {
        Ok(snapshot) => target_in(&snapshot, binding),
        Err(_) => Target::Unknown,
    }
}

/// A session missing from a snapshot that could not list every terminal is unknown, not gone.
fn target_in(snapshot: &TerminalSnapshot, binding: &SessionBinding) -> Target {
    match snapshot
        .sessions()
        .iter()
        .find(|session| session.id == *binding.session_id())
    {
        Some(facts) if facts.binding().as_ref() == Ok(binding) => {
            Target::Live(Box::new(facts.clone()))
        }
        Some(_) => Target::Gone,
        None if snapshot.covers(binding.session_id()) => Target::Gone,
        None => Target::Unknown,
    }
}

pub enum Holder {
    Live(SessionBinding),
    Unknown,
    None,
}

/// The live session that holds the parked conversation: the caller, a session a park resumed,
/// or any session running it.
pub fn holder(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    store: &ParkStore,
    record: &ParkRecord,
    caller: &SessionBinding,
) -> Holder {
    let Ok(snapshot) = terminals.snapshot() else {
        return Holder::Unknown;
    };
    let resumed = store
        .list()
        .unwrap_or_default()
        .into_iter()
        .filter(|parked| parked.external_id == record.external_id)
        .filter_map(|parked| parked.resumed_session?.parse::<SessionBinding>().ok());
    let caller = (!record.caller_closed).then(|| caller.clone());
    let mut unknown = false;
    for binding in caller.into_iter().chain(resumed) {
        match target_in(&snapshot, &binding) {
            Target::Live(_) => return Holder::Live(binding),
            Target::Unknown => unknown = true,
            Target::Gone => {}
        }
    }
    let running = snapshot
        .sessions()
        .iter()
        .filter(|facts| {
            interpreter.describe(facts).external_id.as_deref() == Some(record.external_id.as_str())
        })
        .find_map(|facts| facts.binding().ok());
    match running {
        Some(binding) => Holder::Live(binding),
        None if unknown => Holder::Unknown,
        None => Holder::None,
    }
}

pub fn caller_state(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    caller: &SessionBinding,
) -> CallerState {
    match target(terminals, caller) {
        Target::Live(facts)
            if interpreter.describe(&facts).evidence.runtime == CliRuntimeState::Ready =>
        {
            CallerState::Ready
        }
        Target::Live(_) | Target::Unknown => CallerState::Busy,
        Target::Gone => CallerState::Gone,
    }
}

#[derive(Debug, Default)]
struct CloseGate {
    ready_polls: u32,
}

impl CloseGate {
    fn observe(&mut self, state: &CallerState) -> bool {
        match state {
            CallerState::Ready => {
                self.ready_polls += 1;
                self.ready_polls >= READY_POLLS_BEFORE_CLOSE
            }
            CallerState::Busy | CallerState::Gone => {
                self.ready_polls = 0;
                false
            }
        }
    }
}

/// Holds a delivery until the conversation's finished results stop changing for one poll,
/// so results that finish together go out together.
#[derive(Debug, Default)]
struct SettleGate {
    ids: Vec<String>,
}

impl SettleGate {
    fn settled(&mut self, batch: &[ParkRecord]) -> bool {
        let ids = batch
            .iter()
            .map(|parked| parked.id.clone())
            .collect::<Vec<_>>();
        let settled = ids == self.ids;
        self.ids = ids;
        settled
    }
}

pub fn run_parked(store: &ParkStore, id: &str) -> Result<()> {
    let mut record = store.load(id)?;
    if store.is_claimed(id) {
        return Ok(());
    }
    record.runner_pid = Some(std::process::id());
    record.runner_identity = qol_process::process_identity(std::process::id()).ok();
    store.record(&record)?;
    let terminals = TerminalSessionService::system();
    let interpreter = CliSessionInterpreter::system();
    let caller = record
        .session
        .parse::<SessionBinding>()
        .map_err(|error| anyhow!("invalid parked session token: {error}"))?;
    let log_path = store.log_path(id);
    let exit = match start_command(&record, &log_path) {
        Ok(mut child) => {
            let mut gate = CloseGate::default();
            loop {
                if store.is_claimed(id) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(());
                }
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) => {}
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(anyhow!(error).context("failed to poll the parked command"));
                    }
                }
                if !record.caller_closed {
                    let state = caller_state(&terminals, &interpreter, &caller);
                    if state == CallerState::Gone {
                        record.caller_closed = true;
                        let _ = store.record(&record);
                    } else if gate.observe(&state)
                        && parking_on(&record)
                        && matches!(
                            interpreter.keep_open_reason_for(&terminals, &caller),
                            Ok(None)
                        )
                    {
                        let _ = terminals.close(&caller);
                        record.caller_closed = true;
                        let _ = store.record(&record);
                        qol_runtime::probe!("CLI_SESSION_PARK", "event=caller_closed id={}", id);
                    }
                }
                thread::sleep(POLL);
            }
        }
        Err(error) => Err(error),
    };
    record.exit_code = exit.as_ref().ok().and_then(ExitStatus::code);
    let tail = read_tail(&log_path);
    record.wake = Some(wake_section(&record, &exit, &tail, &log_path));
    record.state = ParkState::Exited;
    store.record(&record)?;
    if deliver(&terminals, &interpreter, store, &mut record, &caller)? {
        watch_woken(&terminals, &interpreter, store, &mut record)?;
    }
    Ok(())
}

fn parking_on(record: &ParkRecord) -> bool {
    ParkingStore::system().is_on(&record.external_id, record.closes_terminal)
}

fn start_command(record: &ParkRecord, log_path: &Path) -> Result<std::process::Child> {
    let (program, args) = record
        .command
        .split_first()
        .ok_or_else(|| anyhow!("the park record has no command"))?;
    let log = File::create(log_path).context("failed to create the park log")?;
    Command::new(program)
        .args(args)
        .current_dir(&record.cwd)
        .stdin(Stdio::null())
        .stdout(log.try_clone().context("failed to share the park log")?)
        .stderr(log)
        .spawn()
        .with_context(|| format!("failed to start `{program}`"))
}

/// Delivers every finished result of this conversation to the one session that holds it once its
/// prompt is free, or resumes the conversation once when no session holds it. Each attempt holds
/// the conversation lock; the wait between attempts does not. Returns whether this runner resumed
/// it and so watches the woken session.
fn deliver(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    store: &ParkStore,
    record: &mut ParkRecord,
    caller: &SessionBinding,
) -> Result<bool> {
    let mut unknown = 0;
    let mut busy = 0;
    let mut settle = SettleGate::default();
    loop {
        {
            let _turn = store.lock_conversation(&record.external_id)?;
            let batch = store.exited(&record.external_id);
            if !batch.iter().any(|parked| parked.id == record.id) {
                return Ok(false);
            }
            if settle.settled(&batch) {
                let prompt = wake_prompt(
                    &batch
                        .iter()
                        .filter_map(|parked| parked.wake.clone())
                        .collect::<Vec<_>>(),
                );
                match holder(terminals, interpreter, store, record, caller) {
                    Holder::Live(binding) => {
                        unknown = 0;
                        if prompt_free(terminals, interpreter, &binding)
                            && terminals
                                .send_text(&binding, &prompt, DeliveryMode::Submit)
                                .is_ok()
                        {
                            settle_batch(store, &batch, ParkState::Delivered, &binding, None);
                            qol_runtime::probe!(
                                "CLI_SESSION_PARK",
                                "event=delivered id={} batch={} session={}",
                                record.id,
                                batch.len(),
                                binding
                            );
                            return Ok(false);
                        }
                        busy += 1;
                        if busy >= BUSY_POLLS_BEFORE_GIVING_UP {
                            let detail = format!("`{binding}` stayed busy or held a draft for an hour, so the result was not submitted; it is in the park log");
                            settle_batch(store, &batch, ParkState::Failed, &binding, Some(&detail));
                            return Ok(false);
                        }
                    }
                    Holder::Unknown if unknown < UNKNOWN_POLLS_BEFORE_RESUME => unknown += 1,
                    Holder::Unknown | Holder::None => {
                        return resume_batch(terminals, interpreter, store, record, &batch, &prompt)
                    }
                }
            }
        }
        thread::sleep(POLL);
    }
}

/// Settles a batch the session `binding` holds; a failed batch names no session, so the user is
/// told and can retry it.
fn settle_batch(
    store: &ParkStore,
    batch: &[ParkRecord],
    state: ParkState,
    binding: &SessionBinding,
    detail: Option<&str>,
) {
    for parked in batch {
        let settled = ParkRecord {
            state,
            runner_pid: None,
            resumed_session: (state == ParkState::Delivered).then(|| binding.to_string()),
            detail: detail.map(str::to_owned).or_else(|| parked.detail.clone()),
            ..parked.clone()
        };
        if let Err(error) = store.record(&settled) {
            log::warn!("[cli-sessions] park {} not settled: {error:#}", parked.id);
        }
    }
}

pub fn prompt_free(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
) -> bool {
    caller_state(terminals, interpreter, binding) == CallerState::Ready
        && match composer_holds_draft(terminals, binding) {
            Ok(draft) => !draft,
            Err(TerminalError::Unsupported { .. }) => true,
            Err(_) => false,
        }
}

fn resume_batch(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    store: &ParkStore,
    record: &mut ParkRecord,
    batch: &[ParkRecord],
    prompt: &str,
) -> Result<bool> {
    if !store.claim_resume(&record.id)? {
        return Ok(false);
    }
    let prompt = if parking_on(record) {
        format!("{prompt}{CLOSE_NOTE}")
    } else {
        prompt.to_owned()
    };
    match resume(terminals, interpreter, store, record, &prompt) {
        Ok(session) => {
            record.state = ParkState::Resumed;
            record.resumed_session = Some(session);
            record.runner_pid = Some(std::process::id());
        }
        Err(error) => {
            record.state = ParkState::Failed;
            record.runner_pid = None;
            record.detail = Some(format!("{error:#}"));
            store.release(&record.id);
        }
    }
    store.record(record)?;
    if let Some(session) = record
        .resumed_session
        .as_deref()
        .and_then(|token| token.parse::<SessionBinding>().ok())
    {
        let siblings = batch
            .iter()
            .filter(|parked| parked.id != record.id)
            .cloned()
            .collect::<Vec<_>>();
        settle_batch(store, &siblings, ParkState::Delivered, &session, None);
    }
    qol_runtime::probe!(
        "CLI_SESSION_PARK",
        "event=woke id={} batch={} state={:?} session={}",
        record.id,
        batch.len(),
        record.state,
        record.resumed_session.as_deref().unwrap_or("-")
    );
    Ok(record.state == ParkState::Resumed)
}

fn woken_turns(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    woken: &SessionBinding,
) -> Vec<ChatTurn> {
    match target(terminals, woken) {
        Target::Live(facts) => interpreter.chat_transcript(&facts).unwrap_or_default(),
        Target::Gone | Target::Unknown => Vec::new(),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum WokenTurn {
    Working,
    Engaged,
    Finished(String),
}

fn opens_watched_turn(turn: &ChatTurn) -> bool {
    turn.role == ChatRole::User
        && (turn.text.contains(WAKE_HEADER) || turn.text.starts_with(FORK_HEADER))
}

fn written_by_the_user(turn: &ChatTurn) -> bool {
    turn.role == ChatRole::User
        && !turn.text.starts_with(KNOCK_HEADER)
        && !turn.text.starts_with(COMPACTED_SUMMARY_HEADER)
}

fn woken_turn(turns: &[ChatTurn]) -> WokenTurn {
    let Some(start) = turns.iter().rposition(opens_watched_turn) else {
        return WokenTurn::Working;
    };
    let after = &turns[start + 1..];
    if after.iter().any(written_by_the_user) {
        return WokenTurn::Engaged;
    }
    match after.last() {
        Some(turn) if turn.role == ChatRole::Assistant && !turn.text.trim().is_empty() => {
            WokenTurn::Finished(turn.text.trim().to_owned())
        }
        _ => WokenTurn::Working,
    }
}

pub fn watch_woken(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    store: &ParkStore,
    record: &mut ParkRecord,
) -> Result<()> {
    let Some(token) = record.resumed_session.clone() else {
        return Ok(());
    };
    let woken = token
        .parse::<SessionBinding>()
        .map_err(|error| anyhow!("invalid woken session token: {error}"))?;
    let mut gate = CloseGate::default();
    let mut unread = 0;
    let outcome = loop {
        thread::sleep(POLL);
        let state = caller_state(terminals, interpreter, &woken);
        if state == CallerState::Gone {
            break "gone";
        }
        if store.parks_again(&token) {
            break "parked_again";
        }
        if !gate.observe(&state) {
            continue;
        }
        let delivered = store.delivered_into(&token);
        match woken_turn(&woken_turns(terminals, interpreter, &woken)) {
            WokenTurn::Working => {
                unread += 1;
                if unread >= WOKEN_UNREAD_LIMIT {
                    break "unread";
                }
            }
            WokenTurn::Engaged => break "engaged",
            WokenTurn::Finished(report) if !parking_on(record) => {
                record.state = ParkState::Finished;
                record.report = Some(report);
                break "kept_open";
            }
            WokenTurn::Finished(report) => {
                match interpreter.keep_open_reason_for(terminals, &woken) {
                    Err(_) => continue,
                    Ok(Some(_)) => {
                        record.state = ParkState::Finished;
                        record.report = Some(report);
                        break "kept_open";
                    }
                    Ok(None) => {
                        let Some(_turn) = store.try_lock_conversation(&record.external_id)? else {
                            continue;
                        };
                        if !store.exited(&record.external_id).is_empty()
                            || store.delivered_into(&token) != delivered
                            || woken_turn(&woken_turns(terminals, interpreter, &woken))
                                != WokenTurn::Finished(report.clone())
                        {
                            continue;
                        }
                        let _ = terminals.close(&woken);
                        record.state = ParkState::Finished;
                        record.report = Some(report);
                        break "closed";
                    }
                }
            }
        }
    };
    record.runner_pid = None;
    store.record(record)?;
    qol_runtime::probe!(
        "CLI_SESSION_PARK",
        "event=woken_{} id={} session={}",
        outcome,
        record.id,
        token
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: ChatRole, text: &str) -> ChatTurn {
        ChatTurn {
            role,
            text: text.to_owned(),
        }
    }

    #[test]
    fn woken_turn_reports_the_reply_after_the_latest_wake() {
        let wake = format!("{WAKE_HEADER}\nYou parked this conversation.");
        let cases = [
            (vec![turn(ChatRole::User, "earlier")], WokenTurn::Working),
            (vec![turn(ChatRole::User, &wake)], WokenTurn::Working),
            (
                vec![
                    turn(ChatRole::User, &wake),
                    turn(ChatRole::Assistant, "first reply"),
                    turn(ChatRole::User, &wake),
                    turn(ChatRole::Assistant, "PR merged.\n\nRestart the tray."),
                ],
                WokenTurn::Finished("PR merged.\n\nRestart the tray.".to_owned()),
            ),
            (
                vec![
                    turn(ChatRole::User, &wake),
                    turn(ChatRole::Assistant, "Checking the PR"),
                    turn(ChatRole::Assistant, "PR merged."),
                ],
                WokenTurn::Finished("PR merged.".to_owned()),
            ),
            (
                vec![
                    turn(ChatRole::User, &wake),
                    turn(ChatRole::Assistant, "PR merged."),
                    turn(ChatRole::User, "prune it now"),
                    turn(ChatRole::Assistant, "Pruned."),
                ],
                WokenTurn::Engaged,
            ),
            (
                vec![
                    turn(ChatRole::User, &wake),
                    turn(ChatRole::User, "prune it now"),
                    turn(ChatRole::Assistant, "Pruned."),
                ],
                WokenTurn::Engaged,
            ),
        ];
        for (turns, expected) in cases {
            assert_eq!(woken_turn(&turns), expected, "{turns:?}");
        }
    }

    #[test]
    fn a_fork_turn_finishes_through_knocks_and_compaction_but_not_a_user_reply() {
        let launch = format!("{FORK_HEADER}\nYou are a detached architect.");
        let knock =
            format!("{KNOCK_HEADER} 1 new message from switchboard. Read it with session_inbox.");
        let summary = format!("{COMPACTED_SUMMARY_HEADER}\nEarlier work.");
        let cases = [
            (vec![turn(ChatRole::User, &launch)], WokenTurn::Working),
            (
                vec![
                    turn(ChatRole::User, &launch),
                    turn(ChatRole::Assistant, "Opened PR 180."),
                    turn(ChatRole::User, &summary),
                    turn(ChatRole::Assistant, "PR 180 merged."),
                ],
                WokenTurn::Finished("PR 180 merged.".to_owned()),
            ),
            (
                vec![
                    turn(ChatRole::User, &launch),
                    turn(ChatRole::Assistant, "Done."),
                    turn(ChatRole::User, &knock),
                ],
                WokenTurn::Working,
            ),
            (
                vec![
                    turn(ChatRole::User, &launch),
                    turn(ChatRole::Assistant, "Done."),
                    turn(ChatRole::User, &knock),
                    turn(ChatRole::Assistant, "Answered the switchboard."),
                ],
                WokenTurn::Finished("Answered the switchboard.".to_owned()),
            ),
            (
                vec![
                    turn(ChatRole::User, &launch),
                    turn(ChatRole::Assistant, "Done."),
                    turn(ChatRole::User, "also bump the version"),
                    turn(ChatRole::Assistant, "Bumped."),
                ],
                WokenTurn::Engaged,
            ),
            (
                vec![
                    turn(ChatRole::User, &launch),
                    turn(ChatRole::User, "also bump the version"),
                    turn(ChatRole::Assistant, "Bumped."),
                ],
                WokenTurn::Engaged,
            ),
            (
                vec![
                    turn(ChatRole::User, &launch),
                    turn(ChatRole::Assistant, "Done."),
                    turn(ChatRole::User, &knock),
                    turn(ChatRole::User, "also bump the version"),
                    turn(ChatRole::Assistant, "Bumped."),
                ],
                WokenTurn::Engaged,
            ),
            (
                vec![
                    turn(ChatRole::User, &launch),
                    turn(ChatRole::Assistant, "Opened PR 180."),
                    turn(ChatRole::User, &summary),
                    turn(ChatRole::User, "also bump the version"),
                    turn(ChatRole::Assistant, "Bumped."),
                ],
                WokenTurn::Engaged,
            ),
            (
                vec![
                    turn(
                        ChatRole::User,
                        "write a fork prompt that starts with [qol session fork]",
                    ),
                    turn(ChatRole::Assistant, "Here it is."),
                ],
                WokenTurn::Working,
            ),
        ];
        for (turns, expected) in cases {
            assert_eq!(woken_turn(&turns), expected, "{turns:?}");
        }
    }

    #[test]
    fn a_batch_settles_once_it_is_unchanged_for_one_poll() {
        let parked = |id: &str| ParkRecord {
            id: id.to_owned(),
            ..crate::park::tests::record(ParkState::Exited)
        };
        let mut gate = SettleGate::default();
        assert!(!gate.settled(&[parked("a")]));
        assert!(!gate.settled(&[parked("a"), parked("b")]));
        assert!(gate.settled(&[parked("a"), parked("b")]));
        assert!(!gate.settled(&[parked("b")]));
    }

    #[test]
    fn close_gate_closes_only_after_consecutive_ready_polls() {
        let mut gate = CloseGate::default();
        assert!(!gate.observe(&CallerState::Ready));
        assert!(!gate.observe(&CallerState::Busy));
        assert!(!gate.observe(&CallerState::Ready));
        assert!(gate.observe(&CallerState::Ready));
    }
}
