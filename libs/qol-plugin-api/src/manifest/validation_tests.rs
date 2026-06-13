use super::*;
use proptest::prelude::*;

fn manifest_from_toml(toml: &str) -> PluginManifest {
    toml::from_str(toml).unwrap()
}

fn validate_toml(toml: &str) -> anyhow::Result<()> {
    manifest_from_toml(toml).validate()
}

fn base_manifest() -> PluginManifest {
    PluginManifest {
        manifest_version: CURRENT_MANIFEST_VERSION,
        plugin: PluginInfo {
            id: Some("test-plugin".into()),
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
        daemon: None,
        dependencies: None,
        runtime: None,
        capabilities: Capabilities::default(),
        build: BuildInfo::default(),
        traits: None,
        config: ConfigDeclarations::default(),
    }
}

fn valid_basename_strategy() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-zA-Z0-9_][a-zA-Z0-9_-]{0,62}").unwrap()
}

mod manifest_rules {
    use super::*;

    #[test]
    fn validate_rejects_unsupported_manifest_version() {
        let toml = r#"
            manifest_version = 999

            [plugin]
            id = "test-plugin"
            name = "P"
            description = ""
            version = "0.0.1"

            [menu]
            label = "M"
            items = []
        "#;

        assert!(validate_toml(toml).is_err());
    }
}

mod command_rules {
    use super::*;

    #[test]
    fn validate_rejects_absolute_runtime_command() {
        let toml = r#"
            [plugin]
            id = "test-plugin"
            name = "P"
            description = ""
            version = "0.0.1"

            [menu]
            label = "M"
            items = []

            [runtime]
            command = "/bin/sh"
        "#;

        assert!(validate_toml(toml).is_err());
    }

    #[test]
    fn validate_rejects_relative_daemon_socket() {
        let toml = r#"
            [plugin]
            id = "test-plugin"
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

        assert!(validate_toml(toml).is_err());
    }

    #[test]
    fn validate_rejects_script_runtime_command() {
        let toml = r#"
            [plugin]
            id = "test-plugin"
            name = "P"
            description = ""
            version = "0.0.1"

            [menu]
            label = "M"
            items = []

            [runtime]
            command = "run.sh"
        "#;

        assert!(validate_toml(toml).is_err());
    }

    #[test]
    fn validate_accepts_binary_command_names() {
        let manifest = PluginManifest {
            daemon: Some(DaemonConfig {
                enabled: true,
                command: "pointzerver".to_string(),
                socket: None,
            }),
            runtime: Some(RuntimeConfig {
                command: "window_actions_2".to_string(),
                actions: None,
            }),
            ..base_manifest()
        };

        assert!(manifest.validate().is_ok());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn valid_action_ids_are_accepted(value in valid_basename_strategy()) {
            prop_assert!(is_valid_action_id(&value));
        }

        #[test]
        fn valid_command_basenames_are_accepted(value in valid_basename_strategy()) {
            prop_assert!(is_valid_command_basename(&value));
        }
    }
}

mod menu_rules {
    use super::*;

    #[test]
    fn validate_rejects_invalid_action_id_in_menu() {
        let toml = r#"
            [plugin]
            id = "test-plugin"
            name = "P"
            description = ""
            version = "0.0.1"

            [menu]
            label = "M"
            items = [
                { type = "action", id = "--bad", label = "Run", action = "run" }
            ]
        "#;

        assert!(validate_toml(toml).is_err());
    }

    #[test]
    fn validate_rejects_duplicate_action_id_in_menu() {
        let toml = r#"
            [plugin]
            id = "test-plugin"
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

        assert!(validate_toml(toml).is_err());
    }
}

mod runtime_rules {
    use super::*;

    #[test]
    fn validate_rejects_invalid_action_id_in_runtime_map() {
        let toml = r#"
            [plugin]
            id = "test-plugin"
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

        assert!(validate_toml(toml).is_err());
    }

    #[test]
    fn validate_rejects_runtime_actions_missing_menu_action_mapping() {
        let toml = r#"
            [plugin]
            id = "test-plugin"
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

        assert!(validate_toml(toml).is_err());
    }

    #[test]
    fn validate_accepts_runtime_actions_covering_all_menu_actions() {
        let toml = r#"
            [plugin]
            id = "test-plugin"
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

        assert!(validate_toml(toml).is_ok());
    }

    #[test]
    fn validate_does_not_require_runtime_mapping_for_checkbox_items() {
        let toml = r#"
            [plugin]
            id = "test-plugin"
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

        assert!(validate_toml(toml).is_ok());
    }

    #[test]
    fn validate_accepts_action_type_runtime_menu_pair() {
        let manifest = PluginManifest {
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
            runtime: Some(RuntimeConfig {
                command: "runner".to_string(),
                actions: Some(
                    [("run".to_string(), vec!["exec".to_string()])]
                        .into_iter()
                        .collect(),
                ),
            }),
            ..base_manifest()
        };

        assert!(manifest.validate().is_ok());
    }
}

mod dependency_rules {
    use super::*;

    #[test]
    fn validate_rejects_invalid_binary_dependency_name() {
        let manifest = PluginManifest {
            dependencies: Some(Dependencies {
                binaries: vec![BinaryDependency {
                    name: "runner.sh".to_string(),
                    repo: "qol-tools/example".to_string(),
                    pattern: "runner-{os}-{arch}".to_string(),
                }],
            }),
            ..base_manifest()
        };

        assert!(manifest.validate().is_err());
    }

    #[test]
    fn validate_accepts_binary_dependency_name() {
        let manifest = PluginManifest {
            dependencies: Some(Dependencies {
                binaries: vec![BinaryDependency {
                    name: "runner".to_string(),
                    repo: "qol-tools/example".to_string(),
                    pattern: "runner-{os}-{arch}".to_string(),
                }],
            }),
            ..base_manifest()
        };

        assert!(manifest.validate().is_ok());
    }
}
