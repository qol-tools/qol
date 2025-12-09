use super::{Plugin, PluginLoader};
use crate::paths;
use anyhow::Result;
use std::collections::HashMap;

pub struct PluginManager {
    plugins: HashMap<String, Plugin>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn load_plugins(&mut self) -> Result<()> {
        kill_orphan_daemons();

        let plugins = PluginLoader::load_all()?;
        let mut pids = Vec::new();

        for mut plugin in plugins {
            if let Err(e) = plugin.start_daemon() {
                log::error!("Failed to start daemon for plugin {}: {}", plugin.id, e);
            }
            if let Some(pid) = plugin.daemon_pid() {
                pids.push(pid);
            }
            self.plugins.insert(plugin.id.clone(), plugin);
        }

        save_daemon_pids(&pids);
        Ok(())
    }

    #[cfg(feature = "dev")]
    pub fn reload_plugins(&mut self) -> Result<()> {
        log::info!("Reloading all plugins...");
        for plugin in self.plugins.values_mut() {
            if let Err(e) = plugin.stop_daemon() {
                log::error!("Failed to stop daemon for plugin {}: {}", plugin.id, e);
            }
        }
        self.plugins.clear();
        self.load_plugins()
    }

    pub fn plugins(&self) -> impl Iterator<Item = &Plugin> {
        self.plugins.values()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

fn daemon_pids_path() -> Option<std::path::PathBuf> {
    paths::config_dir().ok().map(|p| p.join(".daemon-pids"))
}

fn kill_orphan_daemons() {
    let Some(path) = daemon_pids_path() else { return };
    let Ok(content) = std::fs::read_to_string(&path) else { return };

    for line in content.lines() {
        let Ok(pid) = line.trim().parse::<i32>() else { continue };
        #[cfg(unix)]
        unsafe {
            if libc::kill(pid, 0) == 0 {
                log::info!("Killing orphan daemon process: {}", pid);
                libc::kill(pid, libc::SIGTERM);
                std::thread::sleep(std::time::Duration::from_millis(100));
                if libc::kill(pid, 0) == 0 {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    }

    let _ = std::fs::remove_file(&path);
}

fn save_daemon_pids(pids: &[u32]) {
    let Some(path) = daemon_pids_path() else { return };
    let content = pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("\n");
    let _ = std::fs::write(&path, content);
}
