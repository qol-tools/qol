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

    #[cfg(test)]
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
}
