mod platform;

use super::super::super::types::AppState;

pub(super) fn notify_plugin_reload(state: &AppState, plugin_id: &str) -> Result<(), String> {
    let snapshot = daemon_snapshot(state, plugin_id)?;
    if !snapshot.running {
        return Ok(());
    }
    if let Some(socket_path) = snapshot.socket_path.as_deref() {
        if platform::notify_plugin_reload(socket_path) {
            return Ok(());
        }
    }
    restart_running_plugin_daemon(state, plugin_id)
}

struct DaemonSnapshot {
    running: bool,
    socket_path: Option<String>,
}

fn daemon_snapshot(state: &AppState, plugin_id: &str) -> Result<DaemonSnapshot, String> {
    let manager = state
        .plugin_manager
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
            socket_path: daemon.socket.clone(),
        });
    }
    Ok(DaemonSnapshot {
        running: true,
        socket_path: daemon.socket.clone(),
    })
}

fn restart_running_plugin_daemon(state: &AppState, plugin_id: &str) -> Result<(), String> {
    let mut manager = state
        .plugin_manager
        .lock()
        .map_err(|e| format!("Plugin manager mutex poisoned: {}", e))?;
    manager
        .restart_running_plugin_daemon(plugin_id)
        .map_err(|e| e.to_string())
}
