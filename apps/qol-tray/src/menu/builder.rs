use super::router::{EventRouter, EventRoute, EventPattern, EventHandler, HandlerResult};
use crate::plugins::{PluginManager, MenuItem as PluginMenuItem};
use crate::features::FeatureRegistry;
use crate::updates;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tray_icon::menu::{Menu, MenuItem, CheckMenuItem, Submenu, PredefinedMenuItem};

pub fn build_menu(
    _plugin_manager: Arc<Mutex<PluginManager>>,
    feature_registry: Arc<FeatureRegistry>,
    update_available: bool,
) -> Result<(Menu, EventRouter)> {
    let menu = Menu::new();
    let mut all_routes = Vec::new();

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

    if update_available {
        let version_label = updates::latest_version()
            .map(|v| format!("⬆ Update to v{}", v))
            .unwrap_or_else(|| "⬆ Update Available".to_string());
        let update_item = MenuItem::with_id("__update__", &version_label, true, None);
        let _ = menu.append(&update_item);

        let update_route = EventRoute {
            pattern: EventPattern::Exact("__update__".to_string()),
            handler: EventHandler::Sync(Box::new(|_| {
                log::info!("Starting update download and install");
                std::thread::spawn(|| {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    if let Err(e) = rt.block_on(updates::download_and_install()) {
                        log::error!("Update failed: {}", e);
                    }
                });
                Ok(HandlerResult::Continue)
            })),
        };
        all_routes.push(update_route);
    }

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
