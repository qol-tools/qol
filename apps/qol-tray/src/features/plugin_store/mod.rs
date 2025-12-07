mod server;
mod github;
mod installer;
mod plugin_ui;

use crate::features::MenuProvider;
use crate::plugins::{MenuItem as PluginMenuItem, PluginManager};
use anyhow::Result;
use std::sync::{Arc, Mutex};

const SERVER_PORT: u16 = 42700;

pub struct PluginStore;

impl PluginStore {
    pub fn new() -> Self {
        Self
    }

    pub async fn start_server(plugin_manager: Arc<Mutex<PluginManager>>) -> Result<()> {
        log::info!("Starting plugin server with embedded UI");
        let _server = server::start_ui_server(plugin_manager).await?;
        log::info!("Plugin server started at http://127.0.0.1:{}", SERVER_PORT);
        std::mem::forget(_server);
        Ok(())
    }
}

impl MenuProvider for PluginStore {
    fn menu_items(&self) -> Vec<PluginMenuItem> {
        vec![
            PluginMenuItem::Action {
                id: "plugin_store".to_string(),
                label: "🔌 Plugin Store".to_string(),
                action: crate::plugins::ActionType::Run,
                config_key: None,
            },
        ]
    }

    fn handle_event(&self, event_id: &str) -> Result<()> {
        log::info!("PluginStore received event: {}", event_id);
        if event_id.ends_with("::plugin_store") {
            crate::paths::open_url(&format!("http://127.0.0.1:{}", SERVER_PORT))?;
        }
        Ok(())
    }
}
