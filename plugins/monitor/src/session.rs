use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use qol_windowing::display::DisplayHandle;

use crate::monitor::{BrightnessSource, DisplayControl, GammaTable, MonitorError};

pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub session_id: String,
    pub display_id: String,
    pub connector: String,
    pub value: u8,
    pub source: String,
    pub last_value: u8,
    pub mutations: u32,
    pub clean: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lut: Option<GammaTable>,
    #[serde(default)]
    pub checksum: String,
}

impl Snapshot {
    pub fn canonical_checksum(&self) -> String {
        let mut hash = 0xcbf29ce484222325u64;
        let source_byte = self.source.as_bytes().first().copied().unwrap_or(0);
        for field in [
            self.session_id.as_bytes(),
            self.display_id.as_bytes(),
            self.connector.as_bytes(),
            &[self.value, source_byte, self.last_value],
            &self.mutations.to_le_bytes(),
            &[u8::from(self.clean)],
        ] {
            if let Some(lut) = &self.lut {
                for byte in lut.checksum().to_le_bytes() {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x100000001b3);
                }
            }
            for byte in field {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        format!("{hash:016x}")
    }

    pub fn with_checksum(mut self) -> Self {
        self.checksum = self.canonical_checksum();
        self
    }

    pub fn verify_checksum(&self) -> bool {
        !self.checksum.is_empty() && self.checksum == self.canonical_checksum()
    }

    pub fn source_kind(&self) -> Option<BrightnessSource> {
        match self.source.as_str() {
            "ddc" => Some(BrightnessSource::Ddc),
            "gamma" => Some(BrightnessSource::Gamma),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LutRestoreOutcome {
    Restored,
    ForeignLutPreserved,
    Unavailable,
}

pub trait LutProvider: Send + Sync {
    fn capture(&self, connector: &str) -> Option<GammaTable>;
    fn write_guarded(
        &self,
        handle: &DisplayHandle,
        original: &GammaTable,
        last_value: u8,
    ) -> LutRestoreOutcome;
}

pub struct NoLutProvider;

impl LutProvider for NoLutProvider {
    fn capture(&self, _connector: &str) -> Option<GammaTable> {
        None
    }

    fn write_guarded(
        &self,
        _handle: &DisplayHandle,
        _original: &GammaTable,
        _last_value: u8,
    ) -> LutRestoreOutcome {
        LutRestoreOutcome::Unavailable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcome {
    Restored,
    NothingToRestore,
    SkippedDisplayGone,
    ForeignLutPreserved,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub restored: usize,
    pub nothing_to_restore: usize,
    pub skipped_display_gone: usize,
    pub foreign_lut_preserved: usize,
    pub failed: usize,
    pub unreadable: usize,
}

impl RestoreReport {
    pub fn record(&mut self, outcome: RestoreOutcome) {
        match outcome {
            RestoreOutcome::Restored => self.restored += 1,
            RestoreOutcome::NothingToRestore => self.nothing_to_restore += 1,
            RestoreOutcome::SkippedDisplayGone => self.skipped_display_gone += 1,
            RestoreOutcome::ForeignLutPreserved => self.foreign_lut_preserved += 1,
            RestoreOutcome::Failed => self.failed += 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode {
    Exit,
    Recovery,
}

pub struct SessionStore {
    dir: PathBuf,
}
#[derive(Debug, Default)]
pub struct SnapshotInventory {
    pub snapshots: Vec<Snapshot>,
    pub unreadable: Vec<PathBuf>,
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

    fn snapshot_path(&self, display_id: &str) -> PathBuf {
        self.dir.join(format!("{display_id}.json"))
    }

    pub fn write_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("failed to create session dir {}", self.dir.display()))?;
        let content = serde_json::to_vec(&snapshot.clone().with_checksum())
            .context("failed to serialize the brightness snapshot")?;
        qol_fs::atomic_write_durable_mode(
            &self.snapshot_path(&snapshot.display_id),
            &content,
            0o600,
        )
        .with_context(|| {
            format!(
                "failed to commit snapshot {}",
                self.snapshot_path(&snapshot.display_id).display()
            )
        })
    }

    pub fn load_snapshot(&self, display_id: &str) -> Result<Option<Snapshot>> {
        let path = self.snapshot_path(display_id);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read(&path)
            .with_context(|| format!("failed to read snapshot {}", path.display()))?;
        let snapshot: Snapshot = serde_json::from_slice(&content)
            .with_context(|| format!("failed to parse snapshot {}", path.display()))?;
        if snapshot.schema_version != SNAPSHOT_SCHEMA_VERSION {
            anyhow::bail!(
                "snapshot {} carries schema {} (expected {})",
                path.display(),
                snapshot.schema_version,
                SNAPSHOT_SCHEMA_VERSION
            );
        }
        if !snapshot.verify_checksum() {
            anyhow::bail!("snapshot {} failed its checksum", path.display());
        }
        Ok(Some(snapshot))
    }

    pub fn load_all(&self) -> Result<SnapshotInventory> {
        let mut inventory = SnapshotInventory::default();
        if !self.dir.exists() {
            return Ok(inventory);
        }
        for entry in std::fs::read_dir(&self.dir)
            .with_context(|| format!("failed to list session dir {}", self.dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() || path.extension().map(|ext| ext != "json").unwrap_or(true) {
                continue;
            }
            let display_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
                .unwrap_or_default();
            match self.load_snapshot(&display_id) {
                Ok(Some(snapshot)) => inventory.snapshots.push(snapshot),
                Ok(None) => {}
                Err(error) => {
                    eprintln!(
                        "[plugin-monitor] skipping unreadable snapshot {}: {error:#}",
                        path.display()
                    );
                    inventory.unreadable.push(path);
                }
            }
        }
        Ok(inventory)
    }

    pub fn delete_snapshot(&self, display_id: &str) -> Result<()> {
        let path = self.snapshot_path(display_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to remove snapshot {}", path.display()))
            }
        }
    }
}

pub struct Session<C: ?Sized> {
    control: Arc<C>,
    store: SessionStore,
    lut: Arc<dyn LutProvider>,
    snapshotted: Mutex<HashSet<String>>,
}

impl<C: DisplayControl + ?Sized> Session<C> {
    pub fn new(control: Arc<C>, store: SessionStore, lut: Arc<dyn LutProvider>) -> Self {
        Self {
            control,
            store,
            lut,
            snapshotted: Mutex::new(HashSet::new()),
        }
    }

    pub fn control(&self) -> &Arc<C> {
        &self.control
    }

    pub fn store(&self) -> &SessionStore {
        &self.store
    }

    fn is_snapshotted(&self, display_id: &str) -> bool {
        self.snapshotted.lock().unwrap().contains(display_id)
    }

    fn mark_snapshotted(&self, display_id: &str) {
        self.snapshotted
            .lock()
            .unwrap()
            .insert(display_id.to_string());
    }

    fn ensure_snapshot(&self, handle: &DisplayHandle) -> Result<(), MonitorError> {
        if self.is_snapshotted(handle.id()) {
            return Ok(());
        }
        let state = self
            .control
            .get_brightness(handle)
            .map_err(|error| MonitorError::refused("brightness", format!("{error}")))?;
        let lut = if state.source == BrightnessSource::Gamma {
            self.lut.capture(handle.connector())
        } else {
            None
        };
        let snapshot = Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            session_id: qol_host_fixes::policy::new_session_id()
                .unwrap_or_else(|_| "session".to_string()),
            display_id: handle.id().to_string(),
            connector: handle.connector().to_string(),
            value: state.value,
            source: state.source.label().to_string(),
            last_value: state.value,
            mutations: 0,
            clean: false,
            lut,
            checksum: String::new(),
        }
        .with_checksum();
        self.store
            .write_snapshot(&snapshot)
            .map_err(|error| MonitorError::refused("brightness", format!("{error:#}")))?;
        self.mark_snapshotted(handle.id());
        Ok(())
    }

    pub fn mutate(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        self.ensure_snapshot(handle)?;
        let snapshot = self
            .store
            .load_snapshot(handle.id())
            .map_err(|error| MonitorError::refused("brightness", format!("{error:#}")))?
            .ok_or_else(|| {
                MonitorError::refused("brightness", "the brightness snapshot vanished")
            })?;
        let updated = Snapshot {
            mutations: snapshot.mutations + 1,
            last_value: value,
            ..snapshot
        };
        self.store
            .write_snapshot(&updated)
            .map_err(|error| MonitorError::refused("brightness", format!("{error:#}")))?;
        self.control.set_brightness(handle, value)
    }

    fn restore_one(&self, snapshot: &Snapshot, handle: &DisplayHandle) -> RestoreOutcome {
        if snapshot.source_kind() == Some(BrightnessSource::Gamma) {
            if let Some(original) = &snapshot.lut {
                return match self
                    .lut
                    .write_guarded(handle, original, snapshot.last_value)
                {
                    LutRestoreOutcome::Restored => RestoreOutcome::Restored,
                    LutRestoreOutcome::ForeignLutPreserved => RestoreOutcome::ForeignLutPreserved,
                    LutRestoreOutcome::Unavailable => RestoreOutcome::Failed,
                };
            }
        }
        match self.control.set_brightness(handle, snapshot.value) {
            Ok(()) => RestoreOutcome::Restored,
            Err(error) => {
                eprintln!(
                    "[plugin-monitor] restore of {} failed: {error}",
                    handle.connector()
                );
                RestoreOutcome::Failed
            }
        }
    }

    pub fn restore_all(&self, mode: RestoreMode) -> RestoreReport {
        let mut report = RestoreReport::default();
        let Ok(inventory) = self.store.load_all() else {
            eprintln!("[plugin-monitor] session directory is unreadable; restore skipped");
            return report;
        };
        report.unreadable = inventory.unreadable.len();
        for snapshot in inventory.snapshots {
            if snapshot.mutations == 0 {
                let _ = self.store.delete_snapshot(&snapshot.display_id);
                report.record(RestoreOutcome::NothingToRestore);
                continue;
            }
            if snapshot.clean {
                let _ = self.store.delete_snapshot(&snapshot.display_id);
                report.record(RestoreOutcome::NothingToRestore);
                continue;
            }
            let Some(handle) = self.find_handle(&snapshot) else {
                report.record(RestoreOutcome::SkippedDisplayGone);
                continue;
            };
            let outcome = self.restore_one(&snapshot, &handle);
            match mode {
                RestoreMode::Exit => match outcome {
                    RestoreOutcome::Restored => {
                        let _ = self.store.write_snapshot(&Snapshot {
                            clean: true,
                            ..snapshot
                        });
                    }
                    RestoreOutcome::ForeignLutPreserved | RestoreOutcome::NothingToRestore => {
                        let _ = self.store.delete_snapshot(&snapshot.display_id);
                    }
                    RestoreOutcome::SkippedDisplayGone | RestoreOutcome::Failed => {}
                },
                RestoreMode::Recovery => match outcome {
                    RestoreOutcome::Restored
                    | RestoreOutcome::ForeignLutPreserved
                    | RestoreOutcome::NothingToRestore => {
                        let _ = self.store.delete_snapshot(&snapshot.display_id);
                    }
                    RestoreOutcome::SkippedDisplayGone | RestoreOutcome::Failed => {}
                },
            }
            report.record(outcome);
        }
        report
    }

    fn find_handle(&self, snapshot: &Snapshot) -> Option<DisplayHandle> {
        let handles = self.control.enumerate().ok()?;
        handles
            .iter()
            .find(|handle| handle.id() == snapshot.display_id)
            .or_else(|| {
                handles
                    .iter()
                    .find(|handle| handle.connector() == snapshot.connector)
            })
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::{BrightnessState, DisplayCapabilities, DisplayMode, GammaState, HdrState};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    fn handle(id: &str, connector: &str) -> DisplayHandle {
        DisplayHandle::new(id.into(), connector.into(), None, false)
    }

    fn fake_store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("session"));
        (dir, store)
    }

    struct RecordingControl {
        displays: Vec<DisplayHandle>,
        current: StdMutex<u8>,
        sets: AtomicUsize,
        calls: StdMutex<Vec<(String, u8)>>,
    }

    impl RecordingControl {
        fn new(displays: Vec<DisplayHandle>, current: u8) -> Self {
            Self {
                displays,
                current: StdMutex::new(current),
                sets: AtomicUsize::new(0),
                calls: StdMutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, u8)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl DisplayControl for RecordingControl {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
            Ok(self.displays.clone())
        }

        fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
            Ok(DisplayCapabilities::none())
        }

        fn get_brightness(&self, _handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
            Ok(BrightnessState {
                value: *self.current.lock().unwrap(),
                source: BrightnessSource::Ddc,
            })
        }

        fn set_brightness(&self, _handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
            self.sets.fetch_add(1, Ordering::SeqCst);
            *self.current.lock().unwrap() = value;
            self.calls
                .lock()
                .unwrap()
                .push((_handle.id().to_string(), value));
            Ok(())
        }

        fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
            Err(MonitorError::unsupported("modes", "test"))
        }

        fn set_mode(
            &self,
            _handle: &DisplayHandle,
            _mode: &DisplayMode,
        ) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("modes", "test"))
        }

        fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }

        fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }
    }

    fn snapshot(
        display_id: &str,
        connector: &str,
        value: u8,
        last_value: u8,
        mutations: u32,
        clean: bool,
    ) -> Snapshot {
        Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            session_id: "00000000-0000-4000-8000-000000000000".into(),
            display_id: display_id.into(),
            connector: connector.into(),
            value,
            source: "ddc".into(),
            last_value,
            mutations,
            clean,
            lut: None,
            checksum: String::new(),
        }
        .with_checksum()
    }

    #[test]
    fn snapshot_checksum_detects_any_field_change() {
        let original = snapshot("id-1", "card0-DP-1", 100, 60, 3, false);
        assert!(original.verify_checksum());
        let mut changed = original.clone();
        changed.value = 99;
        assert!(!changed.verify_checksum());
        let mut changed_clean = original.clone();
        changed_clean.clean = true;
        assert!(!changed_clean.verify_checksum());
        let mut changed_mutations = original.clone();
        changed_mutations.mutations = 4;
        assert!(!changed_mutations.verify_checksum());
    }

    #[test]
    fn empty_source_checksums_without_panicking() {
        let snap = Snapshot {
            source: String::new(),
            ..snapshot("id-1", "card0-DP-1", 100, 60, 3, false)
        }
        .with_checksum();
        assert!(!snap.checksum.is_empty());
        assert!(snap.verify_checksum());
    }

    #[test]
    fn store_round_trips_and_rejects_tampered_snapshots() {
        let (_dir, store) = fake_store();
        let snap = snapshot("id-1", "card0-DP-1", 100, 60, 3, false);
        store.write_snapshot(&snap).unwrap();
        assert_eq!(store.load_snapshot("id-1").unwrap(), Some(snap.clone()));
        assert!(store.load_snapshot("missing").unwrap().is_none());

        let path = store.dir().join("id-1.json");
        let mut raw = std::fs::read(&path).unwrap();
        raw[20] ^= 0xff;
        std::fs::write(&path, &raw).unwrap();
        assert!(
            store.load_snapshot("id-1").is_err(),
            "tampered snapshot must not load"
        );
    }

    #[test]
    fn mutate_snapshots_before_the_first_change_only() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(RecordingControl::new(vec![display.clone()], 100));
        let session = Session::new(control.clone(), store.clone(), Arc::new(NoLutProvider));
        session.mutate(&display, 60).unwrap();
        session.mutate(&display, 55).unwrap();
        let snap = store.load_snapshot("id-1").unwrap().unwrap();
        assert_eq!(
            snap.value, 100,
            "original is captured before the first change"
        );
        assert_eq!(snap.last_value, 55);
        assert_eq!(snap.mutations, 2);
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 60), ("id-1".to_string(), 55)]
        );
    }

    #[test]
    fn zero_mutation_snapshots_are_deleted_never_restored() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        store
            .write_snapshot(&snapshot("id-1", "card0-DP-1", 100, 100, 0, false))
            .unwrap();
        let control = Arc::new(RecordingControl::new(vec![display.clone()], 100));
        let session = Session::new(control.clone(), store.clone(), Arc::new(NoLutProvider));
        let report = session.restore_all(RestoreMode::Recovery);
        assert_eq!(report.nothing_to_restore, 1);
        assert_eq!(report.restored, 0);
        assert!(store.load_snapshot("id-1").unwrap().is_none());
        assert_eq!(control.sets.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stale_snapshot_restores_and_is_removed() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        store
            .write_snapshot(&snapshot("id-1", "card0-DP-1", 100, 60, 3, false))
            .unwrap();
        let control = Arc::new(RecordingControl::new(vec![display.clone()], 60));
        let session = Session::new(control.clone(), store.clone(), Arc::new(NoLutProvider));
        let report = session.restore_all(RestoreMode::Recovery);
        assert_eq!(report.restored, 1);
        assert_eq!(control.calls(), vec![("id-1".to_string(), 100)]);
        assert!(store.load_snapshot("id-1").unwrap().is_none());
        let second = session.restore_all(RestoreMode::Recovery);
        assert_eq!(second.restored, 0, "restore is idempotent");
    }

    #[test]
    fn clean_marked_snapshots_are_not_restored() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        store
            .write_snapshot(&snapshot("id-1", "card0-DP-1", 100, 60, 3, true))
            .unwrap();
        let control = Arc::new(RecordingControl::new(vec![display.clone()], 60));
        let session = Session::new(control.clone(), store.clone(), Arc::new(NoLutProvider));
        let report = session.restore_all(RestoreMode::Recovery);
        assert_eq!(report.restored, 0);
        assert_eq!(control.sets.load(Ordering::SeqCst), 0);
        assert!(
            store.load_snapshot("id-1").unwrap().is_none(),
            "a clean snapshot is retired at startup"
        );
    }

    #[test]
    fn vanished_display_keeps_its_snapshot_alive() {
        let (_dir, store) = fake_store();
        store
            .write_snapshot(&snapshot("id-1", "card0-DP-1", 100, 60, 3, false))
            .unwrap();
        let control = Arc::new(RecordingControl::new(vec![], 60));
        let session = Session::new(control.clone(), store.clone(), Arc::new(NoLutProvider));
        let report = session.restore_all(RestoreMode::Recovery);
        assert_eq!(report.skipped_display_gone, 1);
        assert_eq!(control.sets.load(Ordering::SeqCst), 0);
        assert!(
            store.load_snapshot("id-1").unwrap().is_some(),
            "the journal stays alive for the same identity returning"
        );
    }

    #[test]
    fn exit_restore_marks_clean_and_recovers_by_connector_fallback() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(RecordingControl::new(vec![display.clone()], 55));
        let session = Session::new(control.clone(), store.clone(), Arc::new(NoLutProvider));
        session.mutate(&display, 55).unwrap();
        control.calls.lock().unwrap().clear();
        let report = session.restore_all(RestoreMode::Exit);
        assert_eq!(report.restored, 1);
        assert_eq!(control.calls(), vec![("id-1".to_string(), 55)]);
        let snap = store.load_snapshot("id-1").unwrap().unwrap();
        assert!(snap.clean, "exit restore leaves the clean-exit marker");
        let again = session.restore_all(RestoreMode::Exit);
        assert_eq!(again.restored, 0, "a second exit restore is a no-op");
    }

    struct FakeLut {
        current: Arc<StdMutex<GammaTable>>,
    }

    impl LutProvider for FakeLut {
        fn capture(&self, _connector: &str) -> Option<GammaTable> {
            Some(self.current.lock().unwrap().clone())
        }

        fn write_guarded(
            &self,
            _handle: &DisplayHandle,
            original: &GammaTable,
            last_value: u8,
        ) -> LutRestoreOutcome {
            let mut current = self.current.lock().unwrap();
            if current.checksum() == original.dimmed(last_value).checksum() {
                *current = original.clone();
                LutRestoreOutcome::Restored
            } else {
                LutRestoreOutcome::ForeignLutPreserved
            }
        }
    }

    fn gamma_table(base: u16) -> GammaTable {
        GammaTable {
            red: vec![base, base],
            green: vec![base, base],
            blue: vec![base, base],
        }
    }

    #[test]
    fn gamma_restore_restores_the_lut_only_while_ours_is_in_place() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        let original = gamma_table(1000);
        let current = Arc::new(StdMutex::new(original.dimmed(60)));
        let lut = FakeLut {
            current: Arc::clone(&current),
        };
        store
            .write_snapshot(&Snapshot {
                source: "gamma".into(),
                lut: Some(original.clone()),
                last_value: 60,
                ..snapshot("id-1", "card0-DP-1", 100, 60, 3, false)
            })
            .unwrap();
        let control = Arc::new(RecordingControl::new(vec![display.clone()], 60));
        let session = Session::new(control.clone(), store, Arc::new(lut));
        let report = session.restore_all(RestoreMode::Recovery);
        assert_eq!(report.restored, 1);
        assert_eq!(
            current.lock().unwrap().checksum(),
            original.checksum(),
            "the original LUT is written back"
        );
    }

    #[test]
    fn gamma_restore_preserves_a_foreign_lut() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        let original = gamma_table(1000);
        let mut foreign = original.dimmed(60);
        foreign.red[0] += 1;
        let current = Arc::new(StdMutex::new(foreign.clone()));
        let lut = FakeLut {
            current: Arc::clone(&current),
        };
        store
            .write_snapshot(&Snapshot {
                source: "gamma".into(),
                lut: Some(original.clone()),
                last_value: 60,
                ..snapshot("id-1", "card0-DP-1", 100, 60, 3, false)
            })
            .unwrap();
        let control = Arc::new(RecordingControl::new(vec![display.clone()], 60));
        let session = Session::new(control.clone(), store, Arc::new(lut));
        let report = session.restore_all(RestoreMode::Recovery);
        assert_eq!(report.foreign_lut_preserved, 1);
        assert_eq!(
            current.lock().unwrap().checksum(),
            foreign.checksum(),
            "a foreign LUT is never overwritten"
        );
    }

    struct SetFailureControl {
        displays: Vec<DisplayHandle>,
        current: StdMutex<u8>,
        calls: StdMutex<Vec<(String, u8)>>,
        fail: AtomicBool,
    }

    impl SetFailureControl {
        fn new(displays: Vec<DisplayHandle>, current: u8) -> Self {
            Self {
                displays,
                current: StdMutex::new(current),
                calls: StdMutex::new(Vec::new()),
                fail: AtomicBool::new(true),
            }
        }

        fn heal(&self) {
            self.fail.store(false, Ordering::SeqCst);
        }

        fn calls(&self) -> Vec<(String, u8)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl DisplayControl for SetFailureControl {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
            Ok(self.displays.clone())
        }

        fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
            Ok(DisplayCapabilities::none())
        }

        fn get_brightness(&self, _handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
            Ok(BrightnessState {
                value: *self.current.lock().unwrap(),
                source: BrightnessSource::Ddc,
            })
        }

        fn set_brightness(&self, _handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
            self.calls
                .lock()
                .unwrap()
                .push((_handle.id().to_string(), value));
            if self.fail.load(Ordering::SeqCst) {
                return Err(MonitorError::refused("brightness", "injected"));
            }
            *self.current.lock().unwrap() = value;
            Ok(())
        }

        fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
            Err(MonitorError::unsupported("modes", "test"))
        }

        fn set_mode(
            &self,
            _handle: &DisplayHandle,
            _mode: &DisplayMode,
        ) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("modes", "test"))
        }

        fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }

        fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }
    }

    #[test]
    fn recovery_keeps_failed_snapshots_for_the_next_attempt() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        store
            .write_snapshot(&snapshot("id-1", "card0-DP-1", 100, 60, 3, false))
            .unwrap();
        let control = Arc::new(SetFailureControl::new(vec![display.clone()], 60));
        let session = Session::new(control.clone(), store.clone(), Arc::new(NoLutProvider));
        let report = session.restore_all(RestoreMode::Recovery);
        assert_eq!(report.failed, 1);
        assert!(
            store.load_snapshot("id-1").unwrap().is_some(),
            "a failed recovery keeps the restore base for the next attempt"
        );
        control.heal();
        let report = session.restore_all(RestoreMode::Recovery);
        assert_eq!(report.restored, 1);
        assert!(
            store.load_snapshot("id-1").unwrap().is_none(),
            "a successful recovery retires the snapshot"
        );
    }

    #[test]
    fn recovery_keeps_a_failed_gamma_restore_base() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        store
            .write_snapshot(&Snapshot {
                source: "gamma".into(),
                lut: Some(gamma_table(1000)),
                last_value: 60,
                ..snapshot("id-1", "card0-DP-1", 100, 60, 3, false)
            })
            .unwrap();
        let control = Arc::new(RecordingControl::new(vec![display.clone()], 60));
        let session = Session::new(control.clone(), store.clone(), Arc::new(NoLutProvider));
        let report = session.restore_all(RestoreMode::Recovery);
        assert_eq!(report.failed, 1);
        assert!(
            store.load_snapshot("id-1").unwrap().is_some(),
            "the gamma LUT base must survive a failed recovery"
        );
    }

    struct JournalOrderControl {
        store: SessionStore,
        current: StdMutex<u8>,
        seen: StdMutex<Option<(u8, u32)>>,
    }

    impl DisplayControl for JournalOrderControl {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
            Ok(vec![])
        }

        fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
            Ok(DisplayCapabilities::none())
        }

        fn get_brightness(&self, _handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
            Ok(BrightnessState {
                value: *self.current.lock().unwrap(),
                source: BrightnessSource::Ddc,
            })
        }

        fn set_brightness(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
            let snap = self
                .store
                .load_snapshot(handle.id())
                .map_err(|error| MonitorError::refused("brightness", format!("{error:#}")))?
                .ok_or_else(|| {
                    MonitorError::refused("brightness", "the brightness snapshot vanished")
                })?;
            *self.seen.lock().unwrap() = Some((snap.last_value, snap.mutations));
            *self.current.lock().unwrap() = value;
            Ok(())
        }

        fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
            Err(MonitorError::unsupported("modes", "test"))
        }

        fn set_mode(
            &self,
            _handle: &DisplayHandle,
            _mode: &DisplayMode,
        ) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("modes", "test"))
        }

        fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }

        fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }
    }

    #[test]
    fn mutate_commits_the_intent_before_the_hardware_write() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(JournalOrderControl {
            store: store.clone(),
            current: StdMutex::new(100),
            seen: StdMutex::new(None),
        });
        let session = Session::new(control.clone(), store.clone(), Arc::new(NoLutProvider));
        session.mutate(&display, 60).unwrap();
        let (last_value, mutations) = control
            .seen
            .lock()
            .unwrap()
            .expect("the hardware write ran");
        assert_eq!(
            last_value, 60,
            "the intent is committed before the hardware write"
        );
        assert_eq!(mutations, 1);
    }

    #[test]
    fn failed_hardware_set_keeps_the_intent_and_recovery_restores_the_base() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(SetFailureControl::new(vec![display.clone()], 100));
        let session = Session::new(control.clone(), store.clone(), Arc::new(NoLutProvider));
        assert!(session.mutate(&display, 60).is_err());
        let snap = store.load_snapshot("id-1").unwrap().unwrap();
        assert_eq!(
            snap.mutations, 1,
            "the intent survives a refused hardware write"
        );
        assert_eq!(snap.last_value, 60);
        control.heal();
        let report = session.restore_all(RestoreMode::Recovery);
        assert_eq!(report.restored, 1);
        assert_eq!(
            control.calls().last(),
            Some(&("id-1".to_string(), 100)),
            "the base is restored"
        );
    }

    #[test]
    fn unreadable_snapshots_are_skipped_and_the_rest_still_restore() {
        let (_dir, store) = fake_store();
        let display = handle("id-1", "card0-DP-1");
        store
            .write_snapshot(&snapshot("id-1", "card0-DP-1", 100, 60, 3, false))
            .unwrap();
        std::fs::write(store.dir().join("id-2.json"), b"{not-json").unwrap();
        let future = store.dir().join("id-3.json");
        std::fs::write(
            &future,
            serde_json::to_vec(&snapshot("id-3", "card0-DP-2", 80, 80, 1, false)).unwrap(),
        )
        .unwrap();
        let mut raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&future).unwrap()).unwrap();
        raw["schema_version"] = serde_json::json!(SNAPSHOT_SCHEMA_VERSION + 1);
        std::fs::write(&future, serde_json::to_vec(&raw).unwrap()).unwrap();

        let control = Arc::new(RecordingControl::new(vec![display.clone()], 60));
        let session = Session::new(control.clone(), store.clone(), Arc::new(NoLutProvider));
        let report = session.restore_all(RestoreMode::Recovery);
        assert_eq!(report.restored, 1);
        assert_eq!(report.unreadable, 2, "both unreadable files are reported");
        assert_eq!(control.calls(), vec![("id-1".to_string(), 100)]);
        assert!(
            store.dir().join("id-2.json").exists(),
            "an unreadable snapshot is preserved, not deleted"
        );
        assert!(store.dir().join("id-3.json").exists());
        assert!(store.load_snapshot("id-1").unwrap().is_none());
    }
}
