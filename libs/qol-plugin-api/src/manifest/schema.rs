use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

pub type ActionCatalog = indexmap::IndexMap<String, ActionDeclaration>;

macro_rules! string_newtype {
    ($name:ident, $expecting:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct Visitor;
                impl<'de> serde::de::Visitor<'de> for Visitor {
                    type Value = $name;
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        f.write_str($expecting)
                    }
                    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<$name, E> {
                        Ok($name(v.to_owned()))
                    }
                    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<$name, E> {
                        Ok($name(v))
                    }
                }
                d.deserialize_string(Visitor)
            }
        }
    };
}

string_newtype!(PluginId, "a plugin id string");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginManifest {
    #[serde(default = "super::default_manifest_version")]
    pub manifest_version: u32,
    pub plugin: PluginInfo,
    pub menu: MenuConfig,
    #[serde(default)]
    pub daemon: Option<DaemonConfig>,
    #[serde(default)]
    pub dependencies: Option<Dependencies>,
    #[serde(default)]
    pub runtime: Option<RuntimeConfig>,
    #[serde(default, rename = "action")]
    pub actions: ActionCatalog,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub build: BuildInfo,
    #[serde(default)]
    pub traits: Option<serde_json::Value>,
    #[serde(default)]
    pub config: ConfigDeclarations,
    #[serde(default)]
    pub shortcuts: Vec<ShortcutDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredAction {
    pub id: String,
    pub label: String,
    pub kind: ActionType,
}

impl PluginManifest {
    pub fn executable_actions(&self) -> Vec<DeclaredAction> {
        if !self.actions.is_empty() {
            return self.catalog_executable_actions();
        }

        self.legacy_menu_executable_actions()
    }

    pub fn executable_action_ids(&self) -> BTreeSet<String> {
        self.executable_actions()
            .into_iter()
            .map(|action| action.id)
            .collect()
    }

    pub fn catalog_runtime_args(&self, action_id: &str) -> Option<Vec<String>> {
        let action = self.actions.get(action_id)?;
        if !action.kind.is_executable() {
            return None;
        }

        Some(
            action
                .args
                .clone()
                .unwrap_or_else(|| vec![action_id.to_string()]),
        )
    }

    fn catalog_executable_actions(&self) -> Vec<DeclaredAction> {
        self.actions
            .iter()
            .filter(|(_, action)| action.kind.is_executable())
            .map(|(id, action)| DeclaredAction {
                id: id.clone(),
                label: action.label.clone(),
                kind: action.kind,
            })
            .collect()
    }

    fn legacy_menu_executable_actions(&self) -> Vec<DeclaredAction> {
        let mut actions = Vec::new();
        collect_legacy_menu_executable_actions(&self.menu.items, &mut actions);
        actions
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ConfigDeclarations {
    #[serde(default)]
    pub default_scope: Option<ConfigScope>,
    #[serde(default)]
    pub scope: HashMap<String, ConfigScope>,
}

impl ConfigDeclarations {
    pub fn scope_for(&self, field: &str) -> ConfigScope {
        self.scope
            .get(field)
            .copied()
            .or(self.default_scope)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ConfigScope {
    #[default]
    #[serde(alias = "any")]
    Core,
    Os,
    Device,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct BuildInfo {
    #[serde(default)]
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuntimeConfig {
    pub command: String,
    #[serde(default)]
    pub actions: Option<HashMap<String, Vec<String>>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ActionDeclaration {
    pub label: String,
    #[serde(default = "default_action_kind")]
    pub kind: ActionType,
    #[serde(default)]
    pub continuous: bool,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub config_key: Option<String>,
    #[serde(default)]
    pub checked: bool,
}

fn default_action_kind() -> ActionType {
    ActionType::Run
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ShortcutDeclaration {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub export_to_launcher: bool,
    #[serde(default = "default_shortcut_action")]
    pub action: String,
}

fn default_true() -> bool {
    true
}

fn default_shortcut_action() -> String {
    "open".to_string()
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

string_newtype!(PluginUid, "a plugin uid string");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginInfo {
    #[serde(default)]
    pub id: Option<PluginId>,
    #[serde(default)]
    pub uid: Option<PluginUid>,
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
        super::supports_current_platform(&self.platforms)
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ActionType {
    Run,
    Settings,
    #[serde(rename = "toggle-config")]
    ToggleConfig,
}

impl ActionType {
    pub fn is_executable(self) -> bool {
        matches!(self, Self::Run | Self::Settings)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DaemonConfig {
    pub enabled: bool,
    pub command: String,
    #[serde(default)]
    pub socket: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub extra_ports: Vec<NamedPort>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NamedPort {
    pub name: String,
    pub port: u16,
    #[serde(default)]
    pub protocol: PortProtocol,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    #[default]
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Capabilities {
    #[serde(default)]
    pub serial: bool,
    #[serde(default)]
    pub gpui: bool,
    #[serde(flatten)]
    pub extras: HashMap<String, toml::Value>,
}

fn collect_legacy_menu_executable_actions(items: &[MenuItem], actions: &mut Vec<DeclaredAction>) {
    for item in items {
        match item {
            MenuItem::Action {
                id, label, action, ..
            } => actions.push(DeclaredAction {
                id: id.clone(),
                label: label.clone(),
                kind: *action,
            }),
            MenuItem::Submenu { items, .. } => {
                collect_legacy_menu_executable_actions(items, actions);
            }
            MenuItem::Checkbox { .. } | MenuItem::Separator => {}
        }
    }
}
