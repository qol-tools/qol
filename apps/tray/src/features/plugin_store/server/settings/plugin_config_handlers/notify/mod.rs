mod platform;

#[cfg(feature = "dev")]
use super::super::super::types::AppState;
use crate::plugins::manager::reload_delivery::ReloadDeliveryOutcome;
use crate::plugins::PluginManager;
use std::sync::{Arc, Mutex};

#[cfg(feature = "dev")]
pub(in crate::features::plugin_store::server) fn notify_plugin_reload(
    state: &AppState,
    plugin_id: &str,
) -> Result<(), String> {
    match notify_plugin_reload_outcome(&state.plugin_manager, plugin_id)? {
        ReloadDeliveryOutcome::Handled
        | ReloadDeliveryOutcome::NoDaemon
        | ReloadDeliveryOutcome::SaveFailed
        | ReloadDeliveryOutcome::SnapshotFailed => Ok(()),
        ReloadDeliveryOutcome::Failed => {
            restart_running_plugin_daemon(&state.plugin_manager, plugin_id)
        }
    }
}

pub(super) fn notify_plugin_reload_outcome(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
) -> Result<ReloadDeliveryOutcome, String> {
    let snapshot = daemon_snapshot(plugin_manager, plugin_id)?;
    if !snapshot.running {
        return Ok(ReloadDeliveryOutcome::NoDaemon);
    }
    if let Some(socket_path) = snapshot.socket_path.as_deref() {
        if platform::notify_plugin_reload(socket_path) {
            return Ok(ReloadDeliveryOutcome::Handled);
        }
    }
    Ok(ReloadDeliveryOutcome::Failed)
}

#[cfg(feature = "dev")]
fn restart_running_plugin_daemon(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
) -> Result<(), String> {
    let mut manager = plugin_manager
        .lock()
        .map_err(|e| format!("Plugin manager mutex poisoned: {}", e))?;
    manager
        .restart_running_plugin_daemon(plugin_id)
        .map_err(|e| e.to_string())
}

struct DaemonSnapshot {
    running: bool,
    socket_path: Option<String>,
}

fn daemon_snapshot(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
) -> Result<DaemonSnapshot, String> {
    let manager = plugin_manager
        .lock()
        .map_err(|e| format!("Plugin manager mutex poisoned: {}", e))?;
    let plugin = manager
        .get(plugin_id)
        .ok_or_else(|| format!("plugin not found: {}", plugin_id))?;
    let daemon = match plugin.manifest.daemon.as_ref() {
        Some(daemon) => daemon,
        None => {
            return Ok(DaemonSnapshot {
                running: false,
                socket_path: None,
            });
        }
    };
    if !daemon.enabled || plugin.daemon_pid().is_none() {
        return Ok(DaemonSnapshot {
            running: false,
            socket_path: resolved_socket(daemon),
        });
    }
    Ok(DaemonSnapshot {
        running: true,
        socket_path: resolved_socket(daemon),
    })
}

fn resolved_socket(daemon: &crate::plugins::manifest::DaemonConfig) -> Option<String> {
    daemon
        .socket
        .as_deref()
        .map(crate::dev_generation::daemon_socket_path)
        .map(|path| path.to_string_lossy().to_string())
}
