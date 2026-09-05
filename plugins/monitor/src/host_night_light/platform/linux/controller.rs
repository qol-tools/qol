use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::super::super::{
    HostNightLight, HostNightLightError, HostNightLightStatus, TakeoverOutcome,
};
use crate::session::{RestoreMode, EVICTION_GENERATION};

const SNAPSHOT_ID: &str = "night-light-enabled";
const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

pub(super) trait Settings: Send + Sync {
    fn native_supported(&self) -> bool {
        false
    }
    fn name(&self) -> &'static str;
    fn native_values(&self) -> Result<Option<BTreeMap<String, String>>, HostNightLightError> {
        Ok(None)
    }
    fn apply_native(&self, _active: bool, _kelvin: u16) -> Result<(), HostNightLightError> {
        Err(HostNightLightError::Unsupported(
            "native night light control unavailable".into(),
        ))
    }
    fn restore_native(
        &self,
        _values: &BTreeMap<String, String>,
    ) -> Result<(), HostNightLightError> {
        Ok(())
    }
    fn get(&self) -> Result<bool, HostNightLightError>;
    fn set(&self, enabled: bool) -> Result<(), HostNightLightError>;
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Snapshot {
    schema_version: u32,
    id: String,
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native_values: Option<BTreeMap<String, String>>,
    mutations: u32,
    clean: bool,
    #[serde(default)]
    handoff: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    adopt_generation: Option<String>,
}

impl qol_host_session::SessionSnapshot for Snapshot {
    const SCHEMA_VERSION: u32 = SNAPSHOT_SCHEMA_VERSION;
    const SUBDIR: &'static str = "host-night-light";

    fn id(&self) -> &str {
        &self.id
    }

    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

#[derive(Debug, Clone, Copy)]
struct State {
    taken_over: bool,
    status: HostNightLightStatus,
}

pub(super) struct Controller<S: Settings> {
    settings: S,
    store: qol_host_session::SessionStore,
    state: Mutex<State>,
    adoption_generation: Option<String>,
}

impl<S: Settings> Controller<S> {
    pub(super) fn new(settings: S, dir: PathBuf) -> Self {
        Self {
            settings,
            store: qol_host_session::SessionStore::new(dir),
            state: Mutex::new(State {
                taken_over: false,
                status: HostNightLightStatus::Off,
            }),
            adoption_generation: std::env::var(qol_conventions::ENV_DEV_GENERATION_ID).ok(),
        }
    }

    #[cfg(test)]
    fn with_adoption_generation(mut self, generation: Option<String>) -> Self {
        self.adoption_generation = generation;
        self
    }

    fn set_state(&self, taken_over: bool, status: HostNightLightStatus) {
        *self.state.lock().unwrap() = State { taken_over, status };
    }

    fn snapshot(&self) -> Result<Option<Snapshot>, HostNightLightError> {
        self.store.load::<Snapshot>(SNAPSHOT_ID).map_err(|error| {
            HostNightLightError::Failed(format!("night light journal failed: {error:#}"))
        })
    }

    fn write(&self, snapshot: &Snapshot) -> Result<(), HostNightLightError> {
        self.store.write(snapshot).map_err(|error| {
            HostNightLightError::Failed(format!("night light journal failed: {error:#}"))
        })
    }

    fn handoff_is_current(&self, snapshot: &Snapshot) -> bool {
        match snapshot.adopt_generation.as_deref() {
            Some(EVICTION_GENERATION) => true,
            Some(generation) => self.adoption_generation.as_deref() == Some(generation),
            None => self.adoption_generation.is_some(),
        }
    }

    fn disable_for_gamma(&self) -> Result<TakeoverOutcome, HostNightLightError> {
        let enabled = self.settings.get().map_err(|error| self.fail(error))?;
        if enabled {
            self.settings.set(false).map_err(|error| self.fail(error))?;
        }
        self.set_state(
            true,
            if enabled {
                HostNightLightStatus::TakenOver
            } else {
                HostNightLightStatus::Off
            },
        );
        Ok(if enabled {
            TakeoverOutcome::Disabled
        } else {
            TakeoverOutcome::AlreadyOff
        })
    }

    fn fail(&self, error: HostNightLightError) -> HostNightLightError {
        let status = match error {
            HostNightLightError::Unsupported(_) => HostNightLightStatus::Unsupported,
            HostNightLightError::Failed(_) => HostNightLightStatus::Failed,
        };
        self.set_state(self.is_taken_over(), status);
        error
    }
}

impl<S: Settings> HostNightLight for Controller<S> {
    fn native_supported(&self) -> bool {
        self.settings.native_supported()
    }
    fn strategy(&self) -> &'static str {
        self.settings.name()
    }

    fn apply_native(&self, active: bool, kelvin: u16) -> Result<bool, HostNightLightError> {
        let mut snapshot = self.snapshot().map_err(|error| self.fail(error))?;
        if snapshot
            .as_ref()
            .is_none_or(|saved| saved.clean || saved.native_values.is_none())
        {
            let Some(mut values) = self.settings.native_values()? else {
                return Ok(false);
            };
            let enabled = snapshot
                .as_ref()
                .filter(|saved| !saved.clean)
                .map(|saved| saved.enabled)
                .map(Ok)
                .unwrap_or_else(|| self.settings.get())?;
            if let Some(value) = values.get_mut("night-light-enabled") {
                *value = enabled.to_string();
            }
            snapshot = Some(Snapshot {
                schema_version: SNAPSHOT_SCHEMA_VERSION,
                id: SNAPSHOT_ID.into(),
                enabled,
                native_values: Some(values),
                mutations: 1,
                clean: false,
                handoff: false,
                adopt_generation: None,
            });
            self.write(snapshot.as_ref().unwrap())
                .map_err(|error| self.fail(error))?;
        }
        self.set_state(true, HostNightLightStatus::TakenOver);
        self.settings
            .apply_native(active, kelvin)
            .map_err(|error| self.fail(error))?;
        self.set_state(true, HostNightLightStatus::TakenOver);
        Ok(true)
    }

    fn take_over(&self) -> Result<TakeoverOutcome, HostNightLightError> {
        if self.is_taken_over() {
            return self.disable_for_gamma();
        }
        if let Some(snapshot) = self.snapshot().map_err(|error| self.fail(error))? {
            if snapshot.clean {
                self.store
                    .delete(SNAPSHOT_ID)
                    .map_err(|error| self.fail(HostNightLightError::Failed(error.to_string())))?;
            } else {
                return self.disable_for_gamma();
            }
        }
        let enabled = self.settings.get().map_err(|error| self.fail(error))?;
        let snapshot = Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            id: SNAPSHOT_ID.to_string(),
            enabled,
            native_values: None,
            mutations: u32::from(enabled),
            clean: false,
            handoff: false,
            adopt_generation: None,
        };
        self.write(&snapshot).map_err(|error| self.fail(error))?;
        if enabled {
            self.settings.set(false).map_err(|error| self.fail(error))?;
            self.set_state(true, HostNightLightStatus::TakenOver);
            Ok(TakeoverOutcome::Disabled)
        } else {
            self.set_state(true, HostNightLightStatus::Off);
            Ok(TakeoverOutcome::AlreadyOff)
        }
    }

    fn release(&self, mode: RestoreMode) -> Result<(), HostNightLightError> {
        let Some(snapshot) = self.snapshot().map_err(|error| self.fail(error))? else {
            self.set_state(false, HostNightLightStatus::Off);
            return Ok(());
        };
        if snapshot.handoff && mode == RestoreMode::Recovery && self.handoff_is_current(&snapshot) {
            let status = if snapshot.enabled {
                HostNightLightStatus::TakenOver
            } else {
                HostNightLightStatus::Off
            };
            self.set_state(true, status);
            return Ok(());
        }
        if !snapshot.clean && snapshot.mutations > 0 {
            if let Some(values) = &snapshot.native_values {
                self.settings
                    .restore_native(values)
                    .map_err(|error| self.fail(error))?;
            } else if snapshot.enabled {
                self.settings.set(true).map_err(|error| self.fail(error))?;
            }
        }
        match mode {
            RestoreMode::Exit => self
                .write(&Snapshot {
                    clean: true,
                    handoff: false,
                    adopt_generation: None,
                    ..snapshot
                })
                .map_err(|error| self.fail(error))?,
            RestoreMode::Recovery => self
                .store
                .delete(SNAPSHOT_ID)
                .map_err(|error| self.fail(HostNightLightError::Failed(error.to_string())))?,
        }
        self.set_state(false, HostNightLightStatus::Off);
        Ok(())
    }

    fn mark_handoff(&self, successor: Option<&str>) {
        let Ok(Some(snapshot)) = self.snapshot() else {
            return;
        };
        if snapshot.clean {
            return;
        }
        let _ = self.write(&Snapshot {
            handoff: true,
            adopt_generation: successor.map(str::to_string),
            ..snapshot
        });
    }

    fn is_taken_over(&self) -> bool {
        self.state.lock().unwrap().taken_over
    }

    fn status(&self) -> HostNightLightStatus {
        self.state.lock().unwrap().status
    }
}

pub(super) fn session_dir(config_root: Option<&Path>, name: Option<&str>) -> PathBuf {
    let base = config_root
        .and_then(|root| crate::config::device_dir(root).ok())
        .map(|dir| dir.join("host-night-light"))
        .unwrap_or_else(|| std::env::temp_dir().join("qol-monitor-host-night-light"));
    name.map(|name| base.join(name)).unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::*;

    struct FakeSettings {
        value: StdMutex<bool>,
        writes: StdMutex<Vec<bool>>,
    }

    impl FakeSettings {
        fn new(value: bool) -> Self {
            Self {
                value: StdMutex::new(value),
                writes: StdMutex::new(Vec::new()),
            }
        }

        fn writes(&self) -> Vec<bool> {
            self.writes.lock().unwrap().clone()
        }
    }

    impl Settings for FakeSettings {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn get(&self) -> Result<bool, HostNightLightError> {
            Ok(*self.value.lock().unwrap())
        }

        fn set(&self, enabled: bool) -> Result<(), HostNightLightError> {
            *self.value.lock().unwrap() = enabled;
            self.writes.lock().unwrap().push(enabled);
            Ok(())
        }
    }

    fn controller(enabled: bool) -> (tempfile::TempDir, Controller<FakeSettings>) {
        let dir = tempfile::tempdir().unwrap();
        let controller = Controller::new(
            FakeSettings::new(enabled),
            dir.path().join("host-night-light"),
        )
        .with_adoption_generation(Some("current".to_string()));
        (dir, controller)
    }

    #[test]
    fn true_is_disabled_once_and_restored_on_release() {
        let (_dir, controller) = controller(true);
        assert_eq!(controller.take_over().unwrap(), TakeoverOutcome::Disabled);
        assert_eq!(controller.take_over().unwrap(), TakeoverOutcome::AlreadyOff);
        assert_eq!(controller.settings.writes(), vec![false]);
        controller.release(RestoreMode::Exit).unwrap();
        assert_eq!(controller.settings.writes(), vec![false, true]);
    }

    #[test]
    fn preexisting_false_is_never_written() {
        let (_dir, controller) = controller(false);
        assert_eq!(controller.take_over().unwrap(), TakeoverOutcome::AlreadyOff);
        controller.release(RestoreMode::Exit).unwrap();
        assert!(controller.settings.writes().is_empty());
    }

    #[test]
    fn recovery_after_a_crash_restores_true() {
        let (dir, first) = controller(true);
        first.take_over().unwrap();
        let second = Controller::new(
            FakeSettings::new(false),
            dir.path().join("host-night-light"),
        );
        second.release(RestoreMode::Recovery).unwrap();
        assert_eq!(second.settings.writes(), vec![true]);
    }

    #[test]
    fn matching_handoff_is_adopted_without_a_write() {
        let (dir, first) = controller(true);
        first.take_over().unwrap();
        first.mark_handoff(Some("next"));
        let second = Controller::new(
            FakeSettings::new(false),
            dir.path().join("host-night-light"),
        )
        .with_adoption_generation(Some("next".to_string()));
        second.release(RestoreMode::Recovery).unwrap();
        assert!(second.is_taken_over());
        assert!(second.settings.writes().is_empty());
    }
    struct NativeSettings {
        values: StdMutex<BTreeMap<String, String>>,
    }

    impl Settings for NativeSettings {
        fn native_supported(&self) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "native-test"
        }
        fn get(&self) -> Result<bool, HostNightLightError> {
            Ok(self.values.lock().unwrap()["night-light-enabled"] == "true")
        }
        fn set(&self, enabled: bool) -> Result<(), HostNightLightError> {
            self.values
                .lock()
                .unwrap()
                .insert("night-light-enabled".into(), enabled.to_string());
            Ok(())
        }
        fn native_values(&self) -> Result<Option<BTreeMap<String, String>>, HostNightLightError> {
            Ok(Some(self.values.lock().unwrap().clone()))
        }
        fn apply_native(&self, active: bool, kelvin: u16) -> Result<(), HostNightLightError> {
            *self.values.lock().unwrap() = BTreeMap::from([
                ("night-light-enabled".into(), active.to_string()),
                ("temperature".into(), kelvin.to_string()),
                ("schedule".into(), "always".into()),
            ]);
            Ok(())
        }
        fn restore_native(
            &self,
            values: &BTreeMap<String, String>,
        ) -> Result<(), HostNightLightError> {
            *self.values.lock().unwrap() = values.clone();
            Ok(())
        }
    }

    #[test]
    fn gamma_takeover_disables_a_live_native_session_without_losing_its_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let original = BTreeMap::from([
            ("night-light-enabled".into(), "true".into()),
            ("temperature".into(), "4500".into()),
            ("schedule".into(), "sunset".into()),
        ]);
        let controller = Controller::new(
            NativeSettings {
                values: StdMutex::new(original.clone()),
            },
            session_dir(Some(dir.path()), Some("test")),
        );
        controller.apply_native(true, 3500).unwrap();
        assert_eq!(controller.take_over().unwrap(), TakeoverOutcome::Disabled);
        assert!(!controller.settings.get().unwrap());
        assert_eq!(controller.take_over().unwrap(), TakeoverOutcome::AlreadyOff);
        controller.release(RestoreMode::Exit).unwrap();
        assert_eq!(*controller.settings.values.lock().unwrap(), original);
    }

    #[test]
    fn native_session_restores_all_original_settings_after_changes_or_crash() {
        for mode in [RestoreMode::Exit, RestoreMode::Recovery] {
            for enabled in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let original = BTreeMap::from([
                    ("night-light-enabled".into(), enabled.to_string()),
                    ("temperature".into(), "4500".into()),
                    ("schedule".into(), "sunset".into()),
                ]);
                let controller = Controller::new(
                    NativeSettings {
                        values: StdMutex::new(original.clone()),
                    },
                    session_dir(Some(dir.path()), Some("test")),
                );
                controller.apply_native(true, 3500).unwrap();
                controller.apply_native(false, 2500).unwrap();
                assert_ne!(*controller.settings.values.lock().unwrap(), original);
                controller.release(mode).unwrap();
                assert_eq!(
                    *controller.settings.values.lock().unwrap(),
                    original,
                    "mode: {mode:?}, enabled: {enabled}"
                );
                assert!(!controller.is_taken_over());
            }
        }
    }
}
