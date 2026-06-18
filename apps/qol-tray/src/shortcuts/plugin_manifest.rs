use super::model::{Shortcut, ShortcutAction, ShortcutSource, ShortcutsConfig};
use crate::plugins::Plugin;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

type SourceKey = (String, String);

pub(super) fn reconcile<'a>(
    config: &mut ShortcutsConfig,
    plugins: impl IntoIterator<Item = &'a Plugin>,
) -> bool {
    let desired = desired_shortcuts(plugins);
    let before = serde_json::to_value(&config.shortcuts).ok();
    let existing = std::mem::take(&mut config.shortcuts);
    let (mut next, seen) = reconcile_existing(existing, &desired);
    append_new_shortcuts(&mut next, &desired, &seen);
    let changed = before != serde_json::to_value(&next).ok();
    config.shortcuts = next;
    changed
}

fn desired_shortcuts<'a>(
    plugins: impl IntoIterator<Item = &'a Plugin>,
) -> BTreeMap<SourceKey, Shortcut> {
    let mut desired = BTreeMap::new();
    for plugin in plugins {
        let plugin_id = plugin.id.as_str();
        for declared in &plugin.manifest.shortcuts {
            let key = (plugin_id.to_string(), declared.id.clone());
            desired.insert(key, shortcut_from_manifest(plugin, declared));
        }
    }
    desired
}

fn shortcut_from_manifest(
    plugin: &Plugin,
    declared: &crate::plugins::manifest::ShortcutDeclaration,
) -> Shortcut {
    let plugin_id = plugin.id.as_str();
    Shortcut {
        id: managed_shortcut_id(plugin_id, &declared.id),
        name: declared.name.clone(),
        enabled: declared.enabled,
        export_to_launcher: declared.export_to_launcher,
        source: Some(ShortcutSource::PluginManifest {
            plugin_id: plugin_id.to_string(),
            shortcut_id: declared.id.clone(),
        }),
        action: ShortcutAction::PluginAction {
            plugin_id: plugin_id.to_string(),
            action: declared.action.clone(),
        },
    }
}

fn reconcile_existing(
    existing: Vec<Shortcut>,
    desired: &BTreeMap<SourceKey, Shortcut>,
) -> (Vec<Shortcut>, BTreeSet<SourceKey>) {
    let mut next = Vec::with_capacity(existing.len() + desired.len());
    let mut seen = BTreeSet::new();
    for shortcut in existing {
        let Some(key) = plugin_manifest_key(&shortcut) else {
            next.push(shortcut);
            continue;
        };
        let Some(desired_shortcut) = desired.get(&key) else {
            continue;
        };
        seen.insert(key);
        next.push(merge_existing_preferences(
            shortcut,
            desired_shortcut.clone(),
        ));
    }
    (next, seen)
}

fn append_new_shortcuts(
    next: &mut Vec<Shortcut>,
    desired: &BTreeMap<SourceKey, Shortcut>,
    seen: &BTreeSet<SourceKey>,
) {
    for (key, shortcut) in desired {
        if seen.contains(key) {
            continue;
        }
        if next.iter().any(|existing| existing.id == shortcut.id) {
            log::warn!(
                "Skipping plugin shortcut {} from {}: shortcut id already exists",
                key.1,
                key.0
            );
            continue;
        }
        next.push(shortcut.clone());
    }
}

fn merge_existing_preferences(existing: Shortcut, mut desired: Shortcut) -> Shortcut {
    desired.enabled = existing.enabled;
    desired.export_to_launcher = existing.export_to_launcher;
    desired
}

fn plugin_manifest_key(shortcut: &Shortcut) -> Option<SourceKey> {
    match shortcut.source.as_ref()? {
        ShortcutSource::PluginManifest {
            plugin_id,
            shortcut_id,
        } => Some((plugin_id.clone(), shortcut_id.clone())),
    }
}

fn managed_shortcut_id(plugin_id: &str, shortcut_id: &str) -> String {
    let plain = format!("{plugin_id}-{shortcut_id}");
    if plain.len() <= 64 {
        return plain;
    }

    let mut hasher = Sha256::new();
    hasher.update(plain.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    let prefix = format!("plugin-{}-", &digest[..12]);
    let remaining = 64usize.saturating_sub(prefix.len());
    format!(
        "{}{}",
        prefix,
        &shortcut_id[..shortcut_id.len().min(remaining)]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::{
        Capabilities, ConfigDeclarations, MenuConfig, PluginInfo, PluginManifest,
        ShortcutDeclaration, CURRENT_MANIFEST_VERSION,
    };
    use crate::plugins::{PluginId, PluginSource};
    use std::path::PathBuf;

    fn plugin(id: &str, shortcuts: Vec<ShortcutDeclaration>) -> Plugin {
        Plugin::new_with_source(
            PluginId::new(id),
            PluginManifest {
                manifest_version: CURRENT_MANIFEST_VERSION,
                plugin: PluginInfo {
                    id: Some(id.into()),
                    name: id.to_string(),
                    description: String::new(),
                    version: "1.0.0".to_string(),
                    author: None,
                    platforms: None,
                },
                menu: MenuConfig {
                    label: id.to_string(),
                    icon: None,
                    items: Vec::new(),
                },
                daemon: None,
                dependencies: None,
                runtime: None,
                capabilities: Capabilities::default(),
                build: Default::default(),
                traits: None,
                config: ConfigDeclarations::default(),
                shortcuts,
            },
            PathBuf::from(format!("/tmp/{id}")),
            PluginSource::Installed,
        )
    }

    fn declaration(id: &str, name: &str) -> ShortcutDeclaration {
        ShortcutDeclaration {
            id: id.to_string(),
            name: name.to_string(),
            enabled: true,
            export_to_launcher: true,
            action: "open".to_string(),
        }
    }

    #[test]
    fn reconcile_adds_declared_plugin_shortcut() {
        let plugins = vec![plugin(
            "plugin-cli-sessions",
            vec![declaration("open", "Claude")],
        )];
        let mut config = ShortcutsConfig::default();

        assert!(reconcile(&mut config, &plugins));

        assert_eq!(config.shortcuts.len(), 1);
        let shortcut = &config.shortcuts[0];
        assert_eq!(shortcut.id, "plugin-cli-sessions-open");
        assert_eq!(shortcut.name, "Claude");
        assert!(shortcut.export_to_launcher);
        assert!(matches!(
            shortcut.action,
            ShortcutAction::PluginAction { .. }
        ));
    }

    #[test]
    fn reconcile_preserves_user_disabled_state_for_existing_managed_shortcut() {
        let plugins = vec![plugin(
            "plugin-cli-sessions",
            vec![declaration("open", "Claude Sessions")],
        )];
        let mut config = ShortcutsConfig {
            shortcuts: vec![Shortcut {
                id: "plugin-cli-sessions-open".to_string(),
                name: "Old".to_string(),
                enabled: false,
                export_to_launcher: false,
                source: Some(ShortcutSource::PluginManifest {
                    plugin_id: "plugin-cli-sessions".to_string(),
                    shortcut_id: "open".to_string(),
                }),
                action: ShortcutAction::PluginAction {
                    plugin_id: "plugin-cli-sessions".to_string(),
                    action: "open".to_string(),
                },
            }],
        };

        assert!(reconcile(&mut config, &plugins));

        let shortcut = &config.shortcuts[0];
        assert_eq!(shortcut.name, "Claude Sessions");
        assert!(!shortcut.enabled);
        assert!(!shortcut.export_to_launcher);
    }

    #[test]
    fn reconcile_removes_orphaned_managed_shortcut_only() {
        let mut config = ShortcutsConfig {
            shortcuts: vec![
                Shortcut {
                    id: "plugin-old-open".to_string(),
                    name: "Old".to_string(),
                    enabled: true,
                    export_to_launcher: true,
                    source: Some(ShortcutSource::PluginManifest {
                        plugin_id: "plugin-old".to_string(),
                        shortcut_id: "open".to_string(),
                    }),
                    action: ShortcutAction::PluginAction {
                        plugin_id: "plugin-old".to_string(),
                        action: "open".to_string(),
                    },
                },
                Shortcut {
                    id: "user-shortcut".to_string(),
                    name: "User".to_string(),
                    enabled: true,
                    export_to_launcher: true,
                    source: None,
                    action: ShortcutAction::OpenUrl {
                        url: "https://example.com".to_string(),
                        browser_override: None,
                    },
                },
            ],
        };

        let plugins: Vec<Plugin> = Vec::new();
        assert!(reconcile(&mut config, &plugins));

        assert_eq!(config.shortcuts.len(), 1);
        assert_eq!(config.shortcuts[0].id, "user-shortcut");
    }
}
