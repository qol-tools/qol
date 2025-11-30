use super::router::{EventRouter, EventRoute, EventPattern, EventHandler, HandlerResult};
use crate::plugins::{PluginManager, MenuItem as PluginMenuItem, Plugin};
use crate::features::FeatureRegistry;
use anyhow::Result;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tray_icon::menu::{Menu, MenuItem, CheckMenuItem, Submenu, PredefinedMenuItem, IconMenuItem, Icon};

pub fn build_menu(
    plugin_manager: Arc<Mutex<PluginManager>>,
    feature_registry: Arc<FeatureRegistry>,
) -> Result<(Menu, EventRouter)> {
    let menu = Menu::new();
    let mut all_routes = Vec::new();

    let manager = plugin_manager.lock().unwrap();

    for plugin in manager.plugins() {
        let plugin_id = plugin.id.clone();
        let full_id = format!("{}::open", plugin_id);
        let label = &plugin.manifest.menu.label;

        if let Some(icon) = load_plugin_icon(plugin) {
            let icon_item = IconMenuItem::with_id(&full_id, label, true, Some(icon), None);
            let _ = menu.append(&icon_item);
        } else {
            let menu_item = MenuItem::with_id(&full_id, label, true, None);
            let _ = menu.append(&menu_item);
        }

        let route = EventRoute {
            pattern: EventPattern::Prefix(format!("{}::", plugin_id)),
            handler: EventHandler::Sync(Box::new(move |_| {
                open_plugin_ui(&plugin_id)
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

fn open_plugin_ui(plugin_id: &str) -> Result<HandlerResult> {
    let url = format!("http://127.0.0.1:42700/plugins/{}/", plugin_id);
    
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open").arg(&url).spawn()?;

    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(&url).spawn()?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd").args(["/C", "start", &url]).spawn()?;

    log::info!("Opened plugin UI: {}", url);
    Ok(HandlerResult::Continue)
}

fn add_menu_item(parent: &Submenu, item: &PluginMenuItem, prefix_id: &str) {
    match item {
        PluginMenuItem::Action { id, label, .. } => {
            let full_id = format!("{}::{}", prefix_id, id);
            let menu_item = MenuItem::with_id(&full_id, label, true, None);
            let _ = parent.append(&menu_item);
        }
        PluginMenuItem::Checkbox { id, label, checked, .. } => {
            let full_id = format!("{}::{}", prefix_id, id);
            let check_item = CheckMenuItem::with_id(&full_id, label, true, *checked, None);
            let _ = parent.append(&check_item);
        }
        PluginMenuItem::Separator => {
            let _ = parent.append(&PredefinedMenuItem::separator());
        }
        PluginMenuItem::Submenu { id, label, items } => {
            let full_id = format!("{}::{}", prefix_id, id);
            let submenu = Submenu::with_id(&full_id, label, true);
            for sub_item in items {
                add_menu_item(&submenu, sub_item, prefix_id);
            }
            let _ = parent.append(&submenu);
        }
    }
}

fn load_plugin_icon(plugin: &Plugin) -> Option<Icon> {
    let icon_path = plugin.manifest.menu.icon.as_ref()?;
    let full_path = plugin.path.join(icon_path);
    load_icon_from_path(&full_path)
}

fn load_icon_from_path(path: &Path) -> Option<Icon> {
    let img = image::open(path).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    let rgba = img.into_raw();
    Icon::from_rgba(rgba, width, height).ok()
}
