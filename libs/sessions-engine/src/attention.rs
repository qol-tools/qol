use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "attention.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionSnapshot {
    pub written_at_ms: u64,
    pub sessions: Vec<AttentionRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionRow {
    pub key: Option<String>,
    pub title: String,
    pub state: AttentionState,
    pub master: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prs: Vec<LanePr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanePr {
    pub url: String,
    pub number: u64,
    pub state: PrState,
    pub draft: bool,
    pub checks: PrChecks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrChecks {
    Pending,
    Passing,
    Failing,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionState {
    NeedsYou,
    YourTurn,
    Working,
    Idle,
    Parked,
    Ready,
}

pub fn snapshot_path() -> Option<PathBuf> {
    Some(qol_config::data_subdir("sessions")?.join(FILE_NAME))
}

pub fn write(snapshot: &AttentionSnapshot) -> std::io::Result<()> {
    let path = snapshot_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no qol data directory for the attention snapshot",
        )
    })?;
    write_to(&path, snapshot)
}

pub fn read() -> Option<AttentionSnapshot> {
    read_from(&snapshot_path()?)
}

pub fn write_to(path: &Path, snapshot: &AttentionSnapshot) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let encoded = serde_json::to_vec(snapshot).map_err(std::io::Error::other)?;
    qol_fs::atomic_write(path, &encoded)
}

pub fn read_from(path: &Path) -> Option<AttentionSnapshot> {
    let encoded = std::fs::read(path).ok()?;
    serde_json::from_slice(&encoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> AttentionSnapshot {
        AttentionSnapshot {
            written_at_ms: 1_700_000_000_123,
            sessions: vec![
                AttentionRow {
                    key: Some("switchboard".to_owned()),
                    title: "switchboard".to_owned(),
                    state: AttentionState::Ready,
                    master: true,
                    prs: Vec::new(),
                },
                AttentionRow {
                    key: None,
                    title: "qol-monorepo".to_owned(),
                    state: AttentionState::NeedsYou,
                    master: false,
                    prs: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn a_written_snapshot_reads_back_unchanged() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("sessions").join(FILE_NAME);
        write_to(&path, &snapshot()).unwrap();
        assert_eq!(read_from(&path), Some(snapshot()));

        let mut later = snapshot();
        later.written_at_ms += 1_000;
        later.sessions.pop();
        write_to(&path, &later).unwrap();
        assert_eq!(read_from(&path), Some(later));
    }

    #[test]
    fn a_missing_or_corrupt_snapshot_reads_as_none() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join(FILE_NAME);
        assert_eq!(read_from(&path), None);
        std::fs::write(&path, b"{\"written_at_ms\":").unwrap();
        assert_eq!(read_from(&path), None);
    }

    #[test]
    fn states_serialize_in_snake_case() {
        let cases = [
            (AttentionState::NeedsYou, "\"needs_you\""),
            (AttentionState::YourTurn, "\"your_turn\""),
            (AttentionState::Working, "\"working\""),
            (AttentionState::Idle, "\"idle\""),
            (AttentionState::Parked, "\"parked\""),
            (AttentionState::Ready, "\"ready\""),
        ];
        for (state, expected) in cases {
            assert_eq!(serde_json::to_string(&state).unwrap(), expected);
        }
    }
}
