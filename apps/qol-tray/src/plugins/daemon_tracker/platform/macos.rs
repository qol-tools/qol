use crate::plugins::Plugin;

pub fn kill_orphan_daemons() {}

pub fn clean_stale_sockets(plugins: &[Plugin]) {
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
