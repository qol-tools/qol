use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;

use qol_agent_launch::account::LaneExecCommand;
use qol_terminal_sessions::cli::{launch_flags, CliSessionInterpreter, CliToolId};
use qol_terminal_sessions::park::ParkRecord;
use qol_terminal_sessions::{
    SessionId, SpawnIdentity, SpawnKey, SpawnRequest, SpawnSurface, TerminalSessionService,
};

const READY_POLL: Duration = Duration::from_millis(250);
const READY_TIMEOUT: Duration = Duration::from_secs(30);

pub fn resume(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    record: &ParkRecord,
    prompt: &str,
) -> Result<String> {
    let tool = CliToolId::new(record.tool.clone())
        .map_err(|error| anyhow!("invalid tool `{}`: {error}", record.tool))?;
    let mut launch = interpreter
        .launch_for(&tool)
        .ok_or_else(|| anyhow!("`{tool}` has no launch program"))?;
    launch.args.extend(resume_args(interpreter, &tool, record)?);
    if !prompt.is_empty() {
        launch.args.push(prompt.to_owned());
    }
    let cap = qol_agent_launch::cap::resolve_spawn_cap(qol_agent_launch::cap::config_spawn_cap()?);
    let mut launch = qol_agent_launch::cap::wrap_launch(&launch, cap.as_ref());
    qol_agent_launch::account::apply(
        &mut launch,
        &tool,
        record.claude_config_dir.as_deref().map(Path::new),
        &LaneExecCommand::current(&[crate::park::LANE_EXEC])?,
    )?;
    let identity = SpawnIdentity {
        key: SpawnKey::new(record.id.clone())
            .map_err(|error| anyhow!("park id `{}` is not a spawn key: {error}", record.id))?,
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
            record.id
        );
    }
    let request = SpawnRequest {
        identity,
        launch,
        cwd,
        title: Some(record.title.clone().unwrap_or_else(|| record.id.clone())),
    };
    let session = terminals
        .spawn_on(qol_terminal_sessions::kitty::backend_id(), &request)
        .context("the terminal refused to open the resumed session")?;
    ready_token(terminals, interpreter, &session, &tool)
}

fn resume_args(
    interpreter: &CliSessionInterpreter,
    tool: &CliToolId,
    record: &ParkRecord,
) -> Result<Vec<String>> {
    let mut args = interpreter
        .resume_args_for(tool, &record.external_id)
        .ok_or_else(|| anyhow!("`{tool}` has no resume command"))?;
    args.extend(launch_flags::resume_permission_flags(
        tool,
        record.permission_mode.as_deref(),
    ));
    args.extend(launch_flags::tier_flags(
        tool,
        record.model.as_deref(),
        record.effort.as_deref(),
    )?);
    Ok(args)
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
        let args = resume_args(&interpreter, &tool, &parked).unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("abc"), "{joined}");
        assert!(
            joined.contains("--dangerously-skip-permissions"),
            "{joined}"
        );
        assert!(joined.ends_with("--model opus-x --effort high"), "{joined}");
    }
}
