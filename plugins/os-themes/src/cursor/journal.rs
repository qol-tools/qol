use crate::session::{session_subdir, SessionSnapshot, SessionStore};

pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const ENTRY_ID: &str = "x11-cursor";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CursorSnapshot {
    pub schema_version: u32,
    pub id: String,
    pub root: u64,
    pub windows: Vec<u64>,
    pub mutations: u32,
    pub clean: bool,
}

impl SessionSnapshot for CursorSnapshot {
    const SCHEMA_VERSION: u32 = SNAPSHOT_SCHEMA_VERSION;
    const SUBDIR: &'static str = "cursor";

    fn id(&self) -> &str {
        &self.id
    }

    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

fn store() -> SessionStore {
    SessionStore::new(session_subdir(CursorSnapshot::SUBDIR))
}

pub fn journal_scaled(root: u64, windows: &[u64]) {
    let result = (|| -> anyhow::Result<()> {
        let store = store();
        if let Some(mut snapshot) = store.load::<CursorSnapshot>(ENTRY_ID)? {
            snapshot.mutations += 1;
            snapshot.root = root;
            snapshot.windows = windows.to_vec();
            return store.write(&snapshot);
        }
        store.write(&CursorSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            id: ENTRY_ID.to_string(),
            root,
            windows: windows.to_vec(),
            mutations: 1,
            clean: false,
        })
    })();
    if let Err(error) = result {
        eprintln!("[os-themes] failed to journal the scaled cursor: {error:#}");
    }
}

pub fn journaled_scale() -> Option<(u64, Vec<u64>)> {
    let snapshot = store().load::<CursorSnapshot>(ENTRY_ID).ok()??;
    if snapshot.mutations == 0 || snapshot.clean {
        return None;
    }
    Some((snapshot.root, snapshot.windows))
}

pub fn clear_journal() {
    let _ = store().delete(ENTRY_ID);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn test_dir() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("os-themes-cursor-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn snapshot(mutations: u32, clean: bool) -> CursorSnapshot {
        CursorSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            id: ENTRY_ID.to_string(),
            root: 0x2000000,
            windows: vec![0x2000001, 0x2000002],
            mutations,
            clean,
        }
    }

    #[test]
    fn store_round_trips_and_ignores_clean_or_zero_entries() {
        let dir = test_dir();
        let store = SessionStore::new(dir.join("cursor"));
        store.write(&snapshot(3, false)).unwrap();
        assert!(store.load::<CursorSnapshot>(ENTRY_ID).unwrap().is_some());

        store.write(&snapshot(3, true)).unwrap();
        let loaded = store.load::<CursorSnapshot>(ENTRY_ID).unwrap().unwrap();
        assert!(loaded.clean);
    }
}
