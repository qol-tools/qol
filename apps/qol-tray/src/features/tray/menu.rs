use crate::features::plugin_manager::{PluginManager, MenuItem as PluginMenuItem, ActionType};
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem, CheckMenuItem, Submenu};

pub fn build_menu(plugin_manager: &Arc<Mutex<PluginManager>>) -> Menu {
    let menu = Menu::new();

    let manager = plugin_manager.lock().unwrap();
    for plugin in manager.plugins() {
        let plugin_submenu = Submenu::new(&plugin.manifest.menu.label, true);

        for item in &plugin.manifest.menu.items {
            add_menu_item(&plugin_submenu, item, &plugin.id);
        }

        let _ = menu.append(&plugin_submenu);
    }
    drop(manager);

    let _ = menu.append(&PredefinedMenuItem::separator());

    let reload_item = MenuItem::with_id("__reload__", "Reload Plugins", true, None);
    let _ = menu.append(&reload_item);

    let quit_item = MenuItem::with_id("__quit__", "Quit", true, None);
    let _ = menu.append(&quit_item);

    menu
}

fn add_menu_item(parent: &Submenu, item: &PluginMenuItem, plugin_id: &str) {
    match item {
        PluginMenuItem::Action { id, label, .. } => {
            let full_id = format!("{}::{}", plugin_id, id);
            let menu_item = MenuItem::with_id(&full_id, label, true, None);
            let _ = parent.append(&menu_item);
        }
        PluginMenuItem::Checkbox { id, label, checked, .. } => {
            let full_id = format!("{}::{}", plugin_id, id);
            let check_item = CheckMenuItem::with_id(&full_id, label, true, *checked, None);
            let _ = parent.append(&check_item);
        }
        PluginMenuItem::Separator => {
            let _ = parent.append(&PredefinedMenuItem::separator());
        }
        PluginMenuItem::Submenu { id, label, items } => {
            let full_id = format!("{}::{}", plugin_id, id);
            let submenu = Submenu::with_id(&full_id, label, true);
            for sub_item in items {
                add_menu_item(&submenu, sub_item, plugin_id);
            }
            let _ = parent.append(&submenu);
        }
    }
}

pub fn handle_menu_event(plugin_manager: &Arc<Mutex<PluginManager>>, event_id: &str) -> Result<()> {
    match event_id {
        "__quit__" => {
            log::info!("Quit requested");
            std::process::exit(0);
        }
        "__reload__" => {
            log::info!("Reload plugins requested");
            let mut manager = plugin_manager.lock().unwrap();
            manager.reload()?;
            drop(manager);
            return Ok(());
        }
        _ => {}
    }

    let parts: Vec<&str> = event_id.split("::").collect();
    if parts.len() != 2 {
        log::warn!("Invalid event ID format: {}", event_id);
        return Ok(());
    }

    let (plugin_id, item_id) = (parts[0], parts[1]);

    let manager = plugin_manager.lock().unwrap();
    let plugin = manager.get(plugin_id)
        .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", plugin_id))?;

    let action = find_menu_item_action(&plugin.manifest.menu.items, item_id);

    match action {
        Some((ActionType::Run, _)) => {
            drop(manager);
            let manager = plugin_manager.lock().unwrap();
            manager.execute_plugin(plugin_id)?;
        }
        Some((ActionType::ToggleConfig, Some(config_key))) => {
            let current_value = get_config_value(plugin, &config_key)?;
            let new_value = !current_value;
            drop(manager);

            let manager = plugin_manager.lock().unwrap();
            manager.update_plugin_config(plugin_id, &config_key, serde_json::json!(new_value))?;
            log::info!("Toggled config {} for plugin {}: {}", config_key, plugin_id, new_value);
        }
        Some((ActionType::Settings, _)) => {
            log::info!("Settings not yet implemented for plugin: {}", plugin_id);
        }
        _ => {
            log::warn!("Unknown action for menu item: {}", item_id);
        }
    }

    Ok(())
}

fn find_menu_item_action(items: &[PluginMenuItem], id: &str) -> Option<(ActionType, Option<String>)> {
    for item in items {
        match item {
            PluginMenuItem::Action { id: item_id, action, config_key, .. } if item_id == id => {
                return Some((*action, config_key.clone()));
            }
            PluginMenuItem::Checkbox { id: item_id, action, config_key, .. } if item_id == id => {
                return Some((*action, config_key.clone()));
            }
            PluginMenuItem::Submenu { items: sub_items, .. } => {
                if let Some(action) = find_menu_item_action(sub_items, id) {
                    return Some(action);
                }
            }
            _ => {}
        }
    }
    None
}

fn get_config_value(plugin: &crate::features::plugin_manager::Plugin, key: &str) -> Result<bool> {
    if !plugin.config_path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(&plugin.config_path)?;
    let config: serde_json::Value = serde_json::from_str(&content)?;

    let keys: Vec<&str> = key.split('.').collect();
    let mut current = &config;

    for k in keys {
        current = current.get(k).ok_or_else(|| anyhow::anyhow!("Key not found: {}", key))?;
    }

    current.as_bool().ok_or_else(|| anyhow::anyhow!("Value is not a boolean"))
}
