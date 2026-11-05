use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use qol_agent_homes::{Harness, Registry};
use qol_agent_launch::account::LaneExecCommand;
use qol_terminal_sessions::cli::{CliLaunchProgram, CliSessionInterpreter, CliToolId};
use serde::{Deserialize, Serialize};

use super::{live_switchboard, open_on, paths, Profile, LANE_EXEC_PREFIX, SWITCHBOARD_SURFACE};

pub(super) const AUTH: &str = "auth";
const REOPEN: &str = "--reopen";
const DEFAULT_NAME: &str = "default";
const CLOSE_SETTLE: Duration = Duration::from_millis(400);
const CLOSE_WAIT: Duration = Duration::from_secs(10);
const CLOSE_POLL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    name: String,
    config_dir: Option<PathBuf>,
    folder: PathBuf,
    names_default_folder: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    #[serde(default)]
    logged_in: bool,
    email: Option<String>,
    org_name: Option<String>,
    subscription_type: Option<String>,
}

#[derive(Debug, Serialize)]
struct Row {
    name: String,
    folder: String,
    config_dir: Option<String>,
    current: bool,
    logged_in: bool,
    email: Option<String>,
    org: Option<String>,
    plan: Option<String>,
}

#[derive(Debug, Serialize)]
struct Listing {
    current: Option<String>,
    profiles: Vec<Row>,
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    let words = args
        .iter()
        .map(|arg| {
            arg.to_str()
                .context("switchboard auth arguments must be valid UTF-8")
        })
        .collect::<Result<Vec<_>>>()?;
    match words.as_slice() {
        [] | ["--list"] => list(),
        [REOPEN] => reopen(),
        [name] if !name.starts_with('-') => switch(name),
        _ => bail!("usage: qol sessions switchboard auth [--list | <profile>]"),
    }
}

fn list() -> Result<()> {
    let registry = Registry::load();
    let current = current(&registry);
    let rows = shown(candidates(&registry), current.as_deref());
    let listing = Listing {
        current: rows
            .iter()
            .find(|row| row.current)
            .map(|row| row.name.clone()),
        profiles: rows,
    };
    crate::cli::line(serde_json::to_string(&listing)?);
    Ok(())
}

fn switch(name: &str) -> Result<()> {
    let registry = Registry::load();
    let all = candidates(&registry);
    let Some(target) = all.iter().find(|candidate| candidate.name == name) else {
        let names = all
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<Vec<_>>();
        bail!(
            "no Claude profile `{name}`; the profiles are {}",
            names.join(", ")
        );
    };
    let switchboard = super::neutral_cwd()?;
    let previous = paths::config_dir(&switchboard, None);
    if current(&registry) == target.config_dir && previous == target.config_dir {
        bail!("the switchboard already runs on `{name}`");
    }
    paths::set_recorded_config_dir(&switchboard, target.config_dir.as_deref())?;
    let mut reopen = Command::new(std::env::current_exe().context("failed to locate this binary")?);
    qol_agent_homes::scrub_claude_session_env(&mut reopen);
    reopen.args(["sessions", "switchboard", AUTH, REOPEN]);
    match &target.config_dir {
        Some(dir) => reopen.env(qol_agent_launch::account::CONFIG_DIR_ENV, dir),
        None => reopen.env_remove(qol_agent_launch::account::CONFIG_DIR_ENV),
    };
    if let Err(error) = qol_process::spawn_detached(&mut reopen) {
        paths::set_recorded_config_dir(&switchboard, previous.as_deref())?;
        return Err(error).context("failed to start the switchboard on the new profile");
    }
    crate::cli::line(serde_json::json!({ "switching": name }));
    Ok(())
}

fn reopen() -> Result<()> {
    std::thread::sleep(CLOSE_SETTLE);
    let terminals = super::super::service()?;
    let interpreter = CliSessionInterpreter::system();
    if let Some(binding) = terminals.snapshot().ok().and_then(|snapshot| {
        live_switchboard(&interpreter, snapshot.sessions()).and_then(|held| held.binding().ok())
    }) {
        terminals
            .close(&binding)
            .context("failed to close the switchboard")?;
    }
    let deadline = Instant::now() + CLOSE_WAIT;
    while !terminals
        .snapshot()
        .is_ok_and(|snapshot| live_switchboard(&interpreter, snapshot.sessions()).is_none())
    {
        if Instant::now() >= deadline {
            bail!(
                "the switchboard did not close within {}s",
                CLOSE_WAIT.as_secs()
            );
        }
        std::thread::sleep(CLOSE_POLL);
    }
    open_on(&Profile::default(), Some(SWITCHBOARD_SURFACE)).map(drop)
}

fn current(registry: &Registry) -> Option<PathBuf> {
    paths::caller_config_dir()
        .map(|dir| PathBuf::from(registry.resolve_caller(Some(dir.to_string_lossy().as_ref()))))
}

fn candidates(registry: &Registry) -> Vec<Candidate> {
    let default_folder = registry.default_for(Harness::Claude).path.clone();
    let mut found = vec![Candidate {
        name: DEFAULT_NAME.to_owned(),
        config_dir: None,
        folder: default_folder.clone(),
        names_default_folder: false,
    }];
    for home in registry.every_home() {
        if home.harness != Harness::Claude {
            continue;
        }
        let folder = home.path;
        if found
            .iter()
            .any(|seen| seen.config_dir.as_deref() == Some(folder.as_path()))
        {
            continue;
        }
        let name = unique(folder_name(&folder), &found);
        found.push(Candidate {
            name,
            config_dir: Some(folder.clone()),
            names_default_folder: folder == default_folder,
            folder,
        });
    }
    found
}

fn folder_name(folder: &Path) -> String {
    let raw = folder
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("profile");
    let bare = raw.trim_start_matches('.');
    let kept = bare.strip_prefix("claude-").unwrap_or(bare);
    let safe = kept
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    let name = safe.trim_start_matches('-');
    if name.is_empty() {
        "profile".to_owned()
    } else {
        name.to_owned()
    }
}

fn unique(name: String, found: &[Candidate]) -> String {
    if !found.iter().any(|seen| seen.name == name) {
        return name;
    }
    (2..)
        .map(|n| format!("{name}-{n}"))
        .find(|numbered| !found.iter().any(|seen| &seen.name == numbered))
        .unwrap_or(name)
}

fn shown(candidates: Vec<Candidate>, current: Option<&Path>) -> Vec<Row> {
    let lane_exec = LaneExecCommand::current(LANE_EXEC_PREFIX).ok();
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let status = lane_exec
                .as_ref()
                .and_then(|lane_exec| status(candidate.config_dir.as_deref(), lane_exec).ok())
                .unwrap_or_default();
            row(candidate, status, current)
        })
        .collect()
}

fn row(candidate: Candidate, status: Status, current: Option<&Path>) -> Option<Row> {
    let is_current = candidate.config_dir.as_deref() == current;
    if candidate.names_default_folder && !status.logged_in && !is_current {
        return None;
    }
    Some(Row {
        name: candidate.name,
        folder: candidate.folder.to_string_lossy().into_owned(),
        config_dir: candidate
            .config_dir
            .map(|dir| dir.to_string_lossy().into_owned()),
        current: is_current,
        logged_in: status.logged_in,
        email: status.email,
        org: status.org_name,
        plan: status.subscription_type,
    })
}

fn status(config_dir: Option<&Path>, lane_exec: &LaneExecCommand) -> Result<Status> {
    let tool =
        CliToolId::new("claude".to_owned()).map_err(|error| anyhow!("invalid tool: {error}"))?;
    let mut launch = CliLaunchProgram::new("claude");
    launch.args = vec!["auth".to_owned(), "status".to_owned(), "--json".to_owned()];
    qol_agent_launch::account::apply(&mut launch, &tool, config_dir, lane_exec)?;
    let mut command = Command::new(&launch.program);
    qol_agent_homes::scrub_claude_session_env(&mut command);
    let output = command
        .args(&launch.args)
        .envs(launch.env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::null())
        .output()
        .context("failed to run claude auth status")?;
    serde_json::from_slice(&output.stdout).context("claude auth status printed no status")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, dir: Option<&str>, folder: &str) -> Candidate {
        Candidate {
            name: name.to_owned(),
            config_dir: dir.map(PathBuf::from),
            folder: PathBuf::from(folder),
            names_default_folder: false,
        }
    }

    #[test]
    fn a_profile_is_named_after_its_folder_without_the_claude_prefix() {
        assert_eq!(folder_name(Path::new("/h/.claude-clauded")), "clauded");
        assert_eq!(folder_name(Path::new("/h/.claude-work")), "work");
        assert_eq!(folder_name(Path::new("/h/.claude")), "claude");
        assert_eq!(folder_name(Path::new("/h/agents/bruno")), "bruno");
        assert_eq!(folder_name(Path::new("/h/.claude-")), "profile");
        assert_eq!(folder_name(Path::new("/h/my work")), "my-work");
        assert_eq!(folder_name(Path::new("/h/--reopen")), "reopen");
        assert_eq!(folder_name(Path::new("/h/.claude-  ")), "profile");
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let found = [
            candidate("work", Some("/a"), "/a"),
            candidate("work-2", Some("/b"), "/b"),
        ];
        assert_eq!(unique("work".to_owned(), &found), "work-3");
        assert_eq!(unique("home".to_owned(), &found), "home");
    }

    #[test]
    fn the_default_folder_set_explicitly_shows_only_with_its_own_login() {
        let explicit = || Candidate {
            names_default_folder: true,
            ..candidate("claude", Some("/h/.claude"), "/h/.claude")
        };
        assert!(row(explicit(), Status::default(), None).is_none());
        let logged_in = Status {
            logged_in: true,
            ..Status::default()
        };
        assert!(row(explicit(), logged_in, None).is_some());
        assert!(row(explicit(), Status::default(), Some(Path::new("/h/.claude"))).is_some());
    }

    #[test]
    fn the_current_row_is_the_one_whose_config_dir_the_switchboard_runs_with() {
        let default = row(
            candidate(DEFAULT_NAME, None, "/h/.claude"),
            Status::default(),
            None,
        )
        .unwrap();
        assert!(default.current);
        let work = row(
            candidate("work", Some("/h/.claude-work"), "/h/.claude-work"),
            Status::default(),
            None,
        )
        .unwrap();
        assert!(!work.current);
        let current = row(
            candidate("work", Some("/h/.claude-work"), "/h/.claude-work"),
            Status::default(),
            Some(Path::new("/h/.claude-work")),
        )
        .unwrap();
        assert!(current.current);
    }

    #[test]
    fn the_auth_status_json_parses() {
        let status: Status = serde_json::from_str(
            r#"{"loggedIn":true,"authMethod":"claude.ai","email":"a@b.c","orgName":"Org","subscriptionType":"max","configDirectory":"/h/.claude"}"#,
        )
        .unwrap();
        assert!(status.logged_in);
        assert_eq!(status.email.as_deref(), Some("a@b.c"));
        assert_eq!(status.subscription_type.as_deref(), Some("max"));
    }

    #[test]
    fn auth_takes_a_list_a_profile_or_nothing() {
        let words = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(run(&words(&["-x"])).is_err());
        assert!(run(&words(&["a", "b"])).is_err());
    }
}
