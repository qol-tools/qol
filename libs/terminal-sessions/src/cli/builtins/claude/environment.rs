use std::fs;
use std::path::{Path, PathBuf};

use qol_agent_homes::{Harness, Registry};
use serde::Deserialize;

#[derive(Clone)]
pub(super) struct ClaudeSessionLocation {
    pub external_id: String,
    pub transcript_path: PathBuf,
}

pub(super) trait ClaudeEnvironment: Send + Sync {
    fn session(&self, pid: i32) -> Option<ClaudeSessionLocation>;

    fn remote_controlled(&self, _pid: i32) -> bool {
        false
    }
}

pub(super) struct SystemClaudeEnvironment;

impl ClaudeEnvironment for SystemClaudeEnvironment {
    fn session(&self, pid: i32) -> Option<ClaudeSessionLocation> {
        let home = Registry::load().current(Harness::Claude).path;
        let record = read_record(&home, pid)?;
        if record.session_id.is_empty() || record.cwd.is_empty() {
            return None;
        }
        let project = encode_project_dir(&record.cwd);
        let transcript_path = home
            .join("projects")
            .join(project)
            .join(format!("{}.jsonl", record.session_id));
        transcript_path.is_file().then_some(ClaudeSessionLocation {
            external_id: record.session_id,
            transcript_path,
        })
    }

    fn remote_controlled(&self, pid: i32) -> bool {
        read_record(&Registry::load().current(Harness::Claude).path, pid)
            .is_some_and(|record| record.bridge_session_id.is_some_and(|id| !id.is_empty()))
    }
}

fn read_record(home: &Path, pid: i32) -> Option<ClaudeSessionRecord> {
    let path = home.join("sessions").join(format!("{pid}.json"));
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

#[derive(Deserialize)]
struct ClaudeSessionRecord {
    #[serde(rename = "sessionId")]
    session_id: String,
    cwd: String,
    #[serde(rename = "bridgeSessionId", default)]
    bridge_session_id: Option<String>,
}

fn encode_project_dir(cwd: &str) -> String {
    cwd.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_replaces_every_non_alphanumeric_with_a_dash() {
        assert_eq!(
            encode_project_dir("/home/u/my project/Work (v2)"),
            "-home-u-my-project-Work--v2-"
        );
    }

    #[test]
    fn only_a_live_bridge_session_id_marks_remote_control() {
        let home = tempfile::TempDir::new().unwrap();
        let sessions = home.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let cases = [
            (r#"{"sessionId":"s","cwd":"/w"}"#, false),
            (
                r#"{"sessionId":"s","cwd":"/w","bridgeSessionId":null}"#,
                false,
            ),
            (
                r#"{"sessionId":"s","cwd":"/w","bridgeSessionId":""}"#,
                false,
            ),
            (
                r#"{"sessionId":"s","cwd":"/w","bridgeSessionId":"session_01"}"#,
                true,
            ),
        ];
        for (record, expected) in cases {
            std::fs::write(sessions.join("7.json"), record).unwrap();
            let marked = read_record(home.path(), 7)
                .is_some_and(|record| record.bridge_session_id.is_some_and(|id| !id.is_empty()));
            assert_eq!(marked, expected, "{record}");
        }
        assert!(read_record(home.path(), 8).is_none());
    }

    #[test]
    fn the_system_environment_resolves_sessions_under_the_registry_home() {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();
        let root = tempfile::TempDir::new().unwrap();
        let home = root.path().join("claude-home");
        let sessions = home.join("sessions");
        let projects = home.join("projects").join("-work-proj");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(
            sessions.join("7.json"),
            r#"{"sessionId":"s-7","cwd":"/work/proj"}"#,
        )
        .unwrap();
        std::fs::write(projects.join("s-7.jsonl"), "{}\n").unwrap();
        let previous = std::env::var_os("CLAUDE_CONFIG_DIR");
        std::env::set_var("CLAUDE_CONFIG_DIR", &home);
        let location = SystemClaudeEnvironment.session(7);
        match previous {
            Some(value) => std::env::set_var("CLAUDE_CONFIG_DIR", value),
            None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
        }
        let location = location.expect("the registry home holds the session and its transcript");
        assert_eq!(location.external_id, "s-7");
        assert_eq!(
            location.transcript_path,
            home.join("projects").join("-work-proj").join("s-7.jsonl")
        );
    }
}
