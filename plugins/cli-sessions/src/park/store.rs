use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use qol_terminal_sessions::park::{ParkRecord, ParkState};
use qol_terminal_sessions::SpawnKey;

pub struct ParkStore {
    dir: PathBuf,
}

impl ParkStore {
    pub fn system() -> Result<Self> {
        qol_terminal_sessions::park::parked_dir()
            .map(Self::with_dir)
            .context("cannot resolve the qol-tray data directory for parked sessions")
    }

    pub fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn log_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.log"))
    }

    pub fn record(&self, record: &ParkRecord) -> Result<()> {
        fs::create_dir_all(&self.dir).context("failed to create the parked directory")?;
        let encoded = serde_json::to_string(record)?;
        qol_fs::atomic_write(&self.record_path(&record.id), encoded.as_bytes())
            .context("failed to publish the park record")
    }

    fn spawn_key_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.key"))
    }

    pub fn record_spawn_key(&self, id: &str, key: &SpawnKey) -> Result<()> {
        fs::create_dir_all(&self.dir).context("failed to create the parked directory")?;
        qol_fs::atomic_write(&self.spawn_key_path(id), key.as_str().as_bytes())
            .context("failed to keep the parked session's spawn key")
    }

    pub fn spawn_key(&self, id: &str) -> Option<SpawnKey> {
        let key = fs::read_to_string(self.spawn_key_path(id)).ok()?;
        SpawnKey::new(key.trim()).ok()
    }

    fn claim_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.claim"))
    }

    fn unpark_lock_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.unpark"))
    }

    pub fn claim_resume(&self, id: &str) -> Result<bool> {
        self.claim(&self.claim_path(id))
    }

    pub fn lock_unpark(&self, id: &str) -> Result<Option<File>> {
        let file = self.lock_file(&self.unpark_lock_path(id))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error).context("failed to lock the park"),
        }
    }

    fn conversation_lock_path(&self, external_id: &str) -> PathBuf {
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
        self.dir.join(format!("{name}.conversation"))
    }

    /// Serializes delivering to and resuming one conversation, so it is resumed at most once.
    pub fn lock_conversation(&self, external_id: &str) -> Result<File> {
        let file = self.lock_file(&self.conversation_lock_path(external_id))?;
        file.lock()
            .context("failed to lock the parked conversation")?;
        Ok(file)
    }

    pub fn try_lock_conversation(&self, external_id: &str) -> Result<Option<File>> {
        let file = self.lock_file(&self.conversation_lock_path(external_id))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => {
                Err(error).context("failed to lock the parked conversation")
            }
        }
    }

    fn lock_file(&self, path: &Path) -> Result<File> {
        fs::create_dir_all(&self.dir).context("failed to create the parked directory")?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .context("failed to open a park lock")
    }

    fn claim(&self, path: &Path) -> Result<bool> {
        fs::create_dir_all(&self.dir).context("failed to create the parked directory")?;
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error).context("failed to claim the park"),
        }
    }

    pub fn release(&self, id: &str) {
        let _ = fs::remove_file(self.claim_path(id));
    }

    pub fn is_claimed(&self, id: &str) -> bool {
        self.claim_path(id).exists()
    }

    pub fn load(&self, id: &str) -> Result<ParkRecord> {
        let path = self.record_path(id);
        let encoded = fs::read_to_string(&path)
            .with_context(|| format!("no parked session `{id}` at {}", path.display()))?;
        serde_json::from_str(&encoded).with_context(|| format!("corrupt park record `{id}`"))
    }

    pub fn list(&self) -> Result<Vec<ParkRecord>> {
        qol_terminal_sessions::park::records(&self.dir)
            .context("failed to read the parked directory")
    }

    pub fn mark_reopened(&self, record: &ParkRecord) -> Result<()> {
        let Some(_turn) = self.try_lock_conversation(&record.external_id)? else {
            return Ok(());
        };
        let mut stored = self.load(&record.id)?;
        if stored.closed_at.take().is_some() {
            self.record(&stored)?;
        }
        Ok(())
    }

    pub fn exited(&self, external_id: &str) -> Vec<ParkRecord> {
        qol_terminal_sessions::park::exited(&self.dir, external_id)
    }

    /// How many results were delivered into a session.
    pub fn delivered_into(&self, session: &str) -> usize {
        self.list()
            .unwrap_or_default()
            .iter()
            .filter(|record| {
                record.state == ParkState::Delivered
                    && record.resumed_session.as_deref() == Some(session)
            })
            .count()
    }

    pub fn parks_again(&self, session: &str) -> bool {
        qol_terminal_sessions::park::waiting(&self.dir)
            .iter()
            .any(|record| record.session == session)
    }
}

#[cfg(test)]
mod tests {
    use qol_terminal_sessions::park::ParkState;

    use super::*;
    use crate::park::tests::record;

    #[test]
    fn store_round_trips_records_newest_first() {
        let root = tempfile::TempDir::new().unwrap();
        let store = ParkStore::with_dir(root.path().join("parked"));
        assert!(store.list().unwrap().is_empty());
        let older = record(ParkState::Resumed);
        let mut newer = record(ParkState::Waiting);
        newer.id = "park-9-2".to_owned();
        newer.created_at = 9;
        store.record(&older).unwrap();
        store.record(&newer).unwrap();
        let ids = store
            .list()
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["park-9-2", "park-1-2"]);
        assert_eq!(store.load("park-1-2").unwrap().state, ParkState::Resumed);
        assert!(store.load("park-missing").is_err());
    }

    #[test]
    fn only_the_first_claim_wins_until_released() {
        let root = tempfile::TempDir::new().unwrap();
        let store = ParkStore::with_dir(root.path().join("parked"));
        assert!(store.claim_resume("park-1-2").unwrap());
        assert!(!store.claim_resume("park-1-2").unwrap());
        assert!(store.is_claimed("park-1-2"));
        store.release("park-1-2");
        assert!(!store.is_claimed("park-1-2"));
    }

    #[test]
    fn the_unpark_lock_is_exclusive_and_freed_when_its_holder_drops() {
        let root = tempfile::TempDir::new().unwrap();
        let store = ParkStore::with_dir(root.path().join("parked"));
        let held = store.lock_unpark("park-1-2").unwrap();
        assert!(held.is_some());
        assert!(store.lock_unpark("park-1-2").unwrap().is_none());
        assert!(store.lock_unpark("park-9-9").unwrap().is_some());
        drop(held);
        assert!(store.lock_unpark("park-1-2").unwrap().is_some());
    }

    #[test]
    fn the_conversation_lock_admits_one_holder_at_a_time() {
        let root = tempfile::TempDir::new().unwrap();
        let store = ParkStore::with_dir(root.path().join("parked"));
        let held = store.lock_conversation("abc").unwrap();
        assert!(store.try_lock_conversation("abc").unwrap().is_none());
        assert!(store.try_lock_conversation("other").unwrap().is_some());
        drop(held);
        assert!(store.try_lock_conversation("abc").unwrap().is_some());
    }

    #[test]
    fn a_park_keeps_the_spawn_key_of_the_session_it_parked() {
        let root = tempfile::TempDir::new().unwrap();
        let store = ParkStore::with_dir(root.path().join("parked"));
        assert_eq!(store.spawn_key("park-1-2"), None);
        store
            .record_spawn_key("park-1-2", &SpawnKey::new("lane-7").unwrap())
            .unwrap();
        assert_eq!(
            store.spawn_key("park-1-2"),
            Some(SpawnKey::new("lane-7").unwrap())
        );
        assert_eq!(store.spawn_key("park-9-9"), None);
        store.record(&record(ParkState::Waiting)).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn a_reopened_conversation_forgets_its_closed_tab_unless_a_resume_holds_it() {
        let root = tempfile::TempDir::new().unwrap();
        let store = ParkStore::with_dir(root.path().join("parked"));
        let closed = ParkRecord {
            closed_at: Some(5),
            ..record(ParkState::Finished)
        };
        store.record(&closed).unwrap();
        let held = store.lock_conversation(&closed.external_id).unwrap();
        store.mark_reopened(&closed).unwrap();
        assert_eq!(store.load(&closed.id).unwrap().closed_at, Some(5));
        drop(held);
        store.mark_reopened(&closed).unwrap();
        assert_eq!(store.load(&closed.id).unwrap().closed_at, None);
    }

    #[test]
    fn a_finished_park_keeps_its_resume_claim_without_blocking_the_unpark_lock() {
        let root = tempfile::TempDir::new().unwrap();
        let store = ParkStore::with_dir(root.path().join("parked"));
        assert!(store.claim_resume("park-1-2").unwrap());
        assert!(store.lock_unpark("park-1-2").unwrap().is_some());
    }
}
