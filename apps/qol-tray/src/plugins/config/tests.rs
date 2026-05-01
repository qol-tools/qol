use super::*;
use serde_json::json;
use std::ffi::OsString;
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

struct ConfigEnvGuard {
    home: Option<OsString>,
    xdg_config_home: Option<OsString>,
    xdg_data_home: Option<OsString>,
}

impl ConfigEnvGuard {
    fn new(root: &std::path::Path) -> Self {
        let home = std::env::var_os("HOME");
        let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
        let xdg_data_home = std::env::var_os("XDG_DATA_HOME");
        let home_dir = root.join("home");
        let xdg_config_dir = root.join("xdg-config");
        let xdg_data_dir = root.join("xdg-data");
        fs::create_dir_all(&home_dir).unwrap();
        fs::create_dir_all(&xdg_config_dir).unwrap();
        fs::create_dir_all(&xdg_data_dir).unwrap();
        std::env::set_var("HOME", &home_dir);
        std::env::set_var("XDG_CONFIG_HOME", &xdg_config_dir);
        std::env::set_var("XDG_DATA_HOME", &xdg_data_dir);
        Self {
            home,
            xdg_config_home,
            xdg_data_home,
        }
    }
}

impl Drop for ConfigEnvGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.home {
            std::env::set_var("HOME", value);
        }
        if self.home.is_none() {
            std::env::remove_var("HOME");
        }
        if let Some(value) = &self.xdg_config_home {
            std::env::set_var("XDG_CONFIG_HOME", value);
        }
        if self.xdg_config_home.is_none() {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        if let Some(value) = &self.xdg_data_home {
            std::env::set_var("XDG_DATA_HOME", value);
        }
        if self.xdg_data_home.is_none() {
            std::env::remove_var("XDG_DATA_HOME");
        }
    }
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
fn plugin_config_path_prefers_active_install_root() {
    let _guard = crate::test_support::env_lock().blocking_lock();
    let root = TempDir::new().unwrap();
    let _env = ConfigEnvGuard::new(root.path());
    let install_id = "install-test-123";
    let active_path = crate::paths::base_data_dir()
        .unwrap()
        .join("active-install-id");
    fs::create_dir_all(active_path.parent().unwrap()).unwrap();
    fs::write(&active_path, format!("{install_id}\n")).unwrap();

    let path = PluginConfigManager::plugin_config_path("plugin-test").unwrap();

    assert!(path.to_string_lossy().contains("installs"));
    assert!(path.to_string_lossy().contains(install_id));
    assert!(path
        .to_string_lossy()
        .ends_with("plugins/plugin-test/config.json"));
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

#[test]
fn load_combined_contracts_returns_both_when_present() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("qol-config.toml"),
        r#"
schema_version = 1

[field.pair]
type = "action"
label = "Pair"
action = "pair_device"
"#,
    )
    .unwrap();
    fs::write(
        root.join("qol-runtime.toml"),
        r#"
schema_version = 1

[action.pair_device]
description = "Pair device"
"#,
    )
    .unwrap();
    let (config, runtime) = load_combined_contracts_from_root(root)
        .expect("load contracts")
        .expect("contract present");
    assert_eq!(config.fields.len(), 1);
    let runtime = runtime.expect("runtime present");
    assert!(runtime.actions.contains_key("pair_device"));
}

#[test]
fn load_combined_contracts_fails_on_dangling_reference() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("qol-config.toml"),
        r#"
schema_version = 1

[field.pair]
type = "action"
label = "Pair"
action = "nonexistent"
"#,
    )
    .unwrap();
    fs::write(root.join("qol-runtime.toml"), "schema_version = 1\n").unwrap();
    let result = load_combined_contracts_from_root(root);
    assert!(result.is_err(), "dangling reference should fail");
}

#[test]
fn load_combined_contracts_allows_missing_runtime_when_not_referenced() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("qol-config.toml"),
        r#"
schema_version = 1

[field.enabled]
type = "boolean"
default = true
"#,
    )
    .unwrap();
    let (config, runtime) = load_combined_contracts_from_root(root)
        .expect("load contracts")
        .expect("contract present");
    assert_eq!(config.fields.len(), 1);
    assert!(runtime.is_none(), "runtime should be None when not present");
}

const MANIFEST_HEAD: &str = r#"
manifest_version = 2

[plugin]
name = "Traits Test"
description = "Traits serialization test"
version = "0.0.0"

[menu]
label = "Traits"
items = []
"#;

#[test]
fn load_plugin_traits_defaults_when_manifest_missing() {
    let tmp = TempDir::new().unwrap();
    let traits = load_plugin_traits_from_root(tmp.path());
    assert_eq!(traits, serde_json::json!({ "confined": {} }));
}

#[test]
fn load_plugin_traits_defaults_when_manifest_has_no_traits_table() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("plugin.toml"), MANIFEST_HEAD).unwrap();
    let traits = load_plugin_traits_from_root(tmp.path());
    assert_eq!(traits, serde_json::json!({ "confined": {} }));
}

#[test]
fn load_plugin_traits_passes_through_manifest_table_verbatim() {
    let tmp = TempDir::new().unwrap();
    let manifest = format!(
        r#"{MANIFEST_HEAD}
[traits]
confined = {{}}

[traits.peripheral-preview]
neighbors = 1

[traits.atmosphere]
preset = "wood"
"#
    );
    fs::write(tmp.path().join("plugin.toml"), manifest).unwrap();

    let traits = load_plugin_traits_from_root(tmp.path());

    assert_eq!(
        traits,
        serde_json::json!({
            "confined": {},
            "peripheral-preview": { "neighbors": 1 },
            "atmosphere": { "preset": "wood" },
        })
    );
}

#[test]
fn load_plugin_traits_tolerates_unknown_keys() {
    let tmp = TempDir::new().unwrap();
    let manifest = format!(
        r#"{MANIFEST_HEAD}
[traits.future_trait]
tunable = 42
kind = "experimental"
"#
    );
    fs::write(tmp.path().join("plugin.toml"), manifest).unwrap();

    let traits = load_plugin_traits_from_root(tmp.path());

    assert_eq!(
        traits,
        serde_json::json!({
            "future_trait": { "tunable": 42, "kind": "experimental" },
        })
    );
}

#[test]
fn load_plugin_traits_falls_back_on_non_object_traits_value() {
    let tmp = TempDir::new().unwrap();
    let cases = [
        (r#"traits = 7"#, "scalar integer"),
        (r#"traits = "hi""#, "scalar string"),
        (r#"traits = [1, 2]"#, "array"),
    ];
    for (fragment, label) in cases {
        let manifest = format!("{MANIFEST_HEAD}\n{fragment}\n");
        fs::write(tmp.path().join("plugin.toml"), manifest).unwrap();
        let traits = load_plugin_traits_from_root(tmp.path());
        assert_eq!(
            traits,
            serde_json::json!({ "confined": {} }),
            "case: {label}"
        );
    }
}
