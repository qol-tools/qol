mod server;
pub(crate) mod github;
mod installer;
mod plugin_ui;

use crate::daemon::Daemon;
use crate::features::MenuProvider;
use crate::plugins::{MenuItem as PluginMenuItem, PluginManager};
use anyhow::Result;
use std::sync::{Arc, Mutex};

const SERVER_PORT: u16 = 42700;
const MENU_ITEM_ID: &str = "plugins";

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
        server::start_ui_server(plugin_manager, daemon).await?;
        log::info!("Plugin server started at http://127.0.0.1:{}", SERVER_PORT);
        Ok(())
    }
}

impl MenuProvider for Plugins {
    fn menu_items(&self) -> Vec<PluginMenuItem> {
        vec![
            PluginMenuItem::Action {
                id: MENU_ITEM_ID.to_string(),
                label: "🔌 Plugins".to_string(),
                action: crate::plugins::ActionType::Run,
                config_key: None,
            },
        ]
    }

    fn handle_event(&self, event_id: &str) -> Result<()> {
        log::info!("Plugins feature received event: {}", event_id);
        if event_id.ends_with(&format!("::{}", MENU_ITEM_ID)) {
            crate::paths::open_url(&format!("http://127.0.0.1:{}", SERVER_PORT))?;
        }
        Ok(())
    }
}
