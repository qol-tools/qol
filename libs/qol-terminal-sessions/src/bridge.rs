use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct BridgeCheckpoint {
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub driver: String,
    pub completion_marker: String,
    pub completed: bool,
    pub closed: bool,
}

pub fn checkpoint_dir() -> Option<PathBuf> {
    qol_config::data_subdir("sessions").map(|path| path.join("pending-bridge"))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct LiveBridges {
    pub driven: HashSet<String>,
    pub driving: HashMap<String, usize>,
}

pub fn live_sessions(dir: &Path) -> LiveBridges {
    let Ok(entries) = fs::read_dir(dir) else {
        return LiveBridges::default();
    };
    let mut live = LiveBridges::default();
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
        live.driven.insert(checkpoint.session);
        if !checkpoint.driver.is_empty() {
            *live.driving.entry(checkpoint.driver).or_default() += 1;
        }
    }
    live
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint(
        dir: &Path,
        stem: &str,
        session: &str,
        driver: &str,
        completed: bool,
        closed: bool,
    ) -> PathBuf {
        let path = dir.join(format!("{stem}.json"));
        fs::write(
            &path,
            serde_json::to_string(&BridgeCheckpoint {
                session: session.to_owned(),
                driver: driver.to_owned(),
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
        checkpoint(
            root.path(),
            "a",
            "v1:kitty:1:10",
            "v1:kitty:9:90",
            false,
            false,
        );
        checkpoint(
            root.path(),
            "b",
            "v1:kitty:2:20",
            "v1:kitty:9:90",
            true,
            false,
        );
        checkpoint(
            root.path(),
            "c",
            "v1:kitty:3:30",
            "v1:kitty:9:90",
            true,
            true,
        );
        checkpoint(root.path(), "d", "v1:kitty:4:40", "", true, false);

        let live = live_sessions(root.path());
        assert_eq!(
            live.driven,
            HashSet::from([
                "v1:kitty:1:10".to_owned(),
                "v1:kitty:2:20".to_owned(),
                "v1:kitty:4:40".to_owned(),
            ])
        );
        assert_eq!(
            live.driving,
            HashMap::from([("v1:kitty:9:90".to_owned(), 2)])
        );
    }

    #[test]
    fn a_missing_directory_reports_no_live_bridges() {
        let root = tempfile::TempDir::new().unwrap();
        assert_eq!(
            live_sessions(&root.path().join("absent")),
            LiveBridges::default()
        );
    }
}
