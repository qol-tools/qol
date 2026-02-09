use super::{action_executor::kill_all_plugin_processes, Plugin, PluginLoader};
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

        let plugins_dir = PluginLoader::ensure_plugin_dir()?;

        #[cfg(feature = "dev")]
        migrate_symlinks_to_registry(&plugins_dir);

        let dev_links = load_dev_links_if_dev();
        let resolved = super::resolver::resolve_all(&plugins_dir, &dev_links);

        for r in &resolved {
            log::info!("Resolved plugin: {} ({:?}) from {:?}", r.id, r.source, r.path);
        }

        let plugins = PluginLoader::load_resolved(&resolved)?;

        clean_stale_sockets(&plugins);

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

    pub fn reload_plugins(&mut self) -> Result<()> {
        log::info!("Reloading all plugins...");
        kill_all_plugin_processes();
        for plugin in self.plugins.values_mut() {
            if let Err(e) = plugin.stop_daemon() {
                log::error!("Failed to stop daemon for plugin {}: {}", plugin.id, e);
            }
        }
        self.plugins.clear();
        self.load_plugins()
    }

    pub fn get(&self, plugin_id: &str) -> Option<&Plugin> {
        self.plugins.get(plugin_id)
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

#[cfg(feature = "dev")]
fn load_dev_links_if_dev() -> HashMap<String, std::path::PathBuf> {
    let Ok(config_dir) = paths::config_dir() else {
        return HashMap::new();
    };
    crate::dev::load_dev_links(&config_dir)
}

#[cfg(not(feature = "dev"))]
fn load_dev_links_if_dev() -> HashMap<String, std::path::PathBuf> {
    HashMap::new()
}

fn daemon_pids_path() -> Option<std::path::PathBuf> {
    paths::config_dir().ok().map(|p| p.join(".daemon-pids"))
}

#[cfg(unix)]
fn kill_orphan_daemons() {
    let Some(path) = daemon_pids_path() else { return };
    let Ok(content) = std::fs::read_to_string(&path) else { return };

    for line in content.lines() {
        let Ok(pid) = line.trim().parse::<i32>() else { continue };
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

#[cfg(not(unix))]
fn kill_orphan_daemons() {}

fn clean_stale_sockets(plugins: &[Plugin]) {
    for plugin in plugins {
        let Some(daemon) = &plugin.manifest.daemon else { continue };
        if !daemon.enabled {
            continue;
        }
        let Some(socket) = daemon.socket.as_deref() else { continue };
        let path = std::path::Path::new(socket);
        if !path.exists() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::net::UnixStream;
            if UnixStream::connect(path).is_ok() {
                log::info!("Stale socket {} has a live listener, skipping", socket);
                continue;
            }
        }
        log::info!("Removing stale socket: {}", socket);
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(feature = "dev")]
fn migrate_symlinks_to_registry(plugins_dir: &std::path::Path) {
    let config_dir = match plugins_dir.parent() {
        Some(d) => d,
        None => return,
    };

    let dev_links_path = config_dir.join("dev-links.json");
    if dev_links_path.exists() {
        return;
    }

    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return;
    };

    let mut migrated = std::collections::HashMap::new();

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_symlink() {
            continue;
        }
        let Ok(target) = std::fs::read_link(&path) else {
            continue;
        };
        let abs_target = if target.is_relative() {
            match plugins_dir.join(&target).canonicalize() {
                Ok(p) => p,
                Err(_) => target,
            }
        } else {
            target
        };
        let id = entry.file_name().to_string_lossy().to_string();
        log::info!("Migrating symlink to dev-link: {} -> {:?}", id, abs_target);
        migrated.insert(id, abs_target);

        let _ = std::fs::remove_file(&path);
    }

    if migrated.is_empty() {
        return;
    }

    if let Ok(content) = serde_json::to_string_pretty(&migrated) {
        let _ = std::fs::write(&dev_links_path, content);
        log::info!("Migrated {} symlinks to dev-links.json", migrated.len());
    }

    for entry in std::fs::read_dir(plugins_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "backup") {
            let restored_name = path.with_extension("");
            if !restored_name.exists() {
                log::info!("Restoring backup: {:?}", path);
                let _ = std::fs::rename(&path, &restored_name);
            } else {
                log::info!("Removing orphan backup: {:?}", path);
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }
}

fn save_daemon_pids(pids: &[u32]) {
    let Some(path) = daemon_pids_path() else { return };
    let content = pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("\n");
    let _ = std::fs::write(&path, content);
}
