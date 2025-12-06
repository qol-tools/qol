use super::{Plugin, PluginManifest};
use crate::paths;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub struct PluginLoader;

impl PluginLoader {
    pub fn default_plugin_dir() -> Result<PathBuf> {
        paths::plugins_dir()
    }

    pub fn ensure_plugin_dir() -> Result<PathBuf> {
        let dir = Self::default_plugin_dir()?;
        if dir.exists() { return Ok(dir); }

        fs::create_dir_all(&dir).context("Failed to create plugins directory")?;
        log::info!("Created plugins directory: {:?}", dir);
        Ok(dir)
    }

    pub fn load_all() -> Result<Vec<Plugin>> {
        let plugin_dir = Self::ensure_plugin_dir()?;
        Self::load_from_dir(&plugin_dir)
    }

    pub fn load_from_dir(dir: &Path) -> Result<Vec<Plugin>> {
        if !dir.exists() {
            log::warn!("Plugin directory does not exist: {:?}", dir);
            return Ok(Vec::new());
        }

        let entries = fs::read_dir(dir).context("Failed to read plugins directory")?;
        let paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();

        let plugins: Vec<Plugin> = paths
            .iter()
            .filter_map(|path| Self::try_load_plugin(path))
            .collect();

        log::info!("Loaded {} plugin(s)", plugins.len());
        Ok(plugins)
    }

    fn try_load_plugin(path: &Path) -> Option<Plugin> {
        match Self::load_plugin(path) {
            Ok(plugin) => {
                log::info!("Loaded plugin: {} ({})", plugin.manifest.plugin.name, plugin.id);
                Some(plugin)
            }
            Err(e) => {
                log::warn!("Failed to load plugin from {:?}: {}", path, e);
                None
            }
        }
    }

    pub fn load_plugin(path: &Path) -> Result<Plugin> {
        let manifest_path = path.join("plugin.toml");

        if !manifest_path.exists() {
            anyhow::bail!("No plugin.toml found in {:?}", path);
        }

        let manifest_content = fs::read_to_string(&manifest_path)
            .context("Failed to read plugin.toml")?;

        let manifest: PluginManifest = toml::from_str(&manifest_content)
            .context("Failed to parse plugin.toml")?;

        let id = path.file_name()
            .and_then(|n| n.to_str())
            .context("Invalid plugin directory name")?
            .to_string();

        Ok(Plugin::new(id, manifest, path.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    fn create_valid_manifest() -> String {
        r#"
[plugin]
name = "Test Plugin"
description = "A test plugin"
version = "1.0.0"

[menu]
label = "Test"
items = []
"#.to_string()
    }

    #[test]
    fn default_plugin_dir_ends_with_qol_tray_plugins() {
        // Act
        let dir = PluginLoader::default_plugin_dir().unwrap();

        // Assert
        assert!(dir.ends_with("qol-tray/plugins"));
    }

    #[test]
    fn load_from_dir_returns_empty_for_nonexistent_dir() {
        // Arrange
        let nonexistent = PathBuf::from("/nonexistent/path/that/does/not/exist");

        // Act
        let result = PluginLoader::load_from_dir(&nonexistent).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn load_from_dir_returns_empty_for_empty_dir() {
        // Arrange
        let temp_dir = TempDir::new().unwrap();

        // Act
        let result = PluginLoader::load_from_dir(temp_dir.path()).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn load_from_dir_skips_files() {
        // Arrange
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("not_a_plugin.txt"), "content").unwrap();

        // Act
        let result = PluginLoader::load_from_dir(temp_dir.path()).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn load_from_dir_skips_dirs_without_manifest() {
        // Arrange
        let temp_dir = TempDir::new().unwrap();
        fs::create_dir(temp_dir.path().join("invalid-plugin")).unwrap();

        // Act
        let result = PluginLoader::load_from_dir(temp_dir.path()).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn load_from_dir_loads_valid_plugin() {
        // Arrange
        let temp_dir = TempDir::new().unwrap();
        let plugin_dir = temp_dir.path().join("test-plugin");
        fs::create_dir(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("plugin.toml"), create_valid_manifest()).unwrap();

        // Act
        let result = PluginLoader::load_from_dir(temp_dir.path()).unwrap();

        // Assert
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "test-plugin");
        assert_eq!(result[0].manifest.plugin.name, "Test Plugin");
    }

    #[test]
    fn load_plugin_fails_without_manifest() {
        // Arrange
        let temp_dir = TempDir::new().unwrap();

        // Act
        let result = PluginLoader::load_plugin(temp_dir.path());

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn load_plugin_fails_with_invalid_toml() {
        // Arrange
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("plugin.toml"), "invalid toml {{{").unwrap();

        // Act
        let result = PluginLoader::load_plugin(temp_dir.path());

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn load_plugin_extracts_id_from_directory_name() {
        // Arrange
        let temp_dir = TempDir::new().unwrap();
        let plugin_dir = temp_dir.path().join("my-custom-plugin");
        fs::create_dir(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("plugin.toml"), create_valid_manifest()).unwrap();

        // Act
        let plugin = PluginLoader::load_plugin(&plugin_dir).unwrap();

        // Assert
        assert_eq!(plugin.id, "my-custom-plugin");
    }

    #[test]
    fn load_plugin_parses_manifest_fields() {
        // Arrange
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("plugin.toml"), create_valid_manifest()).unwrap();

        // Act
        let plugin = PluginLoader::load_plugin(temp_dir.path()).unwrap();

        // Assert
        assert_eq!(plugin.manifest.plugin.name, "Test Plugin");
        assert_eq!(plugin.manifest.plugin.description, "A test plugin");
        assert_eq!(plugin.manifest.plugin.version, "1.0.0");
    }
}
