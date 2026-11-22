use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use qol_terminal_sessions::{SessionBinding, TerminalSessionService};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct LineageRecord {
    pub(super) child: String,
    pub(super) parent: String,
    pub(super) recorded_at_ms: u64,
}

pub(super) struct LineageStore {
    dir: PathBuf,
}

impl LineageStore {
    pub(super) fn system() -> Result<Self> {
        let dir = qol_config::data_subdir("sessions")
            .ok_or_else(|| anyhow!("sessions data directory is unavailable"))?
            .join("lineage");
        Ok(Self { dir })
    }

    pub(super) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, child: &str) -> Result<PathBuf> {
        Ok(self
            .dir
            .join(format!("{}.json", super::mailbox::key_component(child)?)))
    }

    pub(super) fn record(&self, child: &str, parent: Option<&str>) -> Result<()> {
        let path = self.path(child)?;
        let Some(parent) = parent else {
            return match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error).context("failed to clear the lineage record"),
            };
        };
        super::mailbox::key_component(parent)?;
        let record = LineageRecord {
            child: child.to_owned(),
            parent: parent.to_owned(),
            recorded_at_ms: super::mailbox::now_ms(),
        };
        let encoded = serde_json::to_string(&record)?;
        qol_fs::atomic_write(&path, encoded.as_bytes())
            .context("failed to publish the lineage record")
    }

    pub(super) fn parent(&self, child: &str) -> Result<Option<String>> {
        let Ok(path) = self.path(child) else {
            return Ok(None);
        };
        let encoded = match fs::read_to_string(&path) {
            Ok(encoded) => encoded,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("failed to read the lineage record"),
        };
        let record: LineageRecord =
            serde_json::from_str(&encoded).context("the lineage record is invalid")?;
        Ok(Some(record.parent))
    }

    pub(super) fn knows(&self, key: &str) -> bool {
        self.path(key).is_ok_and(|path| path.exists())
    }
}

pub(crate) const TOP_LEVEL_MAX: usize = 24;
pub(crate) const SUFFIX_MAX: usize = 12;
const KEY_MAX: usize = 64;

pub(crate) fn is_top_level(parent: Option<&str>) -> bool {
    parent.is_none_or(|parent| parent == super::switchboard::SWITCHBOARD_KEY)
}

pub(crate) fn child_key(parent: Option<&str>, asked: &str) -> Result<String, String> {
    let parent = parent.filter(|parent| !is_top_level(Some(parent)));
    let Some(parent) = parent else {
        if asked.len() > TOP_LEVEL_MAX {
            return Err(format!(
                "`{asked}` is {} characters, and a top-level key is at most {TOP_LEVEL_MAX}, so every session under it has room for its own word. Name the area first, then the topic, such as `alt-tab-ctrl-bug`.",
                asked.len()
            ));
        }
        return Ok(asked.to_owned());
    };
    let suffix = asked
        .strip_prefix(parent)
        .and_then(|rest| rest.strip_prefix('-'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(asked);
    let key = format!("{parent}-{suffix}");
    if key.len() > KEY_MAX {
        let room = KEY_MAX.saturating_sub(parent.len() + 1).min(SUFFIX_MAX);
        return Err(format!(
            "`{key}` is {} characters, over the {KEY_MAX} a key may have. A key under `{parent}` is `{parent}-<word>`, and `{parent}` leaves room for a word of at most {room}.",
            key.len()
        ));
    }
    if suffix.len() > SUFFIX_MAX {
        return Err(format!(
            "`{suffix}` is {} characters, and the word after `{parent}-` is at most {SUFFIX_MAX}. Pick one short word for the job, such as `fix`, `docs` or `review`.",
            suffix.len()
        ));
    }
    Ok(key)
}

pub(super) fn caller_key(terminals: &TerminalSessionService) -> Option<String> {
    let token = super::bridge::driver_token(terminals);
    session_key(
        terminals,
        Some(token.as_str()).filter(|token| !token.is_empty()),
    )
}

pub(super) fn session_key(
    terminals: &TerminalSessionService,
    token: Option<&str>,
) -> Option<String> {
    let binding = token?.parse::<SessionBinding>().ok()?;
    let facts = super::bridge::resolve_target(terminals, &binding).ok()?;
    facts
        .spawn_identity
        .map(|identity| identity.key.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_parent_reads_back_and_a_missing_one_is_none() {
        let root = tempfile::TempDir::new().unwrap();
        let store = LineageStore::with_dir(root.path().join("lineage"));
        assert_eq!(store.parent("child").unwrap(), None);
        assert!(!store.knows("child"));

        store.record("child", Some("switchboard")).unwrap();
        assert_eq!(
            store.parent("child").unwrap().as_deref(),
            Some("switchboard")
        );
        assert!(store.knows("child"));

        store.record("child", Some("architect")).unwrap();
        assert_eq!(
            store.parent("child").unwrap().as_deref(),
            Some("architect"),
            "a new fork with the same key replaces the old parent"
        );
    }

    #[test]
    fn recording_no_parent_clears_a_stale_record() {
        let root = tempfile::TempDir::new().unwrap();
        let store = LineageStore::with_dir(root.path().join("lineage"));
        store.record("child", Some("old-parent")).unwrap();
        store.record("child", None).unwrap();
        assert_eq!(store.parent("child").unwrap(), None);
        store.record("never-recorded", None).unwrap();
    }

    #[test]
    fn keys_that_would_escape_the_store_are_refused() {
        let root = tempfile::TempDir::new().unwrap();
        let store = LineageStore::with_dir(root.path().join("lineage"));
        for key in ["..", ".", "a/b", ""] {
            assert!(store.record(key, Some("parent")).is_err(), "{key}");
            assert_eq!(store.parent(key).unwrap(), None, "{key}");
        }
        assert!(store.record("child", Some("../x")).is_err());
    }

    #[test]
    fn a_child_key_is_its_parents_full_key_a_dash_and_one_word() {
        let top = "k".repeat(24);
        let long = "k".repeat(60);
        let long_fix = format!("{long}-fix");
        let cases: &[(Option<&str>, &str, Result<&str, &str>)] = &[
            (None, "alt-tab-ctrl-bug", Ok("alt-tab-ctrl-bug")),
            (Some("switchboard"), "repo-structure", Ok("repo-structure")),
            (None, top.as_str(), Ok(top.as_str())),
            (None, "another-bug-tmp-clip-c63f", Err("is 25 characters, and a top-level key is at most 24")),
            (Some("switchboard"), "add-missing-icons-design-a61c", Err("a top-level key is at most 24")),
            (Some("alt-tab-ctrl-bug"), "fix", Ok("alt-tab-ctrl-bug-fix")),
            (Some("alt-tab-ctrl-bug"), "alt-tab-ctrl-bug-fix", Ok("alt-tab-ctrl-bug-fix")),
            (Some("alt-tab-ctrl-bug-fix"), "test", Ok("alt-tab-ctrl-bug-fix-test")),
            (Some("alt-tab-ctrl-bug"), "alt-tab-ctrl-bug", Err("`alt-tab-ctrl-bug` is 16 characters")),
            (Some("repo"), "architecture", Ok("repo-architecture")),
            (Some("alt-tab-ctrl-bug"), "hotkey-regression", Err("`hotkey-regression` is 17 characters, and the word after `alt-tab-ctrl-bug-` is at most 12")),
            (Some(long.as_str()), "fix", Ok(long_fix.as_str())),
            (Some(long.as_str()), "docs", Err("is 65 characters, over the 64 a key may have")),
            (Some(long.as_str()), "docs", Err("leaves room for a word of at most 3")),
        ];
        for (parent, asked, expected) in cases {
            let got = child_key(*parent, asked);
            match expected {
                Ok(key) => assert_eq!(got.as_deref(), Ok(*key), "{parent:?} {asked}"),
                Err(part) => {
                    let error = got.expect_err(&format!("{parent:?} {asked}"));
                    assert!(error.contains(part), "{parent:?} {asked}: {error}");
                }
            }
        }
    }
}
