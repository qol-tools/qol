use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use super::{HostNightLight, HostNightLightError, HostNightLightStatus, TakeoverOutcome};
use crate::session::{RestoreMode, EVICTION_GENERATION};

const SCHEMA: &str = "org.cinnamon.settings-daemon.plugins.color";
const KEY: &str = "night-light-enabled";
const SNAPSHOT_ID: &str = "night-light-enabled";
const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

trait Settings: Send + Sync {
    fn get(&self) -> Result<bool, HostNightLightError>;
    fn set(&self, enabled: bool) -> Result<(), HostNightLightError>;
}

struct Gsettings;

impl Settings for Gsettings {
    fn get(&self) -> Result<bool, HostNightLightError> {
        let output = Command::new("gsettings")
            .args(["get", SCHEMA, KEY])
            .output()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    HostNightLightError::Unsupported("gsettings is not installed".to_string())
                } else {
                    HostNightLightError::Failed(format!("failed to run gsettings: {error}"))
                }
            })?;
        if !output.status.success() {
            return Err(HostNightLightError::Failed(format!(
                "gsettings get {SCHEMA} {KEY} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        parse_bool(String::from_utf8_lossy(&output.stdout).trim()).ok_or_else(|| {
            HostNightLightError::Failed(format!(
                "gsettings returned an invalid boolean for {SCHEMA} {KEY}"
            ))
        })
    }

    fn set(&self, enabled: bool) -> Result<(), HostNightLightError> {
        let value = if enabled { "true" } else { "false" };
        let status = Command::new("gsettings")
            .args(["set", SCHEMA, KEY, value])
            .status()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    HostNightLightError::Unsupported("gsettings is not installed".to_string())
                } else {
                    HostNightLightError::Failed(format!("failed to run gsettings: {error}"))
                }
            })?;
        if !status.success() {
            return Err(HostNightLightError::Failed(format!(
                "gsettings set {SCHEMA} {KEY} {value} failed"
            )));
        }
        Ok(())
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim_matches('\'') {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Snapshot {
    schema_version: u32,
    id: String,
    enabled: bool,
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

struct CinnamonNightLight<S: Settings = Gsettings> {
    settings: S,
    store: qol_host_session::SessionStore,
    state: Mutex<State>,
    adoption_generation: Option<String>,
}

impl CinnamonNightLight<Gsettings> {
    fn system(config_root: Option<&Path>) -> Self {
        let dir = config_root
            .and_then(|root| crate::config::device_dir(root).ok())
            .map(|dir| dir.join("host-night-light"))
            .unwrap_or_else(fallback_dir);
        Self::new(Gsettings, dir)
    }
}

pub(super) fn control(config_root: Option<&Path>) -> Arc<dyn HostNightLight> {
    Arc::new(CinnamonNightLight::system(config_root))
}

impl<S: Settings> CinnamonNightLight<S> {
    fn new(settings: S, dir: PathBuf) -> Self {
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

    fn fail(&self, error: HostNightLightError) -> HostNightLightError {
        let status = match error {
            HostNightLightError::Unsupported(_) => HostNightLightStatus::Unsupported,
            HostNightLightError::Failed(_) => HostNightLightStatus::Failed,
        };
        self.set_state(false, status);
        error
    }
}

impl<S: Settings> HostNightLight for CinnamonNightLight<S> {
    fn take_over(&self) -> Result<TakeoverOutcome, HostNightLightError> {
        if self.is_taken_over() {
            return Ok(match self.status() {
                HostNightLightStatus::TakenOver => TakeoverOutcome::Disabled,
                _ => TakeoverOutcome::AlreadyOff,
            });
        }
        if let Some(snapshot) = self.snapshot().map_err(|error| self.fail(error))? {
            if snapshot.clean {
                self.store
                    .delete(SNAPSHOT_ID)
                    .map_err(|error| self.fail(HostNightLightError::Failed(error.to_string())))?;
            } else {
                let status = if snapshot.enabled {
                    HostNightLightStatus::TakenOver
                } else {
                    HostNightLightStatus::Off
                };
                self.set_state(true, status);
                return Ok(if snapshot.enabled {
                    TakeoverOutcome::Disabled
                } else {
                    TakeoverOutcome::AlreadyOff
                });
            }
        }
        let enabled = self.settings.get().map_err(|error| self.fail(error))?;
        let snapshot = Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            id: SNAPSHOT_ID.to_string(),
            enabled,
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
        if !snapshot.clean && snapshot.mutations > 0 && snapshot.enabled {
            self.settings.set(true).map_err(|error| self.fail(error))?;
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

fn fallback_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("qol-monitor-host-night-light");
    let _ = qol_fs::create_private_dir(&dir);
    dir
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
        fn get(&self) -> Result<bool, HostNightLightError> {
            Ok(*self.value.lock().unwrap())
        }

        fn set(&self, enabled: bool) -> Result<(), HostNightLightError> {
            *self.value.lock().unwrap() = enabled;
            self.writes.lock().unwrap().push(enabled);
            Ok(())
        }
    }

    fn controller(enabled: bool) -> (tempfile::TempDir, CinnamonNightLight<FakeSettings>) {
        let dir = tempfile::tempdir().unwrap();
        let controller = CinnamonNightLight::new(
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
        assert_eq!(controller.take_over().unwrap(), TakeoverOutcome::Disabled);
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
        let second = CinnamonNightLight::new(
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
        let second = CinnamonNightLight::new(
            FakeSettings::new(false),
            dir.path().join("host-night-light"),
        )
        .with_adoption_generation(Some("next".to_string()));
        second.release(RestoreMode::Recovery).unwrap();
        assert!(second.is_taken_over());
        assert!(second.settings.writes().is_empty());
    }

    #[test]
    fn gsettings_boolean_parser_is_strict() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("'false'"), Some(false));
        assert_eq!(parse_bool("enabled"), None);
    }
}
