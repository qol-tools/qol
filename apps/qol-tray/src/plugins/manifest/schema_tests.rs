use super::*;
use serde::Deserialize;

fn make_plugin_info(platforms: Option<Vec<&str>>) -> PluginInfo {
    PluginInfo {
        name: "Test".to_string(),
        description: "Test".to_string(),
        version: "1.0.0".to_string(),
        author: None,
        platforms: platforms.map(|items| items.into_iter().map(String::from).collect()),
    }
}

#[test]
fn supports_current_platform_cases() {
    let current_os = std::env::consts::OS;
    let cases: &[(Option<Vec<&str>>, bool)] = &[
        (None, true),
        (Some(vec![]), false),
        (Some(vec![current_os]), true),
        (Some(vec!["not-a-real-os"]), false),
        (Some(vec!["linux", "windows", "macos"]), true),
        (Some(vec!["fake1", "fake2"]), false),
        (Some(vec!["LINUX", "WINDOWS"]), false),
        (Some(vec![" linux"]), false),
        (Some(vec!["linux "]), false),
    ];

    for (platforms, expected) in cases {
        let info = make_plugin_info(platforms.clone());
        assert_eq!(info.supports_current_platform(), *expected);
    }
}

#[test]
fn parse_action_menu_item() {
    let toml = r#"
        type = "action"
        id = "run"
        label = "Run Script"
        action = "run"
    "#;

    let item: MenuItem = toml::from_str(toml).unwrap();
    match item {
        MenuItem::Action {
            id,
            label,
            action,
            config_key,
        } => {
            assert_eq!(id, "run");
            assert_eq!(label, "Run Script");
            assert_eq!(action, ActionType::Run);
            assert!(config_key.is_none());
        }
        _ => panic!("Expected Action"),
    }
}

#[test]
fn parse_checkbox_menu_item() {
    let toml = r#"
        type = "checkbox"
        id = "enabled"
        label = "Enable Feature"
        checked = true
        action = "toggle-config"
        config_key = "feature.enabled"
    "#;

    let item: MenuItem = toml::from_str(toml).unwrap();
    match item {
        MenuItem::Checkbox {
            id,
            label,
            checked,
            action,
            config_key,
        } => {
            assert_eq!(id, "enabled");
            assert_eq!(label, "Enable Feature");
            assert!(checked);
            assert_eq!(action, ActionType::ToggleConfig);
            assert_eq!(config_key, Some("feature.enabled".to_string()));
        }
        _ => panic!("Expected Checkbox"),
    }
}

#[test]
fn parse_separator() {
    let item: MenuItem = toml::from_str(r#"type = "separator""#).unwrap();
    assert!(matches!(item, MenuItem::Separator));
}

#[test]
fn parse_submenu() {
    let toml = r#"
        type = "submenu"
        id = "more"
        label = "More Options"
        items = [
            { type = "action", id = "a", label = "A", action = "run" },
            { type = "separator" },
        ]
    "#;

    let item: MenuItem = toml::from_str(toml).unwrap();
    match item {
        MenuItem::Submenu { id, label, items } => {
            assert_eq!(id, "more");
            assert_eq!(label, "More Options");
            assert_eq!(items.len(), 2);
        }
        _ => panic!("Expected Submenu"),
    }
}

#[test]
fn parse_action_type_cases() {
    let cases = [
        ("run", ActionType::Run),
        ("settings", ActionType::Settings),
        ("toggle-config", ActionType::ToggleConfig),
    ];

    for (input, expected) in cases {
        let toml = format!(r#"action = "{}""#, input);
        #[derive(Deserialize)]
        struct Wrapper {
            action: ActionType,
        }
        let wrapper: Wrapper = toml::from_str(&toml).unwrap();
        assert_eq!(wrapper.action, expected);
    }
}

#[test]
fn parse_full_manifest() {
    let toml = r#"
        [plugin]
        name = "Test Plugin"
        description = "A test"
        version = "1.2.3"
        author = "Test Author"
        platforms = ["linux", "windows"]

        [menu]
        label = "Test Menu"
        icon = "test.png"
        items = [
            { type = "action", id = "run", label = "Run", action = "run" },
        ]

        [daemon]
        enabled = true
        command = "daemon"
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert_eq!(manifest.plugin.name, "Test Plugin");
    assert_eq!(manifest.plugin.version, "1.2.3");
    assert_eq!(manifest.plugin.author, Some("Test Author".to_string()));
    assert_eq!(
        manifest.plugin.platforms,
        Some(vec!["linux".to_string(), "windows".to_string()])
    );
    assert_eq!(manifest.menu.label, "Test Menu");
    assert_eq!(manifest.menu.icon, Some("test.png".to_string()));
    assert_eq!(manifest.menu.items.len(), 1);
    assert!(manifest.daemon.is_some());
    let daemon = manifest.daemon.unwrap();
    assert!(daemon.enabled);
    assert_eq!(daemon.command, "daemon");
    assert!(daemon.socket.is_none());
}

#[test]
fn parse_minimal_manifest() {
    let toml = r#"
        [plugin]
        name = "Minimal"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = []
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert_eq!(manifest.plugin.name, "Minimal");
    assert!(manifest.plugin.author.is_none());
    assert!(manifest.plugin.platforms.is_none());
    assert!(manifest.daemon.is_none());
    assert!(!manifest.capabilities.serial);
    assert!(manifest.menu.items.is_empty());
}

#[test]
fn parse_capabilities_section() {
    let toml = r#"
        [plugin]
        name = "Serial Plugin"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = []

        [capabilities]
        serial = true
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.capabilities.serial);
}

#[test]
fn parse_forward_compat_unknown_capability() {
    // A newer qol-plugin-api may add capabilities (e.g. restore-rule) that the
    // running qol-tray binary does not yet know. Unknown entries must parse
    // into `extras` rather than rejecting the whole manifest, so plugins keep
    // loading across schema gaps.
    let toml = r#"
        [plugin]
        name = "Forward"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = []

        [capabilities]
        serial = true

        [capabilities.restore-rule]
        templates = ["terminal-pane"]
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.capabilities.serial);
    assert!(manifest.capabilities.extras.contains_key("restore-rule"));
}

#[test]
fn parse_runtime_config() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = []

        [runtime]
        command = "launcher"
        actions = { run = ["show"], settings = ["config"] }
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    let runtime = manifest.runtime.unwrap();
    assert_eq!(runtime.command, "launcher");
    let actions = runtime.actions.unwrap();
    assert_eq!(actions["run"], vec!["show"]);
    assert_eq!(actions["settings"], vec!["config"]);
}

#[test]
fn parse_runtime_without_actions() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = []

        [runtime]
        command = "launcher"
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    let runtime = manifest.runtime.unwrap();
    assert_eq!(runtime.command, "launcher");
    assert!(runtime.actions.is_none());
}

#[test]
fn parse_daemon_socket_config() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = []

        [daemon]
        enabled = true
        command = "daemon"
        socket = "/tmp/qol-p.sock"
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    let daemon = manifest.daemon.unwrap();
    assert_eq!(daemon.socket, Some("/tmp/qol-p.sock".to_string()));
}

#[test]
fn manifest_without_runtime_is_none() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = []
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.runtime.is_none());
}

#[test]
fn manifest_version_defaults_to_current() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = []
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert_eq!(manifest.manifest_version, CURRENT_MANIFEST_VERSION);
}

#[test]
fn checkbox_defaults_to_unchecked() {
    let toml = r#"
        type = "checkbox"
        id = "x"
        label = "X"
        action = "toggle-config"
    "#;

    let item: MenuItem = toml::from_str(toml).unwrap();
    match item {
        MenuItem::Checkbox { checked, .. } => assert!(!checked),
        _ => panic!("Expected Checkbox"),
    }
}

const TRAITS_MANIFEST_HEAD: &str = r#"
[plugin]
name = "Traits Plugin"
description = ""
version = "0.0.0"

[menu]
label = "Traits"
items = []
"#;

#[test]
fn parse_manifest_without_traits_yields_none() {
    let manifest: PluginManifest = toml::from_str(TRAITS_MANIFEST_HEAD).unwrap();
    assert!(manifest.traits.is_none());
}

#[test]
fn manifest_without_config_block_has_empty_scope_map() {
    let manifest: PluginManifest = toml::from_str(TRAITS_MANIFEST_HEAD).unwrap();
    assert!(manifest.config.scope.is_empty());
}

#[test]
fn manifest_config_scope_parses_each_variant() {
    let toml = format!(
        r#"{TRAITS_MANIFEST_HEAD}
[config.scope]
broker_url = "device"
hotkey     = "os"
presets    = "core"
"#
    );
    let manifest: PluginManifest = toml::from_str(&toml).unwrap();
    let cases = [
        ("broker_url", ConfigScope::Device),
        ("hotkey", ConfigScope::Os),
        ("presets", ConfigScope::Core),
    ];
    for (field, expected) in cases {
        assert_eq!(manifest.config.scope_for(field), expected, "field {field}");
    }
}

#[test]
fn config_scope_for_unlisted_field_returns_core_by_default() {
    let manifest: PluginManifest = toml::from_str(TRAITS_MANIFEST_HEAD).unwrap();
    assert_eq!(manifest.config.scope_for("anything"), ConfigScope::Core);
}

#[test]
fn manifest_rejects_unknown_config_scope_value() {
    let toml = format!(
        r#"{TRAITS_MANIFEST_HEAD}
[config.scope]
broker_url = "global"
"#
    );
    assert!(toml::from_str::<PluginManifest>(&toml).is_err());
}

#[test]
fn manifest_accepts_legacy_any_string_as_alias_for_core() {
    let toml = format!(
        r#"{TRAITS_MANIFEST_HEAD}
[config.scope]
presets = "any"
"#
    );
    let manifest: PluginManifest = toml::from_str(&toml).unwrap();
    assert_eq!(
        manifest.config.scope_for("presets"),
        ConfigScope::Core,
        "legacy `any` value must keep parsing as Core so old in-tree manifests do not regress"
    );
}

#[test]
fn manifest_default_scope_applies_to_fields_without_individual_declaration() {
    let toml = format!(
        r#"{TRAITS_MANIFEST_HEAD}
[config]
default_scope = "os"
"#
    );
    let manifest: PluginManifest = toml::from_str(&toml).unwrap();
    assert_eq!(
        manifest.config.scope_for("any_unlisted_field"),
        ConfigScope::Os,
        "plugin-level default_scope is the fallback when a field is not in [config.scope]"
    );
}

#[test]
fn manifest_per_field_scope_overrides_default_scope() {
    let toml = format!(
        r#"{TRAITS_MANIFEST_HEAD}
[config]
default_scope = "os"

[config.scope]
broker_url = "device"
"#
    );
    let manifest: PluginManifest = toml::from_str(&toml).unwrap();
    assert_eq!(
        manifest.config.scope_for("broker_url"),
        ConfigScope::Device,
        "explicit per-field scope must win over plugin-level default_scope"
    );
    assert_eq!(
        manifest.config.scope_for("other_field"),
        ConfigScope::Os,
        "other fields still fall through to the default_scope"
    );
}

#[test]
fn manifest_default_scope_parses_each_variant() {
    for (raw, expected) in [
        ("core", ConfigScope::Core),
        ("os", ConfigScope::Os),
        ("device", ConfigScope::Device),
        ("any", ConfigScope::Core),
    ] {
        let toml = format!(
            r#"{TRAITS_MANIFEST_HEAD}
[config]
default_scope = "{raw}"
"#
        );
        let manifest: PluginManifest = toml::from_str(&toml).unwrap();
        assert_eq!(
            manifest.config.scope_for("unspecified"),
            expected,
            "default_scope = {raw:?} must apply to unspecified fields"
        );
    }
}

#[test]
fn manifest_without_default_scope_falls_through_to_core() {
    let manifest: PluginManifest = toml::from_str(TRAITS_MANIFEST_HEAD).unwrap();
    assert!(
        manifest.config.default_scope.is_none(),
        "default_scope is None when not declared, not Some(Core); keeps the field optional in TOML"
    );
    assert_eq!(manifest.config.scope_for("x"), ConfigScope::Core);
}

#[test]
fn parse_manifest_traits_preserves_kebab_and_snake_keys() {
    let toml = format!(
        r#"{TRAITS_MANIFEST_HEAD}
[traits]
confined = {{}}

[traits.peripheral-preview]
neighbors = 2

[traits.atmosphere]
preset = "spacecraft"

[traits.future_trait]
value = 1
"#
    );
    let manifest: PluginManifest = toml::from_str(&toml).unwrap();
    let traits = manifest.traits.expect("traits should parse");
    assert_eq!(
        traits,
        serde_json::json!({
            "confined": {},
            "peripheral-preview": { "neighbors": 2 },
            "atmosphere": { "preset": "spacecraft" },
            "future_trait": { "value": 1 },
        })
    );
}
