use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Clone)]
pub(super) struct ClaudeSessionLocation {
    pub external_id: String,
    pub transcript_path: PathBuf,
}

pub(super) trait ClaudeEnvironment: Send + Sync {
    fn session(&self, pid: i32) -> Option<ClaudeSessionLocation>;
}

pub(super) struct SystemClaudeEnvironment;

impl ClaudeEnvironment for SystemClaudeEnvironment {
    fn session(&self, pid: i32) -> Option<ClaudeSessionLocation> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        let path = home
            .join(".claude")
            .join("sessions")
            .join(format!("{pid}.json"));
        let record =
            serde_json::from_str::<ClaudeSessionRecord>(&fs::read_to_string(path).ok()?).ok()?;
        if record.session_id.is_empty() || record.cwd.is_empty() {
            return None;
        }
        let project = record
            .cwd
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '-'
                }
            })
            .collect::<String>();
        let transcript_path = home
            .join(".claude")
            .join("projects")
            .join(project)
            .join(format!("{}.jsonl", record.session_id));
        transcript_path.is_file().then_some(ClaudeSessionLocation {
            external_id: record.session_id,
            transcript_path,
        })
    }
}

#[derive(Deserialize)]
struct ClaudeSessionRecord {
    #[serde(rename = "sessionId")]
    session_id: String,
    cwd: String,
}
