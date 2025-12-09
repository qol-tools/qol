use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginManifest {
    pub plugin: PluginInfo,
    pub menu: MenuConfig,
    #[serde(default)]
    pub daemon: Option<DaemonConfig>,
    #[serde(default)]
    pub dependencies: Option<Dependencies>,
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
        match &self.platforms {
            None => true,
            Some(platforms) => platforms.iter().any(|p| p == std::env::consts::OS),
        }
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
            assert_eq!(info.supports_current_platform(), *expected, "platforms: {:?}", platforms);
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
            MenuItem::Action { id, label, action, config_key } => {
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
            MenuItem::Checkbox { id, label, checked, action, config_key } => {
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
            struct Wrapper { action: ActionType }
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
            command = "daemon.sh"
        "#;

        let manifest: PluginManifest = toml::from_str(toml).unwrap();
        assert_eq!(manifest.plugin.name, "Test Plugin");
        assert_eq!(manifest.plugin.version, "1.2.3");
        assert_eq!(manifest.plugin.author, Some("Test Author".to_string()));
        assert_eq!(manifest.plugin.platforms, Some(vec!["linux".to_string(), "windows".to_string()]));
        assert_eq!(manifest.menu.label, "Test Menu");
        assert_eq!(manifest.menu.icon, Some("test.png".to_string()));
        assert_eq!(manifest.menu.items.len(), 1);
        assert!(manifest.daemon.is_some());
        let daemon = manifest.daemon.unwrap();
        assert!(daemon.enabled);
        assert_eq!(daemon.command, "daemon.sh");
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
