pub(crate) mod mirror;

use anyhow::{Context, Result};
use qol_config::placement::Spot;
use qol_plugin_daemon::notification::gate::NativeHandler;
use serde::{Deserialize, Serialize};

pub use mirror::MirrorAccess;

#[derive(Clone, Debug, Serialize)]
struct NotificationSettings {
    #[serde(default)]
    use_system_notifications: bool,
    handler: NativeHandler,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_stay_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    placement: Option<String>,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            use_system_notifications: false,
            handler: NativeHandler::Qol,
            message_stay_seconds: None,
            placement: None,
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
            #[serde(default)]
            message_seconds: Option<f64>,
            #[serde(default)]
            message_stay_seconds: Option<f64>,
            #[serde(default)]
            placement: Option<serde_json::Value>,
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
            message_stay_seconds: qol_theme::stored_message_stay(
                raw.message_stay_seconds,
                raw.message_seconds,
            ),
            placement: raw
                .placement
                .as_ref()
                .and_then(serde_json::Value::as_str)
                .and_then(Spot::parse)
                .map(Spot::token),
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
    update_settings(|settings| {
        settings.use_system_notifications = handler == NativeHandler::Both;
        settings.handler = handler;
    })
}

pub fn message_seconds() -> f64 {
    qol_theme::message_stay(
        settings()
            .ok()
            .and_then(|settings| settings.message_stay_seconds),
    )
    .as_secs_f64()
}

pub fn set_message_seconds(seconds: f64) -> Result<()> {
    if seconds == message_seconds() {
        return Ok(());
    }
    update_settings(|settings| settings.message_stay_seconds = Some(seconds))
}

pub fn placement() -> Spot {
    settings()
        .ok()
        .and_then(|settings| settings.placement)
        .and_then(|token| Spot::parse(&token))
        .unwrap_or(Spot::TOP_RIGHT)
}

pub fn set_placement(spot: Spot) -> Result<()> {
    if spot == placement() {
        return Ok(());
    }
    update_settings(|settings| settings.placement = Some(spot.token()))
}

pub fn apply_native_handler(previous: NativeHandler) {
    if previous == NativeHandler::Qol && native_handler() != NativeHandler::Qol {
        qol_plugin_daemon::notification::platform::set_os_banners(true);
    }
    sync_notification_inhibit();
}

static NOTIFICATION_INHIBIT: std::sync::Mutex<
    Option<qol_plugin_daemon::notification::platform::NotificationInhibit>,
> = std::sync::Mutex::new(None);

pub fn sync_notification_inhibit() {
    let mut held = NOTIFICATION_INHIBIT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let qol_only = native_handler() == NativeHandler::Qol;
    mirror::sync(qol_only);
    if qol_only {
        qol_plugin_daemon::notification::platform::set_os_banners(false);
        if held.is_none() {
            *held = qol_plugin_daemon::notification::platform::acquire_inhibit();
        }
    } else {
        *held = None;
    }
}

pub fn mirror_access() -> MirrorAccess {
    let access = mirror::access();
    if matches!(access, MirrorAccess::Readable) {
        sync_notification_inhibit();
    }
    access
}

fn settings() -> Result<NotificationSettings> {
    let path = settings_path()?;
    crate::file_io::load_json_or_default(&path).context("failed to load notification settings")
}

fn settings_path() -> Result<std::path::PathBuf> {
    crate::paths::shared_config_dir()
        .map(|dir| dir.join(qol_conventions::NOTIFICATIONS_SETTINGS_FILE))
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
    fn message_time_defaults_to_the_theme_and_survives_a_handler_change() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert_eq!(message_seconds(), qol_theme::STAY_MESSAGE.as_secs_f64());
        set_message_seconds(2.5).unwrap();
        set_native_handler(NativeHandler::Os).unwrap();
        assert_eq!(message_seconds(), 2.5);
        assert_eq!(native_handler(), NativeHandler::Os);
    }

    #[test]
    fn saving_the_shown_message_time_keeps_following_the_default() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        set_message_seconds(qol_theme::STAY_MESSAGE.as_secs_f64()).unwrap();
        assert_eq!(settings().unwrap().message_stay_seconds, None);
        let path = settings_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"message_seconds": 3}"#).unwrap();
        assert_eq!(
            message_seconds(),
            qol_theme::STAY_MESSAGE.as_secs_f64(),
            "a whole-form save pinned the old 3 s default, so it counts as unset"
        );
        set_message_seconds(3.0).unwrap();
        assert_eq!(message_seconds(), 3.0, "an explicit 3 s pick is kept");
        std::fs::write(&path, r#"{"message_seconds": 5}"#).unwrap();
        assert_eq!(
            message_seconds(),
            5.0,
            "a deliberate older pick carries over"
        );
    }

    #[test]
    fn placement_defaults_to_the_top_right_and_survives_other_saves() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert_eq!(placement(), Spot::TOP_RIGHT);
        set_placement(Spot::TOP_RIGHT).unwrap();
        assert_eq!(
            settings().unwrap().placement,
            None,
            "saving the shown default keeps following the default"
        );
        let free = Spot::new(0.25, 0.6);
        set_placement(free).unwrap();
        set_native_handler(NativeHandler::Both).unwrap();
        set_message_seconds(4.0).unwrap();
        assert_eq!(placement(), free);
        let saved = std::fs::read_to_string(settings_path().unwrap()).unwrap();
        assert!(saved.contains(r#""placement": "0.25,0.6""#), "{saved}");
    }

    #[test]
    fn a_placement_the_toasts_cannot_read_falls_back_to_the_top_right() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        let path = settings_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for stored in [r#""somewhere""#, "7", r#""2,2""#] {
            std::fs::write(&path, format!(r#"{{"placement": {stored}}}"#)).unwrap();
            assert_eq!(placement(), Spot::TOP_RIGHT, "{stored}");
            assert_eq!(settings().unwrap().placement, None, "{stored}");
        }
        std::fs::write(&path, r#"{"placement": "bottom-center"}"#).unwrap();
        assert_eq!(placement(), Spot::new(0.5, 1.0));
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
