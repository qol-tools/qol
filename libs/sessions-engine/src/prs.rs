use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::fork::{ForkRecord, ForkStore};

const GITHUB: &str = "https://github.com/";
const PULL: &str = "/pull/";

pub(super) fn pull_requests(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (start, _) in text.match_indices(GITHUB) {
        let Some(url) = pull_request_at(&text[start + GITHUB.len()..]) else {
            continue;
        };
        if !found.contains(&url) {
            found.push(url);
        }
    }
    found
}

fn pull_request_at(rest: &str) -> Option<String> {
    let owner = name_prefix(rest);
    let rest = rest[owner.len()..].strip_prefix('/')?;
    let repo = name_prefix(rest);
    let rest = rest[repo.len()..].strip_prefix(PULL)?;
    let number = rest
        .find(|character: char| !character.is_ascii_digit())
        .map_or(rest, |end| &rest[..end]);
    (!owner.is_empty() && !repo.is_empty() && !number.is_empty())
        .then(|| format!("{GITHUB}{owner}/{repo}{PULL}{number}"))
}

fn name_prefix(text: &str) -> &str {
    let end = text
        .find(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
        })
        .unwrap_or(text.len());
    &text[..end]
}

fn merge(into: &mut Vec<String>, prs: &[String]) -> bool {
    let before = into.len();
    for pr in prs {
        if !into.contains(pr) {
            into.push(pr.clone());
        }
    }
    into.len() != before
}

pub(super) struct PrStore {
    dir: PathBuf,
}

impl PrStore {
    pub(super) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub(super) fn system() -> Result<Self> {
        let dir = qol_config::data_subdir("sessions")
            .context("sessions data directory is unavailable")?
            .join("prs");
        Ok(Self { dir })
    }

    fn path(&self, key: &str) -> Result<PathBuf> {
        Ok(self
            .dir
            .join(format!("{}.json", super::mailbox::key_component(key)?)))
    }

    pub(super) fn keyed(&self, key: &str) -> Vec<String> {
        let Ok(path) = self.path(key) else {
            return Vec::new();
        };
        fs::read_to_string(path)
            .ok()
            .and_then(|encoded| serde_json::from_str(&encoded).ok())
            .unwrap_or_default()
    }

    fn add_keyed(&self, key: &str, prs: &[String]) -> Result<()> {
        let mut recorded = self.keyed(key);
        if !merge(&mut recorded, prs) {
            return Ok(());
        }
        let encoded = serde_json::to_string(&recorded)?;
        qol_fs::atomic_write(&self.path(key)?, encoded.as_bytes())
            .context("failed to record the pull requests")
    }

    pub(super) fn record(
        &self,
        forks: &ForkStore,
        session: &str,
        key: &str,
        text: &str,
    ) -> Result<Vec<String>> {
        let prs = pull_requests(text);
        if prs.is_empty() {
            return Ok(prs);
        }
        if let Some(mut record) = forks.find_session(session)? {
            if merge(&mut record.prs, &prs) {
                forks.record(&record)?;
            }
        }
        self.add_keyed(key, &prs)?;
        Ok(prs)
    }

    pub(super) fn of(&self, forks: &[ForkRecord], session: &str, key: Option<&str>) -> Vec<String> {
        match forks.iter().find(|record| record.session == session) {
            Some(record) => record.prs.clone(),
            None => key.map(|key| self.keyed(key)).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_request_links_are_pulled_out_of_text_once_each() {
        let text = "Opened https://github.com/qol-tools/qol-monorepo/pull/235, see also https://github.com/o/r.s/pull/7/files and (https://github.com/qol-tools/qol-monorepo/pull/235).";
        assert_eq!(
            pull_requests(text),
            [
                "https://github.com/qol-tools/qol-monorepo/pull/235",
                "https://github.com/o/r.s/pull/7",
            ]
        );
        for miss in [
            "https://github.com/o/r/issues/3",
            "https://github.com/o/r/pull/",
            "https://github.com//r/pull/4",
            "http://github.com/o/r/pull/5",
            "no links here",
        ] {
            assert!(pull_requests(miss).is_empty(), "{miss}");
        }
    }

    fn fork(key: &str, session: &str) -> ForkRecord {
        serde_json::from_value(serde_json::json!({
            "key": key,
            "tool": "claude",
            "model": "opus",
            "surface": "tab",
            "cwd": "/work",
            "session": session,
            "title": key,
            "brief": "",
            "created_at": 7,
        }))
        .unwrap()
    }

    #[test]
    fn a_fork_keeps_its_pull_requests_on_its_record_and_others_in_a_keyed_record() {
        let root = tempfile::TempDir::new().unwrap();
        let forks = ForkStore::with_dir(root.path().join("forks"));
        let store = PrStore::with_dir(root.path().join("prs"));
        forks.record(&fork("chase", "v1:kitty:1:1")).unwrap();
        let first = "https://github.com/o/r/pull/1";
        let second = "https://github.com/o/r/pull/2";

        store
            .record(&forks, "v1:kitty:1:1", "chase", &format!("opened {first}"))
            .unwrap();
        store
            .record(
                &forks,
                "v1:kitty:1:1",
                "chase",
                &format!("{first} and {second}"),
            )
            .unwrap();
        let records = forks.list().unwrap();
        assert_eq!(records[0].prs, [first, second]);
        assert_eq!(
            store.of(&records, "v1:kitty:1:1", Some("chase")),
            [first, second]
        );
        assert_eq!(
            store.keyed("chase"),
            [first, second],
            "the spawn key keeps them once the fork resumes under a new session"
        );

        store
            .record(&forks, "v1:kitty:2:2", "lane", &format!("merged {second}"))
            .unwrap();
        assert_eq!(store.of(&records, "v1:kitty:2:2", Some("lane")), [second]);
        assert!(store.of(&records, "v1:kitty:3:3", None).is_empty());
        assert!(store
            .record(&forks, "v1:kitty:2:2", "lane", "nothing linked")
            .unwrap()
            .is_empty());
    }
}
