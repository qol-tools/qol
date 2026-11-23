pub mod menu;
pub(crate) mod mirror;
pub mod takeover;

use anyhow::{Context, Result};
use qol_config::placement::Spot;
use qol_config::profile::{Scope, NOTIFICATIONS};
use qol_conventions::{
    LEGACY_MESSAGE_SECONDS_KEY, MESSAGE_STAY_KEY, NOTIFICATION_ENABLED_KEY,
    NOTIFICATION_OTHER_APPS_KEY, NOTIFICATION_PLACEMENT_KEY,
};
use qol_plugin_daemon::notification::gate::NativeHandler;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const HANDLER_KEY: &str = "handler";
const LEGACY_SYSTEM_NOTIFICATIONS_KEY: &str = "use_system_notifications";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OtherApps {
    Os,
    Qol,
}

#[derive(Clone, Debug)]
struct NotificationSettings {
    enabled: bool,
    handler: NativeHandler,
    other_apps: OtherApps,
    message_stay_seconds: Option<f64>,
    placement: Option<String>,
}

impl<'de> Deserialize<'de> for NotificationSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            enabled: Option<bool>,
            #[serde(default)]
            use_system_notifications: bool,
            #[serde(default)]
            handler: Option<NativeHandler>,
            #[serde(default)]
            other_apps: Option<OtherApps>,
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
            enabled: raw.enabled.unwrap_or(true),
            handler,
            other_apps: raw.other_apps.unwrap_or(OtherApps::Qol),
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

fn stored_settings() -> Result<Map<String, Value>> {
    Ok(NOTIFICATIONS.read(&crate::paths::shared_config_dir()?))
}

fn update_settings(update: impl FnOnce(&mut Map<String, Value>)) -> Result<()> {
    let config_dir = crate::paths::shared_config_dir()?;
    {
        let _guard = crate::plugins::config::profile_config_write_guard_unmarked();
        let mut settings = NOTIFICATIONS.read(&config_dir);
        update(&mut settings);
        NOTIFICATIONS
            .write(&config_dir, &settings)
            .context("failed to save notification settings")?;
    }
    takeover::poke();
    Ok(())
}

fn save_value(key: &str, value: Value, replaces: &[&str]) -> Result<()> {
    update_settings(|settings| {
        for legacy in replaces {
            settings.remove(*legacy);
        }
        settings.insert(key.to_string(), value);
    })
}

fn choice_value(choice: impl Serialize) -> Result<Value> {
    serde_json::to_value(choice).context("failed to encode a notification choice")
}

pub(crate) fn exported_settings() -> Map<String, Value> {
    let mut settings = stored_settings().unwrap_or_default();
    settings.retain(|key, _| NOTIFICATIONS.scope_of(key) != Scope::Device);
    settings
}

pub(crate) fn validate_import(imported: &Map<String, Value>) -> Result<()> {
    serde_json::from_value::<NotificationSettings>(Value::Object(imported.clone()))
        .map(drop)
        .context("the imported notification settings do not parse")
}

pub(crate) fn import_settings(imported: &Map<String, Value>) -> Result<()> {
    validate_import(imported)?;
    update_settings(|settings| {
        settings.retain(|key, _| NOTIFICATIONS.scope_of(key) == Scope::Device);
        settings.extend(
            imported
                .iter()
                .filter(|(key, _)| NOTIFICATIONS.scope_of(key) != Scope::Device)
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    })
}

pub fn enabled() -> bool {
    settings().map(|settings| settings.enabled).unwrap_or(true)
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    save_value(NOTIFICATION_ENABLED_KEY, Value::Bool(enabled), &[])
}

pub fn set_use_system_notifications(enabled: bool) -> Result<()> {
    set_native_handler(if enabled {
        NativeHandler::Both
    } else {
        NativeHandler::Qol
    })
}

pub fn use_system_notifications() -> bool {
    native_handler() == NativeHandler::Both
}

pub fn native_handler() -> NativeHandler {
    settings()
        .map(|settings| settings.handler)
        .unwrap_or(NativeHandler::Qol)
}

pub fn set_native_handler(handler: NativeHandler) -> Result<()> {
    save_value(
        HANDLER_KEY,
        choice_value(handler)?,
        &[LEGACY_SYSTEM_NOTIFICATIONS_KEY],
    )
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
    save_value(
        MESSAGE_STAY_KEY,
        choice_value(seconds)?,
        &[LEGACY_MESSAGE_SECONDS_KEY],
    )
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
    save_value(NOTIFICATION_PLACEMENT_KEY, Value::String(spot.token()), &[])
}

pub fn other_apps() -> OtherApps {
    settings()
        .map(|settings| settings.other_apps)
        .unwrap_or(OtherApps::Qol)
}

pub fn set_other_apps(other_apps: OtherApps) -> Result<()> {
    save_value(NOTIFICATION_OTHER_APPS_KEY, choice_value(other_apps)?, &[])
}

pub fn takeover_wanted() -> bool {
    native_handler() == NativeHandler::Qol && other_apps() == OtherApps::Qol
}

fn settings() -> Result<NotificationSettings> {
    serde_json::from_value(Value::Object(stored_settings()?))
        .context("failed to load notification settings")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn slice_file(scope: Scope) -> std::path::PathBuf {
        let profile_root = crate::paths::profile_dir().unwrap();
        NOTIFICATIONS.path_in(
            &qol_config::profile::active_profile_dir(&profile_root),
            scope,
        )
    }

    fn write_core(contents: &str) {
        let path = slice_file(Scope::Core);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
    }

    fn stored_slice(scope: Scope) -> Option<Value> {
        let text = std::fs::read_to_string(slice_file(scope)).ok()?;
        Some(serde_json::from_str(&text).unwrap())
    }

    fn write_legacy_file(use_system_notifications: bool) {
        write_core(&json!({ "use_system_notifications": use_system_notifications }).to_string());
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
        write_core(r#"{"message_seconds": 3}"#);
        assert_eq!(
            message_seconds(),
            qol_theme::STAY_MESSAGE.as_secs_f64(),
            "a whole-form save pinned the old 3 s default, so it counts as unset"
        );
        set_message_seconds(3.0).unwrap();
        assert_eq!(message_seconds(), 3.0, "an explicit 3 s pick is kept");
        write_core(r#"{"message_seconds": 5}"#);
        assert_eq!(
            message_seconds(),
            5.0,
            "a deliberate older pick carries over"
        );
    }

    #[test]
    fn notifications_stay_on_until_turned_off_and_keep_the_other_choices() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert!(enabled());
        write_core(r#"{"handler": "both"}"#);
        assert!(enabled(), "a file without the key keeps notifications on");
        set_enabled(false).unwrap();
        assert!(!enabled());
        assert_eq!(native_handler(), NativeHandler::Both);
        assert_eq!(
            stored_slice(Scope::Core),
            Some(json!({ "handler": "both" }))
        );
        set_enabled(true).unwrap();
        assert!(enabled());
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
        let saved = std::fs::read_to_string(slice_file(Scope::Core)).unwrap();
        assert!(saved.contains(r#""placement": "0.25,0.6""#), "{saved}");
    }

    #[test]
    fn a_placement_the_toasts_cannot_read_falls_back_to_the_top_right() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        for stored in [r#""somewhere""#, "7", r#""2,2""#] {
            write_core(&format!(r#"{{"placement": {stored}}}"#));
            assert_eq!(placement(), Spot::TOP_RIGHT, "{stored}");
            assert_eq!(settings().unwrap().placement, None, "{stored}");
        }
        write_core(r#"{"placement": "bottom-center"}"#);
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

    #[test]
    fn qol_takes_over_other_apps_only_while_its_own_messages_are_toasts() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert_eq!(other_apps(), OtherApps::Qol);
        assert!(takeover_wanted());
        write_core(r#"{"handler": "qol"}"#);
        assert!(
            takeover_wanted(),
            "a file from before the choice existed keeps the old meaning of qol"
        );

        for handler in [NativeHandler::Os, NativeHandler::Both] {
            set_native_handler(handler).unwrap();
            assert_eq!(other_apps(), OtherApps::Qol, "{handler:?}");
            assert!(!takeover_wanted(), "{handler:?}");
        }
        set_native_handler(NativeHandler::Qol).unwrap();
        assert!(takeover_wanted());
        set_other_apps(OtherApps::Os).unwrap();
        assert!(
            !takeover_wanted(),
            "qol toasts no longer force the takeover"
        );
        set_native_handler(NativeHandler::Both).unwrap();
        set_native_handler(NativeHandler::Qol).unwrap();
        assert_eq!(
            other_apps(),
            OtherApps::Os,
            "the choice survives a handler change"
        );
    }

    #[test]
    fn each_choice_is_kept_where_sync_carries_it() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        set_enabled(false).unwrap();
        set_other_apps(OtherApps::Os).unwrap();
        set_native_handler(NativeHandler::Both).unwrap();
        set_placement(Spot::new(0.5, 1.0)).unwrap();
        set_message_seconds(4.0).unwrap();

        assert_eq!(
            stored_slice(Scope::Core),
            Some(json!({
                "handler": "both",
                "placement": Spot::new(0.5, 1.0).token(),
                "message_stay_seconds": 4.0,
            }))
        );
        assert_eq!(stored_slice(Scope::Os), Some(json!({ "other_apps": "os" })));
        assert_eq!(
            stored_slice(Scope::Device),
            Some(json!({ "enabled": false }))
        );
        assert!(!crate::paths::shared_config_dir()
            .unwrap()
            .join(qol_conventions::NOTIFICATIONS_SETTINGS_FILE)
            .exists());
    }

    #[test]
    fn a_mute_or_takeover_choice_from_another_computer_is_not_read() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        write_core(r#"{"enabled": false, "other_apps": "os", "handler": "qol"}"#);

        assert!(enabled());
        assert_eq!(other_apps(), OtherApps::Qol);
        assert!(takeover_wanted());
    }

    #[test]
    fn an_import_keeps_this_computers_mute_and_drops_the_legacy_bool() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        set_enabled(false).unwrap();
        write_legacy_file(true);

        let exported = exported_settings();
        assert_eq!(
            exported,
            *json!({ "use_system_notifications": true })
                .as_object()
                .unwrap()
        );

        let refused = json!({ "handler": "nowhere" });
        assert!(import_settings(refused.as_object().unwrap()).is_err());
        assert_eq!(native_handler(), NativeHandler::Both);

        let imported = json!({ "handler": "os", "other_apps": "os", "enabled": true });
        import_settings(imported.as_object().unwrap()).unwrap();

        assert_eq!(native_handler(), NativeHandler::Os);
        assert_eq!(other_apps(), OtherApps::Os);
        assert!(!enabled(), "the mute stays a choice of this computer");
        assert_eq!(stored_slice(Scope::Core), Some(json!({ "handler": "os" })));
    }
}
