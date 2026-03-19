use crate::plugins::resolver::PluginSource;
use crate::plugins::{Plugin, PluginId};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

const DEV_DAEMON_AUTOSTART_MARKER: &str = ".qol-tray-dev-autostart";

pub(super) fn start_plugin_daemons(
    plugins: &mut [Plugin],
    resolved_sources: &HashMap<PluginId, PluginSource>,
) -> Vec<u32> {
    let pids = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for plugin in plugins {
            if !should_start_daemon(plugin, resolved_sources) {
                continue;
            }
            scope.spawn(|| start_daemon(plugin, &pids));
        }
    });
    pids.into_inner().unwrap()
}

fn should_start_daemon(
    plugin: &Plugin,
    resolved_sources: &HashMap<PluginId, PluginSource>,
) -> bool {
    let daemon_enabled = daemon_enabled(plugin);
    if !daemon_enabled {
        return true;
    }
    let source = resolved_sources.get(&plugin.id);
    should_autostart_daemon_for_source(plugin.id.as_str(), &plugin.path, daemon_enabled, source)
}

fn daemon_enabled(plugin: &Plugin) -> bool {
    plugin
        .manifest
        .daemon
        .as_ref()
        .is_some_and(|daemon| daemon.enabled)
}

fn start_daemon(plugin: &mut Plugin, pids: &Mutex<Vec<u32>>) {
    if let Err(error) = plugin.start_daemon() {
        log::error!("Failed to start daemon for plugin {}: {}", plugin.id, error);
    }
    let Some(pid) = plugin.daemon_pid() else {
        return;
    };
    let Ok(mut guard) = pids.lock() else {
        return;
    };
    guard.push(pid);
}

fn should_autostart_daemon_for_source(
    plugin_id: &str,
    plugin_path: &Path,
    daemon_enabled: bool,
    source: Option<&PluginSource>,
) -> bool {
    if !daemon_enabled {
        return true;
    }
    if !matches!(source, Some(PluginSource::DevLinked)) {
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
