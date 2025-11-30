pub mod manifest;
pub mod loader;
pub mod manager;

pub use manifest::{PluginManifest, MenuItem, ActionType};
pub use loader::PluginLoader;
pub use manager::PluginManager;

use anyhow::Result;
use std::path::PathBuf;
use std::process::{Child, Command};

#[derive(Debug)]
pub struct Plugin {
    pub id: String,
    pub manifest: PluginManifest,
    pub path: PathBuf,
    pub config_path: PathBuf,
    daemon_process: Option<Child>,
}

impl Plugin {
    pub fn new(id: String, manifest: PluginManifest, path: PathBuf) -> Self {
        let config_path = path.join("config.json");
        Self {
            id,
            manifest,
            path,
            config_path,
            daemon_process: None,
        }
    }

    pub fn execute(&self) -> Result<()> {
        let run_script = self.path.join("run.sh");
        if !run_script.exists() {
            anyhow::bail!("Plugin {} has no run.sh script", self.id);
        }

        log::info!("Executing plugin: {}", self.id);
        Command::new(&run_script)
            .current_dir(&self.path)
            .spawn()?;

        Ok(())
    }

    pub fn start_daemon(&mut self) -> Result<()> {
        if let Some(daemon_config) = &self.manifest.daemon {
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
        }
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

    pub fn update_config(&self, key: &str, value: serde_json::Value) -> Result<()> {
        let mut config = if self.config_path.exists() {
            let content = std::fs::read_to_string(&self.config_path)?;
            serde_json::from_str(&content)?
        } else {
            serde_json::Value::Object(Default::default())
        };

        let keys: Vec<&str> = key.split('.').collect();
        let mut current = &mut config;

        for (i, &k) in keys.iter().enumerate() {
            if i == keys.len() - 1 {
                if let Some(obj) = current.as_object_mut() {
                    obj.insert(k.to_string(), value.clone());
                }
            } else {
                if !current.get(k).is_some() {
                    if let Some(obj) = current.as_object_mut() {
                        obj.insert(k.to_string(), serde_json::json!({}));
                    }
                }
                current = current.get_mut(k).unwrap();
            }
        }

        let content = serde_json::to_string_pretty(&config)?;
        std::fs::write(&self.config_path, content)?;

        log::info!("Updated config for plugin {}: {} = {:?}", self.id, key, value);
        Ok(())
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.stop_daemon();
    }
}
