use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path};

use anyhow::{bail, Result};

pub const CURRENT_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginManifest {
    #[serde(default = "default_manifest_version")]
    pub manifest_version: u32,
    pub plugin: PluginInfo,
    pub menu: MenuConfig,
    #[serde(default)]
    pub daemon: Option<DaemonConfig>,
    #[serde(default)]
    pub dependencies: Option<Dependencies>,
    #[serde(default)]
    pub runtime: Option<RuntimeConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuntimeConfig {
    pub command: String,
    #[serde(default)]
    pub actions: Option<HashMap<String, Vec<String>>>,
}

pub fn default_manifest_version() -> u32 {
    CURRENT_MANIFEST_VERSION
}

pub fn is_valid_action_id(action: &str) -> bool {
    !action.is_empty()
        && action.len() <= 64
        && !action.starts_with('-')
        && action
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl PluginManifest {
    pub fn validate(&self) -> Result<()> {
        if self.manifest_version != CURRENT_MANIFEST_VERSION {
            bail!(
                "Unsupported manifest_version {} (expected {})",
                self.manifest_version,
                CURRENT_MANIFEST_VERSION
            );
        }

        let menu_action_ids = collect_menu_action_ids(&self.menu.items)?;

        if let Some(runtime) = &self.runtime {
            runtime.validate()?;
            validate_runtime_action_coverage(runtime.actions.as_ref(), &menu_action_ids.executable)?;
        }
        if let Some(daemon) = &self.daemon {
            daemon.validate()?;
        }

        Ok(())
    }
}

impl RuntimeConfig {
    pub fn validate(&self) -> Result<()> {
        validate_runtime_command(&self.command)?;
        if let Some(actions) = &self.actions {
            validate_runtime_actions(actions)?;
        }
        Ok(())
    }
}

struct MenuActionIds {
    all: BTreeSet<String>,
    executable: BTreeSet<String>,
}

fn collect_menu_action_ids(items: &[MenuItem]) -> Result<MenuActionIds> {
    let mut action_ids = MenuActionIds {
        all: BTreeSet::new(),
        executable: BTreeSet::new(),
    };
    collect_menu_action_ids_inner(items, &mut action_ids)?;
    Ok(action_ids)
}

fn collect_menu_action_ids_inner(items: &[MenuItem], action_ids: &mut MenuActionIds) -> Result<()> {
    for item in items {
        match item {
            MenuItem::Action { id, .. } => {
                if !is_valid_action_id(id) {
                    bail!("menu contains invalid action id {:?}", id);
                }

                if !action_ids.all.insert(id.clone()) {
                    bail!("menu contains duplicate action id {:?}", id);
                }

                action_ids.executable.insert(id.clone());
            }
            MenuItem::Checkbox { id, .. } => {
                if !is_valid_action_id(id) {
                    bail!("menu contains invalid action id {:?}", id);
                }

                if !action_ids.all.insert(id.clone()) {
                    bail!("menu contains duplicate action id {:?}", id);
                }
            }
            MenuItem::Submenu { items, .. } => {
                collect_menu_action_ids_inner(items, action_ids)?;
            }
            MenuItem::Separator => {}
        }
    }

    Ok(())
}

fn validate_runtime_action_coverage(
    actions: Option<&HashMap<String, Vec<String>>>,
    menu_action_ids: &BTreeSet<String>,
) -> Result<()> {
    let Some(actions) = actions else {
        return Ok(());
    };

    for action_id in menu_action_ids {
        if !actions.contains_key(action_id) {
            bail!("runtime.actions missing mapping for menu action {:?}", action_id);
        }
    }

    Ok(())
}

fn validate_runtime_command(command: &str) -> Result<()> {
    validate_command_name("runtime.command", command)
}

fn validate_runtime_actions(actions: &HashMap<String, Vec<String>>) -> Result<()> {
    if actions.is_empty() {
        bail!("runtime.actions cannot be empty when present");
    }

    for (action_id, args) in actions {
        if !is_valid_action_id(action_id) {
            bail!("runtime.actions contains invalid action id {:?}", action_id);
        }

        for arg in args {
            if arg.contains('\0') {
                bail!("runtime.actions for {:?} contains null bytes", action_id);
            }
        }
    }

    Ok(())
}

impl DaemonConfig {
    pub fn validate(&self) -> Result<()> {
        validate_command_name("daemon.command", &self.command)?;
        if let Some(socket) = &self.socket {
            validate_absolute_socket_path(socket)?;
        }
        Ok(())
    }
}

fn validate_command_name(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} cannot be empty");
    }

    if value.trim() != value {
        bail!("{field} cannot have leading or trailing whitespace");
    }

    if value.contains('\0') {
        bail!("{field} cannot contain null bytes");
    }

    if value.starts_with('-') {
        bail!("{field} cannot start with '-'");
    }

    if value.len() > 64 {
        bail!("{field} is too long");
    }

    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("{field} must contain only [A-Za-z0-9_-]");
    }

    Ok(())
}

fn validate_absolute_socket_path(path_value: &str) -> Result<()> {
    if path_value.trim().is_empty() {
        bail!("daemon.socket cannot be empty");
    }

    if path_value.trim() != path_value {
        bail!("daemon.socket cannot have leading or trailing whitespace");
    }

    if path_value.contains('\0') {
        bail!("daemon.socket cannot contain null bytes");
    }

    let path = Path::new(path_value);
    if !path.is_absolute() {
        bail!("daemon.socket must be an absolute path");
    }

    let mut has_normal_component = false;
    for component in path.components() {
        match component {
            Component::ParentDir => {
                bail!("daemon.socket cannot contain parent directory traversal")
            }
            Component::Normal(_) => has_normal_component = true,
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {}
        }
    }

    if !has_normal_component {
        bail!("daemon.socket must reference a socket file path");
    }

    Ok(())
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Dependencies {
    #[serde(default)]
    pub binaries: Vec<BinaryDependency>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BinaryDependency {
    pub name: String,
    pub repo: String,
    pub pattern: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
}

impl PluginInfo {
    pub fn supports_current_platform(&self) -> bool {
        supports_current_platform(&self.platforms)
    }
}

pub fn supports_current_platform(platforms: &Option<Vec<String>>) -> bool {
    match platforms {
        None => true,
        Some(platforms) => platforms.iter().any(|p| p == std::env::consts::OS),
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MenuConfig {
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub items: Vec<MenuItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MenuItem {
    Action {
        id: String,
        label: String,
        action: ActionType,
        #[serde(default)]
        config_key: Option<String>,
    },
    Checkbox {
        id: String,
        label: String,
        #[serde(default)]
        checked: bool,
        action: ActionType,
        #[serde(default)]
        config_key: Option<String>,
    },
    Separator,
    Submenu {
        id: String,
        label: String,
        items: Vec<MenuItem>,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ActionType {
    Run,
    Settings,
    #[serde(rename = "toggle-config")]
    ToggleConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DaemonConfig {
    pub enabled: bool,
    pub command: String,
    #[serde(default)]
    pub socket: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_plugin_info(platforms: Option<Vec<&str>>) -> PluginInfo {
        PluginInfo {
            name: "Test".to_string(),
            description: "Test".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            platforms: platforms.map(|p| p.into_iter().map(String::from).collect()),
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
            assert_eq!(
                info.supports_current_platform(),
                *expected,
                "platforms: {:?}",
                platforms
            );
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
        let toml = r#"type = "separator""#;
        let item: MenuItem = toml::from_str(toml).unwrap();
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
            let w: Wrapper = toml::from_str(&toml).unwrap();
            assert_eq!(w.action, expected, "input: {}", input);
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
        assert!(manifest.menu.items.is_empty());
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
        let toml = r#"
            [plugin]
            name = "P"
            description = ""
            version = "0.0.1"

            [menu]
            label = "M"
            items = []

            [runtime]
            command = "window_actions_2"

            [daemon]
            enabled = true
            command = "pointzerver"
        "#;

        let manifest: PluginManifest = toml::from_str(toml).unwrap();
        assert!(manifest.validate().is_ok());
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
}
