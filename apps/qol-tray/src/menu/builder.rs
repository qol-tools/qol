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
        if items.is_empty() { continue; }

        let feature_id = format!("feature_{}", idx);
        append_feature_items(&menu, &items, &feature_id);

        let route = create_feature_route(feature_registry.clone(), idx, &feature_id);
        all_routes.push(route);
    }

    let _ = menu.append(&PredefinedMenuItem::separator());

    if update_available {
        all_routes.push(create_update_route(&menu));
    }

    all_routes.push(create_quit_route(&menu));

    let router = EventRouter::new(all_routes);
    Ok((menu, router))
}

fn append_feature_items(menu: &Menu, items: &[PluginMenuItem], feature_id: &str) {
    for item in items {
        append_menu_item_to_menu(menu, item, feature_id);
    }
}

fn append_menu_item_to_menu(menu: &Menu, item: &PluginMenuItem, feature_id: &str) {
    match item {
        PluginMenuItem::Submenu { id, label, items: sub_items } => {
            let full_id = format!("{}::{}", feature_id, id);
            log::debug!("Creating submenu with ID: {}", full_id);
            let submenu = Submenu::with_id(&full_id, label, true);
            sub_items.iter().for_each(|sub| add_menu_item(&submenu, sub, feature_id));
            let _ = menu.append(&submenu);
        }
        PluginMenuItem::Action { id, label, .. } => {
            let full_id = format!("{}::{}", feature_id, id);
            let _ = menu.append(&MenuItem::with_id(&full_id, label, true, None));
        }
        PluginMenuItem::Checkbox { id, label, checked, .. } => {
            let full_id = format!("{}::{}", feature_id, id);
            let _ = menu.append(&CheckMenuItem::with_id(&full_id, label, true, *checked, None));
        }
        PluginMenuItem::Separator => {
            let _ = menu.append(&PredefinedMenuItem::separator());
        }
    }
}

fn create_feature_route(
    feature_registry: Arc<FeatureRegistry>,
    idx: usize,
    feature_id: &str,
) -> EventRoute {
    EventRoute {
        pattern: EventPattern::Prefix(format!("{}::", feature_id)),
        handler: EventHandler::Sync(Box::new(move |event_id| {
            if let Some(feature) = feature_registry.features().get(idx) {
                feature.handle_event(event_id)?;
            }
            Ok(HandlerResult::Continue)
        })),
    }
}

fn create_update_route(menu: &Menu) -> EventRoute {
    let version_label = updates::latest_version()
        .map(|v| format!("⬆ Update to v{}", v))
        .unwrap_or_else(|| "⬆ Update Available".to_string());
    let _ = menu.append(&MenuItem::with_id("__update__", &version_label, true, None));

    EventRoute {
        pattern: EventPattern::Exact("__update__".to_string()),
        handler: EventHandler::Sync(Box::new(|_| {
            log::info!("Starting update download and install");
            spawn_update_task();
            Ok(HandlerResult::Continue)
        })),
    }
}

fn spawn_update_task() {
    std::thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        if let Err(e) = rt.block_on(updates::download_and_install()) {
            log::error!("Update failed: {}", e);
        }
    });
}

fn create_quit_route(menu: &Menu) -> EventRoute {
    let _ = menu.append(&MenuItem::with_id("__quit__", "Quit", true, None));

    EventRoute {
        pattern: EventPattern::Exact("__quit__".to_string()),
        handler: EventHandler::Sync(Box::new(|_| {
            log::info!("Quit requested");
            Ok(HandlerResult::Quit)
        })),
    }
}

fn add_menu_item(parent: &Submenu, item: &PluginMenuItem, prefix_id: &str) {
    match item {
        PluginMenuItem::Action { id, label, .. } => {
            let full_id = format!("{}::{}", prefix_id, id);
            let _ = parent.append(&MenuItem::with_id(&full_id, label, true, None));
        }
        PluginMenuItem::Checkbox { id, label, checked, .. } => {
            let full_id = format!("{}::{}", prefix_id, id);
            let _ = parent.append(&CheckMenuItem::with_id(&full_id, label, true, *checked, None));
        }
        PluginMenuItem::Separator => {
            let _ = parent.append(&PredefinedMenuItem::separator());
        }
        PluginMenuItem::Submenu { id, label, items } => {
            let full_id = format!("{}::{}", prefix_id, id);
            let submenu = Submenu::with_id(&full_id, label, true);
            items.iter().for_each(|sub| add_menu_item(&submenu, sub, prefix_id));
            let _ = parent.append(&submenu);
        }
    }
}
