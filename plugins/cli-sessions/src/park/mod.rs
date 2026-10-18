mod args;
mod prompt;
mod resume;
mod runner;
pub(crate) mod store;
mod wake;

use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::park::{ParkRecord, ParkState};
use qol_terminal_sessions::parking::{self, ParkingStore};
use qol_terminal_sessions::{
    SessionBinding, SessionFacts, SessionInventory, TerminalSessionService,
};
use serde::Serialize;

use args::ParkArgs;
pub use args::HELP;
use prompt::{kept_open_instruction, unpark_prompt, PARKED_INSTRUCTION, PARKING_OFF_REASON};
use resume::resume;
use runner::{caller_state, holder, watch_woken, CallerState, Holder};
pub(crate) use runner::{prompt_free, target, Target};
use store::ParkStore;
pub use wake::{wake, wakeable};

pub const PARK: &str = "park";
pub const LANE_EXEC: &str = "lane-exec";
pub const WAKE: &str = "wake";
const STOP_GRACE: Duration = Duration::from_secs(3);
const LAUNCH_POLL: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize)]
pub struct ParkOutcome {
    id: String,
    session: String,
    tool: String,
    external_id: String,
    cwd: String,
    command: Vec<String>,
    log: String,
    instruction: String,
}

pub enum Invocation {
    Help,
    Parked(ParkOutcome),
    Ran,
}

pub fn run(args: &[String]) -> Result<Invocation> {
    let parsed = args::parse(args)?;
    if parsed.help {
        return Ok(Invocation::Help);
    }
    let store = ParkStore::system()?;
    if let Some(id) = parsed.run.as_deref() {
        runner::run_parked(&store, id)?;
        return Ok(Invocation::Ran);
    }
    let terminals = TerminalSessionService::system();
    if let Some(token) = parsed.watch.as_deref() {
        watch_launch(
            &terminals,
            &CliSessionInterpreter::system(),
            &store,
            token,
            &parsed,
        )?;
        return Ok(Invocation::Ran);
    }
    let outcome = park(
        &terminals,
        &CliSessionInterpreter::system(),
        &store,
        &parsed,
    )?;
    Ok(Invocation::Parked(outcome))
}

pub(crate) fn calling_session(terminals: &TerminalSessionService) -> Option<SessionBinding> {
    terminals
        .discover()
        .ok()?
        .into_iter()
        .filter_map(|session| session.binding().ok())
        .find(|binding| terminals.is_current(binding).unwrap_or(false))
}

fn park(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    store: &ParkStore,
    parsed: &ParkArgs,
) -> Result<ParkOutcome> {
    let binding = match parsed.session.as_deref() {
        Some(token) => token
            .parse::<SessionBinding>()
            .map_err(|error| anyhow!("invalid session token `{token}`: {error}"))?,
        None => calling_session(terminals).context(
            "park must run inside the harness terminal it parks; no calling session was found",
        )?,
    };
    let token = binding.token();
    let facts = match target(terminals, &binding) {
        Target::Live(facts) => facts,
        Target::Gone => bail!("`{token}` is no longer present"),
        Target::Unknown => bail!("session discovery failed"),
    };
    if qol_sessions_engine::switchboard::holds_switchboard_key(&facts) {
        bail!("`{token}` is the switchboard, which is never parked");
    }
    let descriptor = interpreter.describe(&facts);
    let title = parsed
        .title
        .clone()
        .or_else(|| descriptor.display_name.clone());
    let tool = descriptor.tool.id;
    let external_id = descriptor
        .external_id
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            anyhow!("`{token}` has no resumable session id, so it cannot be parked and resumed")
        })?;
    if interpreter.resume_args_for(&tool, &external_id).is_none() {
        bail!("`{tool}` has no resume command, so its sessions cannot be parked");
    }
    let closes_terminal =
        ParkingStore::system().is_on(&external_id, parking::launched_with_parking());
    let keep_open_reason = if closes_terminal {
        interpreter.keep_open_reason(&facts)
    } else {
        Some(PARKING_OFF_REASON.to_owned())
    };
    let instruction = keep_open_reason.map_or_else(
        || PARKED_INSTRUCTION.to_owned(),
        |reason| kept_open_instruction(&reason),
    );
    let executable = std::env::current_exe()
        .context("cannot resolve the CLI Sessions executable for the park runner")?;
    let created_at = now_seconds();
    let id = format!("park-{created_at}-{}", std::process::id());
    let record = ParkRecord {
        id: id.clone(),
        tool: tool.to_string(),
        cwd: facts.cwd.clone(),
        external_id: external_id.clone(),
        model: parsed.model.clone(),
        effort: parsed.effort.clone(),
        title,
        permission_mode: interpreter.permission_mode(&facts),
        claude_config_dir: std::env::var(qol_agent_launch::account::CONFIG_DIR_ENV).ok(),
        session: token,
        command: parsed.command.clone(),
        created_at,
        state: ParkState::Waiting,
        runner_pid: None,
        runner_identity: None,
        closes_terminal,
        caller_closed: false,
        exit_code: None,
        wake: None,
        resumed_session: None,
        detail: None,
        report: None,
        notified: false,
        closed_at: None,
    };
    if let Some(identity) = facts.spawn_identity.as_ref() {
        store.record_spawn_key(&id, &identity.key)?;
    }
    store.record(&record)?;
    let mut runner = Command::new(executable);
    runner.args([PARK, "--run", &id]).current_dir(&record.cwd);
    qol_process::spawn_detached(&mut runner).context("failed to start the park runner")?;
    qol_runtime::probe!(
        "CLI_SESSION_PARK",
        "event=parked id={} tool={} session={} external_id={}",
        id,
        record.tool,
        record.session,
        external_id
    );
    Ok(ParkOutcome {
        log: store.log_path(&id).display().to_string(),
        id,
        session: record.session,
        tool: record.tool,
        external_id,
        cwd: record.cwd,
        command: record.command,
        instruction,
    })
}

fn watch_launch(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    store: &ParkStore,
    token: &str,
    parsed: &ParkArgs,
) -> Result<()> {
    let binding = token
        .parse::<SessionBinding>()
        .map_err(|error| anyhow!("invalid session token `{token}`: {error}"))?;
    if let Some(ready) = parsed.ready.as_deref() {
        std::fs::write(ready, "")
            .with_context(|| format!("failed to confirm the watch at {ready}"))?;
    }
    let Some((facts, external_id)) = launched_conversation(terminals, interpreter, &binding) else {
        bail!("the fork's terminal closed before its conversation started");
    };
    let descriptor = interpreter.describe(&facts);
    let tool = descriptor.tool.id;
    if interpreter.resume_args_for(&tool, &external_id).is_none() {
        bail!("{tool} conversations cannot be resumed, so the fork's tab stays open");
    }
    let created_at = now_seconds();
    let mut record = ParkRecord {
        id: format!("park-{created_at}-{}", std::process::id()),
        tool: tool.to_string(),
        cwd: facts.cwd.clone(),
        external_id,
        model: parsed.model.clone(),
        effort: parsed.effort.clone(),
        title: parsed.title.clone().or(descriptor.display_name),
        permission_mode: interpreter.permission_mode(&facts),
        claude_config_dir: std::env::var(qol_agent_launch::account::CONFIG_DIR_ENV).ok(),
        session: token.to_owned(),
        command: Vec::new(),
        created_at,
        state: ParkState::Resumed,
        runner_pid: Some(std::process::id()),
        runner_identity: qol_process::process_identity(std::process::id()).ok(),
        closes_terminal: true,
        caller_closed: false,
        exit_code: None,
        wake: None,
        resumed_session: Some(token.to_owned()),
        detail: None,
        report: None,
        notified: false,
        closed_at: None,
    };
    if let Some(identity) = facts.spawn_identity.as_ref() {
        store.record_spawn_key(&record.id, &identity.key)?;
    }
    store.record(&record)?;
    qol_runtime::probe!(
        "CLI_SESSION_PARK",
        "event=watching_launch id={} session={}",
        record.id,
        token
    );
    watch_woken(terminals, interpreter, store, &mut record)
}

fn launched_conversation(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
) -> Option<(Box<SessionFacts>, String)> {
    loop {
        match target(terminals, binding) {
            Target::Live(facts) => {
                let external_id = interpreter
                    .describe(&facts)
                    .external_id
                    .filter(|id| !id.trim().is_empty());
                if let Some(external_id) = external_id {
                    return Some((facts, external_id));
                }
            }
            Target::Gone => return None,
            Target::Unknown => {}
        }
        thread::sleep(LAUNCH_POLL);
    }
}

pub fn unpark(id: &str) -> Result<String> {
    let store = ParkStore::system()?;
    let Some(_lock) = store.lock_unpark(id)? else {
        bail!("`{id}` is already being resumed or reopened");
    };
    let record = store.load(id)?;
    if record.state == ParkState::Finished {
        return reopen(&store, record, "reopened from its notification");
    }
    let terminals = TerminalSessionService::system();
    let interpreter = CliSessionInterpreter::system();
    let caller = record
        .session
        .parse::<SessionBinding>()
        .map_err(|error| anyhow!("invalid parked session token: {error}"))?;
    if caller_state(&terminals, &interpreter, &caller) != CallerState::Gone {
        bail!(
            "`{id}` is still open in `{}`; continue there",
            record.session
        );
    }
    let Some(_turn) = store.try_lock_conversation(&record.external_id)? else {
        bail!("`{id}` is already being resumed");
    };
    let mut record = store.load(id)?;
    let retry = record.state == ParkState::Failed && record.resumed_session.is_none();
    if record.state != ParkState::Waiting && !retry {
        bail!("`{id}` is no longer waiting ({:?})", record.state);
    }
    ensure_not_open(&store, &terminals, &interpreter, &record, &caller)?;
    if !store.claim_resume(id)? {
        bail!("`{id}` is already being resumed");
    }
    if let (Some(pid), Some(identity)) = (record.runner_pid, record.runner_identity.as_deref()) {
        if qol_process::process_identity_matches(pid, identity) {
            qol_process::terminate_group(pid, STOP_GRACE);
        }
    }
    let prompt = unpark_prompt(&record, &store.log_path(id));
    let session = match resume(&terminals, &interpreter, &store, &record, &prompt) {
        Ok(session) => session,
        Err(error) => {
            store.release(id);
            if retry {
                record.detail = Some(format!("{error:#}"));
                record.notified = false;
                if let Err(write_error) = store.record(&record) {
                    log::warn!("failed to record the resume failure of {id}: {write_error:#}");
                }
            }
            return Err(error);
        }
    };
    record.runner_pid = None;
    record.state = ParkState::Resumed;
    record.caller_closed = true;
    record.resumed_session = Some(session.clone());
    record.detail = Some("resumed early by unpark".to_owned());
    store.record(&record)?;
    Ok(format!("resumed {id} in {session}"))
}

pub fn open(id: &str) -> Result<String> {
    let store = ParkStore::system()?;
    let Some(_lock) = store.lock_unpark(id)? else {
        bail!("`{id}` is already being resumed or reopened");
    };
    let record = store.load(id)?;
    if record.state == ParkState::Finished {
        return reopen(&store, record, "reopened from CLI Sessions");
    }
    let terminals = TerminalSessionService::system();
    let interpreter = CliSessionInterpreter::system();
    let caller = record
        .session
        .parse::<SessionBinding>()
        .map_err(|error| anyhow!("invalid parked session token: {error}"))?;
    let Some(_turn) = store.try_lock_conversation(&record.external_id)? else {
        bail!("`{id}` is already being resumed");
    };
    let record = store.load(id)?;
    if record.state != ParkState::Waiting {
        bail!("`{id}` is no longer parked ({:?})", record.state);
    }
    ensure_not_open(&store, &terminals, &interpreter, &record, &caller)?;
    let session = resume(&terminals, &interpreter, &store, &record, "")?;
    Ok(format!(
        "reopened {id} in {session}; it still waits on its command"
    ))
}

fn reopen(store: &ParkStore, record: ParkRecord, detail: &str) -> Result<String> {
    let terminals = TerminalSessionService::system();
    let interpreter = CliSessionInterpreter::system();
    let caller = record
        .session
        .parse::<SessionBinding>()
        .map_err(|error| anyhow!("invalid parked session token: {error}"))?;
    let Some(_turn) = store.try_lock_conversation(&record.external_id)? else {
        bail!("`{}` is already being resumed", record.id);
    };
    let mut record = store.load(&record.id)?;
    if record.state != ParkState::Finished {
        bail!("`{}` is no longer finished ({:?})", record.id, record.state);
    }
    ensure_not_open(store, &terminals, &interpreter, &record, &caller)?;
    let session = resume(&terminals, &interpreter, store, &record, "")?;
    record.state = ParkState::Resumed;
    record.resumed_session = Some(session.clone());
    record.closed_at = None;
    record.detail = Some(detail.to_owned());
    store.record(&record)?;
    Ok(format!("reopened {} in {session}", record.id))
}

/// Refuses to open a second copy of a conversation that a live session already holds.
fn ensure_not_open(
    store: &ParkStore,
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    record: &ParkRecord,
    caller: &SessionBinding,
) -> Result<()> {
    match holder(terminals, interpreter, store, record, caller) {
        Holder::None => Ok(()),
        Holder::Live(binding) => bail!(
            "`{}` is already open in `{binding}`; continue there",
            record.id
        ),
        Holder::Unknown => bail!(
            "could not list every terminal, so `{}` may already be open; try again",
            record.id
        ),
    }
}

pub fn list() -> Result<Vec<String>> {
    ParkStore::system()?
        .list()?
        .into_iter()
        .map(|record| {
            Ok(format!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                record.id,
                serde_json::to_value(record.state)?
                    .as_str()
                    .unwrap_or_default(),
                record.tool,
                record.cwd,
                record.command.join(" "),
                record.resumed_session.as_deref().unwrap_or("-")
            ))
        })
        .collect()
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn record(state: ParkState) -> ParkRecord {
        ParkRecord {
            id: "park-1-2".to_owned(),
            tool: "claude".to_owned(),
            cwd: "/repo".to_owned(),
            external_id: "abc".to_owned(),
            model: None,
            effort: None,
            title: None,
            permission_mode: None,
            claude_config_dir: None,
            session: "v1:kitty:k1_f1.2:3".to_owned(),
            command: vec!["node".to_owned(), "watch.cjs".to_owned(), "79".to_owned()],
            created_at: 1,
            state,
            runner_pid: None,
            runner_identity: None,
            closes_terminal: false,
            caller_closed: false,
            exit_code: None,
            wake: None,
            resumed_session: None,
            detail: None,
            report: None,
            notified: false,
            closed_at: None,
        }
    }
}
