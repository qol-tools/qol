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
        self.stop_all_plugins();
        self.load_plugins()
    }

    pub fn shutdown(&mut self) {
        log::info!("Shutting down plugins...");
        self.stop_all_plugins();
    }

    pub fn get(&self, plugin_id: &str) -> Option<&Plugin> {
        self.plugins.get(plugin_id)
    }

    pub fn plugins(&self) -> impl Iterator<Item = &Plugin> {
        self.plugins.values()
    }

    fn stop_all_plugins(&mut self) {
        kill_all_plugin_processes();
        for plugin in self.plugins.values_mut() {
            if let Err(e) = plugin.stop_daemon() {
                log::error!("Failed to stop daemon for plugin {}: {}", plugin.id, e);
            }
        }
        self.plugins.clear();
        save_daemon_pids(&[]);
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
    kill_orphan_plugin_binaries();
    let installs_root = paths::installs_dir().ok();

    for path in daemon_pid_files() {
        let Ok(content) = std::fs::read_to_string(&path) else { continue };
        for line in content.lines() {
            let Ok(pid) = line.trim().parse::<i32>() else { continue };
            let Some(installs_root) = installs_root.as_ref() else { continue };
            if !is_pid_from_installed_plugin(pid, installs_root) {
                continue;
            }
            if crate::process_utils::is_pid_alive(pid) {
                log::info!("Killing orphan daemon process: {}", pid);
                crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(100));
            }
        }
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(not(unix))]
fn kill_orphan_daemons() {}

#[cfg(unix)]
fn kill_orphan_plugin_binaries() {
    let Some(installs_root) = paths::installs_dir().ok() else {
        return;
    };
    if !installs_root.exists() {
        return;
    }

    let Ok(entries) = std::fs::read_dir("/proc") else { return };

    for entry in entries.filter_map(|e| e.ok()) {
        let pid = match entry.file_name().to_string_lossy().parse::<i32>() {
            Ok(pid) if pid > 0 => pid,
            _ => continue,
        };

        let exe_path = std::path::Path::new("/proc")
            .join(pid.to_string())
            .join("exe");
        let Ok(target) = std::fs::read_link(exe_path) else {
            continue;
        };
        if !is_installed_plugin_binary_path(&target, &installs_root) {
            continue;
        }

        if crate::process_utils::is_pid_alive(pid) {
            crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(50));
        }
    }
}

#[cfg(unix)]
fn is_installed_plugin_binary_path(target: &std::path::Path, installs_root: &std::path::Path) -> bool {
    let resolved_target = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let resolved_installs_root =
        std::fs::canonicalize(installs_root).unwrap_or_else(|_| installs_root.to_path_buf());

    if !resolved_target.starts_with(&resolved_installs_root) {
        return false;
    }

    resolved_target
        .components()
        .any(|component| component.as_os_str() == std::ffi::OsStr::new("plugins"))
}

fn is_pid_from_installed_plugin(pid: i32, installs_root: &std::path::Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let exe_path = std::path::Path::new("/proc")
            .join(pid.to_string())
            .join("exe");
        let Ok(target) = std::fs::read_link(exe_path) else {
            return false;
        };
        return is_installed_plugin_binary_path(&target, installs_root);
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        let _ = installs_root;
        false
    }
}

#[cfg(unix)]
fn daemon_pid_files() -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    if let Some(current) = daemon_pids_path() {
        files.push(current);
    }

    let Some(installs_dir) = paths::installs_dir().ok() else {
        return files;
    };
    let Ok(entries) = std::fs::read_dir(installs_dir) else {
        return files;
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path().join(".daemon-pids");
        if path.exists() {
            files.push(path);
        }
    }

    files
}

#[cfg(unix)]
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
        if !is_managed_daemon_socket_path(path) {
            log::warn!("Skipping unmanaged daemon socket path: {}", socket);
            continue;
        }
        use std::os::unix::fs::FileTypeExt;
        use std::os::unix::net::UnixStream;
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            log::warn!("Skipping symlink daemon socket path: {}", socket);
            continue;
        }
        if !metadata.file_type().is_socket() {
            log::warn!("Skipping non-socket daemon path: {}", socket);
            continue;
        }
        if UnixStream::connect(path).is_ok() {
            log::info!("Stale socket {} has a live listener, skipping", socket);
            continue;
        }
        log::info!("Removing stale socket: {}", socket);
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(unix)]
fn is_managed_daemon_socket_path(path: &std::path::Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !file_name.starts_with("qol-") || !file_name.ends_with(".sock") {
        return false;
    }
    if path.starts_with(std::env::temp_dir()) {
        return true;
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .is_some_and(|runtime_dir| path.starts_with(runtime_dir))
}

#[cfg(not(unix))]
fn clean_stale_sockets(_plugins: &[Plugin]) {}

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
