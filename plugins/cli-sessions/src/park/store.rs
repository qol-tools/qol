use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{Context, Result};
use qol_terminal_sessions::park::ParkRecord;

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

    fn claim_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.claim"))
    }

    pub fn claim_resume(&self, id: &str) -> Result<bool> {
        fs::create_dir_all(&self.dir).context("failed to create the parked directory")?;
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.claim_path(id))
        {
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
}
