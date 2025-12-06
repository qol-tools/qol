use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn config_dir() -> Result<PathBuf> {
    dirs::config_dir()
        .context("Could not determine config directory")
        .map(|p| p.join("qol-tray"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_ends_with_qol_tray() {
        // Arrange & Act
        let path = config_dir().unwrap();

        // Assert
        assert!(path.ends_with("qol-tray"));
    }

    #[test]
    fn plugins_dir_is_under_config_dir() {
        // Arrange & Act
        let path = plugins_dir().unwrap();

        // Assert
        assert!(path.ends_with("qol-tray/plugins"));
    }

    #[test]
    fn hotkeys_path_has_correct_filename() {
        // Arrange & Act
        let path = hotkeys_path().unwrap();

        // Assert
        assert!(path.ends_with("hotkeys.json"));
    }

    #[test]
    fn plugin_configs_path_has_correct_filename() {
        // Arrange & Act
        let path = plugin_configs_path().unwrap();

        // Assert
        assert!(path.ends_with("plugin-configs.json"));
    }

    #[test]
    fn github_token_path_has_correct_filename() {
        // Arrange & Act
        let path = github_token_path().unwrap();

        // Assert
        assert!(path.ends_with(".github-token"));
    }

    #[test]
    fn plugin_cache_path_has_correct_filename() {
        // Arrange & Act
        let path = plugin_cache_path().unwrap();

        // Assert
        assert!(path.ends_with(".plugin-cache.json"));
    }
}
