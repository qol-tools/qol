mod platform;

use crate::plugins::manifest::walk_menu_items;
use crate::plugins::{ActionType, MenuItem, PluginManager};
use std::path::Path;

pub struct StubInput {
    pub plugin_id: String,
    pub plugin_name: String,
    pub action_id: String,
    pub action_label: String,
}

pub fn collect_stubs(manager: &PluginManager) -> Vec<StubInput> {
    let mut stubs = Vec::new();
    for plugin in manager.plugins() {
        let plugin_id = plugin.id.as_str();
        let plugin_name = &plugin.manifest.plugin.name;
        walk_menu_items(&plugin.manifest.menu.items, &mut |item| {
            let MenuItem::Action {
                id, label, action, ..
            } = item
            else {
                return;
            };
            if *action != ActionType::Run {
                return;
            }
            stubs.push(StubInput {
                plugin_id: plugin_id.to_string(),
                plugin_name: plugin_name.clone(),
                action_id: id.clone(),
                action_label: label.clone(),
            });
        });
    }
    stubs
}

pub fn sync_stubs(stubs: &[StubInput], binary_path: &Path) {
    if let Err(e) = platform::sync(stubs, binary_path) {
        log::error!("Failed to sync launcher stubs: {}", e);
    }
}

pub fn sync_stubs_background(stubs: Vec<StubInput>) {
    std::thread::spawn(move || {
        let Ok(binary_path) = std::env::current_exe() else {
            return;
        };
        sync_stubs(&stubs, &binary_path);
    });
}
