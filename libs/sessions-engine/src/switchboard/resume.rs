use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::background::{self, notice_of};
use super::paths::conversations_dir;

pub(super) const FORK_AGENT: &str = "project-fork";
const META_SUFFIX: &str = ".meta.json";
pub(super) const AGENT_PREFIX: &str = "agent-";
const ASSISTANT_ROW: &str = "\"type\":\"assistant\"";
const NOTICE_OPEN: &str = "<task-notification>";
const USER_STOP_STATUS: &str = "killed";
const STOPPED_BY_USER_FLAG: &str = "stoppedByUser";
const WORKTREE_PIN: &str = "worktreePath";
const WORKTREE_KEYS: [&str; 3] = [WORKTREE_PIN, "worktreeBranch", "spawnedWithWorktree"];
const WAKE_OPENING: &str = "The switchboard restarted, which stopped these forks:";
const HANDOFF_OPENING: &str = "The switchboard restarted under new instructions,";
const LAUNCH_MARK: &str = "The switchboard restarted";
const PROMPT_ORIGIN: &str = "human";
const STOP_SETTLE: Duration = Duration::from_secs(5);
pub(super) const WAKE_FILE: &str = "wake.json";
const WATCHER_SCRIPT: &str = "pr-watch.cjs";
const MID_TURN: &str =
    "The switchboard restarted, which stopped your turn. Continue your work from where you left off.";

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SleepingFork {
    pub(super) agent_id: String,
    pub(super) name: Option<String>,
    pub(super) description: String,
    pub(super) cwd: Option<String>,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) transcript: PathBuf,
    pub(super) is_finished: bool,
    pub(super) background: Vec<background::Background>,
}

fn answered(path: &Path, revision: Option<&str>) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let (mut replied, mut current) = (false, revision.is_none());
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        replied |= line.contains(ASSISTANT_ROW);
        current |= revision.is_some_and(|revision| line.contains(revision));
        if replied && current {
            return true;
        }
    }
    false
}

fn newest_answered(
    config_home: &Path,
    switchboard: &Path,
    revision: Option<&str>,
) -> Option<String> {
    let mut found: Vec<(SystemTime, String)> =
        fs::read_dir(conversations_dir(config_home, switchboard))
            .ok()?
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension()? != "jsonl" {
                    return None;
                }
                let id = path.file_stem()?.to_str()?.to_owned();
                if !answered(&path, revision) {
                    return None;
                }
                Some((entry.metadata().ok()?.modified().ok()?, id))
            })
            .collect();
    found.sort();
    found.pop().map(|(_, id)| id)
}

pub(super) fn latest_conversation(
    config_home: &Path,
    switchboard: &Path,
    revision: &str,
) -> Option<String> {
    newest_answered(config_home, switchboard, Some(revision))
}

pub(super) fn previous_conversation(config_home: &Path, switchboard: &Path) -> Option<String> {
    newest_answered(config_home, switchboard, None)
}

pub(super) fn meta_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(meta_files(&path));
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(AGENT_PREFIX) && name.ends_with(META_SUFFIX))
        {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn unpin(meta_path: &Path) -> Result<()> {
    let Ok(text) = fs::read_to_string(meta_path) else {
        return Ok(());
    };
    let Ok(Value::Object(mut meta)) = serde_json::from_str::<Value>(&text) else {
        return Ok(());
    };
    if meta.get("agentType").and_then(Value::as_str) != Some(FORK_AGENT) {
        return Ok(());
    }
    let Some(Value::String(worktree)) = meta.get(WORKTREE_PIN).cloned() else {
        return Ok(());
    };
    for key in WORKTREE_KEYS {
        meta.remove(key);
    }
    meta.entry("cwd").or_insert(Value::String(worktree));
    qol_fs::atomic_write(meta_path, Value::Object(meta).to_string().as_bytes())?;
    Ok(())
}

pub(super) fn unpin_forks(config_home: &Path, switchboard: &Path) -> Vec<String> {
    let Ok(conversations) = fs::read_dir(conversations_dir(config_home, switchboard)) else {
        return Vec::new();
    };
    let mut failures = Vec::new();
    for conversation in conversations.flatten() {
        for meta_path in meta_files(&conversation.path().join("subagents")) {
            if let Err(error) = unpin(&meta_path) {
                failures.push(format!("{}: {error:#}", meta_path.display()));
            }
        }
    }
    failures
}

fn agent_id_of(meta_path: &Path) -> Option<String> {
    let name = meta_path.file_name()?.to_str()?;
    Some(
        name.strip_prefix(AGENT_PREFIX)?
            .strip_suffix(META_SUFFIX)?
            .to_owned(),
    )
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Ledger {
    stops: HashMap<String, bool>,
    launched: Option<String>,
}

fn launch_prompt_at(row: &Value) -> Option<&str> {
    let typed = row.get("type").and_then(Value::as_str) == Some("user")
        && row
            .pointer("/origin/kind")
            .and_then(Value::as_str)
            .is_none_or(|kind| kind == PROMPT_ORIGIN);
    let text = row.pointer("/message/content")?.as_str()?;
    let opens = text.starts_with(WAKE_OPENING) || text.starts_with(HANDOFF_OPENING);
    if !(typed && opens) {
        return None;
    }
    row.get("timestamp")?.as_str()
}

fn ledger(transcript: impl BufRead) -> Option<Ledger> {
    let mut ledger = Ledger::default();
    for line in transcript.lines() {
        let line = line.ok()?;
        if !line.contains(NOTICE_OPEN) && !line.contains(LAUNCH_MARK) {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some((id, status)) = notice_of(&row) {
            ledger
                .stops
                .insert(id.to_owned(), status == USER_STOP_STATUS);
        }
        if let Some(at) = launch_prompt_at(&row) {
            if ledger.launched.as_deref().is_none_or(|seen| at > seen) {
                ledger.launched = Some(at.to_owned());
            }
        }
    }
    Some(ledger)
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn stopped_after_its_turn(meta: &Path, transcript: &Path, switchboard: Option<SystemTime>) -> bool {
    let (Some(flagged_at), Some(last_turn)) = (modified(meta), modified(transcript)) else {
        return false;
    };
    let after_its_turn = flagged_at > last_turn + STOP_SETTLE;
    let before_the_exit = switchboard.is_none_or(|stopped| flagged_at + STOP_SETTLE < stopped);
    after_its_turn && before_the_exit
}

pub(super) fn sleeping_forks(
    config_home: &Path,
    switchboard: &Path,
    conversation: &str,
    previous_launch: Option<&str>,
) -> Vec<SleepingFork> {
    let conversations = conversations_dir(config_home, switchboard);
    let record = conversations.join(format!("{conversation}.jsonl"));
    let ledger = fs::File::open(&record)
        .ok()
        .and_then(|file| ledger(BufReader::new(file)));
    let stopped_at = modified(&record);
    let launched = ledger
        .as_ref()
        .and_then(|ledger| ledger.launched.as_deref())
        .into_iter()
        .chain(previous_launch)
        .max();
    meta_files(&conversations.join(conversation).join("subagents"))
        .into_iter()
        .filter_map(|path| {
            let meta: Value = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
            let text = |key: &str| meta.get(key).and_then(Value::as_str).map(str::to_owned);
            let agent_id = agent_id_of(&path)?;
            let transcript = path.with_file_name(format!("{AGENT_PREFIX}{agent_id}.jsonl"));
            let flagged = meta.get(STOPPED_BY_USER_FLAG).and_then(Value::as_bool) == Some(true);
            let stopped = match ledger
                .as_ref()
                .and_then(|ledger| ledger.stops.get(&agent_id).copied())
            {
                Some(true) => true,
                Some(false) => flagged && stopped_after_its_turn(&path, &transcript, stopped_at),
                None => flagged,
            };
            if text("agentType").as_deref() != Some(FORK_AGENT) || stopped {
                return None;
            }
            let finished = meta.get("finished").and_then(Value::as_bool) == Some(true);
            let since_launch = |work: &background::Background| match (launched, &work.started_at) {
                (Some(launched), Some(started)) => started.as_str() > launched,
                _ => true,
            };
            let background = fs::read_to_string(&transcript)
                .map(|text| {
                    background::running(&text)
                        .into_iter()
                        .filter(since_launch)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let asleep = !finished || !background.is_empty();
            asleep.then(|| SleepingFork {
                agent_id,
                name: text("name"),
                description: text("description").unwrap_or_default(),
                cwd: text("cwd"),
                model: text("model"),
                effort: text("effort"),
                transcript,
                is_finished: finished,
                background,
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct WakeFork {
    pub(super) agent_id: String,
    pub(super) name: Option<String>,
    pub(super) description: String,
    pub(super) cwd: Option<String>,
    pub(super) transcript: String,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Wake {
    pub(super) conversation: String,
    pub(super) launched_at: String,
    pub(super) is_handoff: bool,
    pub(super) forks: Vec<WakeFork>,
    #[serde(default)]
    pub(super) claimed: bool,
}

impl Wake {
    pub(super) fn of(
        conversation: &str,
        launched_at: String,
        is_handoff: bool,
        forks: Vec<SleepingFork>,
        watched: impl Fn(&str) -> bool,
    ) -> Self {
        let forks = forks
            .into_iter()
            .filter_map(|fork| {
                let message = if is_handoff {
                    handoff_message(&fork, watched(&fork.agent_id))
                } else {
                    wake_message(&fork, watched(&fork.agent_id))
                }?;
                Some(WakeFork {
                    transcript: fork.transcript.to_string_lossy().into_owned(),
                    agent_id: fork.agent_id,
                    name: fork.name,
                    description: fork.description,
                    cwd: fork.cwd,
                    model: fork.model,
                    effort: fork.effort,
                    message,
                })
            })
            .collect();
        Self {
            conversation: conversation.to_owned(),
            launched_at,
            is_handoff,
            forks,
            claimed: false,
        }
    }

    pub(super) fn previous_launch(path: &Path, conversation: &str) -> Option<String> {
        let wake: Wake = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        (wake.conversation == conversation).then_some(wake.launched_at)
    }

    pub(super) fn write(&self, path: &Path) -> Result<()> {
        let encoded = serde_json::to_string(self)?;
        qol_fs::atomic_write(path, encoded.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))
    }
}

fn is_watcher(work: &background::Background) -> bool {
    work.command
        .as_deref()
        .is_some_and(|command| command.contains(WATCHER_SCRIPT))
}

fn stopped_work(fork: &SleepingFork, watched: bool) -> Vec<String> {
    fork.background
        .iter()
        .filter(|work| !work.is_hold() && !(watched && is_watcher(work)))
        .map(|work| {
            let command = work.command.as_deref().unwrap_or(&work.id);
            if is_watcher(work) {
                format!("The switchboard restarted, which stopped your pull request watcher `{command}`. Start it again in the background, then end your turn saying what you wait for.")
            } else {
                format!("The switchboard restarted, which stopped your background command `{command}`. Start it again in the background if you still need it, then continue.")
            }
        })
        .collect()
}

fn wake_message(fork: &SleepingFork, watched: bool) -> Option<String> {
    let mut lines = Vec::new();
    if !fork.is_finished {
        lines.push(MID_TURN.to_owned());
    }
    lines.extend(stopped_work(fork, watched));
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn handoff_message(fork: &SleepingFork, watched: bool) -> Option<String> {
    let only_watched = fork.is_finished
        && !fork.background.is_empty()
        && fork
            .background
            .iter()
            .all(|work| watched && is_watcher(work));
    if only_watched {
        return None;
    }
    Some(format!(
        "You take over the work of a fork that stopped when the switchboard restarted under new instructions. Its conversation is in {}: read the end of it to learn where it stopped and what it was waiting on, then carry on from there, and start its pull request watcher again in the background if it was waiting on one, or its hold if it was waiting on the user.",
        fork.transcript.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::super::background::fixtures::{self, envelope, notice, row};
    use super::*;

    const NOTICE_ORIGIN: &str = "task-notification";

    fn conversation(config_home: &Path, switchboard: &Path, id: &str, body: &str) {
        let dir = conversations_dir(config_home, switchboard);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{id}.jsonl")), body).unwrap();
    }

    fn fork(subagents: &Path, id: &str, meta: &str, transcript: &str) {
        fs::create_dir_all(subagents).unwrap();
        fs::write(subagents.join(format!("agent-{id}.meta.json")), meta).unwrap();
        fs::write(subagents.join(format!("agent-{id}.jsonl")), transcript).unwrap();
    }

    #[test]
    fn the_newest_conversation_with_an_exchange_under_the_current_role_is_resumed() {
        let root = tempfile::tempdir().unwrap();
        let (home, board) = (root.path().join("h"), Path::new("/data/switchboard"));
        let current = "Switchboard role revision b2.";
        let answered = format!("{{\"systemPrompt\":\"{current}\"}}\n{{\"type\":\"assistant\"}}\n");
        assert_eq!(latest_conversation(&home, board, current), None);
        conversation(&home, board, "old", &answered);
        std::thread::sleep(std::time::Duration::from_millis(30));
        conversation(&home, board, "new", &answered);
        std::thread::sleep(std::time::Duration::from_millis(30));
        conversation(
            &home,
            board,
            "empty",
            &format!("{{\"systemPrompt\":\"{current}\"}}\n{{\"type\":\"attachment\"}}\n"),
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
        conversation(
            &home,
            board,
            "stale",
            "{\"systemPrompt\":\"Switchboard role revision a1.\"}\n{\"type\":\"assistant\"}\n",
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
        conversation(&home, board, "unmarked", r#"{"type":"assistant"}"#);
        assert_eq!(
            latest_conversation(&home, board, current).as_deref(),
            Some("new")
        );
    }

    #[test]
    fn the_previous_conversation_is_the_newest_answered_one_under_any_role() {
        let root = tempfile::tempdir().unwrap();
        let (home, board) = (root.path().join("h"), Path::new("/data/switchboard"));
        assert_eq!(previous_conversation(&home, board), None);
        conversation(
            &home,
            board,
            "stale",
            "{\"systemPrompt\":\"Switchboard role revision a1.\"}\n{\"type\":\"assistant\"}\n",
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
        conversation(&home, board, "unanswered", r#"{"type":"attachment"}"#);
        assert_eq!(
            previous_conversation(&home, board).as_deref(),
            Some("stale")
        );
        assert_eq!(
            latest_conversation(&home, board, "Switchboard role revision b2."),
            None
        );
    }

    fn waiting() -> String {
        fixtures::waiting("bw1abc", background::HOLD_COMMAND)
    }

    #[test]
    fn only_the_harness_notice_envelope_records_whether_the_user_stopped_a_fork() {
        let revival = envelope("stopped", "completed", "");
        let text = [
            notice("kept", "completed"),
            notice("stopped", "killed"),
            notice("revived", "killed"),
            notice("revived", "completed"),
            row(
                "user",
                NOTICE_ORIGIN,
                &envelope("quoting", "completed", &format!("\n{revival}")),
            ),
            row("user", "user", &revival),
            row("assistant", NOTICE_ORIGIN, &revival),
            row("queue-operation", NOTICE_ORIGIN, &revival),
            row(
                "user",
                NOTICE_ORIGIN,
                &format!("<system-reminder>\n{revival}"),
            ),
            row(
                "user",
                NOTICE_ORIGIN,
                "<task-notification>\n<task-id>bare</task-id>\n<summary>s</summary>\n<result>\n<status>completed</status>\n</result>",
            ),
            row(
                "user",
                NOTICE_ORIGIN,
                "<task-notification>\n<task-id>late</task-id>\n<summary>Agent \"<status>killed</status>\"</summary>\n<status>killed</status>",
            ),
            row(
                "user",
                NOTICE_ORIGIN,
                "<task-notification>\n<task-id>filed</task-id>\n<tool-use-id>toolu_2</tool-use-id>\n<output-file>/tmp/tasks/filed.output</output-file>\n<status>killed</status>\n<summary>s</summary>\n</task-notification>",
            ),
            "not json <task-notification>\n".to_owned(),
        ]
        .concat();
        let stops = ledger(text.as_bytes()).unwrap().stops;
        let mut seen = stops
            .iter()
            .map(|(id, stopped)| (id.as_str(), *stopped))
            .collect::<Vec<_>>();
        seen.sort();
        assert_eq!(
            seen,
            [
                ("filed", true),
                ("kept", false),
                ("quoting", false),
                ("revived", false),
                ("stopped", true),
            ]
        );
        assert_eq!(ledger(&b""[..]), Some(Ledger::default()));
        assert_eq!(ledger(&b"{}\n\xff\n"[..]), None);
    }

    #[test]
    fn without_a_readable_notice_the_stop_flag_keeps_a_fork_asleep() {
        let root = tempfile::tempdir().unwrap();
        let (home, board) = (root.path().join("h"), Path::new("/data/switchboard"));
        let dir = conversations_dir(&home, board)
            .join("conv")
            .join("subagents");
        fork(
            &dir,
            "exited",
            r#"{"agentType":"project-fork","description":"x","finished":true,"stoppedByUser":true}"#,
            &waiting(),
        );
        fork(
            &dir,
            "plain",
            r#"{"agentType":"project-fork","description":"y"}"#,
            "",
        );
        let ids = |forks: Vec<SleepingFork>| {
            forks
                .into_iter()
                .map(|fork| fork.agent_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(sleeping_forks(&home, board, "conv", None)), ["plain"]);
        conversation(&home, board, "conv", &notice("plain", "completed"));
        assert_eq!(ids(sleeping_forks(&home, board, "conv", None)), ["plain"]);
        conversation(&home, board, "conv", &notice("exited", "completed"));
        assert_eq!(
            ids(sleeping_forks(&home, board, "conv", None)),
            ["exited", "plain"]
        );
    }

    #[test]
    fn unfinished_or_background_waiting_forks_are_woken_unless_the_user_stopped_them() {
        let root = tempfile::tempdir().unwrap();
        let (home, board) = (root.path().join("h"), Path::new("/data/switchboard"));
        let dir = conversations_dir(&home, board)
            .join("conv")
            .join("subagents");
        fork(
            &dir,
            "mid",
            r#"{"agentType":"project-fork","description":"mid turn","name":"mid","cwd":"/work/a","model":"opus","effort":"max"}"#,
            "",
        );
        fork(
            &dir,
            "done",
            r#"{"agentType":"project-fork","description":"done","finished":true}"#,
            r#"{"type":"assistant"}"#,
        );
        let waiting = waiting();
        fork(
            &dir,
            "exited",
            r#"{"agentType":"project-fork","description":"watching at exit","finished":true,"stoppedByUser":true}"#,
            &waiting,
        );
        fork(
            &dir,
            "flagged",
            r#"{"agentType":"project-fork","description":"no notice","stoppedByUser":true}"#,
            "",
        );
        fork(
            &dir,
            "stopped",
            r#"{"agentType":"project-fork","description":"x","stoppedByUser":true}"#,
            "",
        );
        fork(
            &dir,
            "revived",
            r#"{"agentType":"project-fork","description":"y","stoppedByUser":true}"#,
            "",
        );
        fork(
            &dir,
            "other",
            r#"{"agentType":"Explore","description":"x"}"#,
            "",
        );
        fork(
            &dir,
            "asleep",
            r#"{"agentType":"project-fork","description":"watching","finished":true,"name":"zed"}"#,
            &waiting,
        );
        fork(
            &dir.join("agent-parent"),
            "deep",
            r#"{"agentType":"project-fork","description":"deep"}"#,
            "",
        );
        conversation(
            &home,
            board,
            "conv",
            &[
                notice("exited", "completed"),
                notice("stopped", "killed"),
                row("user", "user", &envelope("stopped", "completed", "spoofed")),
                notice("revived", "killed"),
                notice("revived", "completed"),
            ]
            .concat(),
        );
        let woken = sleeping_forks(&home, board, "conv", None);
        assert_eq!(
            woken
                .iter()
                .map(|fork| fork.agent_id.as_str())
                .collect::<Vec<_>>(),
            ["asleep", "exited", "mid", "deep", "revived"]
        );
        assert_eq!(woken[0].name.as_deref(), Some("zed"));
        assert_eq!(
            woken[2],
            SleepingFork {
                agent_id: "mid".to_owned(),
                name: Some("mid".to_owned()),
                description: "mid turn".to_owned(),
                cwd: Some("/work/a".to_owned()),
                model: Some("opus".to_owned()),
                effort: Some("max".to_owned()),
                transcript: dir.join("agent-mid.jsonl"),
                is_finished: false,
                background: Vec::new(),
            }
        );
        assert!(woken[0].is_finished);
        assert_eq!(
            woken[0]
                .background
                .iter()
                .map(|work| work.command.as_deref())
                .collect::<Vec<_>>(),
            [Some(background::HOLD_COMMAND)]
        );
        assert!(sleeping_forks(&home, board, "missing", None).is_empty());
    }

    fn launch_prompt(at: &str, origin: &str, text: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({
                "type": "user",
                "timestamp": at,
                "origin": {"kind": origin},
                "message": {"role": "user", "content": text},
            })
        )
    }

    fn woken_ids(home: &Path, board: &Path) -> Vec<String> {
        woken_since(home, board, None)
    }

    fn woken_since(home: &Path, board: &Path, previous: Option<&str>) -> Vec<String> {
        sleeping_forks(home, board, "conv", previous)
            .into_iter()
            .map(|fork| fork.agent_id)
            .collect()
    }

    #[test]
    fn background_work_started_before_the_last_launch_died_with_that_restart() {
        let root = tempfile::tempdir().unwrap();
        let (home, board) = (root.path().join("h"), Path::new("/data/switchboard"));
        let dir = conversations_dir(&home, board)
            .join("conv")
            .join("subagents");
        let done = r#"{"agentType":"project-fork","description":"x","finished":true}"#;
        let hold = |at: &str| fixtures::waiting_at("bhold", background::HOLD_COMMAND, at);
        fork(&dir, "before", done, &hold("2026-11-01T07:16:00.000Z"));
        fork(&dir, "after", done, &hold("2026-11-01T07:30:00.000Z"));
        fork(&dir, "untimed", done, &waiting());
        fork(
            &dir,
            "mid",
            r#"{"agentType":"project-fork","description":"y"}"#,
            &hold("2026-10-31T21:00:00.000Z"),
        );
        let wake = format!("{WAKE_OPENING}\n- a (agent id b): c");
        let handoff = format!("{HANDOFF_OPENING} so it began a new conversation");
        conversation(
            &home,
            board,
            "conv",
            &launch_prompt("2026-10-31T21:17:08.446Z", "human", &handoff),
        );
        assert_eq!(
            woken_ids(&home, board),
            ["after", "before", "mid", "untimed"]
        );
        conversation(
            &home,
            board,
            "conv",
            &[
                launch_prompt("2026-10-31T21:17:08.446Z", "human", &handoff),
                launch_prompt("2026-11-01T07:28:12.631Z", "human", &wake),
                launch_prompt("2026-11-01T09:00:00.000Z", "peer", &wake),
                launch_prompt(
                    "2026-11-01T09:00:00.000Z",
                    "human",
                    &format!("quoting: {wake}"),
                ),
            ]
            .concat(),
        );
        assert_eq!(woken_ids(&home, board), ["after", "mid", "untimed"]);
        assert_eq!(
            woken_since(&home, board, Some("2026-11-01T07:40:00.000Z")),
            ["mid", "untimed"]
        );
        assert_eq!(
            woken_since(&home, board, Some("2026-11-01T07:00:00.000Z")),
            ["after", "mid", "untimed"]
        );
    }

    fn set_modified(path: &Path, at: SystemTime) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    #[test]
    fn a_fork_the_user_stopped_after_its_last_turn_stays_asleep_past_its_completed_notice() {
        let root = tempfile::tempdir().unwrap();
        let (home, board) = (root.path().join("h"), Path::new("/data/switchboard"));
        let dir = conversations_dir(&home, board)
            .join("conv")
            .join("subagents");
        let flagged = r#"{"agentType":"project-fork","description":"x","finished":true,"stoppedByUser":true}"#;
        let ids = ["exit", "later", "same"];
        for id in ids {
            fork(&dir, id, flagged, &waiting());
        }
        conversation(
            &home,
            board,
            "conv",
            &ids.map(|id| notice(id, "completed")).concat(),
        );
        let turn = SystemTime::UNIX_EPOCH + Duration::from_secs(1_793_500_000);
        let exit = turn + Duration::from_secs(3_600);
        for id in ids {
            set_modified(&dir.join(format!("agent-{id}.jsonl")), turn);
        }
        set_modified(&dir.join("agent-same.meta.json"), turn);
        set_modified(
            &dir.join("agent-later.meta.json"),
            turn + Duration::from_secs(60),
        );
        set_modified(&dir.join("agent-exit.meta.json"), exit);
        set_modified(
            &conversations_dir(&home, board).join("conv.jsonl"),
            exit + Duration::from_secs(1),
        );
        assert_eq!(woken_ids(&home, board), ["exit", "same"]);
    }

    fn sleeping(agent_id: &str, is_finished: bool, commands: &[&str]) -> SleepingFork {
        SleepingFork {
            agent_id: agent_id.to_owned(),
            name: Some(format!("{agent_id}: a fork")),
            description: "fix the bug".to_owned(),
            cwd: Some("/work/a".to_owned()),
            model: Some("opus".to_owned()),
            effort: None,
            transcript: PathBuf::from(format!("/conv/subagents/agent-{agent_id}.jsonl")),
            is_finished,
            background: commands
                .iter()
                .enumerate()
                .map(|(index, command)| background::Background {
                    id: format!("b{index}"),
                    command: Some((*command).to_owned()),
                    started_at: None,
                })
                .collect(),
        }
    }

    const WATCHER: &str = "node /q/pr-watch.cjs https://github.com/o/r/pull/7 --pretty";

    fn messages(wake: &Wake) -> Vec<(&str, &str)> {
        wake.forks
            .iter()
            .map(|fork| (fork.agent_id.as_str(), fork.message.as_str()))
            .collect()
    }

    #[test]
    fn a_wake_continues_only_what_the_restart_stopped_and_spends_no_turn_on_the_rest() {
        let forks = vec![
            sleeping("mid", false, &[]),
            sleeping("held", true, &[background::HOLD_COMMAND]),
            sleeping("tray", true, &[WATCHER, background::HOLD_COMMAND]),
            sleeping("legacy", true, &[WATCHER]),
            sleeping("server", true, &["npm run dev"]),
            sleeping("both", false, &["npm run dev"]),
        ];
        let wake = Wake::of(
            "conv",
            "2026-11-01T10:00:00.000Z".to_owned(),
            false,
            forks,
            |agent| agent == "tray",
        );
        let messages = messages(&wake);
        assert_eq!(
            messages.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            ["mid", "legacy", "server", "both"]
        );
        assert_eq!(messages[0].1, MID_TURN);
        assert!(
            messages[1]
                .1
                .contains(&format!("pull request watcher `{WATCHER}`")),
            "{}",
            messages[1].1
        );
        assert!(
            messages[2].1.contains("background command `npm run dev`"),
            "{}",
            messages[2].1
        );
        assert!(messages[3].1.starts_with(MID_TURN), "{}", messages[3].1);
        assert!(messages[3].1.contains("`npm run dev`"), "{}", messages[3].1);
        let fork = &wake.forks[0];
        assert_eq!(fork.cwd.as_deref(), Some("/work/a"));
        assert_eq!(fork.transcript, "/conv/subagents/agent-mid.jsonl");
        assert_eq!(fork.model.as_deref(), Some("opus"));
        assert!(!wake.is_handoff && !wake.claimed);
    }

    #[test]
    fn a_handoff_starts_each_fork_again_unless_the_tray_holds_all_it_waits_on() {
        let wake = Wake::of(
            "old",
            "2026-11-01T10:00:00.000Z".to_owned(),
            true,
            vec![
                sleeping("held", true, &[background::HOLD_COMMAND]),
                sleeping("tray", true, &[WATCHER]),
                sleeping("legacy", true, &[WATCHER]),
                sleeping("mid", false, &[]),
            ],
            |agent| agent == "tray",
        );
        let messages = messages(&wake);
        assert_eq!(
            messages.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            ["held", "legacy", "mid"]
        );
        for (_, message) in &messages {
            assert!(
                message.contains("You take over the work of a fork"),
                "{message}"
            );
            assert!(
                message.contains("pull request watcher again in the background"),
                "{message}"
            );
        }
        assert!(
            messages[0].1.contains("/conv/subagents/agent-held.jsonl"),
            "{}",
            messages[0].1
        );
    }

    #[test]
    fn the_wake_file_keeps_the_launch_of_its_own_conversation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(WAKE_FILE);
        assert_eq!(Wake::previous_launch(&path, "conv"), None);
        let wake = Wake::of(
            "conv",
            "2026-11-01T10:00:00.000Z".to_owned(),
            false,
            vec![sleeping("mid", false, &[])],
            |_| false,
        );
        wake.write(&path).unwrap();
        assert_eq!(
            Wake::previous_launch(&path, "conv").as_deref(),
            Some("2026-11-01T10:00:00.000Z")
        );
        assert_eq!(Wake::previous_launch(&path, "other"), None);
        let read: Wake = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read, wake);
        fs::write(&path, "not json").unwrap();
        assert_eq!(Wake::previous_launch(&path, "conv"), None);
    }

    fn meta_of(subagents: &Path, id: &str) -> Value {
        let text = fs::read_to_string(subagents.join(format!("agent-{id}.meta.json"))).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn a_worktree_pinned_fork_of_any_conversation_is_moved_to_a_cwd() {
        let root = tempfile::tempdir().unwrap();
        let (home, board) = (root.path().join("h"), Path::new("/data/switchboard"));
        let conversations = conversations_dir(&home, board);
        let older = conversations.join("older").join("subagents");
        let newer = conversations.join("newer").join("subagents");
        let pinned = r#"{"agentType":"project-fork","description":"fix","worktreePath":"/work/app","worktreeBranch":"wt","spawnedWithWorktree":true,"finished":true}"#;
        fork(&older, "p1", pinned, "");
        fork(&newer.join("agent-parent"), "p2", pinned, "");
        fork(
            &newer,
            "kept",
            r#"{"agentType":"project-fork","worktreePath":"/work/app","cwd":"/work/own"}"#,
            "",
        );
        let placed = r#"{"agentType":"project-fork","cwd":"/work/app"}"#;
        fork(&newer, "placed", placed, "");
        let other = r#"{"agentType":"Explore","worktreePath":"/work/app"}"#;
        fork(&newer, "other", other, "");
        fork(&newer, "broken", "not json", "");
        fs::write(conversations.join("newer.jsonl"), "").unwrap();
        assert!(unpin_forks(&home, board).is_empty());
        let expected = serde_json::json!({
            "agentType": "project-fork",
            "description": "fix",
            "cwd": "/work/app",
            "finished": true,
        });
        assert_eq!(meta_of(&older, "p1"), expected);
        assert_eq!(meta_of(&newer.join("agent-parent"), "p2"), expected);
        assert_eq!(
            meta_of(&newer, "kept"),
            serde_json::json!({"agentType": "project-fork", "cwd": "/work/own"})
        );
        assert_eq!(
            fs::read_to_string(newer.join("agent-placed.meta.json")).unwrap(),
            placed
        );
        assert_eq!(
            fs::read_to_string(newer.join("agent-other.meta.json")).unwrap(),
            other
        );
        assert_eq!(
            fs::read_to_string(newer.join("agent-broken.meta.json")).unwrap(),
            "not json"
        );
        assert!(unpin_forks(&root.path().join("none"), board).is_empty());
    }
}
