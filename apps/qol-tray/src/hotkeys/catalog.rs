use crate::plugins::{manifest::walk_menu_items, MenuItem, PluginId, PluginManager};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub(super) type AvailableActions = HashMap<PluginId, HashSet<String>>;

pub(super) fn load_available_actions(
    plugin_manager: &Arc<Mutex<PluginManager>>,
) -> AvailableActions {
    match available_actions(plugin_manager) {
        Ok(actions) => actions,
        Err(error) => {
            log::error!("Failed to resolve available plugin actions: {}", error);
            HashMap::new()
        }
    }
}

fn available_actions(
    plugin_manager: &Arc<Mutex<PluginManager>>,
) -> anyhow::Result<AvailableActions> {
    let manager = plugin_manager
        .lock()
        .map_err(|_| anyhow::anyhow!("plugin manager lock failed"))?;
    let mut actions_by_plugin = HashMap::new();

    for plugin in manager.plugins() {
        actions_by_plugin.insert(
            plugin.id.clone(),
            collect_action_ids(&plugin.manifest.menu.items),
        );
    }

    Ok(actions_by_plugin)
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
