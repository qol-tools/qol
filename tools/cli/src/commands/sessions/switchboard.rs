use std::ffi::OsString;

use anyhow::{anyhow, bail, Context, Result};
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::TerminalSessionService;
use serde::Serialize;

use super::agent_policy::{AgentDispatch, AssignmentRequest};
use super::fork::flag_value;
use super::spawn::{
    config_dispatch_policy, config_spawn_cap, config_surface, resolve_spawn_cap, spawn_detached,
    SpawnCapConfig, SpawnLedger, SpawnLocks,
};

pub(super) const SWITCHBOARD_KEY: &str = "switchboard";
const SWITCHBOARD_TOOL: &str = "claude";
const SWITCHBOARD_EFFORT: &str = "low";
pub(super) const SWITCHBOARD_ROLE: &str = "You are the switchboard: the user's one control chat for their terminal sessions. Your only job is starting, listing and closing other sessions with the qol-sessions tools, which you load with ToolSearch: sessions_list, session_fork and session_close. You have no file, shell or web tools, and you never investigate, explain or discuss the details of any session's work; that belongs to the session that owns it. When the user describes new work, call session_fork with a short key, a brief that keeps the user's own words, copy_chat false, and cwd set to this session's working directory unless the user names another. When the user asks for a new or empty session, call session_fork with blank true. When the user asks what is running, call sessions_list and answer with one line per session. Keep every reply to one short line.";

#[derive(Debug, Serialize)]
pub(super) struct SwitchboardOutcome {
    pub(super) session: String,
    pub(super) tool: String,
    pub(super) key: String,
    pub(super) model: String,
    pub(super) effort: String,
    pub(super) cwd: String,
    pub(super) surface: String,
    pub(super) title: String,
    pub(super) elapsed_ms: u128,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct SwitchboardArgs {
    pub(super) help: bool,
    pub(super) cwd: String,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) surface: Option<String>,
}

pub(super) fn launch_args() -> Vec<String> {
    vec![
        "--tools=ToolSearch".to_owned(),
        "--append-system-prompt".to_owned(),
        SWITCHBOARD_ROLE.to_owned(),
    ]
}

pub(super) fn launch_choice(parsed: &SwitchboardArgs) -> super::launch::LaunchChoice {
    super::launch::LaunchChoice {
        tool: Some(SWITCHBOARD_TOOL.to_owned()),
        model: parsed.model.clone(),
        effort: Some(
            parsed
                .effort
                .clone()
                .unwrap_or_else(|| SWITCHBOARD_EFFORT.to_owned()),
        ),
        surface: parsed.surface.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn launch(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    ledger: &SpawnLedger,
    locks: &SpawnLocks,
    cwd: &str,
    surface: Option<&str>,
    model: Option<&str>,
    effort: &str,
    cap: Option<&SpawnCapConfig>,
    dispatch: &AgentDispatch,
) -> Result<SwitchboardOutcome> {
    let snapshot = terminals.snapshot().context("session discovery failed")?;
    if let Some(held) = snapshot.sessions().iter().find(|facts| {
        facts
            .spawn_identity
            .as_ref()
            .is_some_and(|identity| identity.key.as_str() == SWITCHBOARD_KEY)
    }) {
        let token = held
            .binding()
            .map(|binding| binding.token())
            .unwrap_or_else(|_| held.id.to_string());
        bail!("the switchboard is already running as `{token}`");
    }
    let admission = dispatch.admit_launch(SWITCHBOARD_TOOL, model)?;
    let Some(model) = admission.model.as_deref() else {
        bail!(
            "the switchboard needs --model, a selected profile model, or spawn_model in sessions.toml so the tier is one this host may launch"
        );
    };
    let launched = spawn_detached(
        terminals,
        interpreter,
        ledger,
        locks,
        SWITCHBOARD_TOOL,
        cwd,
        SWITCHBOARD_KEY,
        surface,
        Some(model),
        Some(effort),
        Some(SWITCHBOARD_KEY),
        config_surface()?,
        cap,
        &launch_args(),
        "",
        false,
        admission.assignment.as_ref(),
        None,
        false,
    )?;
    Ok(SwitchboardOutcome {
        session: launched.session,
        tool: SWITCHBOARD_TOOL.to_owned(),
        key: SWITCHBOARD_KEY.to_owned(),
        model: model.to_owned(),
        effort: effort.to_owned(),
        cwd: launched.cwd,
        surface: launched.surface.to_owned(),
        title: launched.title,
        elapsed_ms: launched.elapsed_ms,
    })
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    let parsed = parse_args(args)?;
    if parsed.help {
        println!("{}", help());
        return Ok(());
    }
    let dispatch = AgentDispatch::new(config_dispatch_policy()?, AssignmentRequest::default());
    let terminals = super::service()?;
    let interpreter = CliSessionInterpreter::system();
    let parent = Some(super::bridge::driver_token(&terminals)).filter(|token| !token.is_empty());
    let defaults = super::spawn::config_launch_defaults()?;
    let mut choice = launch_choice(&parsed);
    super::launch::inherit_model(
        &mut choice,
        super::fork::parent_model(&terminals, &interpreter, parent.as_deref()),
        &defaults,
        &dispatch,
    );
    let resolved = super::launch::resolve(
        super::launch::LaunchKind::Fork,
        choice,
        &defaults,
        &dispatch,
    )?;
    let outcome = launch(
        &terminals,
        &interpreter,
        &SpawnLedger::system()?,
        &SpawnLocks::system()?,
        &parsed.cwd,
        resolved.surface.as_deref(),
        resolved.model.as_deref(),
        resolved.effort.as_deref().unwrap_or(SWITCHBOARD_EFFORT),
        resolve_spawn_cap(config_spawn_cap()?).as_ref(),
        &dispatch,
    )?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}

pub(super) fn help() -> String {
    "qol sessions switchboard --cwd PATH [--model MODEL] [--effort LEVEL] [--surface tab|os-window]\n\nLaunch the switchboard: your one control chat for terminal sessions. It is a claude session keyed and titled `switchboard` whose only tools are the qol-sessions MCP tools (sessions_list, session_fork, session_close), loaded through ToolSearch. It starts blank, so you type its first message, and it starts, lists and closes other sessions for you without doing their work.\n\nOnly one switchboard runs at a time: a live session already holding the `switchboard` key is refused. The switchboard is not recorded as a fork and never parks.\n--model resolves as it does for a fork: an explicit value wins, then the model the calling session is running when tool_models and allowed_models admit it, then fork_model, then spawn_model in sessions.toml.\n--effort defaults to low.\n--surface defaults to fork_surface, then spawn_surface in sessions.toml, then tab.".to_owned()
}

pub(super) fn parse_args(args: &[OsString]) -> Result<SwitchboardArgs> {
    let mut parsed = SwitchboardArgs::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index]
            .to_str()
            .ok_or_else(|| anyhow!("switchboard arguments must be valid UTF-8"))?;
        match flag {
            "help" | "--help" | "-h" => {
                parsed.help = true;
                return Ok(parsed);
            }
            "--cwd" => parsed.cwd = flag_value(args, &mut index, "--cwd")?,
            "--model" => parsed.model = Some(flag_value(args, &mut index, "--model")?),
            "--effort" => parsed.effort = Some(flag_value(args, &mut index, "--effort")?),
            "--surface" => parsed.surface = Some(flag_value(args, &mut index, "--surface")?),
            other => bail!("unknown switchboard flag `{other}`\n\n{}", help()),
        }
        index += 1;
    }
    if parsed.cwd.is_empty() {
        bail!("switchboard requires --cwd\n\n{}", help());
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn switchboard_effort_defaults_to_low_over_fork_effort() {
        let defaults = super::super::launch::LaunchDefaults {
            fork_effort: Some("high".to_owned()),
            ..super::super::launch::LaunchDefaults::default()
        };
        let parsed = parse_args(&args(&["--cwd", "/work", "--model", "opus"])).unwrap();
        let launch = super::super::launch::resolve(
            super::super::launch::LaunchKind::Fork,
            launch_choice(&parsed),
            &defaults,
            &AgentDispatch::unconfigured(),
        )
        .unwrap();
        assert_eq!(launch.tool, "claude");
        assert_eq!(launch.effort.as_deref(), Some("low"));

        let parsed = parse_args(&args(&["--cwd", "/work", "--effort", "medium"])).unwrap();
        assert_eq!(launch_choice(&parsed).effort.as_deref(), Some("medium"));
    }

    #[test]
    fn switchboard_args_require_cwd_and_refuse_unknown_flags() {
        let error = parse_args(&args(&["--model", "opus"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--cwd"), "{error}");
        let error = parse_args(&args(&["--cwd", "/work", "--brief", "b"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--brief"), "{error}");
    }
}
