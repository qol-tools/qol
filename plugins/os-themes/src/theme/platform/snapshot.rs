use qol_host_session::{SessionSnapshot, SessionStore};

use crate::session::{RestoreMode, RestoreReport};

pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ThemeSnapshot {
    pub schema_version: u32,
    pub id: String,
    pub schema: String,
    pub key: String,
    pub value: String,
    pub mutations: u32,
    pub clean: bool,
}

impl ThemeSnapshot {
    pub fn set_clean(&mut self) {
        self.clean = true;
    }
}

impl SessionSnapshot for ThemeSnapshot {
    const SCHEMA_VERSION: u32 = SNAPSHOT_SCHEMA_VERSION;
    const SUBDIR: &'static str = "theme";

    fn id(&self) -> &str {
        &self.id
    }

    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

pub fn store() -> SessionStore {
    SessionStore::new(crate::session::platform::session_subdir(
        ThemeSnapshot::SUBDIR,
    ))
}

pub fn id_for(schema: &str, key: &str) -> String {
    format!("{schema}@{key}")
}

pub fn record_baseline(schema: &str, key: &str, value: &str) -> anyhow::Result<()> {
    let store = store();
    let id = id_for(schema, key);
    if let Some(mut snapshot) = store.load::<ThemeSnapshot>(&id)? {
        snapshot.mutations += 1;
        return store.write(&snapshot);
    }
    store.write(&ThemeSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        id,
        schema: schema.to_string(),
        key: key.to_string(),
        value: value.to_string(),
        mutations: 1,
        clean: false,
    })
}

pub fn ids() -> anyhow::Result<Vec<String>> {
    store().ids::<ThemeSnapshot>()
}

pub fn load(id: &str) -> anyhow::Result<Option<ThemeSnapshot>> {
    store().load::<ThemeSnapshot>(id)
}

pub fn delete(id: &str) -> anyhow::Result<()> {
    store().delete(id)
}

pub fn write(snapshot: &ThemeSnapshot) -> anyhow::Result<()> {
    store().write(snapshot)
}

pub fn restore(
    mode: RestoreMode,
    report: &mut RestoreReport,
    apply: impl Fn(&ThemeSnapshot) -> anyhow::Result<()>,
) {
    let Ok(ids) = ids() else {
        return;
    };
    for id in ids {
        let Ok(Some(snapshot)) = load(&id) else {
            report.unreadable += 1;
            continue;
        };
        if snapshot.mutations == 0 || snapshot.clean {
            report.nothing_to_restore += 1;
            let _ = delete(&id);
            continue;
        }
        match apply(&snapshot) {
            Ok(()) => {
                match mode {
                    RestoreMode::Exit => {
                        let mut cleaned = snapshot.clone();
                        cleaned.set_clean();
                        let _ = write(&cleaned);
                    }
                    RestoreMode::Recovery => {
                        let _ = delete(&id);
                    }
                }
                report.restored += 1;
            }
            Err(error) => {
                log::warn!(
                    "failed to restore pre-qol value of {}:{}: {error:#}",
                    snapshot.schema,
                    snapshot.key
                );
                report.failed += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn test_dir() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("os-themes-theme-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn snapshot(id: &str, value: &str, mutations: u32, clean: bool) -> ThemeSnapshot {
        ThemeSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            id: id.to_string(),
            schema: "org.gnome.desktop.interface".to_string(),
            key: "gtk-theme".to_string(),
            value: value.to_string(),
            mutations,
            clean,
        }
    }

    #[test]
    fn store_round_trips_and_rejects_tampering() {
        let dir = test_dir();
        let store = SessionStore::new(dir.join("theme"));
        let snap = snapshot("s@k", "adwaita", 3, false);
        store.write(&snap).unwrap();
        assert_eq!(
            store.load::<ThemeSnapshot>("s@k").unwrap(),
            Some(snap.clone())
        );
        assert!(store.load::<ThemeSnapshot>("missing").unwrap().is_none());

        let path = store.dir().join("s@k.json");
        let mut raw = std::fs::read(&path).unwrap();
        raw[10] ^= 0xff;
        std::fs::write(&path, &raw).unwrap();
        assert!(
            store.load::<ThemeSnapshot>("s@k").is_err(),
            "tampered snapshot must not load"
        );
    }
}
