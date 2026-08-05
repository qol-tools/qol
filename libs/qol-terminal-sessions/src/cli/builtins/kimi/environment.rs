use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
    if let Some(dir) = std::env::var_os("KIMI_CODE_HOME") {
        let dir = PathBuf::from(dir);
        return expand_tilde(dir);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".kimi-code"))
}

fn expand_tilde(path: PathBuf) -> Option<PathBuf> {
    let text = path.to_str()?;
    if text != "~" && !text.starts_with("~/") {
        return Some(path);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(expand_tilde_from(text, &home))
}

fn expand_tilde_from(text: &str, home: &Path) -> PathBuf {
    if text == "~" || text == "~/" {
        return home.to_path_buf();
    }
    if let Some(relative) = text.strip_prefix("~/") {
        return home.join(relative);
    }
    PathBuf::from(text)
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::expand_tilde_from;

    #[test]
    fn literal_tilde_paths_stay_under_home() {
        let home = Path::new("/home/tester");
        let cases = [
            ("~", home.to_path_buf()),
            ("~/", home.to_path_buf()),
            ("~/.kimi-code", home.join(".kimi-code")),
            ("~someone/kimi", PathBuf::from("~someone/kimi")),
            ("relative", PathBuf::from("relative")),
            ("/opt/kimi", PathBuf::from("/opt/kimi")),
        ];

        for (input, expected) in cases {
            assert_eq!(expand_tilde_from(input, home), expected, "input: {input}");
        }
    }
}
