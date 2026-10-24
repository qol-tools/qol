use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::report::Lifetime;
use crate::role::Role;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaimState {
    Pending,
    Applied,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ClaimRecord {
    pub role: String,
    pub plugin_id: String,
    pub store: String,
    pub key: String,
    pub previous: Option<String>,
    pub applied: String,
    pub lifetime: Lifetime,
    pub state: ClaimState,
}

impl ClaimRecord {
    fn same_slot(&self, other: &ClaimRecord) -> bool {
        self.role == other.role && self.store == other.store && self.key == other.key
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseOutcome {
    Restored,
    LeftUserChange,
    AlreadyGone,
}

pub struct Ledger {
    path: PathBuf,
}

impl Ledger {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<Vec<ClaimRecord>> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read ledger {}", self.path.display()))
            }
        };
        serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse ledger {}", self.path.display()))
    }

    pub fn holder(&self, role: Role) -> Result<Option<String>> {
        Ok(self
            .load()?
            .into_iter()
            .find(|record| record.role == role.id())
            .map(|record| record.plugin_id))
    }

    pub fn insert(&self, record: ClaimRecord) -> Result<()> {
        let mut records = self.load()?;
        match records.iter().position(|held| held.same_slot(&record)) {
            Some(index) => records[index] = record,
            None => records.push(record),
        }
        self.save(&records)
    }

    pub fn remove(&self, role: Role) -> Result<Vec<ClaimRecord>> {
        self.take(|record| record.role == role.id())
    }

    pub fn remove_store(&self, role: Role, store: &str) -> Result<Vec<ClaimRecord>> {
        self.take(|record| record.role == role.id() && record.store == store)
    }

    pub(crate) fn drop_record(&self, target: &ClaimRecord) -> Result<()> {
        self.take(|record| record.same_slot(target)).map(drop)
    }

    fn take(&self, matches: impl Fn(&ClaimRecord) -> bool) -> Result<Vec<ClaimRecord>> {
        let (removed, kept): (Vec<_>, Vec<_>) = self.load()?.into_iter().partition(matches);
        if removed.is_empty() {
            return Ok(removed);
        }
        self.save(&kept)?;
        Ok(removed)
    }

    fn save(&self, records: &[ClaimRecord]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create ledger dir {}", parent.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(records).context("failed to encode ledger")?;
        qol_fs::atomic_write(&self.path, &bytes)
            .with_context(|| format!("failed to write ledger {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(role: Role, plugin: &str, store: &str, key: &str) -> ClaimRecord {
        ClaimRecord {
            role: role.id().to_string(),
            plugin_id: plugin.to_string(),
            store: store.to_string(),
            key: key.to_string(),
            previous: Some("old".to_string()),
            applied: "new".to_string(),
            lifetime: Lifetime::PortableSession,
            state: ClaimState::Applied,
        }
    }

    fn ledger() -> (tempfile::TempDir, Ledger) {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::at(dir.path().join("state").join("claims.json"));
        (dir, ledger)
    }

    #[test]
    fn a_missing_file_loads_empty() {
        let (_dir, ledger) = ledger();
        assert!(ledger.load().unwrap().is_empty());
        assert_eq!(ledger.holder(Role::FolderOpen).unwrap(), None);
    }

    #[test]
    fn records_survive_a_reload() {
        let (_dir, ledger) = ledger();
        let first = record(
            Role::FolderOpen,
            "explorer",
            "mimeapps",
            "a|inode/directory",
        );
        let second = ClaimRecord {
            previous: None,
            state: ClaimState::Pending,
            lifetime: Lifetime::ResidentPolicy,
            ..record(Role::SlotScreenshot, "shot", "hotkey", "Print")
        };
        ledger.insert(first.clone()).unwrap();
        ledger.insert(second.clone()).unwrap();
        let reopened = Ledger::at(ledger.path.clone());
        assert_eq!(reopened.load().unwrap(), vec![first, second]);
    }

    #[test]
    fn insert_replaces_the_record_with_the_same_role_store_and_key() {
        let (_dir, ledger) = ledger();
        ledger
            .insert(record(Role::FolderOpen, "explorer", "mimeapps", "k"))
            .unwrap();
        ledger
            .insert(record(Role::FolderOpen, "other", "mimeapps", "k"))
            .unwrap();
        ledger
            .insert(record(Role::FolderOpen, "other", "desktop_entry", "k"))
            .unwrap();
        let records = ledger.load().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].plugin_id, "other");
        assert_eq!(records[1].store, "desktop_entry");
    }

    #[test]
    fn holder_names_the_plugin_of_any_record_for_the_role() {
        let (_dir, ledger) = ledger();
        ledger
            .insert(record(Role::FolderOpen, "explorer", "mimeapps", "k"))
            .unwrap();
        ledger
            .insert(record(Role::SlotLauncher, "launcher", "hotkey", "Super"))
            .unwrap();
        let cases = [
            (Role::FolderOpen, Some("explorer")),
            (Role::SlotLauncher, Some("launcher")),
            (Role::SlotScreenshot, None),
        ];
        for (role, expected) in cases {
            assert_eq!(ledger.holder(role).unwrap().as_deref(), expected);
        }
    }

    #[test]
    fn remove_takes_every_record_of_the_role_only() {
        let (_dir, ledger) = ledger();
        ledger
            .insert(record(Role::FolderOpen, "explorer", "mimeapps", "k"))
            .unwrap();
        ledger
            .insert(record(Role::FolderOpen, "explorer", "desktop_entry", "e"))
            .unwrap();
        ledger
            .insert(record(Role::SlotLauncher, "launcher", "hotkey", "Super"))
            .unwrap();
        let removed = ledger.remove(Role::FolderOpen).unwrap();
        assert_eq!(removed.len(), 2);
        let left = ledger.load().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].role, "slot.launcher");
        assert!(ledger.remove(Role::FolderOpen).unwrap().is_empty());
    }

    #[test]
    fn remove_store_leaves_the_other_stores_of_the_role() {
        let (_dir, ledger) = ledger();
        ledger
            .insert(record(Role::SlotScreenshot, "shot", "hotkey", "Print"))
            .unwrap();
        ledger
            .insert(record(Role::SlotScreenshot, "shot", "dconf", "media-keys"))
            .unwrap();
        let removed = ledger.remove_store(Role::SlotScreenshot, "hotkey").unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].store, "hotkey");
        let left = ledger.load().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].store, "dconf");
    }

    #[test]
    fn a_corrupt_file_is_an_error_not_an_empty_ledger() {
        let (_dir, ledger) = ledger();
        std::fs::create_dir_all(ledger.path.parent().unwrap()).unwrap();
        std::fs::write(&ledger.path, "not json").unwrap();
        assert!(ledger.load().is_err());
        assert!(ledger
            .insert(record(Role::FolderOpen, "explorer", "mimeapps", "k"))
            .is_err());
    }

    #[test]
    fn states_serialize_in_snake_case() {
        let cases = [
            (ClaimState::Pending, "\"pending\""),
            (ClaimState::Applied, "\"applied\""),
        ];
        for (state, expected) in cases {
            assert_eq!(serde_json::to_string(&state).unwrap(), expected);
        }
    }
}
