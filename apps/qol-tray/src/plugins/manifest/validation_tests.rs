use super::*;

#[test]
fn validate_rejects_unsupported_manifest_version() {
    let toml = r#"
        manifest_version = 2

        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = []
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.validate().is_err());
}

#[test]
fn validate_rejects_absolute_runtime_command() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = []

        [runtime]
        command = "/bin/sh"
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.validate().is_err());
}

#[test]
fn validate_rejects_invalid_action_id_in_runtime_map() {
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
        actions = { "--bad" = ["show"] }
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.validate().is_err());
}

#[test]
fn validate_rejects_invalid_action_id_in_menu() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = [
            { type = "action", id = "--bad", label = "Run", action = "run" }
        ]
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.validate().is_err());
}

#[test]
fn validate_rejects_duplicate_action_id_in_menu() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = [
            { type = "action", id = "run", label = "Run", action = "run" },
            { type = "action", id = "run", label = "Run Again", action = "run" }
        ]
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.validate().is_err());
}

#[test]
fn validate_rejects_runtime_actions_missing_menu_action_mapping() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = [
            { type = "action", id = "run", label = "Run", action = "run" },
            { type = "action", id = "settings", label = "Settings", action = "settings" }
        ]

        [runtime]
        command = "launcher"
        actions = { run = ["show"] }
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.validate().is_err());
}

#[test]
fn validate_accepts_runtime_actions_covering_all_menu_actions() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = [
            { type = "action", id = "run", label = "Run", action = "run" },
            { type = "action", id = "settings", label = "Settings", action = "settings" }
        ]

        [runtime]
        command = "launcher"
        actions = { run = ["show"], settings = ["config"] }
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.validate().is_ok());
}

#[test]
fn validate_does_not_require_runtime_mapping_for_checkbox_items() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = [
            { type = "action", id = "record", label = "Record", action = "run" },
            { type = "checkbox", id = "audio-enable", label = "Audio", action = "toggle-config", config_key = "audio.enabled" }
        ]

        [runtime]
        command = "screen-recorder"
        actions = { record = ["record"] }
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.validate().is_ok());
}

#[test]
fn validate_rejects_relative_daemon_socket() {
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
        socket = "qol-p.sock"
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.validate().is_err());
}

#[test]
fn validate_rejects_script_runtime_command() {
    let toml = r#"
        [plugin]
        name = "P"
        description = ""
        version = "0.0.1"

        [menu]
        label = "M"
        items = []

        [runtime]
        command = "run.sh"
    "#;

    let manifest: PluginManifest = toml::from_str(toml).unwrap();
    assert!(manifest.validate().is_err());
}

#[test]
fn validate_accepts_binary_command_names() {
    let manifest = PluginManifest {
        manifest_version: CURRENT_MANIFEST_VERSION,
        plugin: PluginInfo {
            name: "P".to_string(),
            description: "".to_string(),
            version: "0.0.1".to_string(),
            author: None,
            platforms: None,
        },
        menu: MenuConfig {
            label: "M".to_string(),
            icon: None,
            items: Vec::new(),
        },
        daemon: Some(DaemonConfig {
            enabled: true,
            command: "pointzerver".to_string(),
            socket: None,
        }),
        dependencies: None,
        runtime: Some(RuntimeConfig {
            command: "window_actions_2".to_string(),
            actions: None,
        }),
    };

    assert!(manifest.validate().is_ok());
}

#[test]
fn validate_accepts_action_type_runtime_menu_pair() {
    let manifest = PluginManifest {
        manifest_version: CURRENT_MANIFEST_VERSION,
        plugin: PluginInfo {
            name: "P".to_string(),
            description: "".to_string(),
            version: "0.0.1".to_string(),
            author: None,
            platforms: None,
        },
        menu: MenuConfig {
            label: "M".to_string(),
            icon: None,
            items: vec![MenuItem::Action {
                id: "run".to_string(),
                label: "Run".to_string(),
                action: ActionType::Run,
                config_key: None,
            }],
        },
        daemon: None,
        dependencies: None,
        runtime: Some(RuntimeConfig {
            command: "runner".to_string(),
            actions: Some(
                [("run".to_string(), vec!["exec".to_string()])]
                    .into_iter()
                    .collect(),
            ),
        }),
    };

    assert!(manifest.validate().is_ok());
}
