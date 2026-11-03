use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use super::paths::conversations_dir;

pub(super) const FORK_AGENT: &str = "project-fork";
const META_SUFFIX: &str = ".meta.json";
pub(super) const AGENT_PREFIX: &str = "agent-";
const WORKTREE_PIN: &str = "worktreePath";
const WORKTREE_KEYS: [&str; 3] = [WORKTREE_PIN, "worktreeBranch", "spawnedWithWorktree"];

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

#[cfg(test)]
mod tests {
    use super::*;

    fn fork(subagents: &Path, id: &str, meta: &str, transcript: &str) {
        fs::create_dir_all(subagents).unwrap();
        fs::write(subagents.join(format!("agent-{id}.meta.json")), meta).unwrap();
        fs::write(subagents.join(format!("agent-{id}.jsonl")), transcript).unwrap();
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
