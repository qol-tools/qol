use anyhow::{Context, Result};
use qol_config::profile::{Scope, SettingsFile, NOTIFICATIONS, PROFILE_DIR, THEME};
use qol_conventions::{
    LEGACY_MESSAGE_SECONDS_KEY, LEGACY_PINNED_STAY_MESSAGE_SECONDS,
    LEGACY_SYSTEM_NOTIFICATIONS_KEY, NOTIFICATIONS_SETTINGS_FILE, NOTIFICATION_ENABLED_KEY,
    NOTIFICATION_HANDLER_KEY, NOTIFICATION_OTHER_APPS_KEY, THEME_SETTINGS_FILE,
};
use serde_json::{Map, Value};
use std::path::Path;

use crate::fs_util::{list_profile_dirs, read_object, write_json_atomic};
use crate::{FileMigration, MigrationReport};

pub(crate) struct V3_151ToV3_152AppearanceInProfile;

const NAME: &str = "v3.151-to-v3.152-appearance-in-profile";

const QOL_CHOICE: &str = "qol";

impl FileMigration for V3_151ToV3_152AppearanceInProfile {
    fn name(&self) -> &'static str {
        NAME
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        let legacy = [THEME_SETTINGS_FILE, NOTIFICATIONS_SETTINGS_FILE]
            .iter()
            .any(|file| config_dir.join(file).is_file());
        Ok(legacy && !list_profile_dirs(&config_dir.join(PROFILE_DIR))?.is_empty())
    }

    fn migrate(&self, config_dir: &Path, _archive_dir: &Path) -> Result<MigrationReport> {
        let legacy = |file: &str| read_object(&config_dir.join(file)).unwrap_or_default();
        let seeds: [(&SettingsFile, Map<String, Value>); 2] = [
            (&THEME, legacy(THEME_SETTINGS_FILE)),
            (
                &NOTIFICATIONS,
                seeded_notifications(legacy(NOTIFICATIONS_SETTINGS_FILE)),
            ),
        ];
        let mut archived = Vec::new();
        for profile_dir in list_profile_dirs(&config_dir.join(PROFILE_DIR))? {
            for (file, settings) in &seeds {
                for scope in Scope::ALL {
                    let part = file.part(settings, scope);
                    let destination = file.path_in(&profile_dir, scope);
                    if !part.is_empty() && seed_missing_keys(&destination, part)? {
                        archived.push(destination);
                    }
                }
            }
        }
        Ok(MigrationReport {
            name: NAME.to_string(),
            archived,
        })
    }
}

fn seed_missing_keys(destination: &Path, part: Map<String, Value>) -> Result<bool> {
    let mut held = if destination.exists() {
        match read_object(destination) {
            Some(held) => held,
            None => return Ok(false),
        }
    } else {
        Map::new()
    };
    let mut added = false;
    for (key, value) in part {
        if held.get(&key).is_none_or(Value::is_null) {
            held.insert(key, value);
            added = true;
        }
    }
    if !added {
        return Ok(false);
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    write_json_atomic(destination, &Value::Object(held))?;
    Ok(true)
}

fn seeded_notifications(mut settings: Map<String, Value>) -> Map<String, Value> {
    let system = settings.remove(LEGACY_SYSTEM_NOTIFICATIONS_KEY);
    if !settings.contains_key(NOTIFICATION_HANDLER_KEY) && system == Some(Value::Bool(true)) {
        settings.insert(
            NOTIFICATION_HANDLER_KEY.to_string(),
            Value::String("both".to_string()),
        );
    }
    settings.retain(|key, value| !is_unchosen(key, value));
    settings
}

fn is_unchosen(key: &str, value: &Value) -> bool {
    match key {
        NOTIFICATION_ENABLED_KEY => *value == Value::Bool(true),
        NOTIFICATION_HANDLER_KEY | NOTIFICATION_OTHER_APPS_KEY => {
            value.as_str() == Some(QOL_CHOICE)
        }
        LEGACY_MESSAGE_SECONDS_KEY => {
            value.is_null() || value.as_f64() == Some(LEGACY_PINNED_STAY_MESSAGE_SECONDS)
        }
        _ => value.is_null(),
    }
}

#[cfg(test)]
mod tests;
