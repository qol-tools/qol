use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

const THEME_SETTINGS_FILE: &str = "theme.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct ThemeSettings {
    accent: Option<String>,
}

pub fn save_selected_accent_key(key: &str) -> Result<()> {
    let key = validated_accent_key(key)?;
    let settings = ThemeSettings {
        accent: Some(key.to_string()),
    };
    crate::file_io::write_pretty_json(&settings_path()?, &settings)
        .context("failed to save theme settings")
}

pub fn clear_selected_accent_key() -> Result<()> {
    crate::file_io::write_pretty_json(&settings_path()?, &ThemeSettings::default())
        .context("failed to clear theme accent")
}

pub fn selected_accent_key() -> Result<Option<String>> {
    let settings: ThemeSettings = crate::file_io::load_json_or_default(&settings_path()?)
        .context("failed to load theme settings")?;
    match settings.accent.as_deref() {
        Some(key) => Ok(Some(validated_accent_key(key)?.to_string())),
        None => Ok(None),
    }
}

pub fn resolved_accent_key(dev: bool) -> String {
    selected_accent_key()
        .ok()
        .flatten()
        .unwrap_or_else(|| default_accent_key(dev).to_string())
}

pub fn current_accent_key() -> String {
    resolved_accent_key(current_dev_mode())
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

fn default_accent_key(dev: bool) -> &'static str {
    if dev {
        return qol_theme::DEV_ACCENT_KEY;
    }
    qol_theme::PROD_ACCENT_KEY
}

fn current_dev_mode() -> bool {
    cfg!(feature = "dev") && crate::mode::ModeConfig::load().unwrap_or_default().is_dev()
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
        assert_eq!(resolved_accent_key(false), "blue");
    }

    #[test]
    fn selected_accent_rejects_unknown_key() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert!(save_selected_accent_key("not-a-preset").is_err());
    }

    #[test]
    fn selected_accent_can_clear_to_mode_default() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        save_selected_accent_key("blue").unwrap();
        clear_selected_accent_key().unwrap();

        assert_eq!(selected_accent_key().unwrap(), None);
        assert_eq!(resolved_accent_key(false), qol_theme::PROD_ACCENT_KEY);
    }

    #[test]
    fn resolved_accent_uses_mode_default_without_saved_key() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert_eq!(resolved_accent_key(false), qol_theme::PROD_ACCENT_KEY);
        assert_eq!(resolved_accent_key(true), qol_theme::DEV_ACCENT_KEY);
    }
}
