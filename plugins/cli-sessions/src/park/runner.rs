use std::fs::File;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use qol_terminal_sessions::cli::{ChatRole, ChatTurn, CliRuntimeState, CliSessionInterpreter};
use qol_terminal_sessions::park::{ParkRecord, ParkState};
use qol_terminal_sessions::pin::PinStore;
use qol_terminal_sessions::{
    DeliveryMode, SessionBinding, SessionFacts, TerminalSessionService, TextInput,
};

use super::prompt::{read_tail, wake_prompt, CLOSE_NOTE, WAKE_HEADER};
use super::resume::resume;
use super::store::ParkStore;

const POLL: Duration = Duration::from_secs(2);
const READY_POLLS_BEFORE_CLOSE: u32 = 2;
const WOKEN_UNREAD_LIMIT: u32 = 30;

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
    let Ok(sessions) = terminals.snapshot() else {
        return Target::Unknown;
    };
    match sessions
        .sessions()
        .iter()
        .find(|session| session.id == *binding.session_id())
    {
        Some(facts) if facts.binding().as_ref() == Ok(binding) => {
            Target::Live(Box::new(facts.clone()))
        }
        _ => Target::Gone,
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
    let prompt = wake_prompt(&record, &exit, &tail, &log_path);
    wake(
        &terminals,
        &interpreter,
        store,
        &mut record,
        &caller,
        &prompt,
    )
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

fn wake(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    store: &ParkStore,
    record: &mut ParkRecord,
    caller: &SessionBinding,
    prompt: &str,
) -> Result<()> {
    record.runner_pid = None;
    if !record.caller_closed
        && caller_state(terminals, interpreter, caller) != CallerState::Gone
        && terminals
            .send_text(caller, prompt, DeliveryMode::Submit)
            .is_ok()
    {
        record.state = ParkState::Delivered;
        store.record(record)?;
        qol_runtime::probe!("CLI_SESSION_PARK", "event=delivered id={}", record.id);
        return Ok(());
    }
    if !store.claim_resume(&record.id)? {
        return Ok(());
    }
    match resume(
        terminals,
        interpreter,
        record,
        &if PinStore::system().is_pinned(&record.external_id) {
            prompt.to_owned()
        } else {
            format!("{prompt}{CLOSE_NOTE}")
        },
    ) {
        Ok(session) => {
            record.state = ParkState::Resumed;
            record.resumed_session = Some(session);
            record.runner_pid = Some(std::process::id());
        }
        Err(error) => {
            record.state = ParkState::Failed;
            record.detail = Some(format!("{error:#}"));
            store.release(&record.id);
        }
    }
    store.record(record)?;
    qol_runtime::probe!(
        "CLI_SESSION_PARK",
        "event=woke id={} state={:?} session={}",
        record.id,
        record.state,
        record.resumed_session.as_deref().unwrap_or("-")
    );
    if record.state == ParkState::Resumed {
        watch_woken(terminals, interpreter, store, record)?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum WokenTurn {
    Working,
    Engaged,
    Finished(String),
}

fn woken_turn(turns: &[ChatTurn]) -> WokenTurn {
    let Some(wake) = turns
        .iter()
        .rposition(|turn| turn.role == ChatRole::User && turn.text.contains(WAKE_HEADER))
    else {
        return WokenTurn::Working;
    };
    let after = &turns[wake + 1..];
    if after.iter().any(|turn| turn.role == ChatRole::User) {
        return WokenTurn::Engaged;
    }
    match after
        .iter()
        .map(|turn| turn.text.trim())
        .rfind(|text| !text.is_empty())
    {
        Some(report) => WokenTurn::Finished(report.to_owned()),
        None => WokenTurn::Working,
    }
}

fn watch_woken(
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
        let turns = match target(terminals, &woken) {
            Target::Live(facts) => interpreter.chat_transcript(&facts).unwrap_or_default(),
            Target::Gone | Target::Unknown => Vec::new(),
        };
        match woken_turn(&turns) {
            WokenTurn::Working => {
                unread += 1;
                if unread >= WOKEN_UNREAD_LIMIT {
                    break "unread";
                }
            }
            WokenTurn::Engaged => break "engaged",
            WokenTurn::Finished(report) => {
                match interpreter.keep_open_reason_for(terminals, &woken) {
                    Err(_) => continue,
                    Ok(Some(_)) => {
                        record.state = ParkState::Finished;
                        record.report = Some(report);
                        break "kept_open";
                    }
                    Ok(None) => {
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
        ];
        for (turns, expected) in cases {
            assert_eq!(woken_turn(&turns), expected, "{turns:?}");
        }
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
