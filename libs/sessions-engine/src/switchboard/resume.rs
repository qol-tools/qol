use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use super::paths::conversations_dir;

pub(super) const FORK_AGENT: &str = "project-fork";
const META_SUFFIX: &str = ".meta.json";
const AGENT_PREFIX: &str = "agent-";
const BACKGROUND_MARKER: &str = "Command running in background with ID: ";
const WORKTREE_PIN: &str = "worktreePath";
const WORKTREE_KEYS: [&str; 3] = [WORKTREE_PIN, "worktreeBranch", "spawnedWithWorktree"];

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SleepingFork {
    pub(super) agent_id: String,
    pub(super) name: Option<String>,
    pub(super) description: String,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Meta {
    agent_type: String,
    description: String,
    name: Option<String>,
    finished: bool,
    stopped_by_user: bool,
}

fn answered_under_revision(path: &Path, revision: &str) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let (mut answered, mut current) = (false, false);
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        answered |= line.contains("\"type\":\"assistant\"");
        current |= line.contains(revision);
        if answered && current {
            return true;
        }
    }
    false
}

pub(super) fn latest_conversation(
    config_home: &Path,
    switchboard: &Path,
    revision: &str,
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
                if !answered_under_revision(&path, revision) {
                    return None;
                }
                Some((entry.metadata().ok()?.modified().ok()?, id))
            })
            .collect();
    found.sort();
    found.pop().map(|(_, id)| id)
}

pub(super) fn subagents_dir(config_home: &Path, switchboard: &Path, conversation: &str) -> PathBuf {
    conversations_dir(config_home, switchboard)
        .join(conversation)
        .join("subagents")
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

fn pending_background(transcript: &str) -> bool {
    let mut open: Vec<String> = Vec::new();
    for line in transcript.lines() {
        if let Some((_, rest)) = line.split_once(BACKGROUND_MARKER) {
            let id: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if !id.is_empty() {
                open.push(id);
            }
            continue;
        }
        if open.is_empty() || !open.iter().any(|id| line.contains(id.as_str())) {
            continue;
        }
        let is_user =
            serde_json::from_str::<serde_json::Value>(line).is_ok_and(|row| row["type"] == "user");
        if is_user {
            open.retain(|id| !line.contains(id.as_str()));
        }
    }
    !open.is_empty()
}

fn agent_id_of(meta_path: &Path) -> Option<String> {
    let name = meta_path.file_name()?.to_str()?;
    Some(
        name.strip_prefix(AGENT_PREFIX)?
            .strip_suffix(META_SUFFIX)?
            .to_owned(),
    )
}

pub(super) fn sleeping_forks(subagents: &Path) -> Vec<SleepingFork> {
    meta_files(subagents)
        .into_iter()
        .filter_map(|path| {
            let meta: Meta = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
            if meta.agent_type != FORK_AGENT || meta.stopped_by_user {
                return None;
            }
            let agent_id = agent_id_of(&path)?;
            let asleep = !meta.finished || {
                let transcript = path.with_file_name(format!("{AGENT_PREFIX}{agent_id}.jsonl"));
                fs::read_to_string(transcript).is_ok_and(|text| pending_background(&text))
            };
            asleep.then_some(SleepingFork {
                agent_id,
                name: meta.name,
                description: meta.description,
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
        "The switchboard restarted, which stopped these forks:\n{rows}\n\nSendMessage each one by its agent id to continue its work, and to start its pull request watcher again in the background if it was waiting on one. Then tell the user in one line which forks you woke."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn only_unfinished_or_background_waiting_project_forks_are_woken() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("subagents");
        fork(
            &dir,
            "mid",
            r#"{"agentType":"project-fork","description":"mid turn","name":"mid"}"#,
            "",
        );
        fork(
            &dir,
            "done",
            r#"{"agentType":"project-fork","description":"done","finished":true}"#,
            r#"{"type":"assistant"}"#,
        );
        fork(
            &dir,
            "stopped",
            r#"{"agentType":"project-fork","description":"x","stoppedByUser":true}"#,
            "",
        );
        fork(
            &dir,
            "other",
            r#"{"agentType":"Explore","description":"x"}"#,
            "",
        );
        let waiting = format!(
            "{{\"type\":\"user\",\"message\":\"{BACKGROUND_MARKER}bw1abc. Output is\"}}\n{{\"type\":\"assistant\",\"text\":\"waiting for bw1abc\"}}\n"
        );
        fork(
            &dir,
            "asleep",
            r#"{"agentType":"project-fork","description":"watching","finished":true,"name":"zed"}"#,
            &waiting,
        );
        let nested = dir.join("agent-parent");
        fork(
            &nested,
            "deep",
            r#"{"agentType":"project-fork","description":"deep"}"#,
            "",
        );
        let woken: Vec<(String, Option<String>)> = sleeping_forks(&dir)
            .into_iter()
            .map(|fork| (fork.agent_id, fork.name))
            .collect();
        assert_eq!(
            woken,
            [
                ("asleep".to_owned(), Some("zed".to_owned())),
                ("mid".to_owned(), Some("mid".to_owned())),
                ("deep".to_owned(), None),
            ]
        );
    }

    #[test]
    fn a_background_command_is_done_once_a_user_row_mentions_its_id() {
        let started = format!("{{\"type\":\"user\",\"c\":\"{BACKGROUND_MARKER}b42x. Output\"}}\n");
        let says = "{\"type\":\"assistant\",\"text\":\"waiting on b42x\"}\n";
        let notice = "{\"type\":\"user\",\"c\":\"task b42x completed\"}\n";
        assert!(pending_background(&format!("{started}{says}")));
        assert!(!pending_background(&format!("{started}{says}{notice}")));
        assert!(!pending_background(says));
    }

    #[test]
    fn the_wake_prompt_lists_each_fork_and_is_absent_without_any() {
        assert_eq!(wake_prompt(&[]), None);
        let text = wake_prompt(&[
            SleepingFork {
                agent_id: "a1".to_owned(),
                name: Some("alpha".to_owned()),
                description: "fix the bug".to_owned(),
            },
            SleepingFork {
                agent_id: "b2".to_owned(),
                name: None,
                description: "write docs".to_owned(),
            },
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
