pub mod manifest;
pub mod loader;
pub mod manager;
pub mod config;

pub use manifest::{PluginManifest, MenuItem, ActionType};
pub use loader::PluginLoader;
pub use manager::PluginManager;
pub use config::PluginConfigManager;

use anyhow::Result;
use std::path::PathBuf;
use std::process::{Child, Command};

#[derive(Debug)]
pub struct Plugin {
    pub id: String,
    pub manifest: PluginManifest,
    pub path: PathBuf,
    daemon_process: Option<Child>,
}

impl Plugin {
    pub fn new(id: String, manifest: PluginManifest, path: PathBuf) -> Self {
        Self {
            id,
            manifest,
            path,
            daemon_process: None,
        }
    }

    pub fn start_daemon(&mut self) -> Result<()> {
        let Some(daemon_config) = &self.manifest.daemon else {
            return Ok(());
        };

        if !daemon_config.enabled {
            return Ok(());
        }

        let daemon_path = self.path.join(&daemon_config.command);
        if !daemon_path.exists() {
            anyhow::bail!("Daemon executable not found: {:?}", daemon_path);
        }

        log::info!("Starting daemon for plugin: {}", self.id);
        let child = Command::new(&daemon_path)
            .current_dir(&self.path)
            .spawn()?;

        self.daemon_process = Some(child);
        Ok(())
    }

    pub fn stop_daemon(&mut self) -> Result<()> {
        if let Some(mut child) = self.daemon_process.take() {
            log::info!("Stopping daemon for plugin: {}", self.id);
            child.kill()?;
            child.wait()?;
        }
        Ok(())
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.stop_daemon();
    }
}
