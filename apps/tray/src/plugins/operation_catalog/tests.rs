use super::*;
use crate::plugins::manifest::{
    ActionDeclaration, ActionType, BuildInfo, Capabilities, MenuConfig, PluginId, PluginManifest,
    PluginUid, RuntimeConfig,
};
use qol_config::contract::{PeerExposure, PeerReplay};
use qol_plugin_api::operations::{OperationKey, OperationKind};
use std::path::Path;
use tempfile::TempDir;

fn manifest(id: &str, uid: Option<&str>) -> PluginManifest {
    PluginManifest {
        manifest_version: crate::plugins::manifest::CURRENT_MANIFEST_VERSION,
        plugin: crate::plugins::manifest::PluginInfo {
            id: Some(PluginId::new(id)),
            uid: uid.map(PluginUid::new),
            name: "Test".to_string(),
            description: String::new(),
            version: "1.0.0".to_string(),
            author: None,
            platforms: None,
        },
        menu: MenuConfig {
            label: "Test".to_string(),
            icon: None,
            items: Vec::new(),
        },
        daemon: None,
        dependencies: None,
        runtime: None,
        actions: Default::default(),
        capabilities: Capabilities::default(),
        build: BuildInfo::default(),
        traits: None,
        shortcuts: Vec::new(),
        config: Default::default(),
        launcher: None,
    }
}

fn action(label: &str, peer: Option<PeerExposure>) -> ActionDeclaration {
    ActionDeclaration {
        label: label.to_string(),
        kind: ActionType::Run,
        continuous: false,
        args: Some(vec!["blink".to_string()]),
        config_key: None,
        checked: false,
        picture: None,
        hotkey: true,
        peer,
    }
}

fn write_runtime(root: &Path, source: &str) {
    std::fs::write(root.join("qol-runtime.toml"), source).unwrap();
}

fn plugin(dir: &TempDir, id: &str, uid: Option<&str>) -> Plugin {
    std::fs::write(dir.path().join("plugin-bin"), "fixture").unwrap();
    let mut manifest = manifest(id, uid);
    manifest.runtime = Some(RuntimeConfig {
        command: "plugin-bin".to_string(),
        actions: None,
    });
    Plugin::new(PluginId::new(id), manifest, dir.path().to_path_buf())
}

fn key(uid: &str, kind: OperationKind, name: &str) -> OperationKey {
    OperationKey::new(PluginUid::new(uid), kind, name)
}

#[test]
fn installed_catalog_derives_from_the_installed_contract() {
    let dir = TempDir::new().unwrap();
    let mut plugin = plugin(&dir, "qol-lights", Some("uid-lights"));
    plugin
        .manifest
        .actions
        .insert("blink".to_string(), action("Blink", None));
    write_runtime(
        dir.path(),
        "schema_version = 1\n[action.blink]\ndescription = \"Blink the light\"\n",
    );
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(plugin);

    let catalog = installed_catalog(&manager);

    let entry = catalog
        .get(&key("uid-lights", OperationKind::Action, "blink"))
        .expect("blink");
    assert_eq!(entry.description, "Blink the light");
    assert_eq!(entry.plugin_id.as_str(), "qol-lights");
}

#[test]
fn installed_catalog_keeps_activation_only_plugins() {
    let dir = TempDir::new().unwrap();
    let mut plugin = plugin(&dir, "qol-lights", Some("uid-lights"));
    plugin
        .manifest
        .actions
        .insert("blink".to_string(), action("Blink", None));
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(plugin);

    let catalog = installed_catalog(&manager);

    let entry = catalog
        .get(&key("uid-lights", OperationKind::Action, "blink"))
        .expect("blink");
    assert_eq!(entry.description, "Blink");
}

#[test]
fn installed_catalog_drops_a_plugin_with_a_malformed_contract() {
    let dir = TempDir::new().unwrap();
    let mut plugin = plugin(&dir, "qol-lights", Some("uid-lights"));
    plugin
        .manifest
        .actions
        .insert("blink".to_string(), action("Blink", None));
    write_runtime(dir.path(), "this is not a runtime contract");
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(plugin);

    let catalog = installed_catalog(&manager);

    assert!(catalog.is_empty());
}

#[test]
fn installed_catalog_skips_plugins_without_current_platform_support() {
    let dir = TempDir::new().unwrap();
    let mut plugin = plugin(&dir, "qol-lights", Some("uid-lights"));
    plugin.manifest.plugin.platforms = Some(vec!["not-a-real-os".to_string()]);
    plugin
        .manifest
        .actions
        .insert("blink".to_string(), action("Blink", None));
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(plugin);

    let catalog = installed_catalog(&manager);

    assert!(catalog.is_empty());
}

#[test]
fn installed_catalog_rejects_conflicting_peer_declarations() {
    let dir = TempDir::new().unwrap();
    let mut plugin = plugin(&dir, "qol-lights", Some("uid-lights"));
    plugin.manifest.actions.insert(
        "blink".to_string(),
        action(
            "Blink",
            Some(PeerExposure {
                replay: PeerReplay::Idempotent,
            }),
        ),
    );
    write_runtime(
        dir.path(),
        "schema_version = 1\n[action.blink]\ndescription = \"Blink\"\npeer = { replay = \"never\" }\n",
    );
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(plugin);

    let catalog = installed_catalog(&manager);

    assert!(catalog.is_empty());
}

#[test]
fn installed_catalog_keeps_distinct_plugins_apart() {
    let first_dir = TempDir::new().unwrap();
    let second_dir = TempDir::new().unwrap();
    let mut first = plugin(&first_dir, "qol-lights", Some("uid-lights"));
    first
        .manifest
        .actions
        .insert("blink".to_string(), action("Blink", None));
    let mut second = plugin(&second_dir, "qol-sound", Some("uid-sound"));
    second
        .manifest
        .actions
        .insert("blink".to_string(), action("Blink", None));
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(first);
    manager.insert_plugin_for_test(second);

    let catalog = installed_catalog(&manager);

    assert_eq!(catalog.len(), 2);
}

#[test]
fn duplicate_uids_quarantine_all_claimants_in_any_order() {
    for reverse in [false, true] {
        let first_dir = TempDir::new().unwrap();
        let second_dir = TempDir::new().unwrap();
        let mut first = plugin(&first_dir, "first", Some("shared-uid"));
        let mut second = plugin(&second_dir, "second", Some("shared-uid"));
        first
            .manifest
            .actions
            .insert("blink".into(), action("Blink", None));
        second
            .manifest
            .actions
            .insert("different".into(), action("Different", None));
        let plugins = if reverse {
            [second, first]
        } else {
            [first, second]
        };
        let mut manager = PluginManager::new();
        for plugin in plugins {
            manager.insert_plugin_for_test(plugin);
        }
        assert!(installed_catalog(&manager).is_empty(), "reverse={reverse}");
    }
}

#[test]
fn unavailable_execution_contracts_publish_no_operations() {
    for state in [
        "missing_binary",
        "missing_target",
        "invalid_manifest",
        "non_file_runtime",
    ] {
        let dir = TempDir::new().unwrap();
        let mut plugin = plugin(&dir, "fixture", Some("fixture-uid"));
        plugin
            .manifest
            .actions
            .insert("blink".into(), action("Blink", None));
        match state {
            "missing_binary" => plugin.manifest.runtime.as_mut().unwrap().command = "absent".into(),
            "missing_target" => plugin.manifest.runtime = None,
            "invalid_manifest" => plugin.manifest.actions["blink"].label.clear(),
            "non_file_runtime" => std::fs::create_dir(dir.path().join("qol-runtime.toml")).unwrap(),
            _ => unreachable!(),
        }
        let mut manager = PluginManager::new();
        manager.insert_plugin_for_test(plugin);
        assert!(installed_catalog(&manager).is_empty(), "{state}");
    }
}

fn daemon(root: &Path, enabled: bool) -> crate::plugins::manifest::DaemonConfig {
    crate::plugins::manifest::DaemonConfig {
        enabled,
        command: "plugin-bin".into(),
        socket: Some(root.join("fixture.sock").to_string_lossy().into_owned()),
        port: None,
        extra_ports: Vec::new(),
        inherit_listener: false,
    }
}

#[test]
fn typed_input_and_reads_require_an_enabled_daemon_target() {
    for enabled in [false, true] {
        let dir = TempDir::new().unwrap();
        let mut plugin = plugin(&dir, "fixture", Some("fixture-uid"));
        plugin
            .manifest
            .actions
            .insert("blink".into(), action("Blink", None));
        plugin.manifest.daemon = Some(daemon(dir.path(), enabled));
        write_runtime(
            dir.path(),
            r#"
schema_version = 1
[action.blink]
description = "Blink"
input = { target = "target" }
[query.status]
description = "Status"
poll_interval_ms = 1000
[stream.live]
description = "Control"
throttle_ms = 100
"#,
        );
        let mut manager = PluginManager::new();
        manager.insert_plugin_for_test(plugin);
        assert_eq!(
            installed_catalog(&manager).len(),
            if enabled { 3 } else { 0 },
            "enabled={enabled}"
        );
    }
}

#[test]
fn runtime_only_action_cannot_bypass_an_activation_catalog_allowlist() {
    let dir = TempDir::new().unwrap();
    let mut plugin = plugin(&dir, "fixture", Some("fixture-uid"));
    plugin
        .manifest
        .actions
        .insert("blink".into(), action("Blink", None));
    plugin.manifest.daemon = Some(daemon(dir.path(), true));
    write_runtime(
        dir.path(),
        "schema_version = 1\n[action.hidden]\ndescription = \"Hidden\"\nagent_tool = true\n",
    );
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(plugin);
    let catalog = installed_catalog(&manager);
    assert_eq!(catalog.len(), 1);
    assert!(catalog
        .iter()
        .all(|operation| operation.key.name == "blink"));
}

#[test]
fn a_changed_runtime_contract_is_not_cached_as_callable() {
    let dir = TempDir::new().unwrap();
    let mut plugin = plugin(&dir, "fixture", Some("fixture-uid"));
    plugin
        .manifest
        .actions
        .insert("blink".into(), action("Blink", None));
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(plugin);
    write_runtime(
        dir.path(),
        "schema_version = 1\n[action.blink]\ndescription = \"Blink\"\nagent_tool = true\n",
    );
    assert!(installed_catalog(&manager)
        .iter()
        .any(|operation| operation.agent_tool));
    write_runtime(dir.path(), "schema_version = 1\n[action.blink]\ndescription = \"Blink\"\npeer = { replay = \"never\", unexpected = true }\n");
    assert!(installed_catalog(&manager).is_empty());
}

#[test]
fn typed_hosted_settings_preserve_metadata_without_a_daemon() {
    let dir = TempDir::new().unwrap();
    let mut plugin = plugin(&dir, "fixture", Some("fixture-uid"));
    let mut settings = action("Settings", None);
    settings.kind = ActionType::Settings;
    settings.args = Some(vec!["--action".into(), "settings".into()]);
    plugin.manifest.actions.insert("settings".into(), settings);
    plugin.manifest.capabilities.gpui = true;
    std::fs::write(dir.path().join("qol-config.toml"), "").unwrap();
    write_runtime(
        dir.path(),
        "schema_version = 1\n[action.settings]\ndescription = \"Settings\"\ninput = { page = \"Page\" }\n",
    );
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(plugin);
    let catalog = installed_catalog(&manager);
    let settings = catalog
        .get(&key("fixture-uid", OperationKind::Action, "settings"))
        .unwrap();
    assert_eq!(settings.action_kind, Some(ActionType::Settings));
    assert_eq!(
        settings.runtime_args,
        Some(vec!["--action".into(), "settings".into()])
    );
    assert_eq!(settings.input.as_ref().unwrap()["page"], "Page");
    assert!(settings.peer.is_none());
    assert!(!settings.agent_tool);
}

#[test]
fn captured_catalog_resolves_fresh_runtime_declarations() {
    let dir = TempDir::new().unwrap();
    let mut plugin = plugin(&dir, "fixture", Some("fixture-uid"));
    plugin
        .manifest
        .actions
        .insert("blink".into(), action("Blink", None));
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(plugin);
    let capture = CatalogCapture::capture(&manager);
    let operation = key("fixture-uid", OperationKind::Action, "blink");
    write_runtime(dir.path(), "schema_version = 1\n[action.blink]\ndescription = \"First\"\npeer = { replay = \"never\" }\n");
    let first = capture.resolve().select(&operation).unwrap();
    assert_eq!(first.operation.description, "First");
    assert!(first.operation.peer.is_some());
    let capture = CatalogCapture::capture(&manager);
    write_runtime(
        dir.path(),
        "schema_version = 1\n[action.blink]\ndescription = \"Second\"\n",
    );
    let second = capture.resolve().select(&operation).unwrap();
    assert_eq!(second.operation.description, "Second");
    assert!(second.operation.peer.is_none());
    assert_ne!(
        first.operation.declaration_digest().unwrap(),
        second.operation.declaration_digest().unwrap()
    );
    assert!(first.matches_manager(&manager));
    assert!(second.matches_manager(&manager));
    let capture = CatalogCapture::capture(&manager);
    std::fs::remove_file(dir.path().join("plugin-bin")).unwrap();
    assert!(capture.resolve().select(&operation).is_none());
}

#[test]
fn selected_catalog_compares_live_manifest_source_path_and_uid_owners() {
    for change in ["manifest", "source", "path", "duplicate"] {
        let dir = TempDir::new().unwrap();
        let mut selected = plugin(&dir, "fixture", Some("fixture-uid"));
        selected
            .manifest
            .actions
            .insert("blink".into(), action("Blink", None));
        let manifest = selected.manifest.clone();
        let mut manager = PluginManager::new();
        manager.insert_plugin_for_test(selected);
        let resolved = CatalogCapture::capture(&manager).resolve();
        let selected = resolved
            .select(&key("fixture-uid", OperationKind::Action, "blink"))
            .unwrap();
        assert!(selected.matches_manager(&manager), "{change}");
        let mut replacement = Plugin::new(PluginId::new("fixture"), manifest, dir.path().into());
        match change {
            "manifest" => replacement.manifest.actions["blink"].label = "Changed".into(),
            "source" => replacement.source = PluginSource::DevLinked,
            "path" => replacement.path = dir.path().join("replacement"),
            "duplicate" => replacement.id = PluginId::new("duplicate"),
            _ => unreachable!(),
        }
        manager.insert_plugin_for_test(replacement);
        assert!(!selected.matches_manager(&manager), "{change}");
    }
}

#[test]
fn settings_projection_uses_canonical_peer_exposure_and_execution_filtering() {
    for enabled in [false, true] {
        let dir = TempDir::new().unwrap();
        let mut plugin = plugin(&dir, "fixture", Some("fixture-uid"));
        plugin.manifest.daemon = Some(daemon(dir.path(), enabled));
        plugin
            .manifest
            .actions
            .insert("blink".into(), action("Blink", None));
        write_runtime(
            dir.path(),
            r#"
schema_version = 1
[action.blink]
description = "Blink"
peer = { replay = "never" }
[query.status]
description = "Status"
poll_interval_ms = 1000
peer = { replay = "idempotent" }
[query.private]
description = "Private agent tool"
poll_interval_ms = 1000
agent_tool = true
[stream.live]
description = "Live"
throttle_ms = 100
peer = { replay = "never" }
"#,
        );
        let mut manager = PluginManager::new();
        manager.insert_plugin_for_test(plugin);
        let capture = CatalogCapture::capture(&manager);
        drop(manager);
        let resolved = capture.try_resolve().unwrap();
        let projection = crate::features::linked_devices::settings::catalog_operations(&resolved);
        let names: Vec<_> = projection
            .iter()
            .map(|item| item.key.name.as_str())
            .collect();
        assert_eq!(projection.len(), if enabled { 3 } else { 1 });
        assert!(names.contains(&"blink"));
        assert_eq!(names.contains(&"status"), enabled);
        assert_eq!(names.contains(&"live"), enabled);
        assert!(!names.contains(&"private"));
        for item in projection {
            assert!(resolved.select(&item.key).unwrap().operation.peer.is_some());
            assert_eq!(item.plugin_id.as_str(), "fixture");
        }
    }
}

#[test]
fn settings_catalog_read_errors_do_not_change_fail_closed_operation_selection() {
    for malformed in [false, true] {
        let dir = TempDir::new().unwrap();
        let mut plugin = plugin(&dir, "fixture", Some("fixture-uid"));
        plugin
            .manifest
            .actions
            .insert("blink".into(), action("Blink", None));
        let mut manager = PluginManager::new();
        manager.insert_plugin_for_test(plugin);
        if malformed {
            write_runtime(dir.path(), "invalid runtime contract");
        }
        if !malformed {
            std::fs::create_dir(dir.path().join("qol-runtime.toml")).unwrap();
        }
        assert!(CatalogCapture::capture(&manager).try_resolve().is_err());
        assert!(CatalogCapture::capture(&manager)
            .resolve()
            .select(&key("fixture-uid", OperationKind::Action, "blink"),)
            .is_none());
    }
}
