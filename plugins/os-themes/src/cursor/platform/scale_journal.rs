use qol_host_session::{SessionSnapshot, SessionStore};

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const ENTRY_ID: &str = "system-cursors";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct ScaledCursors {
    schema_version: u32,
    id: String,
}

impl SessionSnapshot for ScaledCursors {
    const SCHEMA_VERSION: u32 = SNAPSHOT_SCHEMA_VERSION;
    const SUBDIR: &'static str = "cursor";

    fn id(&self) -> &str {
        &self.id
    }

    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

pub(super) const SUBDIR: &str = ScaledCursors::SUBDIR;

pub(super) struct ScaleJournal {
    store: SessionStore,
    recorded: bool,
}

impl ScaleJournal {
    pub(super) fn new(store: SessionStore) -> Self {
        Self {
            store,
            recorded: false,
        }
    }

    pub(super) fn before_scale(&mut self) -> bool {
        if self.recorded {
            return true;
        }
        let entry = ScaledCursors {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            id: ENTRY_ID.to_string(),
        };
        match self.store.write(&entry) {
            Ok(()) => self.recorded = true,
            Err(error) => log::warn!("refusing to scale the cursors without a journal: {error:#}"),
        }
        self.recorded
    }

    pub(super) fn after_restore(&mut self) {
        if !self.recorded {
            return;
        }
        match self.store.delete(ENTRY_ID) {
            Ok(()) => self.recorded = false,
            Err(error) => log::warn!("failed to clear the scaled cursor journal: {error:#}"),
        }
    }
}

pub(super) fn recover(store: &SessionStore, reload: impl FnOnce()) -> bool {
    let journaled = match store.load::<ScaledCursors>(ENTRY_ID) {
        Ok(entry) => entry.is_some(),
        Err(error) => {
            log::warn!("reloading the cursors after an unreadable journal: {error:#}");
            true
        }
    };
    if journaled {
        reload();
        if let Err(error) = store.delete(ENTRY_ID) {
            log::warn!("failed to clear the scaled cursor journal: {error:#}");
        }
    }
    journaled
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn test_dir() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "os-themes-scale-journal-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn journaled(store: &SessionStore) -> bool {
        store.load::<ScaledCursors>(ENTRY_ID).unwrap().is_some()
    }

    #[test]
    fn a_scale_is_journaled_before_it_applies_and_cleared_after_the_restore() {
        let store = SessionStore::new(test_dir().join(SUBDIR));
        let mut journal = ScaleJournal::new(store.clone());
        journal.after_restore();
        assert!(!journaled(&store));
        assert!(journal.before_scale());
        assert!(journaled(&store));
        assert!(journal.before_scale());
        journal.after_restore();
        assert!(!journaled(&store));
    }

    #[test]
    fn an_unwritable_journal_refuses_the_scale() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"file").unwrap();
        let mut journal = ScaleJournal::new(SessionStore::new(blocker.join(SUBDIR)));
        assert!(!journal.before_scale());
        assert!(!journal.before_scale());
    }

    #[test]
    fn a_restore_leaves_a_journal_it_did_not_record() {
        let store = SessionStore::new(test_dir().join(SUBDIR));
        assert!(ScaleJournal::new(store.clone()).before_scale());
        ScaleJournal::new(store.clone()).after_restore();
        assert!(journaled(&store));
    }

    #[test]
    fn recovery_reloads_the_cursors_only_when_a_scale_was_journaled() {
        let store = SessionStore::new(test_dir().join(SUBDIR));
        let reloads = Cell::new(0);
        assert!(!recover(&store, || reloads.set(reloads.get() + 1)));
        assert_eq!(reloads.get(), 0);

        assert!(ScaleJournal::new(store.clone()).before_scale());
        assert!(recover(&store, || reloads.set(reloads.get() + 1)));
        assert_eq!(reloads.get(), 1);
        assert!(!journaled(&store));

        assert!(!recover(&store, || reloads.set(reloads.get() + 1)));
        assert_eq!(reloads.get(), 1);
    }

    #[test]
    fn recovery_reloads_the_cursors_after_an_unreadable_journal() {
        let store = SessionStore::new(test_dir().join(SUBDIR));
        std::fs::create_dir_all(store.dir()).unwrap();
        let path = store.dir().join(format!("{ENTRY_ID}.json"));
        std::fs::write(&path, b"not json").unwrap();
        let reloads = Cell::new(0);
        assert!(recover(&store, || reloads.set(reloads.get() + 1)));
        assert_eq!(reloads.get(), 1);
        assert!(!path.exists());
    }
}
