pub mod menu;
pub(crate) mod mirror;
pub mod takeover;

use anyhow::{anyhow, Result};
use qol_config::placement::Spot;
use qol_config::profile::{Scope, NOTIFICATIONS};
use qol_conventions::{
    LEGACY_MESSAGE_SECONDS_KEY, LEGACY_SYSTEM_NOTIFICATIONS_KEY, MESSAGE_STAY_KEY,
    NOTIFICATION_ENABLED_KEY, NOTIFICATION_HANDLER_KEY, NOTIFICATION_OTHER_APPS_KEY,
    NOTIFICATION_PLACEMENT_KEY,
};
use qol_plugin_daemon::notification::gate::NativeHandler;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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

impl NotificationSettings {
    fn from_stored(stored: &Map<String, Value>) -> Self {
        let seconds = |key: &str| stored.get(key).and_then(Value::as_f64);
        let legacy_handler = stored
            .get(LEGACY_SYSTEM_NOTIFICATIONS_KEY)
            .and_then(Value::as_bool)
            .map(|system| {
                if system {
                    NativeHandler::Both
                } else {
                    NativeHandler::Qol
                }
            });
        Self {
            enabled: stored
                .get(NOTIFICATION_ENABLED_KEY)
                .and_then(Value::as_bool)
                .unwrap_or(true),
            handler: choice(stored, NOTIFICATION_HANDLER_KEY)
                .or(legacy_handler)
                .unwrap_or(NativeHandler::Qol),
            other_apps: choice(stored, NOTIFICATION_OTHER_APPS_KEY).unwrap_or(OtherApps::Qol),
            message_stay_seconds: qol_theme::stored_message_stay(
                seconds(MESSAGE_STAY_KEY),
                seconds(LEGACY_MESSAGE_SECONDS_KEY),
            ),
            placement: stored
                .get(NOTIFICATION_PLACEMENT_KEY)
                .and_then(Value::as_str)
                .and_then(Spot::parse)
                .map(Spot::token),
        }
    }
}

fn choice<T: DeserializeOwned>(stored: &Map<String, Value>, key: &str) -> Option<T> {
    T::deserialize(stored.get(key)?).ok()
}

fn update_settings(update: impl FnOnce(&mut Map<String, Value>)) -> Result<()> {
    let saved = crate::features::profile::update_settings_file(&NOTIFICATIONS, update);
    takeover::poke();
    saved
}

fn save_value(key: &str, value: Value, replaces: &[&str]) -> Result<()> {
    update_settings(|settings| {
        for legacy in replaces {
            settings.remove(*legacy);
        }
        settings.insert(key.to_string(), value);
    })
}

pub(crate) fn exported_settings() -> Map<String, Value> {
    let Ok(config_dir) = crate::paths::shared_config_dir() else {
        return Map::new();
    };
    let mut settings = NOTIFICATIONS.read(&config_dir);
    settings.retain(|key, value| {
        NOTIFICATIONS.scope_of(key) != Scope::Device && check_imported(key, value).is_ok()
    });
    settings
}

pub(crate) fn validate_import(imported: &Map<String, Value>) -> Result<()> {
    imported
        .iter()
        .try_for_each(|(key, value)| check_imported(key, value))
}

fn check_imported(key: &str, value: &Value) -> Result<()> {
    let valid = value.is_null()
        || match key {
            NOTIFICATION_HANDLER_KEY | NOTIFICATION_OTHER_APPS_KEY | NOTIFICATION_PLACEMENT_KEY => {
                value.is_string()
            }
            MESSAGE_STAY_KEY | LEGACY_MESSAGE_SECONDS_KEY => value.is_number(),
            NOTIFICATION_ENABLED_KEY | LEGACY_SYSTEM_NOTIFICATIONS_KEY => value.is_boolean(),
            _ => true,
        };
    if valid {
        return Ok(());
    }
    Err(anyhow!(
        "the imported notification setting {key} has the wrong type: {value}"
    ))
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
        NOTIFICATION_HANDLER_KEY,
        serde_json::to_value(handler)?,
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
        serde_json::to_value(seconds)?,
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
    save_value(
        NOTIFICATION_OTHER_APPS_KEY,
        serde_json::to_value(other_apps)?,
        &[],
    )
}

pub fn takeover_wanted() -> bool {
    native_handler() == NativeHandler::Qol && other_apps() == OtherApps::Qol
}

fn settings() -> Result<NotificationSettings> {
    let stored = NOTIFICATIONS.read(&crate::paths::shared_config_dir()?);
    Ok(NotificationSettings::from_stored(&stored))
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

    fn write_slice(scope: Scope, contents: &str) {
        let path = slice_file(scope);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
    }

    fn write_core(contents: &str) {
        write_slice(Scope::Core, contents);
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

        let refused = json!({ "handler": 5 });
        assert!(import_settings(refused.as_object().unwrap()).is_err());
        assert_eq!(native_handler(), NativeHandler::Both);

        let imported = json!({ "handler": "os", "other_apps": "os", "enabled": true });
        import_settings(imported.as_object().unwrap()).unwrap();

        assert_eq!(native_handler(), NativeHandler::Os);
        assert_eq!(other_apps(), OtherApps::Os);
        assert!(!enabled(), "the mute stays a choice of this computer");
        assert_eq!(stored_slice(Scope::Core), Some(json!({ "handler": "os" })));
    }

    #[test]
    fn an_imported_setting_must_have_the_type_it_is_stored_as() {
        let cases = [
            (NOTIFICATION_HANDLER_KEY, json!("nowhere"), true),
            (NOTIFICATION_HANDLER_KEY, json!(null), true),
            (NOTIFICATION_HANDLER_KEY, json!(5), false),
            (NOTIFICATION_OTHER_APPS_KEY, json!("os"), true),
            (NOTIFICATION_OTHER_APPS_KEY, json!(true), false),
            (NOTIFICATION_PLACEMENT_KEY, json!("somewhere"), true),
            (NOTIFICATION_PLACEMENT_KEY, json!([0.25, 0.6]), false),
            (MESSAGE_STAY_KEY, json!(2.5), true),
            (MESSAGE_STAY_KEY, json!("2.5"), false),
            (LEGACY_MESSAGE_SECONDS_KEY, json!(3), true),
            (LEGACY_MESSAGE_SECONDS_KEY, json!(false), false),
            (NOTIFICATION_ENABLED_KEY, json!(false), true),
            (NOTIFICATION_ENABLED_KEY, json!("no"), false),
            (LEGACY_SYSTEM_NOTIFICATIONS_KEY, json!(true), true),
            (LEGACY_SYSTEM_NOTIFICATIONS_KEY, json!(1), false),
            ("sound", json!({ "volume": 3 }), true),
        ];
        for (key, value, accepted) in cases {
            assert_eq!(
                check_imported(key, &value).is_ok(),
                accepted,
                "{key} {value}"
            );
        }
    }

    #[test]
    fn an_import_keeps_a_choice_from_a_newer_version_and_reads_it_as_the_default() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let imported = json!({ "handler": "nowhere", "placement": "0.25,0.6" });

        import_settings(imported.as_object().unwrap()).unwrap();

        assert_eq!(native_handler(), NativeHandler::Qol);
        assert_eq!(placement(), Spot::new(0.25, 0.6));
        assert_eq!(exported_settings(), *imported.as_object().unwrap());
    }

    #[test]
    fn a_choice_this_version_cannot_read_leaves_the_other_choices_standing() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        set_enabled(false).unwrap();
        set_other_apps(OtherApps::Os).unwrap();
        write_core(
            r#"{"handler": "pager", "placement": "bottom-left", "message_stay_seconds": 4}"#,
        );

        assert!(!enabled(), "the mute stays");
        assert_eq!(native_handler(), NativeHandler::Qol);
        assert_eq!(other_apps(), OtherApps::Os);
        assert_eq!(placement(), Spot::parse("bottom-left").unwrap());
        assert_eq!(message_seconds(), 4.0);

        write_slice(Scope::Os, r#"{"other_apps": "elsewhere"}"#);
        write_core(r#"{"handler": "both", "use_system_notifications": "yes"}"#);

        assert!(!enabled(), "the mute stays");
        assert_eq!(native_handler(), NativeHandler::Both);
        assert_eq!(other_apps(), OtherApps::Qol);
    }
}
