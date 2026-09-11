use super::import::{
    project_plugin_configs_to_dir, remove_live_plugin_configs_missing_from_profile,
    validate_imported_plugin_configs,
};
use super::plugins_lock::build_plugins_lock;
use super::storage::{
    read_installed_plugin_configs_from_dir, read_plugin_configs_from_dirs,
    read_profile_plugin_configs_from_dir, replace_plugin_configs_in_dir,
    write_plugin_config_in_dir, PluginUidIndex,
};
use super::*;
use crate::plugins::manifest::{Capabilities, MenuConfig, PluginInfo};
use crate::plugins::{Plugin, PluginId, PluginManifest, PluginSource, PluginUid};
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

struct TestPathEnvGuard {
    previous: Option<OsString>,
}

impl TestPathEnvGuard {
    fn new(root: &Path) -> Self {
        let previous = std::env::var_os("QOL_TRAY_TEST_PATH_ROOT");
        std::env::set_var("QOL_TRAY_TEST_PATH_ROOT", root);
        Self { previous }
    }
}

impl Drop for TestPathEnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var("QOL_TRAY_TEST_PATH_ROOT", previous);
            return;
        }
        std::env::remove_var("QOL_TRAY_TEST_PATH_ROOT");
    }
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

fn test_lock_entry(id: &str, uid: &str) -> PluginLockEntry {
    PluginLockEntry {
        uid: PluginUid::new(uid),
        id: id.to_string(),
        repo_url: format!("https://example.invalid/{id}.git"),
        version: "1.0.0".to_string(),
        platforms: None,
    }
}

#[test]
fn import_plugins_prefers_explicit_lock_entries() {
    let bundle = ProfileImportBundle {
        plugins: vec![PluginLockEntry {
            uid: PluginUid::new("plugin-test"),
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
fn concurrent_lock_updates_serialize_scope_and_preserve_unrelated_state() {
    let _env = crate::test_support::env_lock().blocking_lock();
    let root = TempDir::new().unwrap();
    let _path = TestPathEnvGuard::new(root.path());
    save_plugins_lock(&PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![
            test_lock_entry("plugin-a", "uid-a-old"),
            test_lock_entry("plugin-b", "uid-b-old"),
        ],
    })
    .unwrap();
    let baseline = crate::plugins::config::current_profile_config_generation();

    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let first = std::thread::spawn(move || {
        super::storage::update_plugins_lock(|previous| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            let mut next = previous.clone();
            next.plugins
                .iter_mut()
                .find(|entry| entry.id == "plugin-a")
                .unwrap()
                .uid = PluginUid::new("uid-a-new");
            Ok(next)
        })
    });
    entered_rx.recv().unwrap();

    let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
    let second = std::thread::spawn(move || {
        attempted_tx.send(()).unwrap();
        super::storage::update_plugins_lock(|previous| {
            let mut next = previous.clone();
            next.plugins
                .iter_mut()
                .find(|entry| entry.id == "plugin-b")
                .unwrap()
                .uid = PluginUid::new("uid-b-new");
            Ok(next)
        })
    });
    attempted_rx.recv().unwrap();
    release_tx.send(()).unwrap();

    let (_, first_generation) = first.join().unwrap().unwrap();
    let (_, second_generation) = second.join().unwrap().unwrap();

    let lock = load_plugins_lock().unwrap();
    assert_eq!(lock.plugins[0].uid, PluginUid::new("uid-a-new"));
    assert_eq!(lock.plugins[1].uid, PluginUid::new("uid-b-new"));
    assert!(first_generation > baseline);
    assert!(second_generation > first_generation);
}

#[test]
fn plugin_lock_diff_reports_only_changed_plugins() {
    let previous = PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![
            test_lock_entry("plugin-a", "uid-a-old"),
            test_lock_entry("plugin-b", "uid-b-stable"),
        ],
    };
    let mut next = previous.clone();
    next.plugins[0].uid = PluginUid::new("uid-a-new");

    assert_eq!(
        super::storage::changed_plugin_ids(&previous, &next),
        vec!["plugin-a".to_string()]
    );
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
fn import_plugins_dedups_bundle_entries_first_entry_wins() {
    let bundle = ProfileImportBundle {
        plugins: vec![
            test_lock_entry("plugin-a", "u-shared-0001"),
            test_lock_entry("plugin-b", "u-shared-0001"),
            test_lock_entry("plugin-a", "u-late-0002"),
        ],
        ..ProfileImportBundle::default()
    };

    let plugins = import_plugins(&bundle);

    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "plugin-a");
    assert_eq!(plugins[0].uid, PluginUid::new("u-shared-0001"));
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
                uid: PluginUid::new("plugin-existing"),
                id: "plugin-existing".to_string(),
                repo_url: "https://example.com/existing.git".to_string(),
                version: "0.1.0".to_string(),
                platforms: None,
            },
            PluginLockEntry {
                uid: PluginUid::new("plugin-unsupported"),
                id: "plugin-unsupported".to_string(),
                repo_url: "https://example.com/unsupported.git".to_string(),
                version: "9.9.9".to_string(),
                platforms: Some(vec![other_platform().to_string()]),
            },
            PluginLockEntry {
                uid: PluginUid::new("plugin-missing"),
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
        plugins: Vec<(&'static str, &'static str)>,
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
            plugins: Vec::new(),
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
            plugins: Vec::new(),
            expected: HashMap::from([
                ("plugin-a".to_string(), json!({"enabled": true})),
                ("plugin-b".to_string(), json!({"count": 2})),
            ]),
        },
        Case {
            name: "uid profile file wins over legacy id file",
            profile_configs: vec![
                ("u-real-0001", json!({"source": "uid"})),
                ("plugin-test", json!({"source": "legacy"})),
            ],
            installed_configs: Vec::new(),
            plugins: vec![("plugin-test", "u-real-0001")],
            expected: HashMap::from([("u-real-0001".to_string(), json!({"source": "uid"}))]),
        },
        Case {
            name: "legacy id profile file resolves to the plugin uid",
            profile_configs: vec![("plugin-test", json!({"source": "legacy"}))],
            installed_configs: Vec::new(),
            plugins: vec![("plugin-test", "u-real-0001")],
            expected: HashMap::from([("u-real-0001".to_string(), json!({"source": "legacy"}))]),
        },
        Case {
            name: "installed config resolves to the plugin uid",
            profile_configs: Vec::new(),
            installed_configs: vec![("plugin-locked", json!({"source": "locked"}))],
            plugins: vec![("plugin-locked", "u-real-0001")],
            expected: HashMap::from([("u-real-0001".to_string(), json!({"source": "locked"}))]),
        },
        Case {
            name: "installed config without a lock entry falls back to its id",
            profile_configs: Vec::new(),
            installed_configs: vec![("plugin-free", json!({"source": "free"}))],
            plugins: vec![("plugin-locked", "u-real-0001")],
            expected: HashMap::from([("plugin-free".to_string(), json!({"source": "free"}))]),
        },
        Case {
            name: "legacy id profile file wins over installed config for a lock uid",
            profile_configs: vec![("plugin-test", json!({"source": "legacy"}))],
            installed_configs: vec![("plugin-test", json!({"source": "installed"}))],
            plugins: vec![("plugin-test", "u-real-0001")],
            expected: HashMap::from([("u-real-0001".to_string(), json!({"source": "legacy"}))]),
        },
        Case {
            name: "orphan profile stem round-trips unchanged",
            profile_configs: vec![("plugin-orphan", json!({"source": "orphan"}))],
            installed_configs: Vec::new(),
            plugins: vec![("plugin-test", "u-real-0001")],
            expected: HashMap::from([("plugin-orphan".to_string(), json!({"source": "orphan"}))]),
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
        let plugins = case
            .plugins
            .iter()
            .map(|(id, uid)| test_lock_entry(id, uid))
            .collect::<Vec<_>>();
        let uid_index = PluginUidIndex::from_plugins(&plugins_dir, &plugins, &PluginsLock::empty());

        let configs =
            read_plugin_configs_from_dirs(&profile_configs_dir, &plugins_dir, &uid_index).unwrap();

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

    let uid_index = PluginUidIndex::from_plugins(&plugins_dir, &[], &PluginsLock::empty());
    let configs =
        read_plugin_configs_from_dirs(&profile_configs_dir, &plugins_dir, &uid_index).unwrap();

    assert_eq!(
        configs,
        HashMap::from([
            ("plugin-test".to_string(), json!({"source": "profile"})),
            ("plugin-extra".to_string(), json!({"source": "installed"})),
        ])
    );
}

#[test]
fn plugin_uid_index_refines_uid_from_installed_manifest() {
    struct Case {
        name: &'static str,
        entry: PluginLockEntry,
        expected_uid: &'static str,
    }

    let tmp = TempDir::new().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    write_manifest_with_uid(&plugins_dir, "plugin-lights", "u-real-0001", "1.0.0");

    let cases = vec![
        Case {
            name: "lock uid equals id and the manifest declares the real uid",
            entry: test_lock_entry("plugin-lights", "plugin-lights"),
            expected_uid: "u-real-0001",
        },
        Case {
            name: "a missing manifest keeps the lock uid",
            entry: test_lock_entry("plugin-absent", "plugin-absent"),
            expected_uid: "plugin-absent",
        },
        Case {
            name: "a lock uid different from id is kept as is",
            entry: test_lock_entry("plugin-lights", "u-lock-0001"),
            expected_uid: "u-lock-0001",
        },
    ];

    for case in cases {
        let index = PluginUidIndex::from_plugins(
            &plugins_dir,
            std::slice::from_ref(&case.entry),
            &PluginsLock::empty(),
        );
        assert_eq!(
            index.resolve_stem(case.entry.id.as_str()),
            case.expected_uid,
            "case: {}",
            case.name
        );
    }
}

#[test]
fn plugin_uid_index_unions_local_lock_entries_missing_from_bundle() {
    let tmp = TempDir::new().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    let stored = PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![test_lock_entry("plugin-locked", "u-locked-0001")],
    };

    let index = PluginUidIndex::from_plugins(&plugins_dir, &[], &stored);
    assert_eq!(index.resolve_stem("plugin-locked"), "u-locked-0001");
    assert_eq!(index.resolve_plugin_id("u-locked-0001"), "plugin-locked");

    let bundle = vec![test_lock_entry("plugin-locked", "u-bundle-0002")];
    let index = PluginUidIndex::from_plugins(&plugins_dir, &bundle, &stored);
    assert_eq!(index.resolve_stem("plugin-locked"), "u-bundle-0002");
}

#[test]
fn plugin_uid_index_skips_entries_with_unsafe_uid_or_id() {
    let tmp = TempDir::new().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    let plugins = vec![
        test_lock_entry("plugin-good", "u-good-0001"),
        test_lock_entry("plugin-bad-uid", "../evil"),
        test_lock_entry("bad name", "u-bad-id"),
    ];

    let index = PluginUidIndex::from_plugins(&plugins_dir, &plugins, &PluginsLock::empty());

    assert_eq!(index.resolve_stem("u-good-0001"), "u-good-0001");
    assert_eq!(index.resolve_stem("plugin-good"), "u-good-0001");
    assert_eq!(index.resolve_stem("plugin-bad-uid"), "plugin-bad-uid");
    assert_eq!(index.resolve_plugin_id("../evil"), "../evil");
    assert_eq!(index.resolve_plugin_id("u-bad-id"), "u-bad-id");
}

#[test]
fn plugin_uid_index_skips_unsafe_ids_before_manifest_reads() {
    let tmp = TempDir::new().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    write_manifest_with_uid(tmp.path(), "evil", "u-outside-0001", "1.0.0");
    let plugins = vec![
        test_lock_entry("../evil", "../evil"),
        test_lock_entry("plugin-safe", "../evil-uid"),
    ];

    let index = PluginUidIndex::from_plugins(&plugins_dir, &plugins, &PluginsLock::empty());

    assert!(tmp.path().join("evil").join("plugin.toml").exists());
    assert_eq!(index.resolve_stem("../evil"), "../evil");
    assert_eq!(index.resolve_plugin_id("../evil"), "../evil");
    assert_eq!(index.resolve_stem("u-outside-0001"), "u-outside-0001");
    assert_eq!(index.resolve_stem("plugin-safe"), "plugin-safe");
}

#[test]
fn plugin_uid_index_bundle_claims_uid_before_stored_entries() {
    let tmp = TempDir::new().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    let stored = PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![test_lock_entry("plugin-old", "u-claimed-0001")],
    };
    let bundle = vec![test_lock_entry("plugin-new", "u-claimed-0001")];

    let index = PluginUidIndex::from_plugins(&plugins_dir, &bundle, &stored);

    assert_eq!(index.resolve_plugin_id("u-claimed-0001"), "plugin-new");
    assert_eq!(index.resolve_stem("plugin-old"), "plugin-old");
}

#[test]
fn plugin_uid_index_maps_each_uid_at_most_once() {
    let tmp = TempDir::new().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    let bundle = vec![
        test_lock_entry("plugin-a", "u-one-0001"),
        test_lock_entry("plugin-b", "u-one-0001"),
    ];

    let index = PluginUidIndex::from_plugins(&plugins_dir, &bundle, &PluginsLock::empty());

    assert_eq!(index.resolve_plugin_id("u-one-0001"), "plugin-a");
    assert_eq!(index.resolve_stem("plugin-b"), "plugin-b");
}

#[test]
fn canonicalize_plugin_configs_prefers_uid_entries_and_keeps_orphans() {
    let tmp = TempDir::new().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    let index = PluginUidIndex::from_plugins(
        &plugins_dir,
        &[test_lock_entry("plugin-test", "u-real-0001")],
        &PluginsLock::empty(),
    );
    let expected = HashMap::from([
        ("u-real-0001".to_string(), json!({"value": 100})),
        ("plugin-orphan".to_string(), json!({"value": 7})),
    ]);

    for iteration in 0..8 {
        let mut configs = HashMap::new();
        if iteration % 2 == 0 {
            configs.insert("u-real-0001".to_string(), json!({"value": 100}));
            configs.insert("plugin-test".to_string(), json!({"value": 73}));
        } else {
            configs.insert("plugin-test".to_string(), json!({"value": 73}));
            configs.insert("u-real-0001".to_string(), json!({"value": 100}));
        }
        configs.insert("plugin-orphan".to_string(), json!({"value": 7}));

        let canonical = super::storage::canonicalize_plugin_configs(&configs, &index);

        assert_eq!(canonical, expected, "iteration: {iteration}");
    }
}

#[tokio::test]
async fn project_plugin_configs_resolves_uid_keys_to_plugin_ids() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    ensure_profile_dirs().unwrap();
    write_installed_plugin_config(&plugins_dir, "plugin-test", &json!({"threshold": 1}));
    let plugins = vec![test_lock_entry("plugin-test", "u-real-0001")];
    let uid_index = PluginUidIndex::from_plugins(&plugins_dir, &plugins, &PluginsLock::empty());
    let configs = HashMap::from([("u-real-0001".to_string(), json!({"threshold": 7}))]);

    project_plugin_configs_to_dir(&plugins_dir, Some(&configs), &uid_index).unwrap();

    assert_eq!(
        crate::file_io::read_json::<Value>(&crate::plugins::paths::config_path(
            &plugins_dir.join("plugin-test")
        ))
        .unwrap(),
        json!({"threshold": 7})
    );
}

#[test]
fn remove_live_plugin_configs_keeps_uid_keyed_profile_entries() {
    let tmp = TempDir::new().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    write_installed_plugin_config(&plugins_dir, "plugin-test", &json!({"threshold": 7}));
    let plugins = vec![test_lock_entry("plugin-test", "u-real-0001")];
    let uid_index = PluginUidIndex::from_plugins(&plugins_dir, &plugins, &PluginsLock::empty());
    let configs = HashMap::from([("u-real-0001".to_string(), json!({"threshold": 7}))]);

    remove_live_plugin_configs_missing_from_profile(&plugins_dir, &configs, &uid_index).unwrap();

    assert!(crate::plugins::paths::config_path(&plugins_dir.join("plugin-test")).exists());
}

#[test]
fn remove_live_plugin_configs_removes_absent_plugins_and_keeps_named_ones() {
    let tmp = TempDir::new().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    write_installed_plugin_config(&plugins_dir, "plugin-absent", &json!({"value": 1}));
    write_installed_plugin_config(&plugins_dir, "plugin-by-uid", &json!({"value": 2}));
    write_installed_plugin_config(&plugins_dir, "plugin-by-id", &json!({"value": 3}));
    write_installed_plugin_config(&plugins_dir, "plugin-orphan", &json!({"value": 4}));
    let plugins = vec![
        test_lock_entry("plugin-by-uid", "u-real-0001"),
        test_lock_entry("plugin-by-id", "u-real-0002"),
    ];
    let uid_index = PluginUidIndex::from_plugins(&plugins_dir, &plugins, &PluginsLock::empty());
    let configs = HashMap::from([
        ("u-real-0001".to_string(), json!({"value": 2})),
        ("plugin-by-id".to_string(), json!({"value": 3})),
        ("plugin-orphan".to_string(), json!({"value": 4})),
    ]);

    remove_live_plugin_configs_missing_from_profile(&plugins_dir, &configs, &uid_index).unwrap();

    assert!(!crate::plugins::paths::config_path(&plugins_dir.join("plugin-absent")).exists());
    assert!(crate::plugins::paths::config_path(&plugins_dir.join("plugin-by-uid")).exists());
    assert!(crate::plugins::paths::config_path(&plugins_dir.join("plugin-by-id")).exists());
    assert!(crate::plugins::paths::config_path(&plugins_dir.join("plugin-orphan")).exists());
}

#[tokio::test]
async fn project_plugin_configs_defers_removal_until_after_projection() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    ensure_profile_dirs().unwrap();
    write_installed_plugin_config(&plugins_dir, "plugin-absent", &json!({"value": 1}));
    let failing_dir = plugins_dir.join("plugin-failing");
    fs::create_dir_all(crate::plugins::paths::config_path(&failing_dir).join("nested")).unwrap();
    let plugins = vec![test_lock_entry("plugin-failing", "u-failing-0001")];
    let uid_index = PluginUidIndex::from_plugins(&plugins_dir, &plugins, &PluginsLock::empty());
    let configs = HashMap::from([("u-failing-0001".to_string(), json!({"value": 7}))]);

    let result = project_plugin_configs_to_dir(&plugins_dir, Some(&configs), &uid_index);

    let error = result.unwrap_err().to_string();
    assert!(
        error.contains("plugin-failing"),
        "projection error must name the failing plugin, got: {error}"
    );
    assert!(
        crate::plugins::paths::config_path(&plugins_dir.join("plugin-absent")).exists(),
        "removal must not run before projection"
    );
}

#[tokio::test]
async fn validate_imported_plugin_configs_resolves_uid_keys_before_lookup() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    write_installed_plugin_manifest(&plugins_dir, "plugin-test", "1.0.0");
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
    let plugins = vec![test_lock_entry("plugin-test", "u-real-0001")];
    let uid_index = PluginUidIndex::from_plugins(&plugins_dir, &plugins, &PluginsLock::empty());
    let configs = HashMap::from([("u-real-0001".to_string(), json!({"threshold": "three"}))]);

    let error =
        validate_imported_plugin_configs(&plugins_dir, Some(&configs), &plugins, &uid_index)
            .await
            .unwrap_err()
            .to_string();

    assert!(
        error.contains("Invalid config for plugin-test"),
        "expected validation under resolved id, got: {error}"
    );
    assert!(
        error.contains("value does not match field type number"),
        "expected type-mismatch detail, got: {error}"
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

#[cfg(unix)]
#[test]
fn read_installed_plugin_configs_from_dir_skips_symlinked_plugin_dirs() {
    let tmp = TempDir::new().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    let external_dir = tmp.path().join("external").join("plugin-linked");
    fs::create_dir_all(&plugins_dir).unwrap();
    write_installed_plugin_config(&plugins_dir, "plugin-valid", &json!({"ok": true}));
    fs::create_dir_all(&external_dir).unwrap();
    crate::file_io::write_pretty_json(
        &crate::plugins::paths::config_path(&external_dir),
        &json!({"from": "symlink"}),
    )
    .unwrap();
    std::os::unix::fs::symlink(&external_dir, plugins_dir.join("plugin-linked")).unwrap();

    let configs = read_installed_plugin_configs_from_dir(&plugins_dir)
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

#[test]
fn replace_plugin_configs_rejects_unsafe_keys_before_clearing() {
    let tmp = TempDir::new().unwrap();
    let profile_configs_dir = tmp.path().join("profile");
    fs::create_dir_all(&profile_configs_dir).unwrap();
    write_plugin_config_in_dir(&profile_configs_dir, "plugin-keep", &json!({"v": 1})).unwrap();

    let result = replace_plugin_configs_in_dir(
        &profile_configs_dir,
        &HashMap::from([("../evil".to_string(), json!({"value": 2}))]),
    );

    let error = result.unwrap_err().to_string();

    assert!(
        error.contains("../evil"),
        "error must name the key, got: {error}"
    );
    assert!(profile_configs_dir.join("plugin-keep.json").exists());
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
async fn apply_import_bundle_dual_keyed_configs_apply_the_uid_value() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    write_manifest_with_uid(&plugins_dir, "plugin-lights", "u-real-0001", "1.0.0");
    write_plugin_contract(
        &plugins_dir,
        "plugin-lights",
        r#"
schema_version = 1

[field.value]
type = "number"
default = 0
"#,
    );
    write_installed_plugin_config(&plugins_dir, "plugin-lights", &json!({"value": 50}));
    write_profile_plugin_config("plugin-lights", &json!({"value": 73}));
    let configs = HashMap::from([
        ("u-real-0001".to_string(), json!({"value": 100})),
        ("plugin-lights".to_string(), json!({"value": 73})),
    ]);
    let bundle = ProfileImportBundle {
        plugins: vec![test_lock_entry("plugin-lights", "u-real-0001")],
        plugin_configs: Some(configs),
        ..ProfileImportBundle::default()
    };

    let result = apply_import_bundle(&plugins_dir, &bundle).await.unwrap();

    assert!(result.success);
    assert_eq!(
        read_profile_plugin_config("u-real-0001"),
        Some(json!({"value": 100}))
    );
    assert_eq!(read_profile_plugin_config("plugin-lights"), None);
    assert_eq!(
        read_live_plugin_config(&plugins_dir, "plugin-lights"),
        json!({"value": 100})
    );
}

#[tokio::test]
async fn apply_import_bundle_uid_only_config_writes_uid_file_and_removes_legacy() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    write_manifest_with_uid(&plugins_dir, "plugin-lights", "u-real-0001", "1.0.0");
    write_plugin_contract(
        &plugins_dir,
        "plugin-lights",
        r#"
schema_version = 1

[field.value]
type = "number"
default = 0
"#,
    );
    write_installed_plugin_config(&plugins_dir, "plugin-lights", &json!({"value": 50}));
    write_profile_plugin_config("plugin-lights", &json!({"value": 73}));
    let configs = HashMap::from([("u-real-0001".to_string(), json!({"value": 7}))]);
    let bundle = ProfileImportBundle {
        plugins: vec![test_lock_entry("plugin-lights", "u-real-0001")],
        plugin_configs: Some(configs),
        ..ProfileImportBundle::default()
    };

    let result = apply_import_bundle(&plugins_dir, &bundle).await.unwrap();

    assert!(result.success);
    assert_eq!(
        read_profile_plugin_config("u-real-0001"),
        Some(json!({"value": 7}))
    );
    assert_eq!(read_profile_plugin_config("plugin-lights"), None);
    assert_eq!(
        read_live_plugin_config(&plugins_dir, "plugin-lights"),
        json!({"value": 7})
    );
}

#[tokio::test]
async fn apply_import_bundle_fresh_install_dual_keyed_configs_apply_the_uid_value() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    let source_repo = create_monorepo_style_source_repo(
        &plugins_dir,
        "plugin-lights",
        "1.0.0",
        Some("u-real-0001"),
        Some(
            r#"
schema_version = 1

[field.value]
type = "number"
default = 0
"#,
        ),
    );
    let _source_guard = crate::features::plugin_store::source::test_seam::install(vec![
        crate::features::plugin_store::source::PluginSource::new(
            "fixture",
            source_repo.repo.as_str(),
            "main",
        ),
    ]);
    let configs = HashMap::from([
        ("u-real-0001".to_string(), json!({"value": 100})),
        ("plugin-lights".to_string(), json!({"value": 73})),
    ]);
    let bundle = ProfileImportBundle {
        plugins: vec![PluginLockEntry {
            uid: PluginUid::new("plugin-lights"),
            id: "plugin-lights".to_string(),
            repo_url: source_repo.repo.clone(),
            version: "1.0.0".to_string(),
            platforms: None,
        }],
        plugin_configs: Some(configs),
        ..ProfileImportBundle::default()
    };

    let result = apply_import_bundle(&plugins_dir, &bundle).await.unwrap();

    assert!(result.success);
    assert_eq!(result.plugins[0].status, "install");
    assert_eq!(
        read_live_plugin_config(&plugins_dir, "plugin-lights"),
        json!({"value": 100})
    );
    assert_eq!(
        read_profile_plugin_config("u-real-0001"),
        Some(json!({"value": 100}))
    );
    assert_eq!(read_profile_plugin_config("plugin-lights"), None);
}

#[tokio::test]
async fn apply_import_bundle_rejects_unsafe_config_keys_before_any_write() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    ensure_profile_dirs().unwrap();
    write_profile_plugin_config("plugin-keep", &json!({"value": 1}));
    save_plugins_lock(&PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: Vec::new(),
    })
    .unwrap();
    let source_repo =
        create_monorepo_style_source_repo(&plugins_dir, "plugin-lights", "1.0.0", None, None);
    let _source_guard = crate::features::plugin_store::source::test_seam::install(vec![
        crate::features::plugin_store::source::PluginSource::new(
            "fixture",
            source_repo.repo.as_str(),
            "main",
        ),
    ]);

    let install_configs = HashMap::from([("../evil".to_string(), json!({"value": 2}))]);
    let result = apply_import_bundle(
        &plugins_dir,
        &ProfileImportBundle {
            task_runner: Some(json!({"actions": {"sync": {}}})),
            plugins: vec![PluginLockEntry {
                uid: PluginUid::new("plugin-lights"),
                id: "plugin-lights".to_string(),
                repo_url: source_repo.repo.clone(),
                version: "1.0.0".to_string(),
                platforms: None,
            }],
            plugin_configs: Some(install_configs),
            ..ProfileImportBundle::default()
        },
    )
    .await;

    let error = result.unwrap_err().to_string();

    assert!(
        error.contains("../evil"),
        "error must name the key, got: {error}"
    );
    assert!(
        error.contains("invalid plugin config key"),
        "error must come from the early key check, got: {error}"
    );
    assert_eq!(
        read_profile_plugin_config("plugin-keep"),
        Some(json!({"value": 1}))
    );
    assert!(!plugins_dir.join("plugin-lights").exists());
    assert!(load_plugins_lock().unwrap().plugins.is_empty());
    assert!(!crate::paths::task_runner_config_path().unwrap().exists());
}

#[tokio::test]
async fn build_export_bundle_prefers_uid_profile_configs_over_legacy_id_files() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    ensure_profile_dirs().unwrap();
    save_plugins_lock(&PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![test_lock_entry("plugin-lights", "u-real-0001")],
    })
    .unwrap();
    write_installed_plugin_config(&plugins_dir, "plugin-lights", &json!({"value": 42}));
    write_profile_plugin_config("u-real-0001", &json!({"value": 100}));
    write_profile_plugin_config("plugin-lights", &json!({"value": 73}));

    let plugins = vec![test_lock_entry("plugin-lights", "u-real-0001")];
    let bundle = build_export_bundle(String::new(), plugins.clone()).unwrap();

    assert_eq!(
        bundle.plugin_configs,
        HashMap::from([("u-real-0001".to_string(), json!({"value": 100}))])
    );
    assert!(!bundle.plugin_configs.contains_key("plugin-lights"));
    assert_eq!(
        read_plugin_configs(&plugins).unwrap(),
        bundle.plugin_configs
    );
}

#[tokio::test]
async fn build_export_bundle_refines_lock_uid_from_installed_manifest() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    ensure_profile_dirs().unwrap();
    write_manifest_with_uid(&plugins_dir, "plugin-lights", "u-real-0001", "1.0.0");
    save_plugins_lock(&PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![test_lock_entry("plugin-lights", "plugin-lights")],
    })
    .unwrap();
    write_profile_plugin_config("u-real-0001", &json!({"value": 100}));
    write_profile_plugin_config("plugin-lights", &json!({"value": 73}));

    let bundle = build_export_bundle(String::new(), Vec::new()).unwrap();

    assert_eq!(
        bundle.plugin_configs,
        HashMap::from([("u-real-0001".to_string(), json!({"value": 100}))])
    );
}

#[tokio::test]
async fn plugins_lock_for_export_agrees_with_exported_plugin_configs() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    ensure_profile_dirs().unwrap();
    write_manifest_with_uid(&plugins_dir, "plugin-lights", "u-real-0001", "1.0.0");
    let stored = PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![test_lock_entry("plugin-lights", "plugin-lights")],
    };
    save_plugins_lock(&stored).unwrap();
    write_profile_plugin_config("u-real-0001", &json!({"value": 100}));

    let lock = PluginsLock::for_export(&plugins_dir, std::iter::empty::<&Plugin>(), &stored);

    assert_eq!(lock.plugins.len(), 1);
    assert_eq!(lock.plugins[0].uid, PluginUid::new("u-real-0001"));

    let bundle = build_export_bundle(String::new(), lock.plugins.clone()).unwrap();

    assert_eq!(
        bundle.plugin_configs,
        HashMap::from([("u-real-0001".to_string(), json!({"value": 100}))])
    );
}

#[tokio::test]
async fn profile_drift_fixture_round_trips_uid_configs_into_a_clean_env() {
    let (_guard, root, env, plugins_dir) = setup_profile_env().await;
    ensure_profile_dirs().unwrap();
    write_manifest_with_uid(&plugins_dir, "plugin-lights", "u-real-0001", "1.0.0");
    write_installed_plugin_config(&plugins_dir, "plugin-lights", &json!({"value": 42}));
    write_profile_plugin_config("u-real-0001", &json!({"value": 100}));
    write_profile_plugin_config("plugin-lights", &json!({"value": 73}));
    let stored = PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![test_lock_entry("plugin-lights", "plugin-lights")],
    };
    save_plugins_lock(&stored).unwrap();

    let lock = PluginsLock::for_export(&plugins_dir, std::iter::empty::<&Plugin>(), &stored);
    assert_eq!(lock.plugins[0].uid, PluginUid::new("u-real-0001"));
    let exported = build_export_bundle(String::new(), lock.plugins).unwrap();
    assert_eq!(
        exported.plugin_configs,
        HashMap::from([("u-real-0001".to_string(), json!({"value": 100}))])
    );
    let serialized = serde_json::to_string(&exported).unwrap();
    let import_bundle: ProfileImportBundle = serde_json::from_str(&serialized).unwrap();

    drop(env);
    drop(root);
    let clean_root = TempDir::new().unwrap();
    let _clean_env = ConfigEnvGuard::new(clean_root.path());
    let clean_plugins_dir = crate::paths::shared_config_dir().unwrap().join("plugins");
    fs::create_dir_all(&clean_plugins_dir).unwrap();
    let source_repo = create_monorepo_style_source_repo(
        &clean_plugins_dir,
        "plugin-lights",
        "1.0.0",
        Some("u-real-0001"),
        Some(
            r#"
schema_version = 1

[field.value]
type = "number"
default = 0
"#,
        ),
    );
    let _source_guard = crate::features::plugin_store::source::test_seam::install(vec![
        crate::features::plugin_store::source::PluginSource::new(
            "fixture",
            source_repo.repo.as_str(),
            "main",
        ),
    ]);

    let result = apply_import_bundle(&clean_plugins_dir, &import_bundle)
        .await
        .unwrap();

    assert!(result.success);
    assert_eq!(
        read_profile_plugin_config("u-real-0001"),
        Some(json!({"value": 100}))
    );
    assert_eq!(read_profile_plugin_config("plugin-lights"), None);
    assert_eq!(
        read_live_plugin_config(&clean_plugins_dir, "plugin-lights"),
        json!({"value": 100})
    );
}

#[tokio::test]
async fn plugins_lock_for_export_unions_stored_entries_without_rewriting_the_lock() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    let stored = PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![
            PluginLockEntry {
                uid: PluginUid::new("u-stored-0001"),
                id: "plugin-stored".to_string(),
                repo_url: "https://example.com/stored.git".to_string(),
                version: "1.0.0".to_string(),
                platforms: None,
            },
            PluginLockEntry {
                uid: PluginUid::new("plugin-live"),
                id: "plugin-live".to_string(),
                repo_url: "https://example.com/live.git".to_string(),
                version: "0.9.0".to_string(),
                platforms: None,
            },
            PluginLockEntry {
                uid: PluginUid::new("u-unsupported-0001"),
                id: "plugin-unsupported".to_string(),
                repo_url: "https://example.com/unsupported.git".to_string(),
                version: "9.9.9".to_string(),
                platforms: Some(vec![other_platform().to_string()]),
            },
        ],
    };
    save_plugins_lock(&stored).unwrap();
    let lock_path = crate::paths::profile_plugins_lock_path().unwrap();
    let before = fs::read(&lock_path).unwrap();

    let live = [
        test_plugin("plugin-live", "2.0.0", PluginSource::Installed, None),
        test_plugin("plugin-added", "1.0.0", PluginSource::Installed, None),
    ];
    let lock = PluginsLock::for_export(&plugins_dir, live.iter(), &stored);

    let stored_entry = lock
        .plugins
        .iter()
        .find(|entry| entry.id == "plugin-stored")
        .expect("stored-only entry must be kept");
    assert_eq!(stored_entry.uid, PluginUid::new("u-stored-0001"));
    assert_eq!(stored_entry.repo_url, "https://example.com/stored.git");
    let live_entry = lock
        .plugins
        .iter()
        .find(|entry| entry.id == "plugin-live")
        .expect("live entry must be present");
    assert_eq!(live_entry.version, "2.0.0");
    assert_eq!(live_entry.repo_url, "https://example.com/live.git");
    assert_eq!(
        lock.plugins
            .iter()
            .filter(|entry| entry.id == "plugin-unsupported")
            .count(),
        1
    );
    let unsupported_entry = lock
        .plugins
        .iter()
        .find(|entry| entry.id == "plugin-unsupported")
        .expect("unsupported entry must be kept");
    assert_eq!(unsupported_entry.uid, PluginUid::new("u-unsupported-0001"));
    assert_eq!(
        unsupported_entry.repo_url,
        "https://example.com/unsupported.git"
    );
    assert_eq!(
        unsupported_entry.platforms,
        Some(vec![other_platform().to_string()])
    );
    assert!(lock.plugins.iter().any(|entry| entry.id == "plugin-added"));

    let after = fs::read(&lock_path).unwrap();
    assert_eq!(before, after, "export must not rewrite the lock file");
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
    write_profile_plugin_config("plugin-test", &json!({"threshold": 7}));
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
                uid: PluginUid::new("plugin-install"),
                id: "plugin-install".to_string(),
                repo_url: install_repo,
                version: String::new(),
                platforms: None,
            }],
            plugin_configs: Some(HashMap::from([(
                "plugin-test".to_string(),
                json!({"threshold": "three"}),
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
        read_profile_plugin_config("plugin-test").unwrap(),
        json!({"threshold": 7})
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
    let source_repo = create_monorepo_style_source_repo(
        &plugins_dir,
        "plugin-install",
        "0.0.1",
        None,
        Some(
            r#"
schema_version = 1

[field.threshold]
type = "number"
default = 3
"#,
        ),
    );
    let _source_guard = crate::features::plugin_store::source::test_seam::install(vec![
        crate::features::plugin_store::source::PluginSource::new(
            "fixture",
            source_repo.repo.as_str(),
            "main",
        ),
    ]);

    let result = apply_import_bundle(
        &plugins_dir,
        &ProfileImportBundle {
            task_runner: Some(json!({"actions": {"sync": {}}})),
            plugins: vec![PluginLockEntry {
                uid: PluginUid::new("plugin-install"),
                id: "plugin-install".to_string(),
                repo_url: source_repo.repo.clone(),
                version: "0.0.1".to_string(),
                platforms: None,
            }],
            plugin_configs: Some(HashMap::from([(
                "plugin-install".to_string(),
                json!({"threshold": "three"}),
            )])),
            ..ProfileImportBundle::default()
        },
    )
    .await;

    let error = result.unwrap_err().to_string();
    assert!(
        error.contains("Invalid config for plugin-install"),
        "expected contract rejection, got: {error}"
    );
    assert!(
        error.contains("value does not match field type number"),
        "expected type-mismatch detail, got: {error}"
    );
    assert!(
        !plugins_dir.join("plugin-install").exists(),
        "plugin must not be installed when import is rejected"
    );
    assert!(
        load_plugins_lock().unwrap().plugins.is_empty(),
        "lock must stay empty when import is rejected"
    );
    assert!(!crate::paths::task_runner_config_path().unwrap().exists());
}

#[tokio::test]
async fn unsupported_profile_plugins_are_preserved_in_lock_and_sync_output() {
    let (_guard, _root, _env, plugins_dir) = setup_profile_env().await;
    let bundle = ProfileImportBundle {
        plugins: vec![PluginLockEntry {
            uid: PluginUid::new("plugin-skip"),
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
            uid: PluginUid::new("plugin-skip"),
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
            uid: PluginUid::new("plugin-custom"),
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
            uid: PluginUid::new("plugin-custom"),
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
                uid: None,
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
            actions: Default::default(),
            capabilities: Capabilities::default(),
            build: Default::default(),
            traits: None,
            shortcuts: Vec::new(),
            config: Default::default(),
            launcher: None,
        },
        PathBuf::from(format!("/tmp/{id}")),
        source,
    )
}

#[test]
fn build_plugins_lock_coalesces_entries_sharing_same_uid() {
    let shared_uid = "u-shared-0001";
    let existing = PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![PluginLockEntry {
            uid: PluginUid::new(shared_uid),
            id: "plugin-old".to_string(),
            repo_url: "https://example.com/old.git".to_string(),
            version: "1.0.0".to_string(),
            platforms: Some(vec![other_platform().to_string()]),
        }],
    };
    let plugins = [test_plugin_with_uid(
        "plugin-new",
        shared_uid,
        "2.0.0",
        PluginSource::Installed,
        None,
    )];

    let lock = build_plugins_lock(plugins.iter(), &existing, &HashMap::new());

    let uids: Vec<&str> = lock.plugins.iter().map(|e| e.uid.as_str()).collect();
    assert_eq!(
        uids,
        vec![shared_uid],
        "expected exactly one entry for uid {shared_uid}"
    );

    let surviving_entry = lock.plugins.iter().find(|e| e.uid == shared_uid).unwrap();
    assert_eq!(
        surviving_entry.id, "plugin-new",
        "surviving entry must keep loaded plugin id"
    );
    assert_eq!(
        surviving_entry.version, "2.0.0",
        "surviving entry must keep loaded plugin version"
    );
}

#[test]
fn build_plugins_lock_keeps_distinct_uids() {
    let existing = PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: vec![PluginLockEntry {
            uid: PluginUid::new("u-alpha-0001"),
            id: "plugin-alpha".to_string(),
            repo_url: "https://example.com/alpha.git".to_string(),
            version: "1.0.0".to_string(),
            platforms: Some(vec![other_platform().to_string()]),
        }],
    };
    let plugins = [test_plugin_with_uid(
        "plugin-beta",
        "u-beta-0001",
        "2.0.0",
        PluginSource::Installed,
        None,
    )];

    let lock = build_plugins_lock(plugins.iter(), &existing, &HashMap::new());

    assert_eq!(lock.plugins.len(), 2, "both distinct uids must be present");
}

fn test_plugin_with_uid(
    id: &str,
    uid: &str,
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
                uid: Some(PluginUid::new(uid)),
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
            actions: Default::default(),
            capabilities: Capabilities::default(),
            build: Default::default(),
            traits: None,
            shortcuts: Vec::new(),
            config: Default::default(),
            launcher: None,
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

fn write_profile_plugin_config(plugin_id: &str, config: &Value) {
    let dir = crate::paths::profile_plugin_configs_dir().unwrap();
    fs::create_dir_all(&dir).unwrap();
    crate::file_io::write_pretty_json(&dir.join(format!("{plugin_id}.json")), config).unwrap();
}

fn read_profile_plugin_config(plugin_id: &str) -> Option<Value> {
    let path = crate::paths::profile_plugin_configs_dir()
        .unwrap()
        .join(format!("{plugin_id}.json"));
    if !path.exists() {
        return None;
    }
    Some(crate::file_io::read_json(&path).unwrap())
}

fn write_installed_plugin_config(plugins_dir: &Path, plugin_id: &str, config: &Value) {
    let plugin_dir = plugins_dir.join(plugin_id);
    fs::create_dir_all(&plugin_dir).unwrap();
    crate::file_io::write_pretty_json(&crate::plugins::paths::config_path(&plugin_dir), config)
        .unwrap();
}

fn read_live_plugin_config(plugins_dir: &Path, plugin_id: &str) -> Value {
    let path = crate::plugins::paths::config_path(&plugins_dir.join(plugin_id));
    crate::file_io::read_json::<Value>(&path).unwrap()
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

fn write_manifest_with_uid(plugins_dir: &Path, plugin_id: &str, uid: &str, version: &str) {
    let plugin_dir = plugins_dir.join(plugin_id);
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("plugin.toml"),
        format!(
            r#"
[plugin]
id = "{plugin_id}"
uid = "{uid}"
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

struct MonorepoSourceRepo {
    repo: String,
}

fn create_monorepo_style_source_repo(
    plugins_dir: &Path,
    plugin_id: &str,
    version: &str,
    uid: Option<&str>,
    contract: Option<&str>,
) -> MonorepoSourceRepo {
    let repo_dir = plugins_dir.join(format!("{plugin_id}-source-monorepo"));
    let source_directory = plugin_id.strip_prefix("plugin-").unwrap_or(plugin_id);
    let plugin_subdir = repo_dir.join("plugins").join(source_directory);
    let uid_line = uid
        .map(|uid| format!("uid = \"{uid}\"\n"))
        .unwrap_or_default();
    fs::create_dir_all(&plugin_subdir).unwrap();
    fs::write(
        plugin_subdir.join("plugin.toml"),
        format!(
            r#"
[plugin]
id = "{plugin_id}"
{uid_line}name = "{plugin_id}"
description = "Test plugin"
version = "{version}"

[menu]
label = "{plugin_id}"
items = []
"#
        ),
    )
    .unwrap();
    if let Some(contract) = contract {
        fs::write(plugin_subdir.join("qol-config.toml"), contract).unwrap();
    }
    run_git(&repo_dir, ["-c", "init.defaultBranch=main", "init"]);
    run_git(&repo_dir, ["config", "user.email", "test@example.com"]);
    run_git(&repo_dir, ["config", "user.name", "Test User"]);
    run_git(&repo_dir, ["config", "commit.gpgsign", "false"]);
    run_git(&repo_dir, ["add", "."]);
    run_git(
        &repo_dir,
        ["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    let tag = format!("{plugin_id}-v{version}");
    run_git(&repo_dir, ["tag", tag.as_str()]);
    MonorepoSourceRepo {
        repo: repo_dir.display().to_string(),
    }
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

#[test]
fn plugin_lock_entry_deserializes_old_schema_without_uid_backfills_uid_from_id() {
    let cases = [
        (
            r#"{"id":"plugin-a","repo_url":"https://example.com/a.git","version":"1.0.0"}"#,
            "plugin-a",
            "old-schema: uid absent, backfills uid=id",
        ),
        (
            r#"{"uid":"u-real-0001","id":"plugin-a","repo_url":"https://example.com/a.git","version":"1.0.0"}"#,
            "u-real-0001",
            "new-schema: uid present, uses given uid",
        ),
    ];
    for (json, expected_uid, label) in cases {
        let entry: PluginLockEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.uid.as_str(), expected_uid, "case: {label}");
    }
}

#[test]
fn plugin_lock_entry_serializes_uid_field() {
    let entry = PluginLockEntry {
        uid: PluginUid::new("u-real-0001"),
        id: "plugin-a".to_string(),
        repo_url: "https://example.com/a.git".to_string(),
        version: "1.0.0".to_string(),
        platforms: None,
    };
    let value = serde_json::to_value(&entry).unwrap();
    assert_eq!(value["uid"], "u-real-0001", "uid must be emitted");
    assert_eq!(value["id"], "plugin-a");
}

#[test]
fn plugin_lock_entry_round_trips_through_json() {
    let original = PluginLockEntry {
        uid: PluginUid::new("u-real-0001"),
        id: "plugin-a".to_string(),
        repo_url: "https://example.com/a.git".to_string(),
        version: "2.3.4".to_string(),
        platforms: Some(vec!["linux".to_string()]),
    };
    let serialized = serde_json::to_string(&original).unwrap();
    let deserialized: PluginLockEntry = serde_json::from_str(&serialized).unwrap();
    assert_eq!(original, deserialized);
}
