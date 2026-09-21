use anyhow::{Context, Result};
use qol_plugin_daemon::notification::gate::NativeHandler;
use serde::{Deserialize, Serialize};

const NOTIFICATIONS_SETTINGS_FILE: &str = "notifications.json";

#[derive(Clone, Debug, Serialize)]
struct NotificationSettings {
    #[serde(default)]
    use_system_notifications: bool,
    handler: NativeHandler,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            use_system_notifications: false,
            handler: NativeHandler::Qol,
        }
    }
}

impl<'de> Deserialize<'de> for NotificationSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            use_system_notifications: bool,
            #[serde(default)]
            handler: Option<NativeHandler>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let handler = raw.handler.unwrap_or(if raw.use_system_notifications {
            NativeHandler::Both
        } else {
            NativeHandler::Qol
        });
        Ok(Self {
            use_system_notifications: raw.use_system_notifications,
            handler,
        })
    }
}

fn update_settings(update: impl FnOnce(&mut NotificationSettings)) -> Result<()> {
    let path = settings_path()?;
    let mut settings: NotificationSettings = crate::file_io::load_json_or_default(&path)
        .context("failed to load notification settings")?;
    update(&mut settings);
    crate::file_io::write_pretty_json(&path, &settings)
        .context("failed to save notification settings")
}

pub fn set_use_system_notifications(enabled: bool) -> Result<()> {
    update_settings(|settings| {
        settings.use_system_notifications = enabled;
        settings.handler = if enabled {
            NativeHandler::Both
        } else {
            NativeHandler::Qol
        };
    })
}

pub fn use_system_notifications() -> bool {
    settings()
        .map(|settings| settings.use_system_notifications)
        .unwrap_or(false)
}

pub fn native_handler() -> NativeHandler {
    settings()
        .map(|settings| settings.handler)
        .unwrap_or(NativeHandler::Qol)
}

pub fn set_native_handler(handler: NativeHandler) -> Result<()> {
    let result = update_settings(|settings| {
        settings.use_system_notifications = handler == NativeHandler::Both;
        settings.handler = handler;
    });
    sync_notification_inhibit();
    result
}

#[cfg(target_os = "linux")]
static NOTIFICATION_INHIBIT: std::sync::Mutex<
    Option<qol_plugin_daemon::notification::platform::NotificationInhibit>,
> = std::sync::Mutex::new(None);

pub fn sync_notification_inhibit() {
    #[cfg(target_os = "linux")]
    {
        let mut held = NOTIFICATION_INHIBIT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if native_handler() == NativeHandler::Qol {
            if held.is_none() {
                *held = qol_plugin_daemon::notification::platform::acquire_inhibit();
            }
        } else {
            *held = None;
        }
    }
}

fn settings() -> Result<NotificationSettings> {
    let path = settings_path()?;
    crate::file_io::load_json_or_default(&path).context("failed to load notification settings")
}

fn settings_path() -> Result<std::path::PathBuf> {
    crate::paths::shared_config_dir().map(|dir| dir.join(NOTIFICATIONS_SETTINGS_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_legacy_file(use_system_notifications: bool) {
        let path = settings_path().unwrap();
        let json = serde_json::json!({ "use_system_notifications": use_system_notifications });
        crate::file_io::write_pretty_json(&path, &json).unwrap();
    }

    #[test]
    fn system_notification_choice_round_trips_and_defaults_to_toast() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert!(!use_system_notifications());
        assert_eq!(native_handler(), NativeHandler::Qol);
        set_use_system_notifications(true).unwrap();
        assert!(use_system_notifications());
        assert_eq!(native_handler(), NativeHandler::Both);
        set_use_system_notifications(false).unwrap();
        assert!(!use_system_notifications());
        assert_eq!(native_handler(), NativeHandler::Qol);
    }

    #[test]
    fn handler_choice_round_trips_and_mirrors_the_legacy_bool() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        set_native_handler(NativeHandler::Os).unwrap();
        assert_eq!(native_handler(), NativeHandler::Os);
        assert!(!use_system_notifications());
        set_native_handler(NativeHandler::Both).unwrap();
        assert_eq!(native_handler(), NativeHandler::Both);
        assert!(use_system_notifications());
        set_native_handler(NativeHandler::Qol).unwrap();
        assert_eq!(native_handler(), NativeHandler::Qol);
        assert!(!use_system_notifications());
    }

    #[test]
    fn legacy_file_maps_the_bool_to_a_handler() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        write_legacy_file(true);
        assert_eq!(native_handler(), NativeHandler::Both);
        assert!(use_system_notifications());

        write_legacy_file(false);
        assert_eq!(native_handler(), NativeHandler::Qol);
        assert!(!use_system_notifications());
    }

    #[test]
    fn persisted_handler_key_wins_over_the_legacy_bool() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        set_native_handler(NativeHandler::Os).unwrap();
        assert_eq!(native_handler(), NativeHandler::Os);
        assert!(!use_system_notifications());
        set_use_system_notifications(true).unwrap();
        assert_eq!(native_handler(), NativeHandler::Both);
        set_native_handler(NativeHandler::Qol).unwrap();
        assert!(!use_system_notifications());
    }
}
