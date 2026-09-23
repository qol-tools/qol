use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub type MutationId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Residency {
    Portable,
    Resident,
}

impl Residency {
    pub fn is_resident(self) -> bool {
        matches!(self, Residency::Resident)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifetime {
    #[default]
    PortableSession,
    ResidentPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode {
    Exit,
    Recovery,
}

#[derive(Debug, Clone, Default)]
pub struct RestoreReport {
    pub restored: usize,
    pub nothing_to_restore: usize,
    pub failed: usize,
    pub unreadable: usize,
}

pub trait SessionSnapshot:
    serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync
{
    const SCHEMA_VERSION: u32;
    const SUBDIR: &'static str;

    fn id(&self) -> &str;
    fn schema_version(&self) -> u32;
}

pub trait HostMutation {
    type Snapshot: SessionSnapshot;

    fn owner(&self) -> &str;
    fn id(&self) -> MutationId;
    fn lifetime(&self) -> Lifetime;
    fn capture(&self) -> Result<Self::Snapshot>;
    fn restore(snapshot: Self::Snapshot) -> Result<()>;
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Envelope<T> {
    checksum: String,
    #[serde(default)]
    lifetime: Lifetime,
    body: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifetimeGate {
    Both,
    PortableSession,
}

pub struct SessionStore {
    dir: PathBuf,
}

impl Clone for SessionStore {
    fn clone(&self) -> Self {
        Self {
            dir: self.dir.clone(),
        }
    }
}

impl SessionStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn snapshot_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn owner_store(&self, owner: &str) -> SessionStore {
        SessionStore::new(self.dir.join(owner))
    }

    pub fn claim<M: HostMutation>(&self, mutation: &M) -> Result<()> {
        let store = self.owner_store(mutation.owner());
        if store.load::<M::Snapshot>(&mutation.id())?.is_some() {
            return Ok(());
        }
        let snapshot = mutation.capture()?;
        std::fs::create_dir_all(store.dir())
            .with_context(|| format!("failed to create session dir {}", store.dir().display()))?;
        let body = serde_json::to_vec(&snapshot).context("failed to serialize the snapshot")?;
        let envelope = Envelope {
            checksum: format!("{:016x}", fnv1a(&body)),
            lifetime: mutation.lifetime(),
            body: snapshot,
        };
        let content = serde_json::to_vec(&envelope).context("failed to serialize the envelope")?;
        qol_fs::atomic_write_durable_mode(&store.snapshot_path(&mutation.id()), &content, 0o600)
            .with_context(|| {
                format!(
                    "failed to commit snapshot {}",
                    store.snapshot_path(&mutation.id()).display()
                )
            })
    }

    pub fn recover<M: HostMutation>(&self, owner: &str, residency: Residency) -> RestoreReport {
        if residency.is_resident() {
            return RestoreReport::default();
        }
        self.drain_owner::<M>(owner, LifetimeGate::Both)
    }

    pub fn release_session<M: HostMutation>(
        &self,
        owner: &str,
        residency: Residency,
    ) -> RestoreReport {
        if residency.is_resident() {
            return RestoreReport::default();
        }
        self.drain_owner::<M>(owner, LifetimeGate::PortableSession)
    }

    pub fn release_residency<M: HostMutation>(&self, owner: &str) -> RestoreReport {
        self.drain_owner::<M>(owner, LifetimeGate::Both)
    }

    fn drain_owner<M: HostMutation>(&self, owner: &str, gate: LifetimeGate) -> RestoreReport {
        let mut report = RestoreReport::default();
        let store = self.owner_store(owner);
        let ids = match store.ids::<M::Snapshot>() {
            Ok(ids) => ids,
            Err(error) => {
                eprintln!("qol-host-session: failed to list records for {owner}: {error:#}");
                report.unreadable += 1;
                return report;
            }
        };
        let empty = ids.is_empty();
        for id in ids {
            let path = store.snapshot_path(&id);
            let envelope: Envelope<M::Snapshot> = match read_envelope(&path) {
                Ok(envelope) => envelope,
                Err(error) => {
                    eprintln!(
                        "qol-host-session: cannot read record {} for {owner}: {error:#}",
                        path.display()
                    );
                    report.unreadable += 1;
                    continue;
                }
            };
            if !gate.includes(envelope.lifetime) {
                continue;
            }
            match M::restore(envelope.body) {
                Ok(()) => match store.delete(&id) {
                    Ok(()) => report.restored += 1,
                    Err(error) => {
                        eprintln!(
                            "qol-host-session: restored {owner}:{id} but could not clear it: {error:#}"
                        );
                        report.failed += 1;
                    }
                },
                Err(error) => {
                    eprintln!("qol-host-session: failed to restore {owner}:{id}: {error:#}");
                    report.failed += 1;
                }
            }
        }
        if empty {
            report.nothing_to_restore += 1;
        }
        report
    }

    pub fn write<T: SessionSnapshot>(&self, snapshot: &T) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("failed to create session dir {}", self.dir.display()))?;
        let body = serde_json::to_vec(snapshot).context("failed to serialize the snapshot")?;
        let envelope = Envelope {
            checksum: format!("{:016x}", fnv1a(&body)),
            lifetime: Lifetime::PortableSession,
            body: snapshot.clone(),
        };
        let content = serde_json::to_vec(&envelope).context("failed to serialize the envelope")?;
        qol_fs::atomic_write_durable_mode(&self.snapshot_path(snapshot.id()), &content, 0o600)
            .with_context(|| {
                format!(
                    "failed to commit snapshot {}",
                    self.snapshot_path(snapshot.id()).display()
                )
            })
    }

    pub fn load<T: SessionSnapshot>(&self, id: &str) -> Result<Option<T>> {
        let path = self.snapshot_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let envelope: Envelope<T> = read_envelope(&path)?;
        Ok(Some(envelope.body))
    }

    pub fn ids<T: SessionSnapshot>(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        if !self.dir.exists() {
            return Ok(ids);
        }
        for entry in std::fs::read_dir(&self.dir)
            .with_context(|| format!("failed to list session dir {}", self.dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() || path.extension().map(|ext| ext != "json").unwrap_or(true) {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                ids.push(stem.to_string());
            }
        }
        Ok(ids)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.snapshot_path(id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to remove snapshot {}", path.display()))
            }
        }
    }
}

impl LifetimeGate {
    fn includes(self, lifetime: Lifetime) -> bool {
        match self {
            LifetimeGate::Both => true,
            LifetimeGate::PortableSession => lifetime == Lifetime::PortableSession,
        }
    }
}

fn read_envelope<T: SessionSnapshot>(path: &Path) -> Result<Envelope<T>> {
    let content = std::fs::read(path)
        .with_context(|| format!("failed to read snapshot {}", path.display()))?;
    let envelope: Envelope<T> = serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse snapshot {}", path.display()))?;
    let body = serde_json::to_vec(&envelope.body)
        .with_context(|| format!("failed to canonicalize snapshot {}", path.display()))?;
    if format!("{:016x}", fnv1a(&body)) != envelope.checksum {
        anyhow::bail!("snapshot {} failed its checksum", path.display());
    }
    if envelope.body.schema_version() != T::SCHEMA_VERSION {
        anyhow::bail!(
            "snapshot {} carries schema {} (expected {})",
            path.display(),
            envelope.body.schema_version(),
            T::SCHEMA_VERSION
        );
    }
    Ok(envelope)
}

fn fnv1a(body: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in body {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use serde::{Deserialize, Serialize};

    use super::*;

    static RESTORED: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);
    static FAILED: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

    fn test_dir() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("qol-host-session-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestSnapshot {
        schema_version: u32,
        id: String,
        value: u64,
    }

    impl SessionSnapshot for TestSnapshot {
        const SCHEMA_VERSION: u32 = 1;
        const SUBDIR: &'static str = "test";

        fn id(&self) -> &str {
            &self.id
        }

        fn schema_version(&self) -> u32 {
            self.schema_version
        }
    }

    fn snapshot(value: u64) -> TestSnapshot {
        TestSnapshot {
            schema_version: 1,
            id: "k".to_string(),
            value,
        }
    }

    const OWNER: &str = "tests";

    fn bump(map: &Mutex<Option<HashMap<String, usize>>>, key: &str) {
        let mut guard = map.lock().unwrap();
        *guard
            .get_or_insert_with(HashMap::new)
            .entry(key.to_string())
            .or_insert(0) += 1;
    }

    fn count(map: &Mutex<Option<HashMap<String, usize>>>, key: &str) -> usize {
        map.lock()
            .unwrap()
            .as_ref()
            .and_then(|counts| counts.get(key).copied())
            .unwrap_or(0)
    }

    #[derive(Debug, Clone)]
    struct FakeMutation {
        id: String,
        lifetime: Lifetime,
        capture: u64,
    }

    impl FakeMutation {
        fn new(id: &str, lifetime: Lifetime, capture: u64) -> Self {
            Self {
                id: id.to_string(),
                lifetime,
                capture,
            }
        }

        fn restore_snapshot(snapshot: TestSnapshot) -> Result<()> {
            if snapshot.value == u64::MAX {
                bump(&FAILED, &snapshot.id);
                anyhow::bail!("restore rejected value {}", snapshot.value);
            }
            bump(&RESTORED, &snapshot.id);
            Ok(())
        }
    }

    impl HostMutation for FakeMutation {
        type Snapshot = TestSnapshot;

        fn owner(&self) -> &str {
            "tests"
        }

        fn id(&self) -> MutationId {
            self.id.clone()
        }

        fn lifetime(&self) -> Lifetime {
            self.lifetime
        }

        fn capture(&self) -> Result<Self::Snapshot> {
            Ok(TestSnapshot {
                schema_version: 1,
                id: self.id.clone(),
                value: self.capture,
            })
        }

        fn restore(snapshot: Self::Snapshot) -> Result<()> {
            Self::restore_snapshot(snapshot)
        }
    }

    #[test]
    fn store_round_trips_and_rejects_tampering() {
        let dir = test_dir();
        let store = SessionStore::new(dir.join("test"));
        let snap = snapshot(7);
        store.write(&snap).unwrap();
        assert_eq!(store.load::<TestSnapshot>("k").unwrap(), Some(snap.clone()));
        assert!(store.load::<TestSnapshot>("missing").unwrap().is_none());

        let path = store.dir().join("k.json");
        let mut raw = std::fs::read(&path).unwrap();
        raw[10] ^= 0xff;
        std::fs::write(&path, &raw).unwrap();
        assert!(
            store.load::<TestSnapshot>("k").is_err(),
            "tampered snapshot must not load"
        );
    }

    #[test]
    fn envelope_writes_a_lifetime() {
        let dir = test_dir();
        let store = SessionStore::new(dir.join("test"));
        store.write(&snapshot(1)).unwrap();

        let raw = std::fs::read(store.dir().join("k.json")).unwrap();
        let envelope: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(envelope["lifetime"], "portable_session");
    }

    #[test]
    fn legacy_envelope_without_lifetime_still_loads() {
        let dir = test_dir();
        let store = SessionStore::new(dir.join("test"));
        store.write(&snapshot(1)).unwrap();

        let path = store.dir().join("k.json");
        let mut envelope: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        envelope.as_object_mut().unwrap().remove("lifetime");
        std::fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert!(store.load::<TestSnapshot>("k").unwrap().is_some());
    }

    #[test]
    fn claim_twice_keeps_the_first_captured_value() {
        let dir = test_dir();
        let store = SessionStore::new(dir);
        let first = FakeMutation::new("brand", Lifetime::PortableSession, 11);
        let second = FakeMutation::new("brand", Lifetime::PortableSession, 99);
        store.claim(&first).unwrap();
        store.claim(&second).unwrap();
        let loaded = store
            .owner_store(OWNER)
            .load::<TestSnapshot>("brand")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.value, 11);
        assert_eq!(count(&RESTORED, "brand"), 0);
    }

    #[test]
    fn portable_exit_restores_but_resident_exit_does_not() {
        let dir = test_dir();
        let store = SessionStore::new(dir);
        store
            .claim(&FakeMutation::new("port", Lifetime::PortableSession, 1))
            .unwrap();
        let report = store.release_session::<FakeMutation>("tests", Residency::Resident);
        assert_eq!(report.restored, 0);
        assert_eq!(count(&RESTORED, "port"), 0);
        assert!(store
            .owner_store(OWNER)
            .load::<TestSnapshot>("port")
            .unwrap()
            .is_some());

        let report = store.release_session::<FakeMutation>("tests", Residency::Portable);
        assert_eq!(report.restored, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(count(&RESTORED, "port"), 1);
        assert!(store
            .owner_store(OWNER)
            .load::<TestSnapshot>("port")
            .unwrap()
            .is_none());
    }

    #[test]
    fn residency_off_restores_what_a_resident_exit_left_behind() {
        let dir = test_dir();
        let store = SessionStore::new(dir);
        store
            .claim(&FakeMutation::new("res", Lifetime::ResidentPolicy, 5))
            .unwrap();
        let resident_exit = store.release_session::<FakeMutation>("tests", Residency::Resident);
        assert_eq!(resident_exit.restored, 0);
        assert!(store
            .owner_store(OWNER)
            .load::<TestSnapshot>("res")
            .unwrap()
            .is_some());

        let report = store.release_residency::<FakeMutation>("tests");
        assert_eq!(report.restored, 1);
        assert_eq!(count(&RESTORED, "res"), 1);
        assert!(store
            .owner_store(OWNER)
            .load::<TestSnapshot>("res")
            .unwrap()
            .is_none());
    }

    #[test]
    fn recover_restores_a_previous_session_and_skips_on_resident() {
        let dir = test_dir();
        {
            let first = SessionStore::new(dir.clone());
            first
                .claim(&FakeMutation::new("gone", Lifetime::PortableSession, 3))
                .unwrap();
        }
        let second = SessionStore::new(dir.clone());
        let report = second.recover::<FakeMutation>("tests", Residency::Resident);
        assert_eq!(report.restored, 0);
        assert_eq!(count(&RESTORED, "gone"), 0);
        assert!(second
            .owner_store(OWNER)
            .load::<TestSnapshot>("gone")
            .unwrap()
            .is_some());

        let report = second.recover::<FakeMutation>("tests", Residency::Portable);
        assert_eq!(report.restored, 1);
        assert_eq!(count(&RESTORED, "gone"), 1);
        assert!(second
            .owner_store(OWNER)
            .load::<TestSnapshot>("gone")
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_failing_restore_is_counted_failed_and_survives_for_retry() {
        let dir = test_dir();
        let store = SessionStore::new(dir);
        store
            .claim(&FakeMutation::new(
                "poison",
                Lifetime::PortableSession,
                u64::MAX,
            ))
            .unwrap();

        let first = store.release_session::<FakeMutation>("tests", Residency::Portable);
        assert_eq!(first.failed, 1);
        assert_eq!(first.restored, 0);
        assert_eq!(count(&FAILED, "poison"), 1);
        assert!(store
            .owner_store(OWNER)
            .load::<TestSnapshot>("poison")
            .unwrap()
            .is_some());

        store.owner_store(OWNER).delete("poison").unwrap();
        let second = store.release_session::<FakeMutation>("tests", Residency::Portable);
        assert_eq!(second.restored, 0);
        assert_eq!(second.nothing_to_restore, 1);
    }
}
