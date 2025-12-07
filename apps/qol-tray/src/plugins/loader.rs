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
            .filter(|p| !p.extension().is_some_and(|ext| ext == "backup"))
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
                if !plugin.manifest.plugin.supports_current_platform() {
                    log::info!(
                        "Skipping plugin {} (unsupported platform: {})",
                        plugin.id,
                        std::env::consts::OS
                    );
                    return None;
                }
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

    const VALID_MANIFEST: &str = r#"
[plugin]
name = "Test Plugin"
description = "A test plugin"
version = "1.0.0"

[menu]
label = "Test"
items = []
"#;

    #[test]
    fn default_plugin_dir_ends_with_qol_tray_plugins() {
        let dir = PluginLoader::default_plugin_dir().unwrap();
        assert!(dir.ends_with("qol-tray/plugins"));
    }

    #[test]
    fn load_from_dir_returns_empty_when_no_valid_plugins() {
        let temp_dir = TempDir::new().unwrap();

        // Empty dir
        assert!(PluginLoader::load_from_dir(temp_dir.path()).unwrap().is_empty());

        // File instead of dir
        fs::write(temp_dir.path().join("file.txt"), "content").unwrap();
        assert!(PluginLoader::load_from_dir(temp_dir.path()).unwrap().is_empty());

        // Dir without manifest
        fs::create_dir(temp_dir.path().join("no-manifest")).unwrap();
        assert!(PluginLoader::load_from_dir(temp_dir.path()).unwrap().is_empty());

        // Nonexistent dir
        let nonexistent = PathBuf::from("/nonexistent/path");
        assert!(PluginLoader::load_from_dir(&nonexistent).unwrap().is_empty());
    }

    #[test]
    fn load_from_dir_loads_valid_plugin() {
        let temp_dir = TempDir::new().unwrap();
        let plugin_dir = temp_dir.path().join("test-plugin");
        fs::create_dir(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("plugin.toml"), VALID_MANIFEST).unwrap();

        let result = PluginLoader::load_from_dir(temp_dir.path()).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "test-plugin");
        assert_eq!(result[0].manifest.plugin.name, "Test Plugin");
    }

    #[test]
    fn load_plugin_fails_for_invalid_dirs() {
        // No manifest
        let temp_dir = TempDir::new().unwrap();
        assert!(PluginLoader::load_plugin(temp_dir.path()).is_err());

        // Invalid TOML
        fs::write(temp_dir.path().join("plugin.toml"), "invalid {{{").unwrap();
        assert!(PluginLoader::load_plugin(temp_dir.path()).is_err());
    }

    #[test]
    fn load_plugin_extracts_id_from_directory_name() {
        let temp_dir = TempDir::new().unwrap();
        let plugin_dir = temp_dir.path().join("my-custom-plugin");
        fs::create_dir(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("plugin.toml"), VALID_MANIFEST).unwrap();

        let plugin = PluginLoader::load_plugin(&plugin_dir).unwrap();

        assert_eq!(plugin.id, "my-custom-plugin");
    }

    #[test]
    fn load_plugin_parses_manifest_fields() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("plugin.toml"), VALID_MANIFEST).unwrap();

        let plugin = PluginLoader::load_plugin(temp_dir.path()).unwrap();

        assert_eq!(plugin.manifest.plugin.name, "Test Plugin");
        assert_eq!(plugin.manifest.plugin.description, "A test plugin");
        assert_eq!(plugin.manifest.plugin.version, "1.0.0");
    }

    #[test]
    fn load_from_dir_skips_backup_directories() {
        let temp_dir = TempDir::new().unwrap();

        let backup_dir = temp_dir.path().join("plugin-foo.backup");
        fs::create_dir(&backup_dir).unwrap();
        fs::write(backup_dir.join("plugin.toml"), VALID_MANIFEST).unwrap();

        let result = PluginLoader::load_from_dir(temp_dir.path()).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn load_from_dir_handles_mixed_valid_and_invalid() {
        let temp_dir = TempDir::new().unwrap();

        let valid = temp_dir.path().join("valid-plugin");
        fs::create_dir(&valid).unwrap();
        fs::write(valid.join("plugin.toml"), VALID_MANIFEST).unwrap();

        let no_manifest = temp_dir.path().join("no-manifest");
        fs::create_dir(&no_manifest).unwrap();

        let invalid_toml = temp_dir.path().join("invalid-toml");
        fs::create_dir(&invalid_toml).unwrap();
        fs::write(invalid_toml.join("plugin.toml"), "not valid toml {{{").unwrap();

        fs::write(temp_dir.path().join("just-a-file.txt"), "content").unwrap();

        let result = PluginLoader::load_from_dir(temp_dir.path()).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "valid-plugin");
    }

    #[test]
    fn load_plugin_handles_special_characters_in_id() {
        let temp_dir = TempDir::new().unwrap();

        let cases = ["plugin-with-dashes", "plugin_with_underscores", "plugin123"];

        for name in cases {
            let plugin_dir = temp_dir.path().join(name);
            fs::create_dir(&plugin_dir).unwrap();
            fs::write(plugin_dir.join("plugin.toml"), VALID_MANIFEST).unwrap();

            let plugin = PluginLoader::load_plugin(&plugin_dir).unwrap();
            assert_eq!(plugin.id, name, "plugin name: {}", name);
        }
    }

    #[test]
    #[cfg(unix)]
    fn load_from_dir_follows_symlinks() {
        use std::os::unix::fs::symlink;

        let temp_dir = TempDir::new().unwrap();
        let source_dir = TempDir::new().unwrap();

        fs::write(source_dir.path().join("plugin.toml"), VALID_MANIFEST).unwrap();

        let link_path = temp_dir.path().join("symlinked-plugin");
        symlink(source_dir.path(), &link_path).unwrap();

        let result = PluginLoader::load_from_dir(temp_dir.path()).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "symlinked-plugin");
    }
}
