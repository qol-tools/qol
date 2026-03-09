use super::{loading, PluginManager};
use crate::plugins::{action_executor::kill_all_plugin_processes, Plugin};
use anyhow::Result;

pub(super) fn reload_plugins(manager: &mut PluginManager) -> Result<()> {
    log::info!("Reloading all plugins...");
    stop_all_plugins(manager);
    loading::load_plugins(manager)
}

pub(super) fn shutdown(manager: &mut PluginManager) {
    log::info!("Shutting down plugins...");
    stop_all_plugins(manager);
}

pub(super) fn restart_running_plugin_daemon(
    manager: &mut PluginManager,
    plugin_id: &str,
) -> Result<()> {
    let plugin = plugin_mut(manager, plugin_id)?;
    if plugin.daemon_pid().is_none() {
        return Ok(());
    }

    plugin.stop_daemon()?;
    plugin.start_daemon()?;
    sync_ignore_pids(manager);
    persist_daemon_pids(manager);
    Ok(())
}

pub(super) fn sync_ignore_pids(manager: &PluginManager) {
    for plugin in manager.plugins.values() {
        let Some(pid) = plugin.daemon_pid() else {
            continue;
        };
        log::info!("Ignoring daemon pid {} for plugin {}", pid, plugin.id);
        #[cfg(unix)]
        crate::desktop_state::add_ignore_pid(pid);
    }
}

fn stop_all_plugins(manager: &mut PluginManager) {
    kill_all_plugin_processes();
    stop_plugin_daemons(manager);
    manager.plugins.clear();
    super::super::daemon_tracker::save_daemon_pids(&[]);
    super::super::daemon_tracker::kill_orphan_daemons();
}

fn stop_plugin_daemons(manager: &mut PluginManager) {
    for plugin in manager.plugins.values_mut() {
        stop_plugin_daemon(plugin);
    }
}

fn stop_plugin_daemon(plugin: &mut Plugin) {
    if let Err(error) = plugin.stop_daemon() {
        log::error!("Failed to stop daemon for plugin {}: {}", plugin.id, error);
    }
}

pub(super) fn persist_daemon_pids(manager: &PluginManager) {
    let pids: Vec<u32> = manager
        .plugins
        .values()
        .filter_map(|p| p.daemon_pid())
        .collect();
    super::super::daemon_tracker::save_daemon_pids(&pids);
}

fn plugin_mut<'a>(manager: &'a mut PluginManager, plugin_id: &str) -> Result<&'a mut Plugin> {
    manager
        .plugins
        .get_mut(plugin_id)
        .ok_or_else(|| anyhow::anyhow!("plugin not found: {}", plugin_id))
}
