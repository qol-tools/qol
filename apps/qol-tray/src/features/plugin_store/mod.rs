pub(crate) mod github;
mod installer;
mod plugin_ui;
mod release_assets;
pub(crate) mod server;
mod validation;

use crate::daemon::Daemon;
use crate::features::MenuProvider;
use crate::plugins::{MenuItem as PluginMenuItem, PluginManager};
use anyhow::Result;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

const DEFAULT_SERVER_PORT: u16 = 42700;
const MENU_ITEM_ID: &str = "plugins";
static ACTIVE_SERVER_PORT: AtomicU16 = AtomicU16::new(DEFAULT_SERVER_PORT);

fn server_port() -> u16 {
    ACTIVE_SERVER_PORT.load(Ordering::Relaxed)
}

pub struct Plugins;

impl Plugins {
    pub fn new() -> Self {
        Self
    }

    pub async fn start_server(
        plugin_manager: Arc<Mutex<PluginManager>>,
        daemon: &Daemon,
    ) -> Result<()> {
        log::info!("Starting plugin server with embedded UI");
        let port = server::start_ui_server(plugin_manager, daemon).await?;
        ACTIVE_SERVER_PORT.store(port, Ordering::Relaxed);
        log::info!("Plugin server started at http://127.0.0.1:{}", port);
        Ok(())
    }
}

impl MenuProvider for Plugins {
    fn menu_items(&self) -> Vec<PluginMenuItem> {
        vec![PluginMenuItem::Action {
            id: MENU_ITEM_ID.to_string(),
            label: "🌐 Open Dashboard".to_string(),
            action: crate::plugins::ActionType::Run,
            config_key: None,
        }]
    }

    fn handle_event(&self, event_id: &str) -> Result<()> {
        log::info!("Plugins feature received event: {}", event_id);
        if event_id.ends_with(&format!("::{}", MENU_ITEM_ID)) {
            crate::paths::open_url(&format!("http://127.0.0.1:{}", server_port()))?;
        }
        Ok(())
    }
}
