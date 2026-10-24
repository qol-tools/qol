use crate::plugins::manifest::{DefaultFor, PluginManifest};
use crate::plugins::PluginUid;
use anyhow::{anyhow, Result};
use qol_host_defaults::Role;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Declaration {
    pub plugin_id: String,
    pub uid: PluginUid,
    pub role: Role,
    pub action: String,
}

#[derive(Clone, Debug)]
pub(super) struct InstalledPlugin {
    pub id: String,
    pub root: PathBuf,
    pub manifest: PluginManifest,
}

impl InstalledPlugin {
    pub(super) fn uid(&self) -> PluginUid {
        uid_of(&self.id, &self.manifest)
    }

    pub(super) fn declarations(&self) -> Vec<Declaration> {
        declarations(&self.id, self.uid(), &self.manifest.default_for)
    }

    pub(super) fn main_declaration(&self) -> Option<Declaration> {
        self.declarations().into_iter().next()
    }

    pub(super) fn declaration_for(&self, role: Role) -> Option<Declaration> {
        self.declarations()
            .into_iter()
            .find(|declaration| declaration.role == role)
    }
}

pub(super) fn uid_of(plugin_id: &str, manifest: &PluginManifest) -> PluginUid {
    manifest
        .plugin
        .uid
        .clone()
        .unwrap_or_else(|| PluginUid::new(plugin_id))
}

pub(super) fn declarations(
    plugin_id: &str,
    uid: PluginUid,
    entries: &[DefaultFor],
) -> Vec<Declaration> {
    entries
        .iter()
        .filter_map(|entry| {
            let Some(role) = Role::parse(&entry.role) else {
                log::warn!(
                    "plugin {plugin_id} declares unknown default role {:?}; skipped",
                    entry.role
                );
                return None;
            };
            Some(Declaration {
                plugin_id: plugin_id.to_string(),
                uid: uid.clone(),
                role,
                action: entry.action.clone(),
            })
        })
        .collect()
}

pub(super) fn load_plugin(plugin_id: &str) -> Option<InstalledPlugin> {
    let root = crate::plugins::paths::resolve_plugin_root(plugin_id).ok()?;
    read_plugin(plugin_id, root)
}

pub(super) fn installed() -> Vec<InstalledPlugin> {
    let Ok(config_dir) = crate::paths::shared_config_dir() else {
        return Vec::new();
    };
    let registry = match crate::plugins::registry::load_registry(&config_dir) {
        Ok(registry) => registry,
        Err(error) => {
            log::debug!("host defaults: plugin registry unreadable: {error}");
            return Vec::new();
        }
    };
    registry
        .entries
        .iter()
        .filter_map(|entry| load_plugin(&entry.id))
        .collect()
}

fn read_plugin(plugin_id: &str, root: PathBuf) -> Option<InstalledPlugin> {
    let manifest = match PluginManifest::read_from_dir(&root) {
        Ok(manifest) => manifest,
        Err(error) => {
            log::debug!("host defaults: manifest of {plugin_id} unreadable: {error:#}");
            return None;
        }
    };
    if !manifest.plugin.supports_current_platform() {
        return None;
    }
    Some(InstalledPlugin {
        id: plugin_id.to_string(),
        root,
        manifest,
    })
}

pub(super) fn declaring(role: Role, plugins: &[InstalledPlugin]) -> Vec<String> {
    plugins
        .iter()
        .filter(|plugin| plugin.declaration_for(role).is_some())
        .map(|plugin| plugin.id.clone())
        .collect()
}

pub(super) fn declared(plugin_id: &str, role: Role) -> Result<Declaration> {
    let plugin = load_plugin(plugin_id)
        .ok_or_else(|| anyhow!("{plugin_id} is not installed on this computer"))?;
    plugin
        .declaration_for(role)
        .ok_or_else(|| anyhow!("{plugin_id} does not declare {}", role.id()))
}

pub(super) fn holder_label(
    plugin: Option<&InstalledPlugin>,
    uid: &PluginUid,
    action: &str,
) -> String {
    let Some(plugin) = plugin else {
        return format!("{} {action}", uid.as_str());
    };
    let label = plugin
        .manifest
        .executable_actions()
        .into_iter()
        .find(|declared| declared.id == action)
        .map_or_else(|| action.to_string(), |declared| declared.label);
    let name = &plugin.manifest.plugin.name;
    if label.to_lowercase().contains(&name.to_lowercase()) {
        return label;
    }
    format!("{name} {label}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed_plugin(name: &str) -> InstalledPlugin {
        let text = format!(
            "[plugin]\nid = \"qol-shot\"\nname = \"{name}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"{name}\"\nitems = []\n\n[action.screenshot]\nlabel = \"Take screenshot\"\nargs = [\"screenshot\"]\n"
        );
        InstalledPlugin {
            id: "qol-shot".to_string(),
            root: PathBuf::new(),
            manifest: toml::from_str(&text).expect("manifest"),
        }
    }

    #[test]
    fn a_holder_is_named_by_plugin_and_action_label() {
        let plugin = installed_plugin("Shot");
        let label = holder_label(Some(&plugin), &PluginUid::new("uid-shot"), "screenshot");
        assert_eq!(label, "Shot Take screenshot");
    }

    #[test]
    fn an_action_label_that_names_the_plugin_is_not_doubled() {
        let plugin = installed_plugin("Screenshot");
        let label = holder_label(Some(&plugin), &PluginUid::new("uid-shot"), "screenshot");
        assert_eq!(label, "Take screenshot");
    }

    #[test]
    fn an_unknown_action_or_plugin_falls_back_to_the_raw_ids() {
        let plugin = installed_plugin("Screenshot");
        let uid = PluginUid::new("uid-shot");
        assert_eq!(holder_label(Some(&plugin), &uid, "grab"), "Screenshot grab");
        assert_eq!(holder_label(None, &uid, "grab"), "uid-shot grab");
    }

    fn entry(role: &str, action: &str) -> DefaultFor {
        DefaultFor {
            role: role.to_string(),
            action: action.to_string(),
        }
    }

    #[test]
    fn unknown_roles_are_skipped_and_order_is_kept() {
        let declared = declarations(
            "qol-shot",
            PluginUid::new("uid-shot"),
            &[
                entry("slot.nonsense", "x"),
                entry("slot.screenshot", "screenshot"),
                entry("folder.open", "open"),
            ],
        );
        assert_eq!(
            declared
                .iter()
                .map(|declaration| (declaration.role, declaration.action.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (Role::SlotScreenshot, "screenshot"),
                (Role::FolderOpen, "open")
            ]
        );
        assert!(declared
            .iter()
            .all(|declaration| declaration.plugin_id == "qol-shot"));
    }

    #[test]
    fn no_declaration_yields_nothing() {
        assert!(declarations("qol-x", PluginUid::new("u"), &[]).is_empty());
    }
}
