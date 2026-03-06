use super::super::super::types::AppState;

#[cfg(unix)]
pub(super) fn notify_plugin_reload(state: &AppState, plugin_id: &str) {
    let Some(socket_path) = daemon_socket_path(state, plugin_id) else {
        return;
    };
    send_reload(socket_path);
}

#[cfg(unix)]
fn daemon_socket_path(state: &AppState, plugin_id: &str) -> Option<String> {
    let Ok(manager) = state.plugin_manager.lock() else {
        return None;
    };
    manager
        .get(plugin_id)
        .and_then(|plugin| plugin.manifest.daemon.as_ref()?.socket.clone())
}

#[cfg(unix)]
fn send_reload(socket_path: String) {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let Ok(mut stream) = UnixStream::connect(&socket_path) else {
        return;
    };
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(500)));
    let _ = stream.write_all(b"reload");
}

#[cfg(not(unix))]
pub(super) fn notify_plugin_reload(_state: &AppState, _plugin_id: &str) {}
