use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Result;
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

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SleepingFork {
    pub(super) agent_id: String,
    pub(super) name: Option<String>,
    pub(super) description: String,
    pub(super) cwd: Option<String>,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) transcript: PathBuf,
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
) -> Vec<SleepingFork> {
    let conversations = conversations_dir(config_home, switchboard);
    let record = conversations.join(format!("{conversation}.jsonl"));
    let ledger = fs::File::open(&record)
        .ok()
        .and_then(|file| ledger(BufReader::new(file)));
    let stopped_at = modified(&record);
    let launched = ledger
        .as_ref()
        .and_then(|ledger| ledger.launched.as_deref());
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
            let asleep = !finished
                || fs::read_to_string(&transcript)
                    .is_ok_and(|text| background::running(&text).iter().any(since_launch));
            asleep.then(|| SleepingFork {
                agent_id,
                name: text("name"),
                description: text("description").unwrap_or_default(),
                cwd: text("cwd"),
                model: text("model"),
                effort: text("effort"),
                transcript,
            })
        })
        .collect()
}

pub(super) fn wake_prompt(forks: &[SleepingFork]) -> Option<String> {
    if forks.is_empty() {
        return None;
    }
    let rows = forks
        .iter()
        .map(|fork| {
            let name = fork.name.as_deref().unwrap_or("unnamed");
            format!(
                "- {name} (agent id {}): {}",
                fork.agent_id, fork.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "{WAKE_OPENING}\n{rows}\n\nSendMessage each one by its agent id to continue its work, to start its pull request watcher again in the background if it was waiting on one, and to start its hold again if it was waiting on the user. Then tell the user in one line which forks you woke."
    ))
}

pub(super) fn handoff_prompt(forks: &[SleepingFork]) -> Option<String> {
    if forks.is_empty() {
        return None;
    }
    let rows = forks
        .iter()
        .map(|fork| {
            let mut row = format!(
                "- {}: project {}, transcript {}",
                fork.description,
                fork.cwd.as_deref().unwrap_or("unknown"),
                fork.transcript.display()
            );
            for (label, value) in [("model", &fork.model), ("effort", &fork.effort)] {
                if let Some(value) = value {
                    row.push_str(&format!(", {label} {value}"));
                }
            }
            row
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "{HANDOFF_OPENING} so it began a new conversation, and these forks of the last one stopped with it:\n{rows}\n\nStart each one again with the Agent tool: subagent_type project-fork, its description as it is, or a `<key-name>: <friendly description>` one when it has no key-name, its model and effort when its row names them, and a prompt whose first line is `Project: <its project>`, followed by `You take over the work of a fork that stopped when the switchboard restarted. Its conversation is in <its transcript>: read the end of it to learn where it stopped and what it was waiting on, then carry on from there, and start its pull request watcher again in the background if it was waiting on one, or its hold if it was waiting on the user.` Then tell the user in one line which forks you started again."
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
        assert_eq!(ids(sleeping_forks(&home, board, "conv")), ["plain"]);
        conversation(&home, board, "conv", &notice("plain", "completed"));
        assert_eq!(ids(sleeping_forks(&home, board, "conv")), ["plain"]);
        conversation(&home, board, "conv", &notice("exited", "completed"));
        assert_eq!(
            ids(sleeping_forks(&home, board, "conv")),
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
        let woken = sleeping_forks(&home, board, "conv");
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
            }
        );
        assert!(sleeping_forks(&home, board, "missing").is_empty());
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
        sleeping_forks(home, board, "conv")
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

    fn sleeping(agent_id: &str, name: Option<&str>, description: &str) -> SleepingFork {
        SleepingFork {
            agent_id: agent_id.to_owned(),
            name: name.map(str::to_owned),
            description: description.to_owned(),
            cwd: None,
            model: None,
            effort: None,
            transcript: PathBuf::from(format!("/conv/subagents/agent-{agent_id}.jsonl")),
        }
    }

    #[test]
    fn the_handoff_prompt_starts_each_fork_again_from_its_transcript() {
        assert_eq!(handoff_prompt(&[]), None);
        let text = handoff_prompt(&[
            SleepingFork {
                cwd: Some("/work/a".to_owned()),
                model: Some("opus".to_owned()),
                effort: Some("max".to_owned()),
                ..sleeping("a1", None, "fix the bug")
            },
            sleeping("b2", None, "write docs"),
        ])
        .unwrap();
        assert!(
            text.contains("- fix the bug: project /work/a, transcript /conv/subagents/agent-a1.jsonl, model opus, effort max\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "- write docs: project unknown, transcript /conv/subagents/agent-b2.jsonl\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("Start each one again with the Agent tool: subagent_type project-fork"),
            "{text}"
        );
        assert!(
            text.contains("first line is `Project: <its project>`"),
            "{text}"
        );
        assert!(
            text.contains("Its conversation is in <its transcript>"),
            "{text}"
        );
        assert!(
            text.contains("pull request watcher again in the background"),
            "{text}"
        );
        assert!(
            text.contains("in one line which forks you started again"),
            "{text}"
        );
        assert!(!text.contains("SendMessage"), "{text}");
    }

    #[test]
    fn the_wake_prompt_lists_each_fork_and_is_absent_without_any() {
        assert_eq!(wake_prompt(&[]), None);
        let text = wake_prompt(&[
            sleeping("a1", Some("alpha"), "fix the bug"),
            sleeping("b2", None, "write docs"),
        ])
        .unwrap();
        assert!(
            text.contains("- alpha (agent id a1): fix the bug"),
            "{text}"
        );
        assert!(
            text.contains("- unnamed (agent id b2): write docs"),
            "{text}"
        );
        assert!(text.contains("SendMessage each one"), "{text}");
        assert!(text.contains("its hold again"), "{text}");
        assert!(
            text.contains("pull request watcher again in the background"),
            "{text}"
        );
        assert!(text.contains("in one line which forks you woke"), "{text}");
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
