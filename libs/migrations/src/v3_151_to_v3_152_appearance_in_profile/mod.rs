use anyhow::{Context, Result};
use qol_config::profile::{Scope, SettingsFile, NOTIFICATIONS, THEME};
use qol_conventions::{
    NOTIFICATIONS_SETTINGS_FILE, NOTIFICATION_ENABLED_KEY, NOTIFICATION_OTHER_APPS_KEY,
    THEME_SETTINGS_FILE,
};
use serde_json::{Map, Value};
use std::path::Path;

use crate::fs_util::{list_profile_dirs, write_json_atomic};
use crate::{FileMigration, MigrationReport};

pub(crate) struct V3_151ToV3_152AppearanceInProfile;

const NAME: &str = "v3.151-to-v3.152-appearance-in-profile";

const HANDLER_KEY: &str = "handler";
const LEGACY_SYSTEM_KEY: &str = "use_system_notifications";
const QOL_CHOICE: &str = "qol";

impl FileMigration for V3_151ToV3_152AppearanceInProfile {
    fn name(&self) -> &'static str {
        NAME
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        let legacy = [THEME_SETTINGS_FILE, NOTIFICATIONS_SETTINGS_FILE]
            .iter()
            .any(|file| config_dir.join(file).is_file());
        Ok(legacy && !list_profile_dirs(&config_dir.join("profile"))?.is_empty())
    }

    fn migrate(&self, config_dir: &Path, _archive_dir: &Path) -> Result<MigrationReport> {
        let seeds: [(&SettingsFile, Map<String, Value>); 2] = [
            (&THEME, read_legacy(&config_dir.join(THEME_SETTINGS_FILE))),
            (
                &NOTIFICATIONS,
                seeded_notifications(read_legacy(&config_dir.join(NOTIFICATIONS_SETTINGS_FILE))),
            ),
        ];
        let mut archived = Vec::new();
        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            for (file, settings) in &seeds {
                for scope in Scope::ALL {
                    let part = file.part(settings, scope);
                    let destination = file.path_in(&profile_dir, scope);
                    if part.is_empty() || destination.exists() {
                        continue;
                    }
                    if let Some(parent) = destination.parent() {
                        std::fs::create_dir_all(parent)
                            .with_context(|| format!("creating {}", parent.display()))?;
                    }
                    write_json_atomic(&destination, &Value::Object(part))?;
                    archived.push(destination);
                }
            }
        }
        Ok(MigrationReport {
            name: NAME.to_string(),
            archived,
        })
    }
}

fn read_legacy(path: &Path) -> Map<String, Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Map::new();
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(settings)) => settings,
        _ => Map::new(),
    }
}

fn seeded_notifications(mut settings: Map<String, Value>) -> Map<String, Value> {
    let system = settings.remove(LEGACY_SYSTEM_KEY);
    if !settings.contains_key(HANDLER_KEY) && system == Some(Value::Bool(true)) {
        settings.insert(HANDLER_KEY.to_string(), Value::String("both".to_string()));
    }
    settings.retain(|key, value| !is_unchosen(key, value));
    settings
}

fn is_unchosen(key: &str, value: &Value) -> bool {
    match key {
        NOTIFICATION_ENABLED_KEY => *value == Value::Bool(true),
        HANDLER_KEY | NOTIFICATION_OTHER_APPS_KEY => value.as_str() == Some(QOL_CHOICE),
        _ => value.is_null(),
    }
}

#[cfg(test)]
mod tests;
