use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use qol_agent_homes::{Harness, Registry};
use serde::Deserialize;

#[derive(Clone)]
pub(super) struct KimiSessionLocation {
    pub session_id: String,
    pub state_path: PathBuf,
}

pub(super) fn newest_write(session_dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut pending = vec![session_dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push(path);
            } else if let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) {
                if newest.is_none_or(|newest| modified > newest) {
                    newest = Some(modified);
                }
            }
        }
    }
    newest
}

pub(super) trait KimiEnvironment: Send + Sync {
    fn session(&self, cwd: &str) -> Option<KimiSessionLocation>;
}

pub(super) struct SystemKimiEnvironment;

impl KimiEnvironment for SystemKimiEnvironment {
    fn session(&self, cwd: &str) -> Option<KimiSessionLocation> {
        let index = kimi_home()?.join("session_index.jsonl");
        let file = fs::File::open(index).ok()?;
        let mut newest: Option<(SystemTime, KimiSessionLocation)> = None;
        for line in BufRead::lines(BufReader::new(file)) {
            let Ok(line) = line else {
                continue;
            };
            let entry = match serde_json::from_str::<IndexEntry>(&line) {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            if entry.work_dir != cwd {
                continue;
            }
            let state_path = PathBuf::from(entry.session_dir).join("state.json");
            let Some(modified) = newest_write(state_path.parent()?) else {
                continue;
            };
            let replace = newest
                .as_ref()
                .is_none_or(|(current, _)| modified > *current);
            if replace {
                newest = Some((
                    modified,
                    KimiSessionLocation {
                        session_id: entry.session_id,
                        state_path,
                    },
                ));
            }
        }
        newest.map(|(_, location)| location)
    }
}

fn kimi_home() -> Option<PathBuf> {
    Some(Registry::load().current(Harness::Kimi).path)
}

#[derive(Deserialize)]
struct IndexEntry {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "sessionDir")]
    session_dir: String,
    #[serde(rename = "workDir")]
    work_dir: String,
}
