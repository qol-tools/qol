use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cli::tail::FileStamp;

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

const ROLE_RECORD_PREFIX: &str = "role-";

pub fn checkpoint_dir() -> Option<PathBuf> {
    qol_config::data_subdir("sessions").map(|path| path.join("pending-bridge"))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct LiveBridges {
    pub driven: HashSet<String>,
    pub driving: HashMap<String, Vec<String>>,
}

#[derive(Default)]
pub struct LiveBridgeReader {
    rounds: HashMap<PathBuf, ReadRound>,
}

struct ReadRound {
    stamp: FileStamp,
    open: Option<OpenRound>,
}

#[derive(Clone)]
struct OpenRound {
    session: String,
    driver: String,
}

impl LiveBridgeReader {
    pub fn read(&mut self, dir: &Path) -> LiveBridges {
        let Ok(entries) = fs::read_dir(dir) else {
            self.rounds.clear();
            return LiveBridges::default();
        };
        let mut rounds = HashMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") || is_role_record(&path)
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let stamp = FileStamp::of(&metadata);
            let open = match self.rounds.remove(&path) {
                Some(known) if known.stamp == stamp => known.open,
                _ => open_round(&path),
            };
            rounds.insert(path, ReadRound { stamp, open });
        }
        self.rounds = rounds;
        let mut live = LiveBridges::default();
        for round in self.rounds.values().filter_map(|round| round.open.clone()) {
            if !round.driver.is_empty() {
                live.driving
                    .entry(round.driver)
                    .or_default()
                    .push(round.session.clone());
            }
            live.driven.insert(round.session);
        }
        for driven in live.driving.values_mut() {
            driven.sort();
        }
        live
    }
}

fn open_round(path: &Path) -> Option<OpenRound> {
    let checkpoint =
        serde_json::from_str::<BridgeCheckpoint>(&fs::read_to_string(path).ok()?).ok()?;
    (!checkpoint.closed && !checkpoint.session.is_empty()).then_some(OpenRound {
        session: checkpoint.session,
        driver: checkpoint.driver,
    })
}

fn is_role_record(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(ROLE_RECORD_PREFIX))
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

        let live = LiveBridgeReader::default().read(root.path());
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
            HashMap::from([(
                "v1:kitty:9:90".to_owned(),
                vec!["v1:kitty:1:10".to_owned(), "v1:kitty:2:20".to_owned()]
            )])
        );
    }

    #[test]
    fn the_reader_rereads_only_rounds_whose_file_changed() {
        let root = tempfile::TempDir::new().unwrap();
        let path = checkpoint(root.path(), "a", "v1:kitty:1:10", "", false, false);
        let mut reader = LiveBridgeReader::default();
        let only = |token: &str| LiveBridges {
            driven: HashSet::from([token.to_owned()]),
            driving: HashMap::new(),
        };
        assert_eq!(reader.read(root.path()), only("v1:kitty:1:10"));

        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        checkpoint(root.path(), "a", "v1:kitty:2:20", "", false, false);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(
            reader.read(root.path()),
            only("v1:kitty:1:10"),
            "a file with the same size and mtime is not parsed again"
        );

        checkpoint(root.path(), "a", "v1:kitty:2:20", "", true, true);
        assert_eq!(reader.read(root.path()), LiveBridges::default());

        fs::remove_file(&path).unwrap();
        checkpoint(root.path(), "b", "v1:kitty:3:30", "", false, false);
        assert_eq!(reader.read(root.path()), only("v1:kitty:3:30"));
    }

    #[test]
    fn role_records_are_never_read_as_rounds() {
        let root = tempfile::TempDir::new().unwrap();
        checkpoint(root.path(), "role-a", "v1:kitty:1:10", "", false, false);
        assert_eq!(
            LiveBridgeReader::default().read(root.path()),
            LiveBridges::default()
        );
    }

    #[test]
    fn a_missing_directory_reports_no_live_bridges() {
        let root = tempfile::TempDir::new().unwrap();
        assert_eq!(
            LiveBridgeReader::default().read(&root.path().join("absent")),
            LiveBridges::default()
        );
    }
}
