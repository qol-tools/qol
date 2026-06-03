use crate::plugins::{manifest::walk_menu_items, MenuItem, Plugin, PluginId, PluginManager};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub(super) type AvailableActions = HashMap<PluginId, HashSet<String>>;

pub(super) fn load_available_actions(
    plugin_manager: &Arc<Mutex<PluginManager>>,
) -> AvailableActions {
    let manager = match plugin_manager.lock() {
        Ok(manager) => manager,
        Err(_) => {
            log::error!("Failed to resolve available plugin actions: plugin manager lock failed");
            return HashMap::new();
        }
    };
    catalog_for_plugins(manager.plugins())
}

pub(super) fn catalog_for_plugins<'a, I>(plugins: I) -> AvailableActions
where
    I: IntoIterator<Item = &'a Plugin>,
{
    plugins
        .into_iter()
        .map(|plugin| {
            (
                plugin.id.clone(),
                collect_action_ids(&plugin.manifest.menu.items),
            )
        })
        .collect()
}

fn collect_action_ids(items: &[MenuItem]) -> HashSet<String> {
    let mut action_ids = HashSet::new();
    let mut collect = |item: &MenuItem| match item {
        MenuItem::Action { id, .. } | MenuItem::Checkbox { id, .. } => {
            action_ids.insert(id.clone());
        }
        MenuItem::Separator | MenuItem::Submenu { .. } => {}
    };

    walk_menu_items(items, &mut collect);
    action_ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::{
        ActionType, BuildInfo, Capabilities, DaemonConfig, MenuConfig, PluginInfo, PluginManifest,
    };
    use crate::plugins::{Plugin, PluginId, PluginSource};
    use std::path::PathBuf;

    fn run_action(id: &str) -> MenuItem {
        MenuItem::Action {
            id: id.to_string(),
            label: id.to_string(),
            action: ActionType::Run,
            config_key: None,
        }
    }

    fn checkbox(id: &str) -> MenuItem {
        MenuItem::Checkbox {
            id: id.to_string(),
            label: id.to_string(),
            checked: false,
            action: ActionType::ToggleConfig,
            config_key: None,
        }
    }

    fn submenu(id: &str, items: Vec<MenuItem>) -> MenuItem {
        MenuItem::Submenu {
            id: id.to_string(),
            label: id.to_string(),
            items,
        }
    }

    fn sorted(ids: HashSet<String>) -> Vec<String> {
        let mut out: Vec<_> = ids.into_iter().collect();
        out.sort();
        out
    }

    fn manifest(daemon: Option<DaemonConfig>, items: Vec<MenuItem>) -> PluginManifest {
        PluginManifest {
            manifest_version: 1,
            plugin: PluginInfo {
                id: Some("test-plugin".into()),
                name: "Plugin Foo".to_string(),
                description: "test".to_string(),
                version: "0.0.0".to_string(),
                author: None,
                platforms: None,
            },
            menu: MenuConfig {
                label: "Foo".to_string(),
                icon: None,
                items,
            },
            daemon,
            dependencies: None,
            runtime: None,
            capabilities: Capabilities::default(),
            build: BuildInfo::default(),
            traits: None,
            config: Default::default(),
        }
    }

    fn make_plugin(id: &str, daemon: Option<DaemonConfig>, items: Vec<MenuItem>) -> Plugin {
        Plugin::new_with_source(
            PluginId::new(id),
            manifest(daemon, items),
            PathBuf::from(format!("/tmp/plugins/{}", id)),
            PluginSource::Installed,
        )
    }

    #[test]
    fn collect_action_ids_extracts_top_level_actions() {
        let items = vec![
            run_action("alpha"),
            MenuItem::Separator,
            run_action("beta"),
            checkbox("gamma"),
        ];
        assert_eq!(
            sorted(collect_action_ids(&items)),
            vec!["alpha", "beta", "gamma"]
        );
    }

    #[test]
    fn collect_action_ids_descends_into_submenus() {
        let items = vec![submenu(
            "top",
            vec![
                run_action("inner"),
                submenu("deeper", vec![run_action("deepest")]),
            ],
        )];
        assert_eq!(sorted(collect_action_ids(&items)), vec!["deepest", "inner"]);
    }

    #[test]
    fn catalog_includes_action_for_daemon_plugin_with_socket_path() {
        let plugin = make_plugin(
            "plugin-foo",
            Some(DaemonConfig {
                enabled: true,
                command: "plugin-foo".to_string(),
                socket: Some("qol-this-socket-does-not-exist.sock".to_string()),
            }),
            vec![run_action("toggle")],
        );

        let catalog = catalog_for_plugins(std::iter::once(&plugin));

        let actions = catalog
            .get("plugin-foo")
            .expect("daemon-backed plugin must be in the catalog");
        assert!(
            actions.contains("toggle"),
            "daemon-backed action must remain registered when a socket path is configured; got {:?}",
            actions
        );
    }

    #[test]
    fn catalog_includes_actions_for_disabled_daemon_and_no_daemon_plugins() {
        let plugins = [
            make_plugin(
                "plugin-no-daemon",
                None,
                vec![run_action("alpha"), run_action("beta")],
            ),
            make_plugin(
                "plugin-disabled-daemon",
                Some(DaemonConfig {
                    enabled: false,
                    command: "plugin-disabled-daemon".to_string(),
                    socket: None,
                }),
                vec![run_action("gamma")],
            ),
        ];

        let catalog = catalog_for_plugins(plugins.iter());

        assert_eq!(
            sorted(catalog.get("plugin-no-daemon").unwrap().clone()),
            vec!["alpha", "beta"]
        );
        assert_eq!(
            sorted(catalog.get("plugin-disabled-daemon").unwrap().clone()),
            vec!["gamma"]
        );
    }

    #[test]
    fn catalog_omits_plugins_that_declare_no_actions() {
        let plugin = make_plugin("plugin-empty", None, vec![]);
        let catalog = catalog_for_plugins(std::iter::once(&plugin));

        assert_eq!(
            catalog.get("plugin-empty").map(|s| s.is_empty()),
            Some(true),
            "plugin with no menu actions appears with an empty action set"
        );
    }
}
