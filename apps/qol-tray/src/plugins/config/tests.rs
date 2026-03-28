use super::*;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn setup_test_env() -> (PluginConfigManager, TempDir, TempDir) {
    let temp_base = TempDir::new().unwrap();
    let temp_plugins = TempDir::new().unwrap();
    let manager = PluginConfigManager {
        configs_dir: temp_base.path().join("plugin-configs"),
    };
    (manager, temp_base, temp_plugins)
}

#[test]
fn plugin_config_path_returns_plugin_directory() {
    let plugin_id = "test-plugin";
    let path = PluginConfigManager::plugin_config_path(plugin_id).unwrap();
    assert!(path.to_string_lossy().contains("qol-tray"));
    assert!(path.to_string_lossy().contains("plugins"));
    assert!(path.to_string_lossy().contains("test-plugin"));
    assert!(path.to_string_lossy().ends_with("config.json"));
}

#[test]
fn load_configs_returns_default_when_file_missing() {
    let (manager, _temp_base, _temp_plugins) = setup_test_env();
    let result = manager.load_configs().unwrap();
    assert_eq!(result.configs.len(), 0);
}

#[test]
fn load_configs_parses_valid_json() {
    let (manager, _temp_base, _temp_plugins) = setup_test_env();
    let test_data = json!({
        "plugin1": {"enabled": true},
        "plugin2": {"value": 42}
    });
    fs::create_dir_all(&manager.configs_dir).unwrap();
    fs::write(
        manager.configs_dir.join("plugin1.json"),
        serde_json::to_string(&test_data["plugin1"]).unwrap(),
    )
    .unwrap();
    fs::write(
        manager.configs_dir.join("plugin2.json"),
        serde_json::to_string(&test_data["plugin2"]).unwrap(),
    )
    .unwrap();
    let result = manager.load_configs().unwrap();
    assert_eq!(result.configs.len(), 2);
    assert_eq!(
        result.configs.get("plugin1").unwrap(),
        &json!({"enabled": true})
    );
    assert_eq!(
        result.configs.get("plugin2").unwrap(),
        &json!({"value": 42})
    );
}

#[test]
fn save_configs_creates_parent_directory() {
    let (manager, _temp_base, _temp_plugins) = setup_test_env();
    let configs = PluginConfigs::default();
    let result = manager.save_configs(&configs);
    assert!(result.is_ok());
    assert!(manager.configs_dir.exists());
}

#[test]
fn save_configs_writes_pretty_json() {
    let (manager, _temp_base, _temp_plugins) = setup_test_env();
    let mut configs = PluginConfigs::default();
    configs
        .configs
        .insert("test".to_string(), json!({"key": "value"}));
    manager.save_configs(&configs).unwrap();
    let content = fs::read_to_string(manager.configs_dir.join("test.json")).unwrap();
    assert!(content.contains('\n'));
    assert!(content.contains("key"));
    assert!(content.contains("value"));
}

#[test]
fn save_configs_overwrites_existing_file() {
    let (manager, _temp_base, _temp_plugins) = setup_test_env();
    let mut configs1 = PluginConfigs::default();
    configs1
        .configs
        .insert("old".to_string(), json!({"data": 1}));
    manager.save_configs(&configs1).unwrap();
    let mut configs2 = PluginConfigs::default();
    configs2
        .configs
        .insert("new".to_string(), json!({"data": 2}));
    manager.save_configs(&configs2).unwrap();
    let result = manager.load_configs().unwrap();
    assert_eq!(result.configs.len(), 1);
    assert!(result.configs.contains_key("new"));
    assert!(!result.configs.contains_key("old"));
}

#[test]
fn restore_from_backup_returns_none_when_no_backup_exists() {
    let (manager, _temp_base, _temp_plugins) = setup_test_env();
    let result = manager.restore_from_backup("nonexistent").unwrap();
    assert!(result.is_none());
}

#[test]
fn restore_from_backup_returns_config_when_backup_exists() {
    let (manager, _temp_base, _temp_plugins) = setup_test_env();
    let mut configs = PluginConfigs::default();
    let expected_config = json!({"restored": true, "value": 123});
    configs
        .configs
        .insert("test-plugin".to_string(), expected_config.clone());
    manager.save_configs(&configs).unwrap();
    let result = manager.restore_from_backup("test-plugin").unwrap();
    assert_eq!(result, Some(expected_config));
    let _ = std::fs::remove_dir_all(
        PluginConfigManager::plugin_config_path("test-plugin")
            .unwrap()
            .parent()
            .unwrap(),
    );
}

#[test]
fn plugin_config_path_cases() {
    let valid = ["plugin-test", "my_plugin", "a"];
    for id in valid {
        assert!(
            PluginConfigManager::plugin_config_path(id).is_ok(),
            "should work: {:?}",
            id
        );
    }

    let invalid = ["../etc", "foo/bar", "..", ".", "", "a\0b"];
    for id in invalid {
        assert!(
            PluginConfigManager::plugin_config_path(id).is_err(),
            "should fail: {:?}",
            id
        );
    }
}
