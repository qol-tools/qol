use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Core,
    Os,
    Device,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::Core, Scope::Os, Scope::Device];

    fn dir(self, profile_dir: &Path) -> PathBuf {
        match self {
            Scope::Core => profile_dir.join(super::CORE_SUBDIR),
            Scope::Os => profile_dir
                .join(super::OS_SUBDIR)
                .join(qol_platform::os::current_name()),
            Scope::Device => profile_dir.join(super::DEVICE_SUBDIR),
        }
    }
}

pub struct SettingsFile {
    name: &'static str,
    os_keys: &'static [&'static str],
    device_keys: &'static [&'static str],
}

pub const THEME: SettingsFile = SettingsFile {
    name: qol_conventions::THEME_SETTINGS_FILE,
    os_keys: &[],
    device_keys: &[],
};

pub const NOTIFICATIONS: SettingsFile = SettingsFile {
    name: qol_conventions::NOTIFICATIONS_SETTINGS_FILE,
    os_keys: &[qol_conventions::NOTIFICATION_OTHER_APPS_KEY],
    device_keys: &[qol_conventions::NOTIFICATION_ENABLED_KEY],
};

impl SettingsFile {
    pub fn scope_of(&self, key: &str) -> Scope {
        if self.device_keys.contains(&key) {
            Scope::Device
        } else if self.os_keys.contains(&key) {
            Scope::Os
        } else {
            Scope::Core
        }
    }

    pub fn path_in(&self, profile_dir: &Path, scope: Scope) -> PathBuf {
        scope.dir(profile_dir).join(self.name)
    }

    pub fn read(&self, config_dir: &Path) -> Map<String, Value> {
        self.read_in(&active_profile_dir_in(config_dir))
    }

    pub fn read_in(&self, profile_dir: &Path) -> Map<String, Value> {
        let mut settings = Map::new();
        for scope in Scope::ALL {
            let Some(stored) = read_object(&self.path_in(profile_dir, scope)) else {
                continue;
            };
            settings.extend(
                stored
                    .into_iter()
                    .filter(|(key, _)| self.scope_of(key) == scope),
            );
        }
        settings
    }

    pub fn write(&self, config_dir: &Path, settings: &Map<String, Value>) -> std::io::Result<()> {
        self.write_in(&active_profile_dir_in(config_dir), settings)
    }

    pub fn write_in(
        &self,
        profile_dir: &Path,
        settings: &Map<String, Value>,
    ) -> std::io::Result<()> {
        for scope in Scope::ALL {
            write_part(
                &self.path_in(profile_dir, scope),
                &self.part(settings, scope),
            )?;
        }
        Ok(())
    }

    pub fn part(&self, settings: &Map<String, Value>, scope: Scope) -> Map<String, Value> {
        settings
            .iter()
            .filter(|(key, value)| self.scope_of(key) == scope && !value.is_null())
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}

fn active_profile_dir_in(config_dir: &Path) -> PathBuf {
    super::active_profile_dir(&config_dir.join(super::PROFILE_DIR))
}

fn read_object(path: &Path) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(object)) => Some(object),
        _ => None,
    }
}

fn write_part(path: &Path, part: &Map<String, Value>) -> std::io::Result<()> {
    if part.is_empty() {
        return match std::fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(part).map_err(std::io::Error::other)?;
    text.push('\n');
    qol_fs::atomic_write(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(object) => object,
            other => panic!("not an object: {other}"),
        }
    }

    fn stored(path: &Path) -> Option<Value> {
        let text = std::fs::read_to_string(path).ok()?;
        Some(serde_json::from_str(&text).unwrap())
    }

    #[test]
    fn the_scopes_are_the_profile_folders_sync_knows() {
        let profile = Path::new("/nowhere/profile/default");
        let os = profile.join("os").join(qol_platform::os::current_name());
        assert_eq!(
            THEME.path_in(profile, Scope::Core),
            profile.join("core").join("theme.json")
        );
        assert_eq!(
            NOTIFICATIONS.path_in(profile, Scope::Os),
            os.join("notifications.json")
        );
        assert_eq!(
            NOTIFICATIONS.path_in(profile, Scope::Device),
            profile.join("device").join("notifications.json")
        );
    }

    #[test]
    fn each_key_lands_in_its_scope_and_reads_back_as_one_object() {
        let dir = tempfile::tempdir().unwrap();
        let settings = object(json!({
            "handler": "both",
            "placement": "top-left",
            "other_apps": "os",
            "enabled": false,
        }));

        NOTIFICATIONS.write_in(dir.path(), &settings).unwrap();

        assert_eq!(
            stored(&NOTIFICATIONS.path_in(dir.path(), Scope::Core)),
            Some(json!({ "handler": "both", "placement": "top-left" }))
        );
        assert_eq!(
            stored(&NOTIFICATIONS.path_in(dir.path(), Scope::Os)),
            Some(json!({ "other_apps": "os" }))
        );
        assert_eq!(
            stored(&NOTIFICATIONS.path_in(dir.path(), Scope::Device)),
            Some(json!({ "enabled": false }))
        );
        assert_eq!(NOTIFICATIONS.read_in(dir.path()), settings);
    }

    #[test]
    fn a_key_left_in_another_scope_is_ignored_and_dropped_on_the_next_write() {
        let dir = tempfile::tempdir().unwrap();
        let core = NOTIFICATIONS.path_in(dir.path(), Scope::Core);
        std::fs::create_dir_all(core.parent().unwrap()).unwrap();
        std::fs::write(&core, r#"{"enabled": false, "handler": "os"}"#).unwrap();

        let read = NOTIFICATIONS.read_in(dir.path());
        assert_eq!(read, object(json!({ "handler": "os" })));

        NOTIFICATIONS.write_in(dir.path(), &read).unwrap();
        assert_eq!(stored(&core), Some(json!({ "handler": "os" })));
        assert!(!NOTIFICATIONS.path_in(dir.path(), Scope::Device).exists());
    }

    #[test]
    fn unreadable_files_read_as_empty_and_an_empty_scope_keeps_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let core = THEME.path_in(dir.path(), Scope::Core);
        assert!(THEME.read_in(dir.path()).is_empty());
        std::fs::create_dir_all(core.parent().unwrap()).unwrap();
        for text in ["not json", "[1, 2]", "\"slate\"", ""] {
            std::fs::write(&core, text).unwrap();
            assert!(THEME.read_in(dir.path()).is_empty(), "{text:?}");
        }

        THEME
            .write_in(
                dir.path(),
                &object(json!({ "accent": "blue", "theme": null })),
            )
            .unwrap();
        assert_eq!(stored(&core), Some(json!({ "accent": "blue" })));

        THEME
            .write_in(dir.path(), &object(json!({ "accent": null })))
            .unwrap();
        assert!(!core.exists());
    }

    #[test]
    fn the_config_dir_reads_and_writes_the_active_profile() {
        let config = tempfile::tempdir().unwrap();
        let root = config.path().join(super::super::PROFILE_DIR);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(super::super::ACTIVE_PROFILE_FILE), "work\n").unwrap();

        THEME
            .write(config.path(), &object(json!({ "accent": "green" })))
            .unwrap();

        assert_eq!(
            stored(&root.join("work").join("core").join("theme.json")),
            Some(json!({ "accent": "green" }))
        );
        assert_eq!(
            THEME.read(config.path()),
            object(json!({ "accent": "green" }))
        );
        assert!(THEME.read_in(&root.join("default")).is_empty());
    }
}
