use super::*;
use serde_json::json;
use std::ffi::OsString;
use std::fs;
use tempfile::TempDir;

fn setup_test_env() -> (PluginConfigManager, TempDir, TempDir) {
    let temp_base = TempDir::new().unwrap();
    let temp_plugins = TempDir::new().unwrap();
    let scope_store = crate::features::profile::ProfileScopeStore::at_dir(
        temp_base.path().to_path_buf(),
        crate::paths::current_os_subdir().to_string(),
    )
    .unwrap();
    let manager = PluginConfigManager::with_store(scope_store);
    (manager, temp_base, temp_plugins)
}

fn store_at(profile: &std::path::Path, os: &str) -> crate::features::profile::ProfileScopeStore {
    crate::features::profile::ProfileScopeStore::at_dir(profile.to_path_buf(), os.to_string())
        .unwrap()
}

struct ConfigEnvGuard {
    home: Option<OsString>,
    user_profile: Option<OsString>,
    app_data: Option<OsString>,
    local_app_data: Option<OsString>,
    xdg_config_home: Option<OsString>,
    xdg_data_home: Option<OsString>,
    _path_root: crate::paths::TestPathRootGuard,
    _env_lock: tokio::sync::MutexGuard<'static, ()>,
}

impl ConfigEnvGuard {
    fn new(root: &std::path::Path) -> Self {
        let env_lock = crate::test_support::env_lock().blocking_lock();
        let path_root = crate::paths::push_test_path_root(root);
        let home = std::env::var_os("HOME");
        let user_profile = std::env::var_os("USERPROFILE");
        let app_data = std::env::var_os("APPDATA");
        let local_app_data = std::env::var_os("LOCALAPPDATA");
        let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
        let xdg_data_home = std::env::var_os("XDG_DATA_HOME");
        let home_dir = root.join("home");
        let app_data_dir = root.join("app-data");
        let local_app_data_dir = root.join("local-app-data");
        let xdg_config_dir = root.join("xdg-config");
        let xdg_data_dir = root.join("xdg-data");
        fs::create_dir_all(&home_dir).unwrap();
        fs::create_dir_all(&app_data_dir).unwrap();
        fs::create_dir_all(&local_app_data_dir).unwrap();
        fs::create_dir_all(&xdg_config_dir).unwrap();
        fs::create_dir_all(&xdg_data_dir).unwrap();
        std::env::set_var("HOME", &home_dir);
        std::env::set_var("USERPROFILE", &home_dir);
        std::env::set_var("APPDATA", &app_data_dir);
        std::env::set_var("LOCALAPPDATA", &local_app_data_dir);
        std::env::set_var("XDG_CONFIG_HOME", &xdg_config_dir);
        std::env::set_var("XDG_DATA_HOME", &xdg_data_dir);
        Self {
            home,
            user_profile,
            app_data,
            local_app_data,
            xdg_config_home,
            xdg_data_home,
            _path_root: path_root,
            _env_lock: env_lock,
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
        if let Some(value) = &self.user_profile {
            std::env::set_var("USERPROFILE", value);
        }
        if self.user_profile.is_none() {
            std::env::remove_var("USERPROFILE");
        }
        if let Some(value) = &self.app_data {
            std::env::set_var("APPDATA", value);
        }
        if self.app_data.is_none() {
            std::env::remove_var("APPDATA");
        }
        if let Some(value) = &self.local_app_data {
            std::env::set_var("LOCALAPPDATA", value);
        }
        if self.local_app_data.is_none() {
            std::env::remove_var("LOCALAPPDATA");
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
fn plugin_config_path_uses_shared_plugin_directory() {
    let root = TempDir::new().unwrap();
    let _env = ConfigEnvGuard::new(root.path());
    let install_id = "install-test-123";
    let active_path = crate::paths::base_data_dir()
        .unwrap()
        .join("active-install-id");
    fs::create_dir_all(active_path.parent().unwrap()).unwrap();
    fs::write(&active_path, format!("{install_id}\n")).unwrap();

    let path = PluginConfigManager::plugin_config_path("plugin-test").unwrap();

    assert!(!path.to_string_lossy().contains("installs"));
    assert!(!path.to_string_lossy().contains(install_id));
    assert!(path.ends_with(
        std::path::Path::new("qol-tray")
            .join("plugins")
            .join("plugin-test")
            .join("config.json")
    ));
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
    fs::create_dir_all(manager.store().core_plugin_configs_dir()).unwrap();
    fs::write(
        manager
            .store()
            .core_plugin_configs_dir()
            .join("plugin1.json"),
        serde_json::to_string(&test_data["plugin1"]).unwrap(),
    )
    .unwrap();
    fs::write(
        manager
            .store()
            .core_plugin_configs_dir()
            .join("plugin2.json"),
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
    assert!(manager.store().core_plugin_configs_dir().exists());
}

#[test]
fn save_configs_writes_pretty_json() {
    let (manager, _temp_base, _temp_plugins) = setup_test_env();
    let mut configs = PluginConfigs::default();
    configs
        .configs
        .insert("test".to_string(), json!({"key": "value"}));
    manager.save_configs(&configs).unwrap();
    let content =
        fs::read_to_string(manager.store().core_plugin_configs_dir().join("test.json")).unwrap();
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
fn get_config_returns_none_when_no_runtime_and_no_profile_slices() {
    let env_root = TempDir::new().unwrap();
    let _env = ConfigEnvGuard::new(env_root.path());
    let (manager, _temp_base, _temp_plugins) = setup_test_env();
    let result = manager.get_config_with("nonexistent", None, None).unwrap();
    assert!(result.is_none());
}

#[test]
fn get_config_restores_from_core_slice_when_runtime_missing() {
    let env_root = TempDir::new().unwrap();
    let _env = ConfigEnvGuard::new(env_root.path());
    let (manager, _temp_base, _temp_plugins) = setup_test_env();
    let expected_config = json!({"restored": true, "value": 123});
    fs::create_dir_all(manager.store().core_plugin_configs_dir()).unwrap();
    fs::write(
        manager
            .store()
            .core_plugin_configs_dir()
            .join("test-plugin.json"),
        serde_json::to_string(&expected_config).unwrap(),
    )
    .unwrap();
    let result = manager.get_config_with("test-plugin", None, None).unwrap();
    assert_eq!(
        result,
        Some(expected_config.clone()),
        "config in core slice must be returned when runtime path is missing"
    );
    let runtime = PluginConfigManager::plugin_config_path("test-plugin").unwrap();
    assert!(
        runtime.is_file(),
        "restored config must be written to runtime path so the daemon picks it up"
    );
    let runtime_value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&runtime).unwrap()).unwrap();
    assert_eq!(runtime_value, expected_config);
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

mod scoped_io {
    use super::*;
    use crate::features::profile::core::PluginLockEntry;
    use crate::plugins::manifest::{
        Capabilities, ConfigDeclarations, ConfigScope, MenuConfig, PluginInfo, PluginManifest,
    };
    use std::collections::HashMap;
    use std::path::Path;

    fn lock_entry(id: &str, platforms: Option<Vec<&str>>) -> PluginLockEntry {
        PluginLockEntry {
            id: id.to_string(),
            repo_url: "https://example/repo.git".to_string(),
            version: "1.0.0".to_string(),
            platforms: platforms.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
        }
    }

    fn manifest_with(
        platforms: Option<Vec<&str>>,
        default_scope: Option<ConfigScope>,
        per_field: &[(&str, ConfigScope)],
    ) -> PluginManifest {
        let config = ConfigDeclarations {
            default_scope,
            scope: per_field
                .iter()
                .map(|(k, s)| ((*k).to_string(), *s))
                .collect::<HashMap<_, _>>(),
        };
        PluginManifest {
            manifest_version: 1,
            plugin: PluginInfo {
                name: "p".to_string(),
                description: String::new(),
                version: "1.0.0".to_string(),
                author: None,
                platforms: platforms.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            },
            menu: MenuConfig {
                label: "p".to_string(),
                icon: None,
                items: vec![],
            },
            daemon: None,
            dependencies: None,
            runtime: None,
            capabilities: Capabilities::default(),
            build: Default::default(),
            traits: None,
            config,
        }
    }

    fn read_json(path: &Path) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn pre_seed(path: &Path, value: serde_json::Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }

    #[test]
    fn round_trips_single_field_without_manifest_into_core_slot() {
        let tmp = TempDir::new().unwrap();
        let profile = tmp.path();
        let store = store_at(profile, "linux");
        let config = json!({ "presets": ["a"] });

        save_plugin_config_split(&store, "plugin-x", &config, None, None).unwrap();
        let merged = load_plugin_config_merged(&store, "plugin-x", None, None).unwrap();
        assert_eq!(merged, config);
        assert!(profile.join("core/plugin-configs/plugin-x.json").is_file());
        assert!(!profile
            .join("os/linux/plugin-configs/plugin-x.json")
            .exists());
        assert!(!profile.join("device/plugin-configs/plugin-x.json").exists());
    }

    #[test]
    fn merge_precedence_is_core_then_os_then_device_with_later_scope_winning() {
        let tmp = TempDir::new().unwrap();
        let profile = tmp.path();
        let store = store_at(profile, "linux");
        pre_seed(
            &profile.join("core/plugin-configs/plugin-x.json"),
            json!({ "k": "from-core", "shared": "core" }),
        );
        pre_seed(
            &profile.join("os/linux/plugin-configs/plugin-x.json"),
            json!({ "k": "from-os", "os_only": true }),
        );
        pre_seed(
            &profile.join("device/plugin-configs/plugin-x.json"),
            json!({ "k": "from-device", "device_only": 42 }),
        );

        let merged = load_plugin_config_merged(&store, "plugin-x", None, None).unwrap();
        assert_eq!(merged["k"], json!("from-device"), "device wins for k");
        assert_eq!(merged["shared"], json!("core"));
        assert_eq!(merged["os_only"], json!(true));
        assert_eq!(merged["device_only"], json!(42));
    }

    #[test]
    fn save_routes_os_scoped_field_to_resolved_bucket_when_lock_pins_single_platform_from_another_os(
    ) {
        let tmp = TempDir::new().unwrap();
        let profile = tmp.path();
        let store = store_at(profile, "linux");
        let lock = lock_entry("plugin-keyremap", Some(vec!["macos"]));
        let manifest = manifest_with(Some(vec!["macos"]), None, &[("rules", ConfigScope::Os)]);
        let config = json!({ "rules": ["caps_to_ctrl"], "enabled": true });

        save_plugin_config_split(
            &store,
            "plugin-keyremap",
            &config,
            Some(&lock),
            Some(&manifest),
        )
        .unwrap();

        let os_macos = profile.join("os/macos/plugin-configs/plugin-keyremap.json");
        assert!(
            os_macos.is_file(),
            "OS-scoped field on a Mac-only plugin written from Linux must land in os/macos, not os/linux"
        );
        assert_eq!(read_json(&os_macos), json!({ "rules": ["caps_to_ctrl"] }));
        assert!(
            !profile
                .join("os/linux/plugin-configs/plugin-keyremap.json")
                .exists(),
            "must not create an os/linux slot for a Mac-only plugin"
        );
        let core_path = profile.join("core/plugin-configs/plugin-keyremap.json");
        assert_eq!(read_json(&core_path), json!({ "enabled": true }));
    }

    #[test]
    fn save_routes_explicit_device_scope_to_device_path() {
        let tmp = TempDir::new().unwrap();
        let profile = tmp.path();
        let store = store_at(profile, "linux");
        let manifest = manifest_with(None, None, &[("broker_url", ConfigScope::Device)]);
        let config = json!({ "broker_url": "10.0.0.1", "presets": ["a"] });

        save_plugin_config_split(&store, "plugin-x", &config, None, Some(&manifest)).unwrap();

        let device = profile.join("device/plugin-configs/plugin-x.json");
        assert_eq!(read_json(&device), json!({ "broker_url": "10.0.0.1" }));
        let core = profile.join("core/plugin-configs/plugin-x.json");
        assert_eq!(read_json(&core), json!({ "presets": ["a"] }));
    }

    #[test]
    fn save_does_not_touch_other_plugin_files_in_any_scope() {
        let tmp = TempDir::new().unwrap();
        let profile = tmp.path();
        let store = store_at(profile, "linux");
        pre_seed(
            &profile.join("core/plugin-configs/plugin-other.json"),
            json!({ "untouched": "core" }),
        );
        pre_seed(
            &profile.join("os/linux/plugin-configs/plugin-other.json"),
            json!({ "untouched": "os" }),
        );
        pre_seed(
            &profile.join("device/plugin-configs/plugin-other.json"),
            json!({ "untouched": "device" }),
        );

        save_plugin_config_split(&store, "plugin-x", &json!({ "x": 1 }), None, None).unwrap();

        assert_eq!(
            read_json(&profile.join("core/plugin-configs/plugin-other.json")),
            json!({ "untouched": "core" })
        );
        assert_eq!(
            read_json(&profile.join("os/linux/plugin-configs/plugin-other.json")),
            json!({ "untouched": "os" })
        );
        assert_eq!(
            read_json(&profile.join("device/plugin-configs/plugin-other.json")),
            json!({ "untouched": "device" })
        );
    }

    #[test]
    fn load_for_an_unknown_plugin_returns_anything_pre_existing_at_the_resolved_paths() {
        let tmp = TempDir::new().unwrap();
        let profile = tmp.path();
        let store = store_at(profile, "linux");
        pre_seed(
            &profile.join("core/plugin-configs/plugin-unknown.json"),
            json!({ "legacy_field": "preserved" }),
        );

        let merged = load_plugin_config_merged(&store, "plugin-unknown", None, None).unwrap();
        assert_eq!(
            merged,
            json!({ "legacy_field": "preserved" }),
            "unknown plugin with only a core file still loads cleanly"
        );
    }

    #[test]
    fn save_clears_a_slice_file_when_no_fields_in_that_scope_remain() {
        let tmp = TempDir::new().unwrap();
        let profile = tmp.path();
        let store = store_at(profile, "linux");
        let manifest = manifest_with(None, None, &[("broker_url", ConfigScope::Device)]);

        save_plugin_config_split(
            &store,
            "plugin-x",
            &json!({ "broker_url": "10.0.0.1", "presets": ["a"] }),
            None,
            Some(&manifest),
        )
        .unwrap();
        let device = profile.join("device/plugin-configs/plugin-x.json");
        assert!(device.is_file());

        save_plugin_config_split(
            &store,
            "plugin-x",
            &json!({ "presets": ["b"] }),
            None,
            Some(&manifest),
        )
        .unwrap();
        assert!(
            !device.exists(),
            "device slice becomes empty so the slice file must be removed to keep storage tidy"
        );
        assert_eq!(
            read_json(&profile.join("core/plugin-configs/plugin-x.json")),
            json!({ "presets": ["b"] })
        );
    }

    #[test]
    fn default_scope_routes_every_unspecified_field_to_that_scope() {
        let tmp = TempDir::new().unwrap();
        let profile = tmp.path();
        let store = store_at(profile, "macos");
        let manifest = manifest_with(None, Some(ConfigScope::Os), &[]);

        save_plugin_config_split(
            &store,
            "plugin-x",
            &json!({ "a": 1, "b": 2 }),
            None,
            Some(&manifest),
        )
        .unwrap();

        let os_file = profile.join("os/macos/plugin-configs/plugin-x.json");
        assert_eq!(read_json(&os_file), json!({ "a": 1, "b": 2 }));
        assert!(!profile.join("core/plugin-configs/plugin-x.json").exists());
    }

    #[test]
    fn write_then_read_round_trip_with_multi_scope_manifest_preserves_every_field() {
        let tmp = TempDir::new().unwrap();
        let profile = tmp.path();
        let store = store_at(profile, "macos");
        let manifest = manifest_with(
            Some(vec!["linux", "macos"]),
            None,
            &[
                ("hotkey", ConfigScope::Os),
                ("broker_url", ConfigScope::Device),
            ],
        );
        let lock = lock_entry("p", Some(vec!["linux", "macos"]));
        let config = json!({
            "presets": ["a", "b"],
            "hotkey": "Super+Space",
            "broker_url": "10.0.0.1",
            "deep": { "nested": true }
        });

        save_plugin_config_split(&store, "p", &config, Some(&lock), Some(&manifest)).unwrap();
        let merged = load_plugin_config_merged(&store, "p", Some(&lock), Some(&manifest)).unwrap();
        assert_eq!(merged, config, "round trip must preserve every field");
    }
}
