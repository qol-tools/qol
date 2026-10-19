use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const RETENTION_SECS: u64 = 14 * 24 * 60 * 60;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ClosedRecord {
    pub external_id: String,
    pub tool: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lane: bool,
    pub closed_at: u64,
}

pub fn closed_dir() -> Option<PathBuf> {
    qol_config::data_subdir("sessions").map(|path| path.join("closed"))
}

fn record_path(dir: &Path, external_id: &str) -> PathBuf {
    let name = external_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    dir.join(format!("{name}.json"))
}

pub fn write(dir: &Path, record: &ClosedRecord) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = record_path(dir, &record.external_id);
    let staging = path.with_extension("json.tmp");
    fs::write(&staging, serde_json::to_vec(record)?)?;
    fs::rename(staging, path)
}

pub fn forget(dir: &Path, external_id: &str) -> std::io::Result<()> {
    remove_if_present(&record_path(dir, external_id))
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn find(dir: &Path, external_id: &str) -> Option<ClosedRecord> {
    let encoded = fs::read_to_string(record_path(dir, external_id)).ok()?;
    serde_json::from_str::<ClosedRecord>(&encoded)
        .ok()
        .filter(|record| record.external_id == external_id)
}

fn filed_records(dir: &Path) -> std::io::Result<Vec<(PathBuf, ClosedRecord)>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| {
            let encoded = fs::read_to_string(&path).ok()?;
            let record = serde_json::from_str::<ClosedRecord>(&encoded).ok()?;
            Some((path, record))
        })
        .collect())
}

pub fn records(dir: &Path) -> std::io::Result<Vec<ClosedRecord>> {
    let mut records = filed_records(dir)?
        .into_iter()
        .map(|(_, record)| record)
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        right
            .closed_at
            .cmp(&left.closed_at)
            .then_with(|| left.external_id.cmp(&right.external_id))
    });
    Ok(records)
}

pub fn expired(record: &ClosedRecord, now: u64) -> bool {
    record.closed_at.saturating_add(RETENTION_SECS) < now
        || record.closed_at > now.saturating_add(RETENTION_SECS)
}

pub fn prune(dir: &Path, now: u64) -> std::io::Result<()> {
    let mut first_error = None;
    for (path, record) in filed_records(dir)? {
        if !expired(&record, now) {
            continue;
        }
        if let Err(error) = remove_if_present(&path) {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(external_id: &str, closed_at: u64) -> ClosedRecord {
        ClosedRecord {
            external_id: external_id.to_owned(),
            tool: "claude".to_owned(),
            cwd: "/project".to_owned(),
            title: Some("Old chat".to_owned()),
            key: None,
            permission_mode: Some("bypassPermissions".to_owned()),
            model: None,
            effort: None,
            claude_config_dir: None,
            lane: false,
            closed_at,
        }
    }

    #[test]
    fn records_are_newest_first_and_found_by_their_external_id() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), &record("old", 10)).unwrap();
        write(root.path(), &record("new", 20)).unwrap();
        fs::write(root.path().join("broken.json"), "{").unwrap();
        let ids: Vec<String> = records(root.path())
            .unwrap()
            .into_iter()
            .map(|record| record.external_id)
            .collect();
        assert_eq!(ids, ["new", "old"]);
        assert_eq!(find(root.path(), "old"), Some(record("old", 10)));
        assert_eq!(find(root.path(), "missing"), None);
        assert!(records(&root.path().join("absent")).unwrap().is_empty());
    }

    #[test]
    fn a_rewrite_replaces_the_record_and_forget_removes_it_once() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), &record("abc", 10)).unwrap();
        write(root.path(), &record("abc", 30)).unwrap();
        assert_eq!(records(root.path()).unwrap().len(), 1);
        assert_eq!(find(root.path(), "abc").unwrap().closed_at, 30);
        forget(root.path(), "abc").unwrap();
        forget(root.path(), "abc").unwrap();
        assert_eq!(find(root.path(), "abc"), None);
    }

    #[test]
    fn ids_that_would_escape_the_directory_are_filed_inside_it() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), &record("../x/y", 10)).unwrap();
        assert_eq!(
            find(root.path(), "../x/y").map(|record| record.external_id),
            Some("../x/y".to_owned())
        );
        assert!(root.path().join("___x_y.json").is_file());
    }

    #[test]
    fn prune_drops_records_older_than_the_retention() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), &record("stale", 0)).unwrap();
        write(root.path(), &record("kept", 100)).unwrap();
        prune(root.path(), RETENTION_SECS + 50).unwrap();
        let ids: Vec<String> = records(root.path())
            .unwrap()
            .into_iter()
            .map(|record| record.external_id)
            .collect();
        assert_eq!(ids, ["kept"]);
    }

    #[test]
    fn resume_fields_round_trip_and_stay_out_of_the_json_when_unset() {
        let root = tempfile::tempdir().unwrap();
        let full = ClosedRecord {
            model: Some("claude-opus-5-5".to_owned()),
            effort: Some("max".to_owned()),
            claude_config_dir: Some("/home/u/.claude-work".to_owned()),
            lane: true,
            ..record("full", 10)
        };
        write(root.path(), &full).unwrap();
        assert_eq!(find(root.path(), "full"), Some(full));

        let encoded = serde_json::to_value(record("bare", 10)).unwrap();
        for key in ["model", "effort", "claude_config_dir", "lane"] {
            assert!(encoded.get(key).is_none(), "{key}");
        }

        fs::write(
            root.path().join("old.json"),
            r#"{"external_id":"old","tool":"claude","cwd":"/project","closed_at":10}"#,
        )
        .unwrap();
        let old = find(root.path(), "old").expect("a record without the new fields still parses");
        assert_eq!(old.model, None);
        assert_eq!(old.effort, None);
        assert_eq!(old.claude_config_dir, None);
        assert!(!old.lane);
    }

    #[test]
    fn expired_keeps_the_retention_window_and_drops_records_outside_it() {
        let now = 10 * RETENTION_SECS;
        let cases = [
            (now, false),
            (now - RETENTION_SECS, false),
            (now - RETENTION_SECS - 1, true),
            (now + RETENTION_SECS, false),
            (now + RETENTION_SECS + 1, true),
        ];
        for (closed_at, expected) in cases {
            assert_eq!(
                expired(&record("abc", closed_at), now),
                expected,
                "closed_at: {closed_at}"
            );
        }
    }

    #[test]
    fn prune_removes_an_expired_record_filed_under_another_name() {
        let root = tempfile::tempdir().unwrap();
        let misfiled = root.path().join("renamed.json");
        fs::write(&misfiled, serde_json::to_vec(&record("stale", 0)).unwrap()).unwrap();
        write(root.path(), &record("kept", 100)).unwrap();
        prune(root.path(), RETENTION_SECS + 50).unwrap();
        assert!(!misfiled.exists());
        assert_eq!(find(root.path(), "kept"), Some(record("kept", 100)));
    }
}
