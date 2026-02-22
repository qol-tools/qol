use super::{action_executor::kill_all_plugin_processes, Plugin, PluginLoader};
use crate::paths;
use anyhow::Result;
use std::collections::HashMap;

const DEV_DAEMON_AUTOSTART_MARKER: &str = ".qol-tray-dev-autostart";

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
        let resolved_sources: HashMap<String, super::resolver::PluginSource> = resolved
            .iter()
            .map(|resolved| (resolved.id.clone(), resolved.source.clone()))
            .collect();

        for r in &resolved {
            log::info!(
                "Resolved plugin: {} ({:?}) from {:?}",
                r.id,
                r.source,
                r.path
            );
        }

        let plugins = PluginLoader::load_resolved(&resolved)?;

        clean_stale_sockets(&plugins);

        let mut pids = Vec::new();

        for mut plugin in plugins {
            let daemon_enabled = plugin
                .manifest
                .daemon
                .as_ref()
                .is_some_and(|daemon| daemon.enabled);
            let source = resolved_sources.get(&plugin.id);
            if !should_autostart_daemon_for_source(&plugin.id, &plugin.path, daemon_enabled, source)
            {
                self.plugins.insert(plugin.id.clone(), plugin);
                continue;
            }

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

    pub fn restart_running_plugin_daemon(&mut self, plugin_id: &str) -> Result<()> {
        let Some(plugin) = self.plugins.get_mut(plugin_id) else {
            anyhow::bail!("plugin not found: {}", plugin_id);
        };

        if plugin.daemon_pid().is_none() {
            return Ok(());
        }

        plugin.stop_daemon()?;
        plugin.start_daemon()?;
        Ok(())
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
    let Ok(config_dir) = paths::shared_config_dir() else {
        return HashMap::new();
    };
    crate::dev::load_dev_links(&config_dir)
}

#[cfg(not(feature = "dev"))]
fn load_dev_links_if_dev() -> HashMap<String, std::path::PathBuf> {
    HashMap::new()
}

fn should_autostart_daemon_for_source(
    plugin_id: &str,
    plugin_path: &std::path::Path,
    daemon_enabled: bool,
    source: Option<&super::resolver::PluginSource>,
) -> bool {
    if !daemon_enabled {
        return true;
    }

    if !matches!(source, Some(super::resolver::PluginSource::DevLinked)) {
        return true;
    }

    let marker_path = plugin_path.join(DEV_DAEMON_AUTOSTART_MARKER);
    if marker_path.is_file() {
        return true;
    }

    log::warn!(
        "Daemon autostart blocked for dev-linked plugin {} at {}. Create {} to opt in.",
        plugin_id,
        plugin_path.display(),
        marker_path.display()
    );
    false
}

fn daemon_pids_path() -> Option<std::path::PathBuf> {
    paths::config_dir().ok().map(|p| p.join(".daemon-pids"))
}

#[cfg(target_os = "linux")]
fn kill_orphan_daemons() {
    kill_orphan_plugin_binaries();
    let installs_root = paths::installs_dir().ok();
    let shared_plugins_root = paths::plugins_dir().ok();

    for path in daemon_pid_files() {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in content.lines() {
            let Ok(pid) = line.trim().parse::<i32>() else {
                continue;
            };
            if !is_pid_from_managed_plugin(
                pid,
                installs_root.as_deref(),
                shared_plugins_root.as_deref(),
            ) {
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

#[cfg(not(target_os = "linux"))]
fn kill_orphan_daemons() {}

#[cfg(target_os = "linux")]
fn kill_orphan_plugin_binaries() {
    let installs_root = paths::installs_dir().ok().filter(|root| root.exists());
    let shared_plugins_root = paths::plugins_dir().ok().filter(|root| root.exists());
    if installs_root.is_none() && shared_plugins_root.is_none() {
        return;
    }

    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };

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
        if !is_managed_plugin_binary_path(
            &target,
            installs_root.as_deref(),
            shared_plugins_root.as_deref(),
        ) {
            continue;
        }

        if crate::process_utils::is_pid_alive(pid) {
            crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(50));
        }
    }
}

#[cfg(target_os = "linux")]
fn is_managed_plugin_binary_path(
    target: &std::path::Path,
    installs_root: Option<&std::path::Path>,
    shared_plugins_root: Option<&std::path::Path>,
) -> bool {
    let resolved_target = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());

    if let Some(shared_plugins_root) = shared_plugins_root {
        let resolved_shared_root = std::fs::canonicalize(shared_plugins_root)
            .unwrap_or_else(|_| shared_plugins_root.to_path_buf());
        if resolved_target.starts_with(&resolved_shared_root) {
            return true;
        }
    }

    let Some(installs_root) = installs_root else {
        return false;
    };
    let resolved_installs_root =
        std::fs::canonicalize(installs_root).unwrap_or_else(|_| installs_root.to_path_buf());
    if !resolved_target.starts_with(&resolved_installs_root) {
        return false;
    }
    resolved_target
        .components()
        .any(|component| component.as_os_str() == std::ffi::OsStr::new("plugins"))
}

#[cfg(target_os = "linux")]
fn is_pid_from_managed_plugin(
    pid: i32,
    installs_root: Option<&std::path::Path>,
    shared_plugins_root: Option<&std::path::Path>,
) -> bool {
    let exe_path = std::path::Path::new("/proc")
        .join(pid.to_string())
        .join("exe");
    let Ok(target) = std::fs::read_link(exe_path) else {
        return false;
    };
    is_managed_plugin_binary_path(&target, installs_root, shared_plugins_root)
}

#[cfg(target_os = "linux")]
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
        let Some(daemon) = &plugin.manifest.daemon else {
            continue;
        };
        if !daemon.enabled {
            continue;
        }
        let Some(socket) = daemon.socket.as_deref() else {
            continue;
        };
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
    let Some(path) = daemon_pids_path() else {
        return;
    };
    let content = pids
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(&path, content);
}

#[cfg(test)]
mod tests {
    use super::{should_autostart_daemon_for_source, DEV_DAEMON_AUTOSTART_MARKER};
    use crate::plugins::resolver::PluginSource;

    #[test]
    fn allows_installed_plugin_daemon_autostart() {
        let temp = tempfile::TempDir::new().unwrap();
        assert!(should_autostart_daemon_for_source(
            "plugin",
            temp.path(),
            true,
            Some(&PluginSource::Installed),
        ));
    }

    #[test]
    fn allows_dev_linked_plugin_when_daemon_disabled() {
        let temp = tempfile::TempDir::new().unwrap();
        assert!(should_autostart_daemon_for_source(
            "plugin",
            temp.path(),
            false,
            Some(&PluginSource::DevLinked),
        ));
    }

    #[test]
    fn blocks_dev_linked_plugin_daemon_autostart_without_marker() {
        let temp = tempfile::TempDir::new().unwrap();
        assert!(!should_autostart_daemon_for_source(
            "plugin",
            temp.path(),
            true,
            Some(&PluginSource::DevLinked),
        ));
    }

    #[test]
    fn allows_dev_linked_plugin_daemon_autostart_with_marker() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join(DEV_DAEMON_AUTOSTART_MARKER), "").unwrap();

        assert!(should_autostart_daemon_for_source(
            "plugin",
            temp.path(),
            true,
            Some(&PluginSource::DevLinked),
        ));
    }
}
