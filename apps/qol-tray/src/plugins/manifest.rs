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
    Custom,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DaemonConfig {
    pub enabled: bool,
    pub command: String,
    #[serde(default)]
    pub restart_on_crash: bool,
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
    fn supports_current_platform_when_none() {
        let info = make_plugin_info(None);
        assert!(info.supports_current_platform());
    }

    #[test]
    fn supports_current_platform_when_empty() {
        let info = make_plugin_info(Some(vec![]));
        assert!(!info.supports_current_platform());
    }

    #[test]
    fn supports_current_platform_when_listed() {
        let info = make_plugin_info(Some(vec![std::env::consts::OS]));
        assert!(info.supports_current_platform());
    }

    #[test]
    fn supports_current_platform_when_not_listed() {
        let info = make_plugin_info(Some(vec!["not-a-real-os"]));
        assert!(!info.supports_current_platform());
    }

    #[test]
    fn supports_current_platform_with_multiple() {
        let info = make_plugin_info(Some(vec!["linux", "windows", "macos"]));
        assert!(info.supports_current_platform());
    }
}
