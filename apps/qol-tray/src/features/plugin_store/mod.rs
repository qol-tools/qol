mod server;
mod github;
mod installer;
mod plugin_ui;

use crate::features::MenuProvider;
use crate::plugins::MenuItem as PluginMenuItem;
use anyhow::Result;

const SERVER_PORT: u16 = 42700;

pub struct PluginStore;

impl PluginStore {
    pub fn new() -> Self {
        Self
    }

    pub async fn start_server() -> Result<()> {
        let ui_dir = std::env::current_dir()?.join("ui");
        log::info!("Starting plugin server from: {:?}", ui_dir);
        let _server = server::start_ui_server(ui_dir.to_str().unwrap()).await?;
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
            open_url(&format!("http://127.0.0.1:{}", SERVER_PORT))?;
        }
        Ok(())
    }
}

fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open").arg(url).spawn()?;

    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(url).spawn()?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd").args(["/C", "start", url]).spawn()?;

    Ok(())
}
