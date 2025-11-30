mod server;
mod github;
mod installer;
mod plugin_ui;

use crate::features::MenuProvider;
use crate::plugins::MenuItem as PluginMenuItem;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct PluginStore {
    server_handle: Arc<Mutex<Option<server::UiServerHandle>>>,
}

impl PluginStore {
    pub fn new() -> Self {
        Self {
            server_handle: Arc::new(Mutex::new(None)),
        }
    }
}

impl MenuProvider for PluginStore {
    fn menu_items(&self) -> Vec<PluginMenuItem> {
        vec![
            PluginMenuItem::Submenu {
                id: "plugin_store".to_string(),
                label: "Plugin Store".to_string(),
                items: vec![
                    PluginMenuItem::Action {
                        id: "browse".to_string(),
                        label: "Browse Plugins".to_string(),
                        action: crate::plugins::ActionType::Run,
                        config_key: None,
                    },
                ],
            },
        ]
    }

    fn handle_event(&self, event_id: &str) -> Result<()> {
        log::info!("PluginStore received event: {}", event_id);
        if event_id.ends_with("::plugin_store::browse") || event_id.ends_with("::browse") {
            let handle = self.server_handle.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    if let Err(e) = open_plugin_browser(handle).await {
                        log::error!("Failed to open plugin browser: {}", e);
                    }
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
                    }
                });
            });
        }
        Ok(())
    }
}

async fn open_plugin_browser(handle: Arc<Mutex<Option<server::UiServerHandle>>>) -> Result<()> {
    let mut guard = handle.lock().await;

    if guard.is_none() {
        let ui_dir = std::env::current_dir()?.join("ui");
        log::info!("Serving UI from: {:?}", ui_dir);
        let server = server::start_ui_server(ui_dir.to_str().unwrap()).await?;
        let url = format!("http://{}", server.addr);

        #[cfg(target_os = "linux")]
        std::process::Command::new("xdg-open").arg(&url).spawn()?;

        #[cfg(target_os = "macos")]
        std::process::Command::new("open").arg(&url).spawn()?;

        #[cfg(target_os = "windows")]
        std::process::Command::new("cmd").args(&["/C", "start", &url]).spawn()?;

        log::info!("Plugin store UI started at {}", url);
        *guard = Some(server);
    } else {
        log::info!("Plugin store UI already running");
    }

    Ok(())
}
