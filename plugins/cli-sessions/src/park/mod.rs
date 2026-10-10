mod args;
mod prompt;
mod resume;
mod runner;
mod store;

use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::park::{ParkRecord, ParkState};
use qol_terminal_sessions::{SessionBinding, SessionInventory, TerminalSessionService};
use serde::Serialize;

use args::ParkArgs;
pub use args::HELP;
use prompt::{kept_open_instruction, unpark_prompt, PARKED_INSTRUCTION};
use resume::resume;
use runner::{caller_state, CallerState};
pub(crate) use runner::{target, Target};
use store::ParkStore;

pub const PARK: &str = "park";
pub const LANE_EXEC: &str = "lane-exec";
const STOP_GRACE: Duration = Duration::from_secs(3);

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
    let descriptor = interpreter.describe(&facts);
    let instruction = interpreter.keep_open_reason(&facts).map_or_else(
        || PARKED_INSTRUCTION.to_owned(),
        |reason| kept_open_instruction(&reason),
    );
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
        caller_closed: false,
        exit_code: None,
        resumed_session: None,
        detail: None,
        report: None,
        notified: false,
    };
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

pub fn unpark(id: &str) -> Result<String> {
    let store = ParkStore::system()?;
    let Some(_lock) = store.lock_unpark(id)? else {
        bail!("`{id}` is already being resumed or reopened");
    };
    let mut record = store.load(id)?;
    if record.state == ParkState::Finished {
        return reopen(&store, record);
    }
    let retry = record.state == ParkState::Failed && record.resumed_session.is_none();
    if record.state != ParkState::Waiting && !retry {
        bail!("`{id}` is no longer waiting ({:?})", record.state);
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
    if !store.claim_resume(id)? {
        bail!("`{id}` is already being resumed");
    }
    if let (Some(pid), Some(identity)) = (record.runner_pid, record.runner_identity.as_deref()) {
        if qol_process::process_identity_matches(pid, identity) {
            qol_process::terminate_group(pid, STOP_GRACE);
        }
    }
    let prompt = unpark_prompt(&record, &store.log_path(id));
    let session = match resume(&terminals, &interpreter, &record, &prompt) {
        Ok(session) => session,
        Err(error) => {
            store.release(id);
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

fn reopen(store: &ParkStore, mut record: ParkRecord) -> Result<String> {
    let terminals = TerminalSessionService::system();
    let session = resume(&terminals, &CliSessionInterpreter::system(), &record, "")?;
    record.state = ParkState::Resumed;
    record.resumed_session = Some(session.clone());
    record.detail = Some("reopened from its notification".to_owned());
    store.record(&record)?;
    Ok(format!("reopened {} in {session}", record.id))
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
            caller_closed: false,
            exit_code: None,
            resumed_session: None,
            detail: None,
            report: None,
            notified: false,
        }
    }
}
