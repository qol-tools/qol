use super::router::{EventRouter, EventRoute, EventPattern, EventHandler, HandlerResult};
use crate::plugins::{PluginManager, PluginManifest, MenuItem as PluginMenuItem, ActionType};
use crate::features::FeatureRegistry;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tray_icon::menu::{Menu, MenuItem, CheckMenuItem, Submenu, PredefinedMenuItem};

pub fn build_menu(
    plugin_manager: Arc<Mutex<PluginManager>>,
    feature_registry: Arc<FeatureRegistry>,
) -> Result<(Menu, EventRouter)> {
    let menu = Menu::new();
    let mut all_routes = Vec::new();

    let manager = plugin_manager.lock().unwrap();

    for plugin in manager.plugins() {
        let submenu = Submenu::new(&plugin.manifest.menu.label, true);

        for item in &plugin.manifest.menu.items {
            add_menu_item(&submenu, item, &plugin.id);
        }

        let _ = menu.append(&submenu);

        let plugin_id = plugin.id.clone();
        let manifest = plugin.manifest.clone();
        let pm = plugin_manager.clone();

        let route = EventRoute {
            pattern: EventPattern::Prefix(format!("{}::", plugin_id)),
            handler: EventHandler::Sync(Box::new(move |event_id| {
                handle_plugin_event(&pm, event_id, &plugin_id, &manifest)
            })),
        };
        all_routes.push(route);
    }

    drop(manager);

    for (idx, feature) in feature_registry.features().iter().enumerate() {
        let items = feature.menu_items();
        if items.is_empty() {
            continue;
        }

        let feature_id = format!("feature_{}", idx);

        for item in &items {
            match item {
                PluginMenuItem::Submenu { id, label, items: sub_items } => {
                    let full_id = format!("{}::{}", feature_id, id);
                    log::debug!("Creating submenu with ID: {}", full_id);
                    let submenu = Submenu::with_id(&full_id, label, true);
                    for sub_item in sub_items {
                        add_menu_item(&submenu, sub_item, &feature_id);
                    }
                    let _ = menu.append(&submenu);
                }
                PluginMenuItem::Action { id, label, .. } => {
                    let full_id = format!("{}::{}", feature_id, id);
                    let menu_item = MenuItem::with_id(&full_id, label, true, None);
                    let _ = menu.append(&menu_item);
                }
                PluginMenuItem::Checkbox { id, label, checked, .. } => {
                    let full_id = format!("{}::{}", feature_id, id);
                    let check_item = CheckMenuItem::with_id(&full_id, label, true, *checked, None);
                    let _ = menu.append(&check_item);
                }
                PluginMenuItem::Separator => {
                    let _ = menu.append(&PredefinedMenuItem::separator());
                }
            }
        }

        let feature_clone = feature_registry.clone();
        let feature_idx = idx;

        let route = EventRoute {
            pattern: EventPattern::Prefix(format!("{}::", feature_id)),
            handler: EventHandler::Sync(Box::new(move |event_id| {
                if let Some(feature) = feature_clone.features().get(feature_idx) {
                    feature.handle_event(event_id)?;
                }
                Ok(HandlerResult::Continue)
            })),
        };
        all_routes.push(route);
    }

    let _ = menu.append(&PredefinedMenuItem::separator());

    let quit_item = MenuItem::with_id("__quit__", "Quit", true, None);
    let _ = menu.append(&quit_item);

    let quit_route = EventRoute {
        pattern: EventPattern::Exact("__quit__".to_string()),
        handler: EventHandler::Sync(Box::new(|_| {
            log::info!("Quit requested");
            Ok(HandlerResult::Quit)
        })),
    };
    all_routes.push(quit_route);

    let router = EventRouter::new(all_routes);
    Ok((menu, router))
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

fn handle_plugin_event(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    event_id: &str,
    plugin_id: &str,
    manifest: &PluginManifest,
) -> Result<HandlerResult> {
    let parts: Vec<&str> = event_id.split("::").collect();
    if parts.len() != 2 {
        return Ok(HandlerResult::Continue);
    }

    let item_id = parts[1];
    let action = find_menu_item_action(&manifest.menu.items, item_id);

    match action {
        Some((ActionType::Run, _)) => {
            let manager = plugin_manager.lock().unwrap();
            manager.execute_plugin(plugin_id)?;
            Ok(HandlerResult::Continue)
        }
        Some((ActionType::ToggleConfig, Some(config_key))) => {
            let manager = plugin_manager.lock().unwrap();
            let plugin = manager.get(plugin_id)
                .ok_or_else(|| anyhow::anyhow!("Plugin not found"))?;

            let current_value = get_config_value(plugin, &config_key)?;
            drop(manager);

            let manager = plugin_manager.lock().unwrap();
            manager.update_plugin_config(plugin_id, &config_key, serde_json::json!(!current_value))?;
            log::info!("Toggled config {} for plugin {}: {}", config_key, plugin_id, !current_value);
            Ok(HandlerResult::Continue)
        }
        Some((ActionType::Settings, _)) => {
            log::info!("Settings not yet implemented for plugin: {}", plugin_id);
            Ok(HandlerResult::Continue)
        }
        _ => {
            log::warn!("Unknown action for menu item: {}", item_id);
            Ok(HandlerResult::Continue)
        }
    }
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

fn get_config_value(plugin: &crate::plugins::Plugin, key: &str) -> Result<bool> {
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
