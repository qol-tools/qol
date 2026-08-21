use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

const THEME_SETTINGS_FILE: &str = "theme.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct ThemeSettings {
    accent: Option<String>,
    theme: Option<String>,
    native_theme: Option<String>,
}

fn update_settings(update: impl FnOnce(&mut ThemeSettings)) -> Result<()> {
    let path = settings_path()?;
    let mut settings: ThemeSettings =
        crate::file_io::load_json_or_default(&path).context("failed to load theme settings")?;
    update(&mut settings);
    crate::file_io::write_pretty_json(&path, &settings).context("failed to save theme settings")
}

pub fn save_selected_accent_key(key: &str) -> Result<()> {
    let key = validated_accent_key(key)?.to_string();
    update_settings(|settings| settings.accent = Some(key))
}

pub fn clear_selected_accent_key() -> Result<()> {
    update_settings(|settings| settings.accent = None)
}

pub fn save_selected_theme_key(key: &str) -> Result<()> {
    let key = validated_theme_key(key)?.to_string();
    update_settings(|settings| settings.theme = Some(key))
}

pub fn clear_selected_theme_key() -> Result<()> {
    update_settings(|settings| settings.theme = None)
}

pub fn selected_theme_key() -> Result<Option<String>> {
    let settings: ThemeSettings = crate::file_io::load_json_or_default(&settings_path()?)
        .context("failed to load theme settings")?;
    match settings.theme.as_deref() {
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

pub fn save_selected_native_theme_key(key: &str) -> Result<()> {
    let key = validated_native_theme_key(key)?.to_string();
    update_settings(|settings| settings.native_theme = Some(key))
}

pub fn clear_selected_native_theme_key() -> Result<()> {
    update_settings(|settings| settings.native_theme = None)
}

pub fn selected_native_theme_key() -> Result<Option<String>> {
    let settings: ThemeSettings = crate::file_io::load_json_or_default(&settings_path()?)
        .context("failed to load theme settings")?;
    match settings.native_theme.as_deref() {
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
    let settings: ThemeSettings = crate::file_io::load_json_or_default(&settings_path()?)
        .context("failed to load theme settings")?;
    match settings.accent.as_deref() {
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

pub fn apply_accent_env(command: &mut Command) {
    command.env(qol_conventions::ENV_THEME_ACCENT, current_accent_key());
}

fn settings_path() -> Result<std::path::PathBuf> {
    crate::paths::shared_config_dir().map(|dir| dir.join(THEME_SETTINGS_FILE))
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
    use tempfile::TempDir;

    #[test]
    fn selected_accent_round_trips_valid_preset_key() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        save_selected_accent_key("blue").unwrap();

        assert_eq!(selected_accent_key().unwrap().as_deref(), Some("blue"));
        assert_eq!(resolved_accent_key(), "blue");
    }

    #[test]
    fn selected_accent_rejects_unknown_key() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert!(save_selected_accent_key("not-a-preset").is_err());
    }

    #[test]
    fn selected_accent_can_clear_to_theme_default() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        save_selected_accent_key("blue").unwrap();
        clear_selected_accent_key().unwrap();

        assert_eq!(selected_accent_key().unwrap(), None);
        assert_eq!(resolved_accent_key(), "amber");
    }

    #[test]
    fn resolved_accent_follows_theme_without_saved_key() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert_eq!(resolved_accent_key(), "amber");
        save_selected_theme_key("midnight").unwrap();
        assert_eq!(resolved_accent_key(), "violet");
        save_selected_accent_key("blue").unwrap();
        assert_eq!(resolved_accent_key(), "blue");
    }

    #[test]
    fn selected_theme_round_trips_and_rejects_unknown_key() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert!(save_selected_theme_key("not-a-theme").is_err());
        save_selected_theme_key("midnight").unwrap();
        assert_eq!(selected_theme_key().unwrap().as_deref(), Some("midnight"));
        assert_eq!(current_theme_key(), "midnight");
    }

    #[test]
    fn selected_native_theme_round_trips_valid_key_and_defaults_to_bone() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert_eq!(current_native_theme_key(), "bone");

        save_selected_native_theme_key("slate").unwrap();

        assert_eq!(
            selected_native_theme_key().unwrap().as_deref(),
            Some("slate")
        );
        assert_eq!(current_native_theme_key(), "slate");
        assert!(save_selected_native_theme_key("not-a-native-theme").is_err());
    }

    #[test]
    fn theme_clears_to_default_and_preserves_accent() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        save_selected_accent_key("blue").unwrap();
        save_selected_theme_key("midnight").unwrap();
        clear_selected_theme_key().unwrap();

        assert_eq!(selected_theme_key().unwrap(), None);
        assert_eq!(current_theme_key(), qol_theme::DEFAULT_TRAY_THEME_KEY);
        assert_eq!(selected_accent_key().unwrap().as_deref(), Some("blue"));
    }
}
