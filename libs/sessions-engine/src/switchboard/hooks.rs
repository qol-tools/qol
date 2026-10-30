use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::agents::admit_model;
use super::context;
use super::paths::{claude_home, switchboard_dir};
use super::resume::{recorded_worktree, FORK_AGENT};

pub(super) const PRE_AGENT: &str = "pre-agent";
pub(super) const POST_AGENT: &str = "post-agent";
pub(super) const WORKTREE_CREATE: &str = "worktree-create";
pub(super) const FORK_CONTEXT: &str = "fork-context";
pub(super) const AGENT_ROWS: &str = "agent-rows";
const PIN_TTL: Duration = Duration::from_secs(120);
const AGENT_NAME_PREFIX: &str = "agent-";
const PROJECT_PREFIX: &str = "Project:";

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
    session_id: Option<String>,
    tool_use_id: Option<String>,
    agent_id: Option<String>,
    cwd: Option<String>,
    name: Option<String>,
    transcript_path: Option<String>,
    tool_input: ToolInput,
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Pass,
    Deny(String),
}

struct Pins {
    dir: PathBuf,
}

fn safe_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl Pins {
    fn system() -> Result<Self> {
        let dir = qol_config::data_subdir("sessions")
            .ok_or_else(|| anyhow!("sessions data directory is unavailable"))?
            .join("fork-pins");
        Ok(Self { dir })
    }

    #[cfg(test)]
    fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn session_dir(&self, session: &str) -> PathBuf {
        self.dir.join(safe_name(session))
    }

    fn lock(&self, session: &str) -> Result<fs::File> {
        fs::create_dir_all(&self.dir).context("failed to create the fork pin directory")?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join(format!("{}.lock", safe_name(session))))
            .context("failed to open the fork pin lock")?;
        file.lock().context("failed to take the fork pin lock")?;
        Ok(file)
    }

    fn live(&self, session: &str, now: SystemTime) -> Vec<(String, String)> {
        let Ok(entries) = fs::read_dir(self.session_dir(session)) else {
            return Vec::new();
        };
        let mut live = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let age = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .unwrap_or_default();
            if age > PIN_TTL {
                let _ = fs::remove_file(&path);
                continue;
            }
            let Ok(project) = fs::read_to_string(&path) else {
                continue;
            };
            let Some(tool_use) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            live.push((tool_use.to_owned(), project.trim().to_owned()));
        }
        live
    }

    fn write(&self, session: &str, tool_use: &str, project: &Path) -> Result<()> {
        let dir = self.session_dir(session);
        fs::create_dir_all(&dir).context("failed to create the fork pin directory")?;
        qol_fs::atomic_write(
            &dir.join(safe_name(tool_use)),
            project.to_string_lossy().as_bytes(),
        )
        .context("failed to write the fork pin")
    }

    fn remove(&self, session: &str, tool_use: &str) {
        let _ = fs::remove_file(self.session_dir(session).join(safe_name(tool_use)));
    }
}

fn project_of(prompt: &str) -> Result<PathBuf, String> {
    let format_help = "A project-fork's prompt must start with the line `Project: <absolute path of an existing project folder>`, followed by the user's words.";
    let first = prompt.lines().next().unwrap_or_default().trim();
    let Some(path) = first.strip_prefix(PROJECT_PREFIX) else {
        return Err(format!(
            "The prompt's first line is not a project line. {format_help}"
        ));
    };
    let path = Path::new(path.trim());
    if !path.is_absolute() {
        return Err(format!(
            "`{}` is not an absolute path. {format_help}",
            path.display()
        ));
    }
    if !path.is_dir() {
        return Err(format!(
            "`{}` is not an existing directory. {format_help}",
            path.display()
        ));
    }
    fs::canonicalize(path).map_err(|error| {
        format!(
            "`{}` cannot be resolved ({error}). {format_help}",
            path.display()
        )
    })
}

fn pre_agent(
    pins: &Pins,
    payload: &Payload,
    now: SystemTime,
    admit: &dyn Fn(&str) -> Result<()>,
) -> Result<Verdict> {
    if payload.agent_id.is_some() || payload.tool_input.subagent_type.as_deref() != Some(FORK_AGENT)
    {
        return Ok(Verdict::Pass);
    }
    let project = match project_of(payload.tool_input.prompt.as_deref().unwrap_or_default()) {
        Ok(project) => project,
        Err(reason) => return Ok(Verdict::Deny(reason)),
    };
    if let Some(model) = payload.tool_input.model.as_deref() {
        if let Err(error) = admit(model) {
            return Ok(Verdict::Deny(format!("{error:#}")));
        }
    }
    let (Some(session), Some(tool_use)) = (&payload.session_id, &payload.tool_use_id) else {
        return Ok(Verdict::Deny(
            "The hook payload carried no session or tool use id, so the fork's folder cannot be pinned.".to_owned(),
        ));
    };
    let _lock = pins.lock(session)?;
    let project_text = project.to_string_lossy();
    if let Some((_, other)) = pins
        .live(session, now)
        .into_iter()
        .find(|(id, pinned)| id != &safe_name(tool_use) && pinned.as_str() != &*project_text)
    {
        return Ok(Verdict::Deny(format!(
            "Another fork is still starting in {other}. Start this one in a later message, after that fork's Agent call has returned."
        )));
    }
    pins.write(session, tool_use, &project)?;
    Ok(Verdict::Pass)
}

fn post_agent(pins: &Pins, payload: &Payload) -> Result<()> {
    let (Some(session), Some(tool_use)) = (&payload.session_id, &payload.tool_use_id) else {
        return Ok(());
    };
    let _lock = pins.lock(session)?;
    pins.remove(session, tool_use);
    Ok(())
}

fn worktree_create(pins: &Pins, payload: &Payload, now: SystemTime) -> Result<PathBuf> {
    let session = payload.session_id.as_deref();
    if let (Some(name), Some(transcript), Some(session)) = (
        payload.name.as_deref(),
        payload.transcript_path.as_deref(),
        session,
    ) {
        let agent_id = name.strip_prefix(AGENT_NAME_PREFIX).unwrap_or(name);
        let subagents = Path::new(transcript)
            .parent()
            .map(|dir| dir.join(session).join("subagents"));
        if let Some(worktree) = subagents.and_then(|dir| recorded_worktree(&dir, agent_id)) {
            return Ok(worktree);
        }
    }
    if let Some(session) = session {
        let _lock = pins.lock(session)?;
        let mut projects: Vec<String> = pins
            .live(session, now)
            .into_iter()
            .map(|(_, project)| project)
            .collect();
        projects.sort();
        projects.dedup();
        if let [project] = projects.as_slice() {
            return Ok(PathBuf::from(project));
        }
    }
    bail!(
        "a project-fork's prompt must start with `Project: <absolute path of an existing folder>`"
    )
}

fn fork_context(part: usize, payload: &Payload) -> Result<Option<String>> {
    let cwd = payload
        .cwd
        .as_deref()
        .ok_or_else(|| anyhow!("the hook payload carries no cwd"))?;
    let parts = context::parts(Path::new(cwd), &claude_home(), switchboard_dir().as_deref());
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

fn deny_json(reason: &str) -> String {
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": reason,
    }})
    .to_string()
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
            "usage: switchboard hook <{PRE_AGENT}|{POST_AGENT}|{WORKTREE_CREATE}|{FORK_CONTEXT} N|{AGENT_ROWS}>"
        )
    })?;
    if name == AGENT_ROWS {
        return super::rows::run();
    }
    let payload = read_payload()?;
    let now = SystemTime::now();
    match name {
        PRE_AGENT => {
            if let Verdict::Deny(reason) = pre_agent(&Pins::system()?, &payload, now, &admit_model)?
            {
                crate::cli::line(deny_json(&reason));
            }
        }
        POST_AGENT => post_agent(&Pins::system()?, &payload)?,
        WORKTREE_CREATE => {
            crate::cli::line(worktree_create(&Pins::system()?, &payload, now)?.display());
        }
        FORK_CONTEXT => {
            let part = args
                .get(1)
                .and_then(|arg| arg.to_str())
                .and_then(|arg| arg.parse::<usize>().ok())
                .ok_or_else(|| anyhow!("{FORK_CONTEXT} takes a part number"))?;
            if let Some(line) = fork_context(part, &payload)? {
                crate::cli::line(line);
            }
        }
        other => bail!("unknown switchboard hook `{other}`"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rig {
        _root: tempfile::TempDir,
        pins: Pins,
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
            pins: Pins::with_dir(root.path().join("pins")),
            project_a: fs::canonicalize(project_a).unwrap(),
            project_b: fs::canonicalize(project_b).unwrap(),
            _root: root,
        }
    }

    fn admit_all(_model: &str) -> Result<()> {
        Ok(())
    }

    fn admit_none(model: &str) -> Result<()> {
        bail!("model {model} is not allowed")
    }

    fn call(tool_use: &str, subagent: &str, prompt: &str) -> Payload {
        Payload {
            session_id: Some("s1".to_owned()),
            tool_use_id: Some(tool_use.to_owned()),
            tool_input: ToolInput {
                subagent_type: Some(subagent.to_owned()),
                prompt: Some(prompt.to_owned()),
                model: None,
            },
            ..Payload::default()
        }
    }

    fn fork_call(tool_use: &str, project: &Path) -> Payload {
        call(
            tool_use,
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
        let now = SystemTime::now();
        let missing = rig.project_a.join("gone");
        for prompt in [
            "fix it".to_owned(),
            "Project: relative/dir\nfix it".to_owned(),
            format!("Project: {}\nfix it", missing.display()),
            format!("fix it\nProject: {}", rig.project_a.display()),
        ] {
            let reason = denied(
                pre_agent(&rig.pins, &call("t1", FORK_AGENT, &prompt), now, &admit_all).unwrap(),
            );
            assert!(reason.contains("Project: <absolute path"), "{reason}");
        }
        assert!(rig.pins.live("s1", now).is_empty());
    }

    #[test]
    fn a_different_in_flight_project_is_denied_and_the_same_one_is_allowed() {
        let rig = rig();
        let now = SystemTime::now();
        assert_eq!(
            pre_agent(&rig.pins, &fork_call("t1", &rig.project_a), now, &admit_all).unwrap(),
            Verdict::Pass
        );
        assert_eq!(
            pre_agent(&rig.pins, &fork_call("t2", &rig.project_a), now, &admit_all).unwrap(),
            Verdict::Pass
        );
        let reason = denied(
            pre_agent(&rig.pins, &fork_call("t3", &rig.project_b), now, &admit_all).unwrap(),
        );
        assert!(reason.contains("later message"), "{reason}");
        assert!(
            reason.contains(&rig.project_a.display().to_string()),
            "{reason}"
        );
        assert_eq!(rig.pins.live("s1", now).len(), 2);
    }

    #[test]
    fn a_stale_pin_is_ignored_and_removed() {
        let rig = rig();
        let now = SystemTime::now();
        pre_agent(&rig.pins, &fork_call("t1", &rig.project_a), now, &admit_all).unwrap();
        let later = now + PIN_TTL + Duration::from_secs(5);
        assert_eq!(
            pre_agent(
                &rig.pins,
                &fork_call("t2", &rig.project_b),
                later,
                &admit_all
            )
            .unwrap(),
            Verdict::Pass
        );
        let live = rig.pins.live("s1", now);
        assert_eq!(live.len(), 1, "{live:?}");
        assert_eq!(live[0].1, rig.project_b.to_string_lossy());
    }

    #[test]
    fn calls_from_other_agents_and_from_subagents_are_ignored() {
        let rig = rig();
        let now = SystemTime::now();
        let other = call("t1", "general-purpose", "no project line");
        assert_eq!(
            pre_agent(&rig.pins, &other, now, &admit_all).unwrap(),
            Verdict::Pass
        );
        let mut from_subagent = fork_call("t2", &rig.project_a);
        from_subagent.agent_id = Some("a123".to_owned());
        assert_eq!(
            pre_agent(&rig.pins, &from_subagent, now, &admit_all).unwrap(),
            Verdict::Pass
        );
        let mut unscoped = call("t3", FORK_AGENT, "no project line");
        unscoped.agent_id = Some("a123".to_owned());
        assert_eq!(
            pre_agent(&rig.pins, &unscoped, now, &admit_all).unwrap(),
            Verdict::Pass
        );
        assert!(rig.pins.live("s1", now).is_empty());
    }

    #[test]
    fn a_model_the_policy_does_not_admit_is_denied_and_none_is_always_fine() {
        let rig = rig();
        let now = SystemTime::now();
        let mut named = fork_call("t1", &rig.project_a);
        named.tool_input.model = Some("opus".to_owned());
        let reason = denied(pre_agent(&rig.pins, &named, now, &admit_none).unwrap());
        assert!(reason.contains("model opus is not allowed"), "{reason}");
        assert!(rig.pins.live("s1", now).is_empty());
        assert_eq!(
            pre_agent(&rig.pins, &named, now, &admit_all).unwrap(),
            Verdict::Pass
        );
        let unnamed = fork_call("t2", &rig.project_a);
        assert_eq!(
            pre_agent(&rig.pins, &unnamed, now, &admit_none).unwrap(),
            Verdict::Pass
        );
    }

    #[test]
    fn post_agent_removes_the_pin_of_its_tool_use() {
        let rig = rig();
        let now = SystemTime::now();
        pre_agent(&rig.pins, &fork_call("t1", &rig.project_a), now, &admit_all).unwrap();
        pre_agent(&rig.pins, &fork_call("t2", &rig.project_a), now, &admit_all).unwrap();
        post_agent(&rig.pins, &fork_call("t1", &rig.project_a)).unwrap();
        let live = rig.pins.live("s1", now);
        assert_eq!(
            live.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["t2"]
        );
        post_agent(&rig.pins, &fork_call("t2", &rig.project_a)).unwrap();
        assert!(rig.pins.live("s1", now).is_empty());
        post_agent(&rig.pins, &Payload::default()).unwrap();
    }

    fn creation(name: &str, session: &str, transcript: &Path) -> Payload {
        Payload {
            session_id: Some(session.to_owned()),
            name: Some(name.to_owned()),
            transcript_path: Some(transcript.to_string_lossy().into_owned()),
            ..Payload::default()
        }
    }

    #[test]
    fn worktree_create_prints_the_single_pinned_project() {
        let rig = rig();
        let now = SystemTime::now();
        let transcript = rig._root.path().join("t/s1.jsonl");
        pre_agent(&rig.pins, &fork_call("t1", &rig.project_a), now, &admit_all).unwrap();
        pre_agent(&rig.pins, &fork_call("t2", &rig.project_a), now, &admit_all).unwrap();
        let got =
            worktree_create(&rig.pins, &creation("agent-new", "s1", &transcript), now).unwrap();
        assert_eq!(got, rig.project_a);
    }

    #[test]
    fn worktree_create_with_no_pin_or_two_projects_fails_with_the_format() {
        let rig = rig();
        let now = SystemTime::now();
        let transcript = rig._root.path().join("t/s1.jsonl");
        let error = worktree_create(&rig.pins, &creation("agent-new", "s1", &transcript), now)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Project: <absolute path"), "{error}");
        pre_agent(&rig.pins, &fork_call("t1", &rig.project_a), now, &admit_all).unwrap();
        rig.pins.write("s1", "t9", &rig.project_b).unwrap();
        assert!(
            worktree_create(&rig.pins, &creation("agent-new", "s1", &transcript), now).is_err()
        );
        assert!(
            worktree_create(&rig.pins, &creation("agent-new", "other", &transcript), now).is_err()
        );
    }

    #[test]
    fn worktree_create_resumes_into_the_folder_its_meta_recorded() {
        let rig = rig();
        let now = SystemTime::now();
        let transcript = rig._root.path().join("t/s1.jsonl");
        let subagents = rig._root.path().join("t/s1/subagents/deeper");
        fs::create_dir_all(&subagents).unwrap();
        let meta = json!({"agentType": "project-fork", "worktreePath": rig.project_b});
        fs::write(subagents.join("agent-abc123.meta.json"), meta.to_string()).unwrap();
        pre_agent(&rig.pins, &fork_call("t1", &rig.project_a), now, &admit_all).unwrap();
        let got =
            worktree_create(&rig.pins, &creation("agent-abc123", "s1", &transcript), now).unwrap();
        assert_eq!(got, rig.project_b);
    }

    #[test]
    fn fork_context_prints_the_part_as_subagent_start_json_and_nothing_past_the_end() {
        let root = tempfile::tempdir().unwrap();
        let payload = Payload {
            cwd: Some(root.path().to_string_lossy().into_owned()),
            ..Payload::default()
        };
        let line = fork_context(0, &payload).unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            value["hookSpecificOutput"]["hookEventName"],
            "SubagentStart"
        );
        assert!(value["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .starts_with("[project context part 1/1]\nYour project folder is"));
        assert!(fork_context(context::PART_SLOTS - 1, &Payload::default()).is_err());
        assert_eq!(fork_context(9, &payload).unwrap(), None);
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
}
