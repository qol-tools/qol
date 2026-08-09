use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct BridgeCheckpoint {
    #[serde(default)]
    pub session: String,
    pub completion_marker: String,
    pub completed: bool,
    pub closed: bool,
}

pub fn checkpoint_dir() -> Option<PathBuf> {
    qol_config::data_subdir("sessions").map(|path| path.join("pending-bridge"))
}

pub fn live_sessions(dir: &Path) -> HashSet<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return HashSet::new();
    };
    let mut live = HashSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let Ok(encoded) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(checkpoint) = serde_json::from_str::<BridgeCheckpoint>(&encoded) else {
            continue;
        };
        if checkpoint.closed || checkpoint.session.is_empty() {
            continue;
        }
        live.insert(checkpoint.session);
    }
    live
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint(dir: &Path, stem: &str, session: &str, completed: bool, closed: bool) -> PathBuf {
        let path = dir.join(format!("{stem}.json"));
        fs::write(
            &path,
            serde_json::to_string(&BridgeCheckpoint {
                session: session.to_owned(),
                completion_marker: "MARK".to_owned(),
                completed,
                closed,
            })
            .unwrap(),
        )
        .unwrap();
        path
    }

    #[test]
    fn every_open_loop_is_live_until_closed() {
        let root = tempfile::TempDir::new().unwrap();
        checkpoint(root.path(), "a", "v1:kitty:1:10", false, false);
        checkpoint(root.path(), "b", "v1:kitty:2:20", true, false);
        checkpoint(root.path(), "c", "v1:kitty:3:30", true, true);

        assert_eq!(
            live_sessions(root.path()),
            HashSet::from(["v1:kitty:1:10".to_owned(), "v1:kitty:2:20".to_owned()])
        );
    }

    #[test]
    fn a_missing_directory_reports_no_live_bridges() {
        let root = tempfile::TempDir::new().unwrap();
        assert!(live_sessions(&root.path().join("absent")).is_empty());
    }
}
