use crate::paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginConfigs {
    #[serde(flatten)]
    pub configs: HashMap<String, serde_json::Value>,
}

pub struct PluginConfigManager {
    config_path: PathBuf,
}

impl PluginConfigManager {
    pub fn new() -> Result<Self> {
        let config_path = paths::plugin_configs_path()?;
        Ok(Self { config_path })
    }

    fn plugin_config_path(plugin_id: &str) -> Result<PathBuf> {
        paths::plugins_dir().map(|p| p.join(plugin_id).join("config.json"))
    }

    pub fn load_configs(&self) -> Result<PluginConfigs> {
        if !self.config_path.exists() {
            return Ok(PluginConfigs::default());
        }

        let content = std::fs::read_to_string(&self.config_path)?;
        let configs: PluginConfigs = serde_json::from_str(&content)?;
        Ok(configs)
    }

    pub fn save_configs(&self, configs: &PluginConfigs) -> Result<()> {
        ensure_parent_dir(&self.config_path)?;
        let content = serde_json::to_string_pretty(configs)?;
        std::fs::write(&self.config_path, content)?;
        Ok(())
    }

    pub fn get_config(&self, plugin_id: &str) -> Result<Option<serde_json::Value>> {
        let plugin_path = Self::plugin_config_path(plugin_id)?;

        if plugin_path.exists() {
            let content = std::fs::read_to_string(&plugin_path)?;
            let config = serde_json::from_str(&content)?;
            return Ok(Some(config));
        }

        self.restore_from_backup(plugin_id)
    }

    fn restore_from_backup(&self, plugin_id: &str) -> Result<Option<serde_json::Value>> {
        let configs = self.load_configs()?;
        let Some(config) = configs.configs.get(plugin_id).cloned() else {
            return Ok(None);
        };

        log::info!("Restoring config for plugin from backup: {}", plugin_id);
        write_plugin_config(plugin_id, &config)?;
        log::info!("Config restored for plugin: {}", plugin_id);
        Ok(Some(config))
    }

    pub fn set_config(&self, plugin_id: &str, config: serde_json::Value) -> Result<()> {
        write_plugin_config(plugin_id, &config)?;

        let mut configs = self.load_configs()?;
        configs.configs.insert(plugin_id.to_string(), config);
        self.save_configs(&configs)?;

        Ok(())
    }
}

fn write_plugin_config(plugin_id: &str, config: &serde_json::Value) -> Result<()> {
    let plugin_path = PluginConfigManager::plugin_config_path(plugin_id)?;
    ensure_parent_dir(&plugin_path)?;
    let content = serde_json::to_string_pretty(config)?;
    std::fs::write(&plugin_path, content)?;
    Ok(())
}

fn ensure_parent_dir(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_env() -> (PluginConfigManager, TempDir, TempDir) {
        let temp_base = TempDir::new().unwrap();
        let temp_plugins = TempDir::new().unwrap();
        let global_config_path = temp_base.path().join("plugin-configs.json");
        let manager = PluginConfigManager {
            config_path: global_config_path,
        };
        (manager, temp_base, temp_plugins)
    }

    #[test]
    fn plugin_config_path_returns_plugin_directory() {
        // Arrange
        let plugin_id = "test-plugin";

        // Act
        let path = PluginConfigManager::plugin_config_path(plugin_id).unwrap();

        // Assert
        assert!(path.to_string_lossy().contains("qol-tray"));
        assert!(path.to_string_lossy().contains("plugins"));
        assert!(path.to_string_lossy().contains("test-plugin"));
        assert!(path.to_string_lossy().ends_with("config.json"));
    }

    #[test]
    fn load_configs_returns_default_when_file_missing() {
        // Arrange
        let (manager, _temp_base, _temp_plugins) = setup_test_env();

        // Act
        let result = manager.load_configs().unwrap();

        // Assert
        assert_eq!(result.configs.len(), 0);
    }

    #[test]
    fn load_configs_parses_valid_json() {
        // Arrange
        let (manager, _temp_base, _temp_plugins) = setup_test_env();
        let test_data = json!({
            "plugin1": {"enabled": true},
            "plugin2": {"value": 42}
        });
        fs::write(&manager.config_path, test_data.to_string()).unwrap();

        // Act
        let result = manager.load_configs().unwrap();

        // Assert
        assert_eq!(result.configs.len(), 2);
        assert_eq!(result.configs.get("plugin1").unwrap(), &json!({"enabled": true}));
        assert_eq!(result.configs.get("plugin2").unwrap(), &json!({"value": 42}));
    }

    #[test]
    fn save_configs_creates_parent_directory() {
        // Arrange
        let (manager, _temp_base, _temp_plugins) = setup_test_env();
        let configs = PluginConfigs::default();

        // Act
        let result = manager.save_configs(&configs);

        // Assert
        assert!(result.is_ok());
        assert!(manager.config_path.exists());
    }

    #[test]
    fn save_configs_writes_pretty_json() {
        // Arrange
        let (manager, _temp_base, _temp_plugins) = setup_test_env();
        let mut configs = PluginConfigs::default();
        configs.configs.insert("test".to_string(), json!({"key": "value"}));

        // Act
        manager.save_configs(&configs).unwrap();

        // Assert
        let content = fs::read_to_string(&manager.config_path).unwrap();
        assert!(content.contains('\n'));
        assert!(content.contains("test"));
        assert!(content.contains("key"));
        assert!(content.contains("value"));
    }

    #[test]
    fn save_configs_overwrites_existing_file() {
        // Arrange
        let (manager, _temp_base, _temp_plugins) = setup_test_env();
        let mut configs1 = PluginConfigs::default();
        configs1.configs.insert("old".to_string(), json!({"data": 1}));
        manager.save_configs(&configs1).unwrap();
        let mut configs2 = PluginConfigs::default();
        configs2.configs.insert("new".to_string(), json!({"data": 2}));

        // Act
        manager.save_configs(&configs2).unwrap();

        // Assert
        let result = manager.load_configs().unwrap();
        assert_eq!(result.configs.len(), 1);
        assert!(result.configs.contains_key("new"));
        assert!(!result.configs.contains_key("old"));
    }

    #[test]
    fn restore_from_backup_returns_none_when_no_backup_exists() {
        // Arrange
        let (manager, _temp_base, _temp_plugins) = setup_test_env();

        // Act
        let result = manager.restore_from_backup("nonexistent").unwrap();

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn restore_from_backup_returns_config_when_backup_exists() {
        // Arrange
        let (manager, _temp_base, _temp_plugins) = setup_test_env();
        let mut configs = PluginConfigs::default();
        let expected_config = json!({"restored": true, "value": 123});
        configs.configs.insert("test-plugin".to_string(), expected_config.clone());
        manager.save_configs(&configs).unwrap();

        // Act
        let result = manager.restore_from_backup("test-plugin").unwrap();

        // Assert
        assert_eq!(result, Some(expected_config));

        // Cleanup - restore_from_backup writes to real plugin dir
        let _ = std::fs::remove_dir_all(
            PluginConfigManager::plugin_config_path("test-plugin")
                .unwrap()
                .parent()
                .unwrap(),
        );
    }
}
