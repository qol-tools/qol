use crate::plugins::{
    manifest::{walk_menu_items, DaemonConfig},
    MenuItem, PluginId, PluginManager,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(super) type AvailableActions = HashMap<PluginId, HashSet<String>>;

pub(super) fn load_available_actions(
    plugin_manager: &Arc<Mutex<PluginManager>>,
) -> AvailableActions {
    load_available_actions_with(plugin_manager, &default_socket_reachable)
}

fn load_available_actions_with(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    is_reachable: &dyn Fn(&Path) -> bool,
) -> AvailableActions {
    match available_actions(plugin_manager, is_reachable) {
        Ok(actions) => actions,
        Err(error) => {
            log::error!("Failed to resolve available plugin actions: {}", error);
            HashMap::new()
        }
    }
}

fn available_actions(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    is_reachable: &dyn Fn(&Path) -> bool,
) -> anyhow::Result<AvailableActions> {
    let manager = plugin_manager
        .lock()
        .map_err(|_| anyhow::anyhow!("plugin manager lock failed"))?;
    let mut actions_by_plugin = HashMap::new();

    for plugin in manager.plugins() {
        if !daemon_actions_published(plugin.manifest.daemon.as_ref(), is_reachable) {
            log::warn!(
                "Skipping hotkey actions for plugin {}: daemon enabled but socket unreachable",
                plugin.id
            );
            continue;
        }
        actions_by_plugin.insert(
            plugin.id.clone(),
            collect_action_ids(&plugin.manifest.menu.items),
        );
    }

    Ok(actions_by_plugin)
}

fn daemon_actions_published(
    daemon: Option<&DaemonConfig>,
    is_reachable: &dyn Fn(&Path) -> bool,
) -> bool {
    let Some(daemon) = daemon else {
        return true;
    };
    if !daemon.enabled {
        return true;
    }
    let Some(socket) = daemon.socket.as_deref() else {
        return true;
    };
    is_reachable(Path::new(socket))
}

#[cfg(unix)]
fn default_socket_reachable(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

#[cfg(not(unix))]
fn default_socket_reachable(_path: &Path) -> bool {
    false
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
    use crate::plugins::manifest::DaemonConfig;

    fn daemon(enabled: bool, socket: Option<&str>) -> DaemonConfig {
        DaemonConfig {
            enabled,
            command: "plugin-foo".to_string(),
            socket: socket.map(|s| s.to_string()),
        }
    }

    #[test]
    fn daemon_actions_published_table() {
        let always: fn(&Path) -> bool = |_| true;
        let never: fn(&Path) -> bool = |_| false;

        struct Case {
            name: &'static str,
            daemon: Option<DaemonConfig>,
            reachable: fn(&Path) -> bool,
            expected: bool,
        }

        let cases = [
            Case {
                name: "no daemon -> publish",
                daemon: None,
                reachable: never,
                expected: true,
            },
            Case {
                name: "daemon disabled -> publish",
                daemon: Some(daemon(false, Some("/tmp/qol-foo.sock"))),
                reachable: never,
                expected: true,
            },
            Case {
                name: "daemon enabled no socket declared -> publish",
                daemon: Some(daemon(true, None)),
                reachable: never,
                expected: true,
            },
            Case {
                name: "daemon enabled and socket reachable -> publish",
                daemon: Some(daemon(true, Some("/tmp/qol-foo.sock"))),
                reachable: always,
                expected: true,
            },
            Case {
                name: "daemon enabled and socket unreachable -> withhold",
                daemon: Some(daemon(true, Some("/tmp/qol-foo.sock"))),
                reachable: never,
                expected: false,
            },
        ];

        for case in cases {
            let actual = daemon_actions_published(case.daemon.as_ref(), &case.reachable);
            assert_eq!(actual, case.expected, "{}", case.name);
        }
    }

    #[cfg(unix)]
    #[test]
    fn default_socket_reachable_returns_true_for_real_listener() {
        use std::os::unix::net::UnixListener;
        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("qol-test.sock");
        let _listener = UnixListener::bind(&socket_path).unwrap();
        assert!(default_socket_reachable(&socket_path));
    }

    #[cfg(unix)]
    #[test]
    fn default_socket_reachable_returns_false_when_no_listener() {
        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("qol-missing.sock");
        assert!(!default_socket_reachable(&socket_path));
    }
}
