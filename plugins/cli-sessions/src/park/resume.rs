use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;

use qol_agent_launch::account::LaneExecCommand;
use qol_agent_launch::cap::SpawnCapConfig;
use qol_sessions_engine::resume::{resume_launch, ResumePoint};
use qol_terminal_sessions::cli::{CliLaunchProgram, CliSessionInterpreter, CliToolId};
use qol_terminal_sessions::park::ParkRecord;
use qol_terminal_sessions::{
    SessionId, SpawnIdentity, SpawnKey, SpawnRequest, SpawnSurface, TerminalSessionService,
};

use super::store::ParkStore;

const READY_POLL: Duration = Duration::from_millis(250);
const READY_TIMEOUT: Duration = Duration::from_secs(30);

pub fn resume(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    store: &ParkStore,
    record: &ParkRecord,
    prompt: &str,
) -> Result<String> {
    let tool = CliToolId::new(record.tool.clone())
        .map_err(|error| anyhow!("invalid tool `{}`: {error}", record.tool))?;
    let cap = qol_agent_launch::cap::resolve_spawn_cap(qol_agent_launch::cap::config_spawn_cap()?);
    let mut launch = resume_program(interpreter, &tool, record, cap.as_ref())?;
    if !prompt.is_empty() {
        launch.args.push(prompt.to_owned());
    }
    launch.env.push(qol_terminal_sessions::parking::launch_env(
        record.closes_terminal,
    ));
    let identity = SpawnIdentity {
        key: resumed_key(record, store.spawn_key(&record.id))?,
        tool: tool.clone(),
        surface: qol_agent_launch::surface::config_surface()?.unwrap_or(SpawnSurface::Tab),
    };
    let cwd = PathBuf::from(&record.cwd);
    if !cwd.is_dir() {
        bail!("the parked directory `{}` no longer exists", record.cwd);
    }
    let held = terminals
        .snapshot()
        .context("session discovery failed before the resumed session started")?
        .sessions()
        .iter()
        .any(|facts| {
            facts.spawn_identity.as_ref().map(|tagged| &tagged.key) == Some(&identity.key)
        });
    if held {
        bail!(
            "`{}` is already held by a live session; continue there",
            identity.key
        );
    }
    let request = SpawnRequest {
        identity,
        launch,
        cwd,
        title: Some(record.title.clone().unwrap_or_else(|| record.id.clone())),
    };
    let session = terminals
        .spawn(&request)
        .context("the terminal refused to open the resumed session")?;
    ready_token(terminals, interpreter, &session, &tool)
}

fn resumed_key(record: &ParkRecord, kept: Option<SpawnKey>) -> Result<SpawnKey> {
    match kept {
        Some(key) => Ok(key),
        None => SpawnKey::new(record.id.clone())
            .map_err(|error| anyhow!("park id `{}` is not a spawn key: {error}", record.id)),
    }
}

fn resume_program(
    interpreter: &CliSessionInterpreter,
    tool: &CliToolId,
    record: &ParkRecord,
    cap: Option<&SpawnCapConfig>,
) -> Result<CliLaunchProgram> {
    resume_launch(
        interpreter,
        &ResumePoint {
            tool,
            external_id: &record.external_id,
            permission_mode: record.permission_mode.as_deref(),
            model: record.model.as_deref(),
            effort: record.effort.as_deref(),
            claude_config_dir: record.claude_config_dir.as_deref().map(Path::new),
            remote_control: None,
        },
        cap,
        &LaneExecCommand::current(&[crate::park::LANE_EXEC])?,
    )
}

fn ready_token(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    session: &SessionId,
    tool: &CliToolId,
) -> Result<String> {
    let started = Instant::now();
    let mut appeared = false;
    loop {
        let snapshot = terminals
            .snapshot()
            .context("session discovery failed while the resumed session started")?;
        match snapshot
            .sessions()
            .iter()
            .find(|facts| facts.id == *session)
        {
            Some(facts) => {
                appeared = true;
                if let Ok(binding) = facts.binding() {
                    if interpreter.describe(facts).tool.id == *tool {
                        return Ok(binding.token());
                    }
                }
            }
            None if appeared => {
                bail!("the resumed session `{session}` exited before `{tool}` was running in it")
            }
            None => {}
        }
        if started.elapsed() >= READY_TIMEOUT {
            bail!(
                "the resumed session `{session}` was not running `{tool}` within {}s",
                READY_TIMEOUT.as_secs()
            );
        }
        thread::sleep(READY_POLL);
    }
}

#[cfg(test)]
mod tests {
    use qol_terminal_sessions::park::ParkState;

    use super::*;
    use crate::park::tests::record;

    #[test]
    fn resume_args_keep_the_permission_mode_and_the_requested_tier() {
        let interpreter = CliSessionInterpreter::system();
        let tool = CliToolId::new("claude").unwrap();
        let mut parked = record(ParkState::Waiting);
        parked.permission_mode = Some("bypassPermissions".to_owned());
        parked.model = Some("opus-x".to_owned());
        parked.effort = Some("high".to_owned());
        let launch = resume_program(&interpreter, &tool, &parked, None).unwrap();
        assert_eq!(launch.program, "claude");
        let joined = launch.args.join(" ");
        assert!(joined.contains("abc"), "{joined}");
        assert!(
            joined.contains("--dangerously-skip-permissions"),
            "{joined}"
        );
        assert!(joined.ends_with("--model opus-x --effort high"), "{joined}");
    }

    #[test]
    fn a_resumed_session_keeps_its_spawn_key_so_its_mail_still_reaches_it() {
        let root = tempfile::TempDir::new().unwrap();
        let store = ParkStore::with_dir(root.path().join("parked"));
        let parked = record(ParkState::Waiting);
        assert_eq!(
            resumed_key(&parked, store.spawn_key(&parked.id)).unwrap(),
            SpawnKey::new("park-1-2").unwrap(),
            "a session spawned without a key is keyed by its park"
        );
        store
            .record_spawn_key(&parked.id, &SpawnKey::new("lane-7").unwrap())
            .unwrap();
        assert_eq!(
            resumed_key(&parked, store.spawn_key(&parked.id)).unwrap(),
            SpawnKey::new("lane-7").unwrap()
        );
    }
}
