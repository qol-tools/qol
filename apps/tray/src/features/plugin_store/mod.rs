pub(crate) mod github;
pub mod installer;
mod platform;
mod plugin_ui;
mod release_assets;
pub(crate) mod release_integrity;
pub(crate) mod server;
pub(crate) mod source;
mod validation;

use crate::daemon::Daemon;
use crate::features::MenuProvider;
use crate::plugins::{MenuItem as PluginMenuItem, PluginManager};
use anyhow::Result;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) const DEFAULT_SERVER_PORT: u16 = qol_conventions::DEFAULT_PORT;
const MENU_ITEM_ID: &str = "plugins";
const SETTINGS_MENU_ITEM_ID: &str = "settings";
static ACTIVE_SERVER_PORT: AtomicU16 = AtomicU16::new(DEFAULT_SERVER_PORT);

fn server_port() -> u16 {
    ACTIVE_SERVER_PORT.load(Ordering::Relaxed)
}

pub struct Plugins;

impl Default for Plugins {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugins {
    pub fn new() -> Self {
        Self
    }

    pub async fn start_server(
        plugin_manager: Arc<Mutex<PluginManager>>,
        daemon: &Daemon,
        shutdown_tx: tokio::sync::broadcast::Sender<()>,
        sync_service: Arc<crate::features::profile::sync::SyncService>,
        #[cfg(feature = "dev")] daemon_health: tokio::sync::watch::Receiver<
            crate::plugins::daemon_health::HealthSnapshot,
        >,
        #[cfg(feature = "dev")] core_log_controls: crate::logging::CoreControlsHandle,
    ) -> Result<u16> {
        log::info!("Starting plugin server with embedded UI");
        let port = server::start_ui_server(
            plugin_manager,
            daemon,
            shutdown_tx,
            sync_service,
            #[cfg(feature = "dev")]
            daemon_health,
            #[cfg(feature = "dev")]
            core_log_controls,
        )
        .await?;
        ACTIVE_SERVER_PORT.store(port, Ordering::Relaxed);
        log::info!("Plugin server started at http://127.0.0.1:{}", port);
        Ok(port)
    }
}

impl MenuProvider for Plugins {
    fn menu_items(&self) -> Vec<PluginMenuItem> {
        menu_items_with(crate::settings_surface::native_available())
    }

    fn handle_event(&self, event_id: &str) -> Result<()> {
        log::info!("Plugins feature received event: {}", event_id);
        if event_id.ends_with(&format!("::{}", MENU_ITEM_ID)) {
            let url = server::security::browser_url("", server_port());
            crate::paths::open_url(&url)?;
        }
        if let Some(plugin_id) = settings_event_target(event_id) {
            let _ = crate::settings_surface::request(plugin_id);
        }
        Ok(())
    }
}

fn menu_items_with(native_settings: bool) -> Vec<PluginMenuItem> {
    let mut items = vec![PluginMenuItem::Action {
        id: MENU_ITEM_ID.to_string(),
        label: "🌐 Open Dashboard".to_string(),
        action: crate::plugins::ActionType::Run,
        config_key: None,
    }];
    if native_settings {
        items.push(PluginMenuItem::Action {
            id: SETTINGS_MENU_ITEM_ID.to_string(),
            label: "⚙️ Settings".to_string(),
            action: crate::plugins::ActionType::Run,
            config_key: None,
        });
    }
    items
}

fn settings_event_target(event_id: &str) -> Option<&'static str> {
    if event_id.ends_with(&format!("::{}", SETTINGS_MENU_ITEM_ID)) {
        Some(qol_conventions::CORE_PANEL_ID)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_item_appears_only_with_native_surface() {
        let with_native = menu_items_with(true);
        assert_eq!(with_native.len(), 2);
        assert_eq!(menu_item_id(&with_native[0]), MENU_ITEM_ID);
        assert_eq!(menu_item_id(&with_native[1]), SETTINGS_MENU_ITEM_ID);

        let without_native = menu_items_with(false);
        assert_eq!(without_native.len(), 1);
        assert_eq!(menu_item_id(&without_native[0]), MENU_ITEM_ID);
    }

    fn menu_item_id(item: &PluginMenuItem) -> &str {
        match item {
            PluginMenuItem::Action { id, .. } => id,
            PluginMenuItem::Checkbox { id, .. } => id,
            PluginMenuItem::Submenu { id, .. } => id,
            PluginMenuItem::Separator => panic!("unexpected separator"),
        }
    }

    #[test]
    fn settings_event_routes_to_the_core_panel() {
        assert_eq!(
            settings_event_target("feature_0::settings"),
            Some(qol_conventions::CORE_PANEL_ID)
        );
        assert_eq!(settings_event_target("feature_0::plugins"), None);
        assert_eq!(settings_event_target("settings"), None);
        assert_eq!(settings_event_target("__quit__"), None);
    }
}
