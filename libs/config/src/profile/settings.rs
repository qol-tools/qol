use serde_json::{Map, Value};
use std::io;
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
    pub fn name(&self) -> &'static str {
        self.name
    }

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
        let stored = Scope::ALL.map(|scope| {
            let object = load_object(&self.path_in(profile_dir, scope)).unwrap_or_default();
            (scope, object)
        });
        self.owned(&stored)
    }

    pub fn update(
        &self,
        config_dir: &Path,
        update: impl FnOnce(&mut Map<String, Value>),
    ) -> io::Result<()> {
        self.update_in(&active_profile_dir_in(config_dir), update)
    }

    pub fn check_updatable(&self, config_dir: &Path) -> io::Result<()> {
        self.load_scopes(&active_profile_dir_in(config_dir))
            .map(drop)
    }

    pub fn update_in(
        &self,
        profile_dir: &Path,
        update: impl FnOnce(&mut Map<String, Value>),
    ) -> io::Result<()> {
        let stored = self.load_scopes(profile_dir)?;
        let mut settings = self.owned(&stored);
        update(&mut settings);
        for (scope, object) in stored {
            let mut part = self.part(&settings, scope);
            if scope != Scope::Core {
                part.extend(
                    object
                        .into_iter()
                        .filter(|(key, _)| self.scope_of(key) == Scope::Core),
                );
            }
            write_part(&self.path_in(profile_dir, scope), &part)?;
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

    fn load_scopes(&self, profile_dir: &Path) -> io::Result<Vec<(Scope, Map<String, Value>)>> {
        Scope::ALL
            .into_iter()
            .map(|scope| {
                load_object(&self.path_in(profile_dir, scope)).map(|object| (scope, object))
            })
            .collect()
    }

    fn owned(&self, stored: &[(Scope, Map<String, Value>)]) -> Map<String, Value> {
        stored
            .iter()
            .flat_map(|(scope, object)| {
                object
                    .iter()
                    .filter(move |(key, _)| self.scope_of(key) == *scope)
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}

fn active_profile_dir_in(config_dir: &Path) -> PathBuf {
    super::active_profile_dir(&config_dir.join(super::PROFILE_DIR))
}

fn load_object(path: &Path) -> io::Result<Map<String, Value>> {
    let text = match std::fs::read_to_string(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        read => read?,
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} does not hold a JSON object", path.display()),
        )),
    }
}

fn write_part(path: &Path, part: &Map<String, Value>) -> io::Result<()> {
    if part.is_empty() {
        return match std::fs::remove_file(path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(part).map_err(io::Error::other)?;
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

        NOTIFICATIONS
            .update_in(dir.path(), |stored| *stored = settings.clone())
            .unwrap();

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

        assert_eq!(
            NOTIFICATIONS.read_in(dir.path()),
            object(json!({ "handler": "os" }))
        );

        NOTIFICATIONS.update_in(dir.path(), |_| {}).unwrap();
        assert_eq!(stored(&core), Some(json!({ "handler": "os" })));
        assert!(!NOTIFICATIONS.path_in(dir.path(), Scope::Device).exists());
    }

    #[test]
    fn a_key_a_newer_version_keeps_on_this_computer_survives_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let files = [
            (
                &NOTIFICATIONS,
                Scope::Os,
                json!({ "other_apps": "os", "sound": "chime" }),
            ),
            (
                &NOTIFICATIONS,
                Scope::Device,
                json!({ "enabled": false, "focus": true }),
            ),
            (&THEME, Scope::Os, json!({ "font_scale": 1.25 })),
            (&THEME, Scope::Device, json!({ "corner_radius": 4 })),
        ];
        for (file, scope, contents) in &files {
            let path = file.path_in(dir.path(), *scope);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents.to_string()).unwrap();
        }

        assert_eq!(
            NOTIFICATIONS.read_in(dir.path()),
            object(json!({ "other_apps": "os", "enabled": false }))
        );
        assert!(THEME.read_in(dir.path()).is_empty());

        NOTIFICATIONS
            .update_in(dir.path(), |settings| {
                settings.insert("handler".to_string(), json!("both"));
            })
            .unwrap();
        THEME
            .update_in(dir.path(), |settings| {
                settings.insert("accent".to_string(), json!("blue"));
            })
            .unwrap();

        for (file, scope, contents) in &files {
            assert_eq!(
                stored(&file.path_in(dir.path(), *scope)).as_ref(),
                Some(contents),
                "{scope:?}"
            );
        }

        NOTIFICATIONS
            .update_in(dir.path(), |settings| {
                settings.remove("other_apps");
            })
            .unwrap();
        assert_eq!(
            stored(&NOTIFICATIONS.path_in(dir.path(), Scope::Os)),
            Some(json!({ "sound": "chime" }))
        );
    }

    #[test]
    fn unreadable_files_read_as_empty_and_a_save_leaves_them_for_repair() {
        let dir = tempfile::tempdir().unwrap();
        for scope in Scope::ALL {
            let path = NOTIFICATIONS.path_in(dir.path(), scope);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            for text in ["not json", "[1, 2]", "\"slate\"", ""] {
                std::fs::write(&path, text).unwrap();
                assert!(NOTIFICATIONS.read_in(dir.path()).is_empty(), "{text:?}");

                let saved = NOTIFICATIONS.update_in(dir.path(), |settings| {
                    settings.insert("handler".to_string(), json!("os"));
                });

                assert!(saved.is_err(), "{scope:?} {text:?}");
                assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
            }
            std::fs::remove_file(&path).unwrap();
        }
    }

    #[test]
    fn an_unreadable_scope_file_is_found_before_an_update_is_tried() {
        let config = tempfile::tempdir().unwrap();
        let profile = config
            .path()
            .join(super::super::PROFILE_DIR)
            .join(super::super::DEFAULT_PROFILE_NAME);
        assert!(NOTIFICATIONS.check_updatable(config.path()).is_ok());

        for scope in Scope::ALL {
            let path = NOTIFICATIONS.path_in(&profile, scope);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "{}").unwrap();
            assert!(
                NOTIFICATIONS.check_updatable(config.path()).is_ok(),
                "{scope:?}"
            );

            std::fs::write(&path, "").unwrap();
            assert!(
                NOTIFICATIONS.check_updatable(config.path()).is_err(),
                "{scope:?}"
            );
            assert!(THEME.check_updatable(config.path()).is_ok(), "{scope:?}");
            std::fs::remove_file(&path).unwrap();
        }
    }

    #[test]
    fn a_cleared_choice_keeps_no_file_for_an_empty_scope() {
        let dir = tempfile::tempdir().unwrap();
        let core = THEME.path_in(dir.path(), Scope::Core);

        THEME
            .update_in(dir.path(), |settings| {
                *settings = object(json!({ "accent": "blue", "theme": null }));
            })
            .unwrap();
        assert_eq!(stored(&core), Some(json!({ "accent": "blue" })));

        THEME
            .update_in(dir.path(), |settings| {
                settings.insert("accent".to_string(), Value::Null);
            })
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
            .update(config.path(), |settings| {
                settings.insert("accent".to_string(), json!("green"));
            })
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
