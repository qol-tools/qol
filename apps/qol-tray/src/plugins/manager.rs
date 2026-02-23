use super::{action_executor::kill_all_plugin_processes, Plugin, PluginLoader};
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
        super::daemon_tracker::kill_orphan_daemons();

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

        let mut plugins = PluginLoader::load_resolved(&resolved)?;

        super::daemon_tracker::clean_stale_sockets(&plugins);

        let pids = std::sync::Mutex::new(Vec::new());

        std::thread::scope(|s| {
            for plugin in &mut plugins {
                let daemon_enabled = plugin
                    .manifest
                    .daemon
                    .as_ref()
                    .is_some_and(|daemon| daemon.enabled);
                let source = resolved_sources.get(&plugin.id);
                if !should_autostart_daemon_for_source(
                    &plugin.id,
                    &plugin.path,
                    daemon_enabled,
                    source,
                ) {
                    continue;
                }

                s.spawn(|| {
                    if let Err(e) = plugin.start_daemon() {
                        log::error!("Failed to start daemon for plugin {}: {}", plugin.id, e);
                    }
                    if let Some(pid) = plugin.daemon_pid() {
                        let mut guard = pids.lock().unwrap();
                        guard.push(pid);
                    }
                });
            }
        });

        for plugin in plugins {
            self.plugins.insert(plugin.id.clone(), plugin);
        }

        super::daemon_tracker::save_daemon_pids(&pids.into_inner().unwrap());
        self.sync_ignore_pids();
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
        self.sync_ignore_pids();
        Ok(())
    }

    fn sync_ignore_pids(&self) {
        for plugin in self.plugins.values() {
            if let Some(pid) = plugin.daemon_pid() {
                log::info!("Ignoring daemon pid {} for plugin {}", pid, plugin.id);
                crate::os::display::add_ignore_pid(pid);
            }
        }
    }

    fn stop_all_plugins(&mut self) {
        kill_all_plugin_processes();
        for plugin in self.plugins.values_mut() {
            if let Err(e) = plugin.stop_daemon() {
                log::error!("Failed to stop daemon for plugin {}: {}", plugin.id, e);
            }
        }
        self.plugins.clear();
        super::daemon_tracker::save_daemon_pids(&[]);
        super::daemon_tracker::kill_orphan_daemons();
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "dev")]
fn load_dev_links_if_dev() -> HashMap<String, std::path::PathBuf> {
    let Ok(config_dir) = crate::paths::shared_config_dir() else {
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
