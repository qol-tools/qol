use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use qol_platform::windows_path::strip_verbatim;
use serde::Deserialize;
use serde_json::json;

use super::agents::admit_model;
use super::context;
use super::paths::{claude_home, switchboard_dir};
use super::plugin;
use super::resume::FORK_AGENT;

pub(super) const PRE_AGENT: &str = "pre-agent";
pub(super) const FORK_CONTEXT: &str = "fork-context";
pub(super) const AGENT_ROWS: &str = "agent-rows";
pub(super) const ROLE_REFRESH: &str = "role-refresh";
pub(super) const ROLE_REFRESH_SLOTS: usize = 3;
pub(super) const ROLE_REFRESH_SOURCES: &str = "resume|compact";
const COMPACT_BOUNDARY: &str = "\"compact_boundary\"";
const ROLE_REFRESH_LEAD: &str = "The switchboard's instructions changed after this conversation started, and a resumed conversation keeps the system prompt it started with. The instructions below are the current ones: where they differ from the switchboard instructions in your system prompt, follow these.\n\n";
const RETIRED_POST_AGENT: &str = "post-agent";
const RETIRED_WORKTREE_CREATE: &str = "worktree-create";
const RETIRED_PINS: &str = "fork-pins";
const PROJECT_PREFIX: &str = "Project:";
const FORMAT_HELP: &str = "A project-fork's prompt must start with the line `Project: <absolute path of an existing project folder>`, followed by the user's words.";

#[derive(Default, Deserialize)]
#[serde(default)]
struct ToolInput {
    subagent_type: Option<String>,
    prompt: Option<String>,
    model: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Payload {
    cwd: Option<String>,
    tool_input: ToolInput,
    source: Option<String>,
    transcript_path: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Pass,
    Deny(String),
}

fn canonical(path: &Path) -> Result<PathBuf, std::io::Error> {
    fs::canonicalize(path).map(strip_verbatim)
}

fn check_project(prompt: &str) -> Result<(), String> {
    let first = prompt.lines().next().unwrap_or_default().trim();
    let Some(written) = first.strip_prefix(PROJECT_PREFIX).map(str::trim) else {
        return Err(format!(
            "The prompt's first line is not a project line. {FORMAT_HELP}"
        ));
    };
    let path = Path::new(written);
    if !path.is_absolute() {
        return Err(format!(
            "`{written}` is not an absolute path. {FORMAT_HELP}"
        ));
    }
    if !path.is_dir() {
        return Err(format!(
            "`{written}` is not an existing directory. {FORMAT_HELP}"
        ));
    }
    let resolved = canonical(path)
        .map_err(|error| format!("`{written}` cannot be resolved ({error}). {FORMAT_HELP}"))?;
    if resolved.as_os_str() != written {
        return Err(format!(
            "`{written}` is not the folder's own spelling, so the fork's memory would not be found. Start the prompt with `Project: {}` instead.",
            resolved.display()
        ));
    }
    Ok(())
}

fn pre_agent(payload: &Payload, admit: &dyn Fn(&str) -> Result<()>) -> Verdict {
    if payload.tool_input.subagent_type.as_deref() != Some(FORK_AGENT) {
        return Verdict::Pass;
    }
    if let Err(reason) = check_project(payload.tool_input.prompt.as_deref().unwrap_or_default()) {
        return Verdict::Deny(reason);
    }
    if let Some(model) = payload.tool_input.model.as_deref() {
        if let Err(error) = admit(model) {
            return Verdict::Deny(format!("{error:#}"));
        }
    }
    Verdict::Pass
}

fn unplaced_notice(switchboard: &Path, config_home: &Path) -> String {
    format!(
        "Claude Code started you in the switchboard's own folder, {}, not in your project folder, because the {} plugin, which starts each fork in its project folder, did not run. Work in the folder on the first line of your task all the same: start each Bash command with `cd <that folder> &&` until you make a worktree, and before anything else read that folder's CLAUDE.md files and your memory index, {}, where <key> is that folder's path with each character that is not a letter or digit replaced by `-`. End your final message with the line `The {} plugin did not run.`\n",
        switchboard.display(),
        plugin::NAME,
        config_home
            .join("projects")
            .join("<key>")
            .join("memory")
            .join("MEMORY.md")
            .display(),
        plugin::NAME,
    )
}

fn fork_context(
    part: usize,
    payload: &Payload,
    config_home: &Path,
    switchboard: Option<&Path>,
) -> Result<Option<String>> {
    let cwd = payload
        .cwd
        .as_deref()
        .map(Path::new)
        .ok_or_else(|| anyhow!("the hook payload carries no cwd"))?;
    let cwd = canonical(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let parts = match switchboard {
        Some(own) if own == cwd => vec![unplaced_notice(own, config_home)],
        _ => context::parts(&cwd, config_home, switchboard),
    };
    Ok(parts.get(part).map(|text| {
        let text = format!(
            "[project context part {}/{}]\n{text}",
            part + 1,
            parts.len()
        );
        json!({"hookSpecificOutput": {"hookEventName": "SubagentStart", "additionalContext": text}})
            .to_string()
    }))
}

fn refreshed_since_compaction(transcript: &str, revision: &str) -> bool {
    let rows: Vec<&str> = transcript.lines().collect();
    let start = rows
        .iter()
        .rposition(|row| row.contains(COMPACT_BOUNDARY))
        .map_or(0, |at| at + 1);
    rows[start..].iter().any(|row| row.contains(revision))
}

fn role_refresh(
    part: usize,
    payload: &Payload,
    transcript: &str,
    revision: &str,
    role: &dyn Fn() -> String,
) -> Option<String> {
    let source = payload.source.as_deref()?;
    if !ROLE_REFRESH_SOURCES.split('|').any(|one| one == source) {
        return None;
    }
    if refreshed_since_compaction(transcript, revision) {
        return None;
    }
    let parts = context::split_parts(&format!("{ROLE_REFRESH_LEAD}{}", role()));
    let text = parts.get(part)?;
    let text = format!(
        "[switchboard instructions part {}/{}]\n{text}",
        part + 1,
        parts.len()
    );
    Some(
        json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": text}})
            .to_string(),
    )
}

fn current_role(cwd: Option<&str>) -> String {
    let projects = match (crate::spawn::SpawnLedger::system(), cwd) {
        (Ok(ledger), Some(cwd)) => ledger.recent_cwds(super::PROJECTS_SHOWN, Path::new(cwd)),
        _ => Vec::new(),
    };
    super::role(&projects)
}

fn deny_json(reason: &str) -> String {
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": reason,
    }})
    .to_string()
}

pub(super) fn remove_retired_pins(sessions: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(sessions.join(RETIRED_PINS)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn read_payload() -> Result<Payload> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .context("failed to read the hook payload")?;
    if raw.trim().is_empty() {
        return Ok(Payload::default());
    }
    serde_json::from_str(&raw).context("the hook payload is not valid JSON")
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    let name = args.first().and_then(|arg| arg.to_str()).ok_or_else(|| {
        anyhow!(
            "usage: switchboard hook <{PRE_AGENT}|{FORK_CONTEXT} N|{ROLE_REFRESH} N|{AGENT_ROWS}>"
        )
    })?;
    match name {
        AGENT_ROWS => super::rows::run(),
        RETIRED_POST_AGENT => Ok(()),
        RETIRED_WORKTREE_CREATE => bail!(
            "this switchboard was started by an older CLI Sessions whose forks ran in worktree isolation; close the switchboard and open it again to start forks"
        ),
        PRE_AGENT => {
            if let Verdict::Deny(reason) = pre_agent(&read_payload()?, &admit_model) {
                crate::cli::line(deny_json(&reason));
            }
            Ok(())
        }
        FORK_CONTEXT => {
            let part = args
                .get(1)
                .and_then(|arg| arg.to_str())
                .and_then(|arg| arg.parse::<usize>().ok())
                .ok_or_else(|| anyhow!("{FORK_CONTEXT} takes a part number"))?;
            let payload = read_payload()?;
            if let Some(line) =
                fork_context(part, &payload, &claude_home(), switchboard_dir().as_deref())?
            {
                crate::cli::line(line);
            }
            Ok(())
        }
        ROLE_REFRESH => {
            let part = args
                .get(1)
                .and_then(|arg| arg.to_str())
                .and_then(|arg| arg.parse::<usize>().ok())
                .ok_or_else(|| anyhow!("{ROLE_REFRESH} takes a part number"))?;
            let payload = read_payload()?;
            let transcript = payload
                .transcript_path
                .as_deref()
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default();
            let role = || current_role(payload.cwd.as_deref());
            if let Some(line) =
                role_refresh(part, &payload, &transcript, &super::role_revision(), &role)
            {
                crate::cli::line(line);
            }
            Ok(())
        }
        other => bail!("unknown switchboard hook `{other}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rig {
        root: tempfile::TempDir,
        project_a: PathBuf,
        project_b: PathBuf,
    }

    fn rig() -> Rig {
        let root = tempfile::tempdir().unwrap();
        let project_a = root.path().join("a");
        let project_b = root.path().join("b");
        fs::create_dir_all(&project_a).unwrap();
        fs::create_dir_all(&project_b).unwrap();
        Rig {
            project_a: canonical(&project_a).unwrap(),
            project_b: canonical(&project_b).unwrap(),
            root,
        }
    }

    fn admit_all(_model: &str) -> Result<()> {
        Ok(())
    }

    fn admit_none(model: &str) -> Result<()> {
        bail!("model {model} is not allowed")
    }

    fn call(subagent: &str, prompt: &str) -> Payload {
        Payload {
            tool_input: ToolInput {
                subagent_type: Some(subagent.to_owned()),
                prompt: Some(prompt.to_owned()),
                model: None,
            },
            ..Payload::default()
        }
    }

    fn fork_call(project: &Path) -> Payload {
        call(
            FORK_AGENT,
            &format!("Project: {}\nfix it", project.display()),
        )
    }

    fn denied(verdict: Verdict) -> String {
        match verdict {
            Verdict::Deny(reason) => reason,
            Verdict::Pass => panic!("expected a denial"),
        }
    }

    #[test]
    fn a_missing_relative_or_nonexistent_project_is_denied_with_the_format() {
        let rig = rig();
        let missing = rig.project_a.join("gone");
        for prompt in [
            "fix it".to_owned(),
            "Project: relative/dir\nfix it".to_owned(),
            format!("Project: {}\nfix it", missing.display()),
            format!("fix it\nProject: {}", rig.project_a.display()),
        ] {
            let reason = denied(pre_agent(&call(FORK_AGENT, &prompt), &admit_all));
            assert!(reason.contains("Project: <absolute path"), "{reason}");
        }
    }

    #[test]
    fn a_project_spelled_otherwise_than_on_disk_is_denied_with_its_own_spelling() {
        let rig = rig();
        let own = rig.project_a.display().to_string();
        for written in [
            format!("{own}{}", std::path::MAIN_SEPARATOR),
            rig.project_a.join(".").display().to_string(),
            rig.project_b.join("..").join("a").display().to_string(),
        ] {
            let prompt = format!("Project: {written}\nfix it");
            let reason = denied(pre_agent(&call(FORK_AGENT, &prompt), &admit_all));
            assert!(
                reason.contains(&format!("Start the prompt with `Project: {own}` instead")),
                "{reason}"
            );
        }
        let spaced = format!("  Project:   {own}  \r\nfix it");
        assert_eq!(
            pre_agent(&call(FORK_AGENT, &spaced), &admit_all),
            Verdict::Pass
        );
    }

    #[test]
    fn only_project_fork_calls_are_checked() {
        let other = call("general-purpose", "no project line");
        assert_eq!(pre_agent(&other, &admit_none), Verdict::Pass);
        let unscoped = call(FORK_AGENT, "no project line");
        assert!(denied(pre_agent(&unscoped, &admit_all)).contains("not a project line"));
        let unnamed = Payload::default();
        assert_eq!(pre_agent(&unnamed, &admit_none), Verdict::Pass);
    }

    #[test]
    fn a_model_the_policy_does_not_admit_is_denied_and_none_is_always_fine() {
        let rig = rig();
        let mut named = fork_call(&rig.project_a);
        named.tool_input.model = Some("opus".to_owned());
        let reason = denied(pre_agent(&named, &admit_none));
        assert!(reason.contains("model opus is not allowed"), "{reason}");
        assert_eq!(pre_agent(&named, &admit_all), Verdict::Pass);
        let unnamed = fork_call(&rig.project_a);
        assert_eq!(pre_agent(&unnamed, &admit_none), Verdict::Pass);
    }

    fn context_line(line: &str) -> String {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(
            value["hookSpecificOutput"]["hookEventName"],
            "SubagentStart"
        );
        value["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn fork_context_prints_the_part_as_subagent_start_json_and_nothing_past_the_end() {
        let rig = rig();
        let home = rig.root.path().join("home");
        let board = rig.root.path().join("board");
        let payload = Payload {
            cwd: Some(rig.project_a.join(".").to_string_lossy().into_owned()),
            ..Payload::default()
        };
        let text = context_line(
            &fork_context(0, &payload, &home, Some(&board))
                .unwrap()
                .unwrap(),
        );
        assert!(
            text.starts_with(&format!(
                "[project context part 1/1]\nYour project folder is {}.",
                rig.project_a.display()
            )),
            "{text}"
        );
        assert!(fork_context(0, &Payload::default(), &home, None).is_err());
        assert_eq!(fork_context(9, &payload, &home, None).unwrap(), None);
    }

    #[test]
    fn a_fork_started_in_the_switchboard_folder_is_told_to_work_in_its_project() {
        let rig = rig();
        let home = rig.root.path().join("home");
        let board = rig.project_b.clone();
        let payload = Payload {
            cwd: Some(board.to_string_lossy().into_owned()),
            ..Payload::default()
        };
        let text = context_line(
            &fork_context(0, &payload, &home, Some(&board))
                .unwrap()
                .unwrap(),
        );
        assert!(text.starts_with("[project context part 1/1]\n"), "{text}");
        for needle in [
            format!("the switchboard's own folder, {}", board.display()),
            "the qol-switchboard plugin".to_owned(),
            "`cd <that folder> &&`".to_owned(),
            home.join("projects")
                .join("<key>")
                .join("memory")
                .join("MEMORY.md")
                .display()
                .to_string(),
            "End your final message with the line `The qol-switchboard plugin did not run.`"
                .to_owned(),
        ] {
            assert!(text.contains(&needle), "{needle}: {text}");
        }
        assert_eq!(
            fork_context(1, &payload, &home, Some(&board)).unwrap(),
            None
        );
    }

    #[test]
    fn the_retired_worktree_hooks_tell_an_old_switchboard_to_restart() {
        run(&[OsString::from("post-agent")]).unwrap();
        let error = run(&[OsString::from("worktree-create")])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("close the switchboard and open it again"),
            "{error}"
        );
        let error = run(&[OsString::from("bogus")]).unwrap_err().to_string();
        assert!(
            error.contains("unknown switchboard hook `bogus`"),
            "{error}"
        );
    }

    #[test]
    fn the_retired_pins_folder_is_removed_and_a_missing_one_is_fine() {
        let root = tempfile::tempdir().unwrap();
        let session = root.path().join(RETIRED_PINS).join("session-1");
        fs::create_dir_all(&session).unwrap();
        fs::write(session.join("agent-a"), "/work/app").unwrap();
        fs::write(root.path().join("kept"), "").unwrap();
        remove_retired_pins(root.path()).unwrap();
        assert!(!root.path().join(RETIRED_PINS).exists());
        assert!(root.path().join("kept").exists());
        remove_retired_pins(root.path()).unwrap();
    }

    #[test]
    fn the_deny_output_has_the_pre_tool_use_shape() {
        let value: serde_json::Value = serde_json::from_str(&deny_json("why")).unwrap();
        assert_eq!(value["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(
            value["hookSpecificOutput"]["permissionDecisionReason"],
            "why"
        );
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    }

    fn session_start(source: &str) -> Payload {
        Payload {
            source: Some(source.to_owned()),
            ..Payload::default()
        }
    }

    fn refreshed(
        part: usize,
        source: &str,
        transcript: &str,
        role: &str,
    ) -> Option<serde_json::Value> {
        let role = role.to_owned();
        role_refresh(
            part,
            &session_start(source),
            transcript,
            "Switchboard role revision new.",
            &move || role.clone(),
        )
        .map(|line| serde_json::from_str(&line).unwrap())
    }

    #[test]
    fn a_resumed_or_compacted_switchboard_gets_the_current_role_until_it_holds_it() {
        let role = "the role\nSwitchboard role revision new.";
        let stale = "{\"type\":\"attachment\",\"x\":\"Switchboard role revision old.\"}\n";
        for source in ["resume", "compact"] {
            let value = refreshed(0, source, stale, role).unwrap();
            assert_eq!(value["hookSpecificOutput"]["hookEventName"], "SessionStart");
            let text = value["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap();
            assert!(text.starts_with("[switchboard instructions part 1/1]\n"));
            assert!(text.contains(ROLE_REFRESH_LEAD));
            assert!(text.ends_with(role));
            assert!(refreshed(1, source, stale, role).is_none());
        }
        assert!(refreshed(0, "startup", stale, role).is_none());
        assert!(refreshed(0, "clear", stale, role).is_none());
        let current = format!("{stale}{{\"x\":\"Switchboard role revision new.\"}}\n");
        assert!(refreshed(0, "resume", &current, role).is_none());
        let compacted =
            format!("{current}{{\"type\":\"system\",\"subtype\":\"compact_boundary\"}}\n");
        assert!(refreshed(0, "compact", &compacted, role).is_some());
    }

    #[test]
    fn a_long_role_is_split_over_the_refresh_slots() {
        let role = format!("{}\n", "r".repeat(99)).repeat(context::PART_CHARS / 100 + 1);
        let first = refreshed(0, "resume", "", &role).unwrap();
        let second = refreshed(1, "resume", "", &role).unwrap();
        let text = |value: &serde_json::Value| {
            value["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert!(text(&first).starts_with("[switchboard instructions part 1/2]\n"));
        assert!(text(&second).starts_with("[switchboard instructions part 2/2]\n"));
        assert!(refreshed(ROLE_REFRESH_SLOTS - 1, "resume", "", &role).is_none());
        let full = super::super::role(&[]);
        assert!(
            context::split_parts(&format!("{ROLE_REFRESH_LEAD}{full}")).len() <= ROLE_REFRESH_SLOTS
        );
    }
}
