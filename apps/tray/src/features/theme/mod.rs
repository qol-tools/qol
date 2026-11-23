mod look;

use anyhow::{anyhow, Result};
use qol_config::profile::THEME;
use serde_json::{Map, Value};
use std::process::Command;

pub use look::{remember_shown_look, show_current_look};

const ACCENT_KEY: &str = "accent";
const THEME_KEY: &str = "theme";
const NATIVE_THEME_KEY: &str = "native_theme";
const CHOICE_KEYS: [&str; 4] = [
    ACCENT_KEY,
    THEME_KEY,
    NATIVE_THEME_KEY,
    qol_conventions::ANIMATIONS_KEY,
];

fn stored_key(key: &str) -> Result<Option<String>> {
    match THEME.read(&crate::paths::shared_config_dir()?).remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(other) => Err(anyhow!("theme setting {key} is not a string: {other}")),
    }
}

fn save_key(key: &str, value: Option<String>) -> Result<()> {
    crate::features::profile::update_settings_file(&THEME, |settings| {
        match value {
            Some(value) => settings.insert(key.to_string(), Value::String(value)),
            None => settings.remove(key),
        };
    })
}

pub(crate) fn exported_settings() -> Map<String, Value> {
    let Ok(config_dir) = crate::paths::shared_config_dir() else {
        return Map::new();
    };
    let mut settings = THEME.read(&config_dir);
    settings.retain(|key, value| check_imported(key, value).is_ok());
    settings
}

pub(crate) fn validate_import(imported: &Map<String, Value>) -> Result<()> {
    imported
        .iter()
        .try_for_each(|(key, value)| check_imported(key, value))
}

pub(crate) fn import_settings(imported: &Map<String, Value>) -> Result<()> {
    validate_import(imported)?;
    let imported = imported.clone();
    crate::features::profile::update_settings_file(&THEME, |settings| *settings = imported)
}

fn check_imported(key: &str, value: &Value) -> Result<()> {
    if !CHOICE_KEYS.contains(&key) || value.is_string() || value.is_null() {
        return Ok(());
    }
    Err(anyhow!(
        "the imported theme setting {key} is not a string: {value}"
    ))
}

pub fn save_selected_accent_key(key: Option<&str>) -> Result<()> {
    let key = key.map(validated_accent_key).transpose()?;
    save_key(ACCENT_KEY, key.map(str::to_string))
}

pub fn save_selected_theme_key(key: Option<&str>) -> Result<()> {
    let key = key.map(validated_theme_key).transpose()?;
    save_key(THEME_KEY, key.map(str::to_string))
}

pub fn selected_theme_key() -> Result<Option<String>> {
    match stored_key(THEME_KEY)?.as_deref() {
        Some(key) => Ok(Some(validated_theme_key(key)?.to_string())),
        None => Ok(None),
    }
}

pub fn current_theme_key() -> String {
    selected_theme_key()
        .ok()
        .flatten()
        .unwrap_or_else(|| qol_theme::DEFAULT_TRAY_THEME_KEY.to_string())
}

pub fn save_selected_native_theme_key(key: Option<&str>) -> Result<()> {
    let key = key.map(validated_native_theme_key).transpose()?;
    save_key(NATIVE_THEME_KEY, key.map(str::to_string))
}

pub fn selected_native_theme_key() -> Result<Option<String>> {
    match stored_key(NATIVE_THEME_KEY)?.as_deref() {
        Some(key) => Ok(Some(validated_native_theme_key(key)?.to_string())),
        None => Ok(None),
    }
}

pub fn current_native_theme_key() -> String {
    selected_native_theme_key()
        .ok()
        .flatten()
        .unwrap_or_else(|| qol_theme::DEFAULT_NATIVE_THEME_KEY.to_string())
}

fn validated_native_theme_key(key: &str) -> Result<&str> {
    if qol_theme::NATIVE_THEME_KEYS.contains(&key) {
        return Ok(key);
    }
    Err(anyhow!("unknown native theme: {key}"))
}

pub fn save_animations(key: &str) -> Result<()> {
    let choice = qol_theme::Animations::parse(key)
        .ok_or_else(|| anyhow!("unknown animations choice: {key}"))?;
    save_key(
        qol_conventions::ANIMATIONS_KEY,
        Some(choice.key().to_string()),
    )
}

pub fn current_animations() -> qol_theme::Animations {
    stored_key(qol_conventions::ANIMATIONS_KEY)
        .ok()
        .flatten()
        .and_then(|key| qol_theme::Animations::parse(&key))
        .unwrap_or_default()
}

pub fn apply_theme_name_env(command: &mut Command) {
    command.env(qol_conventions::ENV_THEME_NAME, current_native_theme_key());
}

fn validated_theme_key(key: &str) -> Result<&str> {
    if qol_theme::tray_theme_preset(key).is_some() {
        return Ok(key);
    }
    Err(anyhow!("unknown theme: {key}"))
}

pub fn selected_accent_key() -> Result<Option<String>> {
    match stored_key(ACCENT_KEY)?.as_deref() {
        Some(key) => Ok(Some(validated_accent_key(key)?.to_string())),
        None => Ok(None),
    }
}

pub fn resolved_accent_key() -> String {
    selected_accent_key()
        .ok()
        .flatten()
        .unwrap_or_else(theme_accent_key)
}

fn theme_accent_key() -> String {
    qol_theme::tray_theme_preset(&current_theme_key())
        .map(|preset| preset.accent_key.to_string())
        .unwrap_or_else(|| qol_theme::PROD_ACCENT_KEY.to_string())
}

pub fn current_accent_key() -> String {
    resolved_accent_key()
}

pub fn native_accent_wire() -> String {
    resolved_accent_key()
}

pub fn apply_accent_env(command: &mut Command) {
    command.env(qol_conventions::ENV_THEME_ACCENT, resolved_accent_key());
}

fn validated_accent_key(key: &str) -> Result<&str> {
    if qol_theme::dark_accent_preset(key).is_some() {
        return Ok(key);
    }
    Err(anyhow!("unknown theme accent: {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn profile_core_file(profile: &str) -> std::path::PathBuf {
        crate::paths::profile_dir()
            .unwrap()
            .join(profile)
            .join("core")
            .join(qol_conventions::THEME_SETTINGS_FILE)
    }

    fn stored_json(path: &std::path::Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn selected_accent_round_trips_valid_preset_key() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        save_selected_accent_key(Some("blue")).unwrap();

        assert_eq!(selected_accent_key().unwrap().as_deref(), Some("blue"));
        assert_eq!(resolved_accent_key(), "blue");
    }

    #[test]
    fn selected_accent_rejects_unknown_key() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert!(save_selected_accent_key(Some("not-a-preset")).is_err());
    }

    #[test]
    fn selected_accent_can_clear_to_theme_default() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        save_selected_accent_key(Some("blue")).unwrap();
        save_selected_accent_key(None).unwrap();

        assert_eq!(selected_accent_key().unwrap(), None);
        assert_eq!(resolved_accent_key(), "amber");
    }

    #[test]
    fn resolved_accent_follows_theme_without_saved_key() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert_eq!(resolved_accent_key(), "amber");
        save_selected_theme_key(Some("midnight")).unwrap();
        assert_eq!(resolved_accent_key(), "violet");
        save_selected_accent_key(Some("blue")).unwrap();
        assert_eq!(resolved_accent_key(), "blue");
    }

    #[test]
    fn selected_theme_round_trips_and_rejects_unknown_key() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert!(save_selected_theme_key(Some("not-a-theme")).is_err());
        save_selected_theme_key(Some("midnight")).unwrap();
        assert_eq!(selected_theme_key().unwrap().as_deref(), Some("midnight"));
        assert_eq!(current_theme_key(), "midnight");
    }

    #[test]
    fn selected_native_theme_round_trips_valid_key_and_defaults_to_bone() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert_eq!(current_native_theme_key(), "bone");

        save_selected_native_theme_key(Some("slate")).unwrap();

        assert_eq!(
            selected_native_theme_key().unwrap().as_deref(),
            Some("slate")
        );
        assert_eq!(current_native_theme_key(), "slate");
        assert!(save_selected_native_theme_key(Some("not-a-native-theme")).is_err());
    }

    #[test]
    fn theme_clears_to_default_and_preserves_accent() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        save_selected_accent_key(Some("blue")).unwrap();
        save_selected_theme_key(Some("midnight")).unwrap();
        save_selected_theme_key(None).unwrap();

        assert_eq!(selected_theme_key().unwrap(), None);
        assert_eq!(current_theme_key(), qol_theme::DEFAULT_TRAY_THEME_KEY);
        assert_eq!(selected_accent_key().unwrap().as_deref(), Some("blue"));
    }

    #[test]
    fn native_surfaces_wire_the_one_selected_accent() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert_eq!(native_accent_wire(), current_accent_key());

        save_selected_accent_key(Some("blue")).unwrap();

        assert_eq!(native_accent_wire(), "blue");
    }

    #[test]
    fn the_look_is_kept_in_the_synced_core_of_the_active_profile() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        save_selected_accent_key(Some("blue")).unwrap();
        save_animations("off").unwrap();

        assert_eq!(
            stored_json(&profile_core_file("default")),
            json!({ "accent": "blue", "animations": "off" })
        );
        assert!(!crate::paths::shared_config_dir()
            .unwrap()
            .join(qol_conventions::THEME_SETTINGS_FILE)
            .exists());

        std::fs::write(
            crate::paths::active_profile_marker_path().unwrap(),
            "work\n",
        )
        .unwrap();
        assert_eq!(selected_accent_key().unwrap(), None);
        save_selected_accent_key(Some("green")).unwrap();
        assert_eq!(
            stored_json(&profile_core_file("work")),
            json!({ "accent": "green" })
        );
        assert_eq!(
            stored_json(&profile_core_file("default"))["accent"],
            json!("blue")
        );
    }

    #[test]
    fn a_setting_written_by_a_newer_version_survives_a_save() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let path = profile_core_file("default");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"accent": "blue", "corner_radius": 4}"#).unwrap();

        save_selected_theme_key(Some("midnight")).unwrap();

        assert_eq!(
            stored_json(&path),
            json!({ "accent": "blue", "corner_radius": 4, "theme": "midnight" })
        );
    }

    #[test]
    fn an_import_replaces_the_look_and_keeps_a_choice_from_a_newer_version() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        save_selected_theme_key(Some("midnight")).unwrap();

        let refused = json!({ "accent": 5 });
        assert!(import_settings(refused.as_object().unwrap()).is_err());
        assert_eq!(current_theme_key(), "midnight");

        let imported = json!({ "accent": "a-newer-preset", "native_theme": "slate" });
        import_settings(imported.as_object().unwrap()).unwrap();

        assert_eq!(exported_settings(), *imported.as_object().unwrap());
        assert_eq!(current_theme_key(), qol_theme::DEFAULT_TRAY_THEME_KEY);
        assert_eq!(resolved_accent_key(), "amber");
        assert_eq!(current_native_theme_key(), "slate");
    }

    #[test]
    fn an_imported_choice_must_be_text() {
        let cases = [
            (ACCENT_KEY, json!("blue"), true),
            (ACCENT_KEY, json!("a-newer-preset"), true),
            (ACCENT_KEY, json!(null), true),
            (ACCENT_KEY, json!(5), false),
            (THEME_KEY, json!(["midnight"]), false),
            (NATIVE_THEME_KEY, json!(true), false),
            (
                qol_conventions::ANIMATIONS_KEY,
                json!({ "off": true }),
                false,
            ),
            ("corner_radius", json!(4), true),
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
    fn an_export_leaves_out_a_choice_an_import_would_refuse() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let path = profile_core_file("default");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"accent": 5, "theme": "midnight", "corner_radius": 4}"#,
        )
        .unwrap();

        assert_eq!(
            Value::Object(exported_settings()),
            json!({ "theme": "midnight", "corner_radius": 4 })
        );
    }
}
