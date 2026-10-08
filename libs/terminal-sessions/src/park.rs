use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParkState {
    Waiting,
    Delivered,
    Resumed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ParkRecord {
    pub id: String,
    pub tool: String,
    pub cwd: String,
    pub external_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    pub session: String,
    pub command: Vec<String>,
    pub created_at: u64,
    pub state: ParkState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_identity: Option<String>,
    #[serde(default)]
    pub caller_closed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub fn parked_dir() -> Option<PathBuf> {
    qol_config::data_subdir("sessions").map(|path| path.join("parked"))
}

pub fn records(dir: &Path) -> std::io::Result<Vec<ParkRecord>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut records = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|encoded| serde_json::from_str::<ParkRecord>(&encoded).ok())
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(records)
}

pub fn waiting(dir: &Path) -> Vec<ParkRecord> {
    records(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| record.state == ParkState::Waiting)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, created_at: u64, state: ParkState) -> ParkRecord {
        ParkRecord {
            id: id.to_owned(),
            tool: "claude".to_owned(),
            cwd: "/repo".to_owned(),
            external_id: "abc".to_owned(),
            model: None,
            effort: None,
            title: None,
            permission_mode: None,
            session: "v1:kitty:k1_f1.2:3".to_owned(),
            command: vec!["sleep".to_owned(), "5".to_owned()],
            created_at,
            state,
            runner_pid: Some(7),
            runner_identity: None,
            caller_closed: false,
            exit_code: None,
            resumed_session: None,
            detail: None,
        }
    }

    fn write(dir: &Path, record: &ParkRecord) {
        fs::write(
            dir.join(format!("{}.json", record.id)),
            serde_json::to_string(record).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn a_missing_directory_has_no_records() {
        let root = tempfile::tempdir().unwrap();
        assert!(records(&root.path().join("parked")).unwrap().is_empty());
        assert!(waiting(&root.path().join("parked")).is_empty());
    }

    #[test]
    fn records_are_newest_first_and_skip_logs_and_corrupt_files() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), &record("old", 1, ParkState::Waiting));
        write(root.path(), &record("new", 2, ParkState::Resumed));
        fs::write(root.path().join("new.log"), "output").unwrap();
        fs::write(root.path().join("broken.json"), "{").unwrap();
        let ids: Vec<String> = records(root.path())
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids, ["new", "old"]);
    }

    #[test]
    fn waiting_keeps_only_records_still_waiting() {
        let root = tempfile::tempdir().unwrap();
        for (id, state) in [
            ("w", ParkState::Waiting),
            ("d", ParkState::Delivered),
            ("r", ParkState::Resumed),
            ("f", ParkState::Failed),
        ] {
            write(root.path(), &record(id, 1, state));
        }
        let ids: Vec<String> = waiting(root.path())
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids, ["w"]);
    }
}
