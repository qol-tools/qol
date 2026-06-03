use super::plugins_lock::build_plugins_lock;
use super::storage::{
    read_installed_plugin_configs_from_dir, read_plugin_configs_from_dirs,
    read_profile_plugin_configs_from_dir, replace_plugin_configs_in_dir,
    write_plugin_config_in_dir,
};
use super::*;
use crate::plugins::manifest::{Capabilities, MenuConfig, PluginInfo};
use crate::plugins::{Plugin, PluginId, PluginManifest, PluginSource};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

struct ConfigEnvGuard {
    home: Option<OsString>,
    user_profile: Option<OsString>,
    app_data: Option<OsString>,
    local_app_data: Option<OsString>,
    xdg_config_home: Option<OsString>,
    _path_root: crate::paths::TestPathRootGuard,
}

impl ConfigEnvGuard {
    fn new(root: &Path) -> Self {
        let path_root = crate::paths::push_test_path_root(root);
        let home = std::env::var_os("HOME");
        let user_profile = std::env::var_os("USERPROFILE");
        let app_data = std::env::var_os("APPDATA");
        let local_app_data = std::env::var_os("LOCALAPPDATA");
        let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
        let home_dir = root.join("home");
        let app_data_dir = root.join("app-data");
        let local_app_data_dir = root.join("local-app-data");
        let xdg_dir = root.join("xdg-config");
        fs::create_dir_all(&home_dir).unwrap();
        fs::create_dir_all(&app_data_dir).unwrap();
        fs::create_dir_all(&local_app_data_dir).unwrap();
        fs::create_dir_all(&xdg_dir).unwrap();
        std::env::set_var("HOME", &home_dir);
        std::env::set_var("USERPROFILE", &home_dir);
        std::env::set_var("APPDATA", &app_data_dir);
        std::env::set_var("LOCALAPPDATA", &local_app_data_dir);
        std::env::set_var("XDG_CONFIG_HOME", &xdg_dir);
        Self {
            home,
            user_profile,
            app_data,
            local_app_data,
            xdg_config_home,
            _path_root: path_root,
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
    }
}

async fn setup_profile_env() -> (
    tokio::sync::MutexGuard<'static, ()>,
    TempDir,
    ConfigEnvGuard,
    PathBuf,
) {
    let guard = crate::test_support::env_lock().lock().await;
    let root = TempDir::new().unwrap();
    let env = ConfigEnvGuard::new(root.path());
    let config_dir = crate::paths::shared_config_dir().unwrap();
    let plugins_dir = config_dir.join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();
    (guard, root, env, plugins_dir)
}

#[test]
fn import_plugins_prefers_explicit_lock_entries() {
    let bundle = ProfileImportBundle {
        plugins: vec![PluginLockEntry {
            id: "plugin-test".to_string(),
            repo_url: "https://github.com/qol-tools/plugin-test".to_string(),
            version: "1.2.3".to_string(),
            platforms: Some(vec!["linux".to_string()]),
        }],
        installed_plugins: vec!["plugin-ignored".to_string()],
        plugin_configs: Some(HashMap::new()),
        ..ProfileImportBundle::default()
    };

    let plugins = import_plugins(&bundle);

    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "plugin-test");
    assert_eq!(plugins[0].version, "1.2.3");
}

#[test]
fn import_plugins_falls_back_to_legacy_installed_plugins() {
    let bundle = ProfileImportBundle {
        installed_plugins: vec!["plugin-test".to_string()],
        ..ProfileImportBundle::default()
    };

    let plugins = import_plugins(&bundle);

    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "plugin-test");
    assert_eq!(
        plugins[0].repo_url,
        "https://github.com/qol-tools/plugin-test.git"
    );
    assert!(plugins[0].version.is_empty());
}

#[test]
fn profile_import_bundle_accepts_flat_and_legacy_list_shapes() {
    struct Case {
        name: &'static str,
        input: Value,
    }

    let cases = vec![
        Case {
            name: "flat arrays",
            input: json!({
                "hotkeys": [{"id": "hk-1"}],
                "shortcuts": [{"id": "sc-1"}]
            }),
        },
        Case {
            name: "legacy wrapped objects",
            input: json!({
                "hotkeys": {"hotkeys": [{"id": "hk-1"}]},
                "shortcuts": {"shortcuts": [{"id": "sc-1"}]}
            }),
        },
    ];

    for case in cases {
        let bundle: ProfileImportBundle = serde_json::from_value(case.input).unwrap();

        assert_eq!(
            bundle.hotkeys,
            Some(vec![json!({"id": "hk-1"})]),
            "case: {}",
            case.name
        );
        assert_eq!(
            bundle.shortcuts,
            Some(vec![json!({"id": "sc-1"})]),
            "case: {}",
            case.name
        );
    }
}

#[test]
fn profile_import_bundle_rejects_non_array_hotkeys_and_shortcuts() {
    let cases = vec![
        json!({"hotkeys": {"hotkeys": {}}}),
        json!({"shortcuts": {"shortcuts": {}}}),
    ];

    for input in cases {
        let result = serde_json::from_value::<ProfileImportBundle>(input);
        assert!(result.is_err());
    }
}

#[test]
fn profile_export_bundle_serializes_flat_hotkeys_and_shortcuts() {
    let bundle = ProfileExportBundle {
        version: CURRENT_PROFILE_VERSION,
        exported_at: "2026-03-28T00:00:00+00:00".to_string(),
        hotkeys: vec![json!({"id": "hk-1"})],
        shortcuts: vec![json!({"id": "sc-1"})],
        task_runner: json!({"actions": {}}),
        plugin_configs: HashMap::new(),
        plugins: Vec::new(),
    };

    let value = serde_json::to_value(bundle).unwrap();

    assert_eq!(value["hotkeys"], json!([{"id": "hk-1"}]));
    assert_eq!(value["shortcuts"], json!([{"id": "sc-1"}]));
}

#[test]
fn build_plugins_lock_preserves_unsupported_entries_and_resolves_repo_sources() {
    let existing = PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![
            PluginLockEntry {
                id: "plugin-existing".to_string(),
                repo_url: "https://example.com/existing.git".to_string(),
                version: "0.1.0".to_string(),
                platforms: None,
            },
            PluginLockEntry {
                id: "plugin-unsupported".to_string(),
                repo_url: "https://example.com/unsupported.git".to_string(),
                version: "9.9.9".to_string(),
                platforms: Some(vec![other_platform().to_string()]),
            },
            PluginLockEntry {
                id: "plugin-missing".to_string(),
                repo_url: "https://example.com/missing.git".to_string(),
                version: "4.5.6".to_string(),
                platforms: None,
            },
        ],
    };
    let cached_urls = HashMap::from([(
        "plugin-cached".to_string(),
        "https://example.com/cached.git".to_string(),
    )]);
    let plugins = [
        test_plugin("plugin-existing", "1.2.3", PluginSource::Installed, None),
        test_plugin("plugin-cached", "2.0.0", PluginSource::Installed, None),
        test_plugin("plugin-default", "3.0.0", PluginSource::Installed, None),
        test_plugin("plugin-dev", "8.8.8", PluginSource::DevLinked, None),
    ];

    let lock = build_plugins_lock(plugins.iter(), &existing, &cached_urls);
    let ids = lock
        .plugins
        .iter()
        .map(|plugin| plugin.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        ids,
        vec![
            "plugin-cached",
            "plugin-default",
            "plugin-dev",
            "plugin-existing",
            "plugin-unsupported",
        ]
    );
    assert_eq!(
        repo_url_for(&lock, "plugin-existing"),
        "https://example.com/existing.git"
    );
    assert_eq!(
        repo_url_for(&lock, "plugin-dev"),
        "https://github.com/qol-tools/plugin-dev.git"
    );
    assert_eq!(
        repo_url_for(&lock, "plugin-cached"),
        "https://example.com/cached.git"
    );
    assert_eq!(
        repo_url_for(&lock, "plugin-default"),
        "https://github.com/qol-tools/plugin-default.git"
    );
    assert_eq!(version_for(&lock, "plugin-existing"), "1.2.3");
    assert_eq!(version_for(&lock, "plugin-dev"), "8.8.8");
    assert_eq!(version_for(&lock, "plugin-unsupported"), "9.9.9");
    assert!(!lock
        .plugins
        .iter()
        .any(|plugin| plugin.id == "plugin-missing"));
}

#[test]
fn read_plugin_configs_from_dirs_cases() {
    struct Case {
        name: &'static str,
        profile_configs: Vec<(&'static str, Value)>,
        installed_configs: Vec<(&'static str, Value)>,
        expected: HashMap<String, Value>,
    }

    let cases = vec![
        Case {
            name: "profile wins over installed",
            profile_configs: vec![("plugin-test", json!({"source": "profile"}))],
            installed_configs: vec![
                ("plugin-test", json!({"source": "installed"})),
                ("plugin-extra", json!({"source": "installed"})),
            ],
            expected: HashMap::from([
                ("plugin-test".to_string(), json!({"source": "profile"})),
                ("plugin-extra".to_string(), json!({"source": "installed"})),
            ]),
        },
        Case {
            name: "installed configs are used when profile is empty",
            profile_configs: Vec::new(),
            installed_configs: vec![
                ("plugin-a", json!({"enabled": true})),
                ("plugin-b", json!({"count": 2})),
            ],
            expected: HashMap::from([
                ("plugin-a".to_string(), json!({"enabled": true})),
                ("plugin-b".to_string(), json!({"count": 2})),
            ]),
        },
    ];

    for case in cases {
        let tmp = TempDir::new().unwrap();
        let profile_configs_dir = tmp.path().join("profile");
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir_all(&profile_configs_dir).unwrap();

        for (plugin_id, config) in case.profile_configs {
            write_plugin_config_in_dir(&profile_configs_dir, plugin_id, &config).unwrap();
        }
        for (plugin_id, config) in case.installed_configs {
            write_installed_plugin_config(&plugins_dir, plugin_id, &config);
        }

        let configs = read_plugin_configs_from_dirs(&profile_configs_dir, &plugins_dir).unwrap();

        assert_eq!(configs, case.expected, "case: {}", case.name);
    }
}

#[test]
fn read_plugin_configs_from_dirs_merges_profile_entries_with_other_installed_configs() {
    let tmp = TempDir::new().unwrap();
    let profile_configs_dir = tmp.path().join("profile");
    let plugins_dir = tmp.path().join("plugins");
    fs::create_dir_all(&profile_configs_dir).unwrap();
    write_plugin_config_in_dir(
        &profile_configs_dir,
        "plugin-test",
        &json!({"source": "profile"}),
    )
    .unwrap();
    write_installed_plugin_config(&plugins_dir, "plugin-test", &json!({"source": "installed"}));
    write_installed_plugin_config(
        &plugins_dir,
        "plugin-extra",
        &json!({"source": "installed"}),
    );

    let configs = read_plugin_configs_from_dirs(&profile_configs_dir, &plugins_dir).unwrap();

    assert_eq!(
        configs,
        HashMap::from([
            ("plugin-test".to_string(), json!({"source": "profile"})),
            ("plugin-extra".to_string(), json!({"source": "installed"})),
        ])
    );
}

#[test]
fn read_profile_plugin_configs_from_dir_filters_invalid_entries() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    write_plugin_config_in_dir(dir, "plugin-valid", &json!({"ok": true})).unwrap();
    fs::write(dir.join("plugin-ignored.txt"), "{}").unwrap();
    fs::write(dir.join("bad name.json"), "{}").unwrap();
    fs::write(dir.join("plugin-broken.json"), "{").unwrap();
    fs::create_dir_all(dir.join("plugin-dir.json")).unwrap();

    let configs = read_profile_plugin_configs_from_dir(dir).unwrap();

    assert_eq!(
        configs,
        HashMap::from([("plugin-valid".to_string(), json!({"ok": true}))])
    );
}

#[test]
fn read_installed_plugin_configs_from_dir_filters_invalid_entries() {
    let tmp = TempDir::new().unwrap();
    let plugins_dir = tmp.path();
    write_installed_plugin_config(plugins_dir, "plugin-valid", &json!({"ok": true}));
    fs::create_dir_all(plugins_dir.join("plugin-no-config")).unwrap();
    fs::create_dir_all(plugins_dir.join("bad name")).unwrap();
    fs::write(
        crate::plugins::paths::config_path(&plugins_dir.join("bad name")),
        "{}",
    )
    .unwrap();
    fs::create_dir_all(plugins_dir.join("plugin-broken")).unwrap();
    fs::write(
        crate::plugins::paths::config_path(&plugins_dir.join("plugin-broken")),
        "{",
    )
    .unwrap();
    fs::write(plugins_dir.join("plain-file"), "x").unwrap();

    let configs = read_installed_plugin_configs_from_dir(plugins_dir)
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();

    assert_eq!(
        configs,
        HashMap::from([("plugin-valid".to_string(), json!({"ok": true}))])
    );
}

#[test]
fn replace_plugin_configs_removes_stale_profile_entries() {
    let tmp = TempDir::new().unwrap();
    let profile_configs_dir = tmp.path().join("profile");
    fs::create_dir_all(&profile_configs_dir).unwrap();
    write_plugin_config_in_dir(&profile_configs_dir, "plugin-old", &json!({"stale": true}))
        .unwrap();

    replace_plugin_configs_in_dir(
        &profile_configs_dir,
        &HashMap::from([("plugin-new".to_string(), json!({"fresh": true}))]),
    )
    .unwrap();

    assert!(!profile_configs_dir.join("plugin-old.json").exists());
    assert_eq!(
        crate::file_io::read_json::<Value>(&profile_configs_dir.join("plugin-new.json")).unwrap(),
        json!({"fresh": true})
    );
}

#[test]
fn replace_plugin_configs_rejects_invalid_plugin_ids() {
    let invalid_ids = ["../bad", "bad name", "", "."];

    for plugin_id in invalid_ids {
        let tmp = TempDir::new().unwrap();
        let profile_configs_dir = tmp.path().join("profile");
        fs::create_dir_all(&profile_configs_dir).unwrap();

        let result = replace_plugin_configs_in_dir(
            &profile_configs_dir,
            &HashMap::from([(plugin_id.to_string(), json!({"bad": true}))]),
        );

        assert!(result.is_err(), "plugin_id: {plugin_id}");
    }
}

#[tokio::test]
async fn apply_import_bundle_removes_live_plugin_configs_missing_from_profile() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    write_installed_plugin_config(&plugins_dir, "plugin-test", &json!({"stale": true}));

    let bundle = ProfileImportBundle {
        plugin_configs: Some(HashMap::new()),
        ..ProfileImportBundle::default()
    };

    apply_import_bundle(&plugins_dir, &bundle).await.unwrap();

    assert!(!crate::plugins::paths::config_path(&plugins_dir.join("plugin-test")).exists());
}

#[tokio::test]
async fn apply_import_bundle_rejects_wrong_typed_plugin_configs_without_mutating_state() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    write_plugin_contract(
        &plugins_dir,
        "plugin-test",
        r#"
schema_version = 1

[field.threshold]
type = "number"
default = 3
"#,
    );
    write_installed_plugin_config(&plugins_dir, "plugin-test", &json!({"threshold": 7}));
    save_plugin_config("plugin-test", &json!({"threshold": 7})).unwrap();
    save_plugins_lock(&PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: Vec::new(),
    })
    .unwrap();
    let install_repo = create_installable_plugin_repo(&plugins_dir, "plugin-install", None);

    let result = apply_import_bundle(
        &plugins_dir,
        &ProfileImportBundle {
            task_runner: Some(json!({"actions": {"sync": {}}})),
            plugins: vec![PluginLockEntry {
                id: "plugin-install".to_string(),
                repo_url: install_repo,
                version: String::new(),
                platforms: None,
            }],
            plugin_configs: Some(HashMap::from([(
                "plugin-test".to_string(),
                json!({"threshold": "3"}),
            )])),
            ..ProfileImportBundle::default()
        },
    )
    .await;

    let error = result.unwrap_err().to_string();
    assert!(error.contains("Invalid config for plugin-test"));
    assert!(error.contains("value does not match field type number"));
    assert!(!plugins_dir.join("plugin-install").exists());
    assert_eq!(
        load_plugin_config("plugin-test").unwrap(),
        Some(json!({"threshold": 7}))
    );
    assert!(load_plugins_lock().unwrap().plugins.is_empty());
    assert_eq!(
        crate::file_io::read_json::<Value>(&crate::plugins::paths::config_path(
            &plugins_dir.join("plugin-test")
        ))
        .unwrap(),
        json!({"threshold": 7})
    );
    assert!(!crate::paths::task_runner_config_path().unwrap().exists());
}

#[tokio::test]
async fn apply_import_bundle_rejects_wrong_typed_new_plugin_configs_before_install() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    save_plugins_lock(&PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: Vec::new(),
    })
    .unwrap();
    let install_repo = create_installable_plugin_repo(
        &plugins_dir,
        "plugin-install",
        Some(
            r#"
schema_version = 1

[field.threshold]
type = "number"
default = 3
"#,
        ),
    );

    let result = apply_import_bundle(
        &plugins_dir,
        &ProfileImportBundle {
            task_runner: Some(json!({"actions": {"sync": {}}})),
            plugins: vec![PluginLockEntry {
                id: "plugin-install".to_string(),
                repo_url: install_repo,
                version: String::new(),
                platforms: None,
            }],
            plugin_configs: Some(HashMap::from([(
                "plugin-install".to_string(),
                json!({"threshold": "3"}),
            )])),
            ..ProfileImportBundle::default()
        },
    )
    .await;

    let error = result.unwrap_err().to_string();
    assert!(error.contains("Invalid config for plugin-install"));
    assert!(error.contains("value does not match field type number"));
    assert!(!plugins_dir.join("plugin-install").exists());
    assert!(load_plugins_lock().unwrap().plugins.is_empty());
    assert!(!crate::paths::task_runner_config_path().unwrap().exists());
}

#[tokio::test]
async fn unsupported_profile_plugins_are_preserved_in_lock_and_sync_output() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    let bundle = ProfileImportBundle {
        plugins: vec![PluginLockEntry {
            id: "plugin-skip".to_string(),
            repo_url: "https://github.com/qol-tools/plugin-skip.git".to_string(),
            version: "1.2.3".to_string(),
            platforms: Some(vec![other_platform().to_string()]),
        }],
        ..ProfileImportBundle::default()
    };

    let result = apply_import_bundle(&plugins_dir, &bundle).await.unwrap();
    let lock = load_plugins_lock().unwrap();

    assert_eq!(result.plugins[0].status, "skipped");
    assert_eq!(
        lock.plugins,
        vec![PluginLockEntry {
            id: "plugin-skip".to_string(),
            repo_url: "https://github.com/qol-tools/plugin-skip.git".to_string(),
            version: "1.2.3".to_string(),
            platforms: Some(vec![other_platform().to_string()]),
        }]
    );
}

#[tokio::test]
async fn apply_import_bundle_preserves_existing_repo_urls_for_unlisted_installed_plugins() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    write_installed_plugin_manifest(&plugins_dir, "plugin-custom", "1.0.0");
    save_plugins_lock(&PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![PluginLockEntry {
            id: "plugin-custom".to_string(),
            repo_url: "https://example.com/custom.git".to_string(),
            version: "0.9.0".to_string(),
            platforms: None,
        }],
    })
    .unwrap();

    apply_import_bundle(&plugins_dir, &ProfileImportBundle::default())
        .await
        .unwrap();

    assert_eq!(
        load_plugins_lock().unwrap().plugins,
        vec![PluginLockEntry {
            id: "plugin-custom".to_string(),
            repo_url: "https://example.com/custom.git".to_string(),
            version: "1.0.0".to_string(),
            platforms: None,
        }]
    );
}

fn test_plugin(
    id: &str,
    version: &str,
    source: PluginSource,
    platforms: Option<Vec<String>>,
) -> Plugin {
    Plugin::new_with_source(
        PluginId::new(id),
        PluginManifest {
            manifest_version: crate::plugins::manifest::CURRENT_MANIFEST_VERSION,
            plugin: PluginInfo {
                id: Some(id.into()),
                name: id.to_string(),
                description: String::new(),
                version: version.to_string(),
                author: None,
                platforms,
            },
            menu: MenuConfig {
                label: id.to_string(),
                icon: None,
                items: Vec::new(),
            },
            daemon: None,
            dependencies: None,
            runtime: None,
            capabilities: Capabilities::default(),
            build: Default::default(),
            traits: None,
            config: Default::default(),
        },
        PathBuf::from(format!("/tmp/{id}")),
        source,
    )
}

fn repo_url_for<'a>(lock: &'a PluginsLock, plugin_id: &str) -> &'a str {
    lock.plugins
        .iter()
        .find(|plugin| plugin.id == plugin_id)
        .map(|plugin| plugin.repo_url.as_str())
        .unwrap()
}

fn version_for<'a>(lock: &'a PluginsLock, plugin_id: &str) -> &'a str {
    lock.plugins
        .iter()
        .find(|plugin| plugin.id == plugin_id)
        .map(|plugin| plugin.version.as_str())
        .unwrap()
}

fn write_installed_plugin_config(plugins_dir: &Path, plugin_id: &str, config: &Value) {
    let plugin_dir = plugins_dir.join(plugin_id);
    fs::create_dir_all(&plugin_dir).unwrap();
    crate::file_io::write_pretty_json(&crate::plugins::paths::config_path(&plugin_dir), config)
        .unwrap();
}

fn write_installed_plugin_manifest(plugins_dir: &Path, plugin_id: &str, version: &str) {
    let plugin_dir = plugins_dir.join(plugin_id);
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("plugin.toml"),
        format!(
            r#"
[plugin]
id = "{plugin_id}"
name = "{plugin_id}"
description = "Test plugin"
version = "{version}"

[menu]
label = "{plugin_id}"
items = []
"#
        ),
    )
    .unwrap();
}

fn write_plugin_contract(plugins_dir: &Path, plugin_id: &str, contract: &str) {
    let plugin_dir = plugins_dir.join(plugin_id);
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(plugin_dir.join("qol-config.toml"), contract).unwrap();
}

fn create_installable_plugin_repo(
    plugins_dir: &Path,
    plugin_id: &str,
    contract: Option<&str>,
) -> String {
    let repo_dir = plugins_dir.join(format!("{plugin_id}-repo"));
    fs::create_dir_all(&repo_dir).unwrap();
    fs::write(
        repo_dir.join("plugin.toml"),
        format!(
            r#"
[plugin]
id = "{plugin_id}"
name = "{plugin_id}"
description = "Test plugin"
version = "1.0.0"

[menu]
label = "{plugin_id}"
items = []
"#
        ),
    )
    .unwrap();
    if let Some(contract) = contract {
        fs::write(repo_dir.join("qol-config.toml"), contract).unwrap();
    }
    run_git(&repo_dir, ["-c", "init.defaultBranch=main", "init"]);
    run_git(&repo_dir, ["config", "user.email", "test@example.com"]);
    run_git(&repo_dir, ["config", "user.name", "Test User"]);
    run_git(&repo_dir, ["add", "."]);
    run_git(
        &repo_dir,
        ["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    repo_dir.display().to_string()
}

fn run_git<const N: usize>(repo_dir: &Path, args: [&str; N]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo_dir)
        .status()
        .unwrap();
    assert!(status.success(), "git command failed: {:?}", args);
}

fn other_platform() -> &'static str {
    ["linux", "macos", "windows"]
        .into_iter()
        .find(|platform| *platform != std::env::consts::OS)
        .unwrap()
}
