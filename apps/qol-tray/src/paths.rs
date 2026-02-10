use anyhow::{anyhow, Context, Result};
use std::env;
use std::fs;
use std::path::PathBuf;

const APP_NAME: &str = "qol-tray";
const INSTALL_ID_ENV: &str = "QOL_TRAY_INSTALL_ID";
const INSTALL_ID_FILE: &str = "qol-tray.install-id";

pub fn is_safe_path_component(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('/')
        && !s.contains('\\')
        && !s.contains('\0')
        && s != ".."
        && s != "."
}

fn legacy_config_dir() -> Result<PathBuf> {
    dirs::config_dir()
        .context("Could not determine config directory")
        .map(|p| p.join(APP_NAME))
}

fn base_data_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .context("Could not determine local data directory")
        .map(|p| p.join(APP_NAME))
}

fn installs_dir() -> Result<PathBuf> {
    base_data_dir().map(|p| p.join("installs"))
}

fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn install_id_from_env() -> Option<String> {
    let value = env::var(INSTALL_ID_ENV).ok()?;
    let trimmed = value.trim();
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn install_id_from_marker_file() -> Option<String> {
    let exe = env::current_exe().ok()?;
    let parent = exe.parent()?;
    let marker_path = parent.join(INSTALL_ID_FILE);
    let content = fs::read_to_string(marker_path).ok()?;
    let trimmed = content.trim();
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub fn config_dir_for_install_id(install_id: &str) -> Result<PathBuf> {
    if !valid_install_id(install_id) {
        return Err(anyhow!("invalid install id"));
    }
    installs_dir().map(|p| p.join(install_id))
}

pub fn config_dir() -> Result<PathBuf> {
    if let Some(install_id) = install_id_from_env().or_else(install_id_from_marker_file) {
        return config_dir_for_install_id(&install_id);
    }
    legacy_config_dir()
}

pub fn plugins_dir() -> Result<PathBuf> {
    config_dir().map(|p| p.join("plugins"))
}

pub fn hotkeys_path() -> Result<PathBuf> {
    config_dir().map(|p| p.join("hotkeys.json"))
}

pub fn plugin_configs_path() -> Result<PathBuf> {
    config_dir().map(|p| p.join("plugin-configs.json"))
}

pub fn github_token_path() -> Result<PathBuf> {
    config_dir().map(|p| p.join(".github-token"))
}

pub fn plugin_cache_path() -> Result<PathBuf> {
    config_dir().map(|p| p.join(".plugin-cache.json"))
}

#[cfg(feature = "dev")]
pub fn dev_config_path() -> Result<PathBuf> {
    config_dir().map(|p| p.join("dev.json"))
}

pub fn open_url(url: &str) -> Result<()> {
    open::that(url)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_have_correct_suffixes() {
        let cases: Vec<(Result<PathBuf>, &str)> = vec![
            (config_dir(), "qol-tray"),
            (plugins_dir(), "qol-tray/plugins"),
            (hotkeys_path(), "hotkeys.json"),
            (plugin_configs_path(), "plugin-configs.json"),
            (github_token_path(), ".github-token"),
            (plugin_cache_path(), ".plugin-cache.json"),
        ];

        for (result, expected_suffix) in cases {
            let path = result.unwrap();
            assert!(path.ends_with(expected_suffix), "path {:?} should end with {}", path, expected_suffix);
        }
    }

    #[test]
    fn is_safe_path_component_cases() {
        let valid = [
            "plugin-launcher",
            "my_plugin",
            "plugin123",
            "UPPERCASE",
            "a",
            " ",
            ".hidden",
            "..hidden",
            "plugin..name",
            "MixedCase123",
        ];

        for s in valid {
            assert!(is_safe_path_component(s), "should be valid: {:?}", s);
        }

        let invalid = [
            "../etc",
            "foo/bar",
            "foo\\bar",
            "..",
            ".",
            "",
            "plugin\0evil",
            "plugin/",
            "/plugin",
            "plugin\\",
            "\\plugin",
            "a/b/c",
            "../..",
            "foo/../bar",
        ];

        for s in invalid {
            assert!(!is_safe_path_component(s), "should be invalid: {:?}", s);
        }
    }
}
