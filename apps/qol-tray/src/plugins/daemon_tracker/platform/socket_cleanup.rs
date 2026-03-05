use crate::plugins::Plugin;
use std::path::{Path, PathBuf};

pub(super) enum SocketPathPolicy {
    StandardUnix,
    MacOs,
}

struct SocketCandidate {
    path: PathBuf,
    socket: String,
}

pub(super) fn clean_stale_sockets(plugins: &[Plugin], policy: SocketPathPolicy) {
    for plugin in plugins {
        let Some(candidate) = socket_candidate(plugin, &policy) else {
            continue;
        };
        if has_live_listener(&candidate) {
            continue;
        }
        remove_socket(&candidate);
    }
}

fn socket_candidate(plugin: &Plugin, policy: &SocketPathPolicy) -> Option<SocketCandidate> {
    let candidate = existing_socket(plugin)?;
    let candidate = managed_socket(candidate, policy)?;
    socket_file(candidate)
}

fn existing_socket(plugin: &Plugin) -> Option<SocketCandidate> {
    let daemon = plugin.manifest.daemon.as_ref()?;
    if !daemon.enabled {
        return None;
    }
    let socket = daemon.socket.clone()?;
    let path = PathBuf::from(&socket);
    if !path.exists() {
        return None;
    }
    Some(SocketCandidate { path, socket })
}

fn managed_socket(
    candidate: SocketCandidate,
    policy: &SocketPathPolicy,
) -> Option<SocketCandidate> {
    if is_managed_daemon_socket_path(&candidate.path, policy) {
        return Some(candidate);
    }
    log::warn!(
        "Skipping unmanaged daemon socket path: {}",
        candidate.socket
    );
    None
}

fn socket_file(candidate: SocketCandidate) -> Option<SocketCandidate> {
    use std::os::unix::fs::FileTypeExt;

    let metadata = std::fs::symlink_metadata(&candidate.path).ok()?;
    if metadata.file_type().is_symlink() {
        log::warn!("Skipping symlink daemon socket path: {}", candidate.socket);
        return None;
    }
    if metadata.file_type().is_socket() {
        return Some(candidate);
    }
    log::warn!("Skipping non-socket daemon path: {}", candidate.socket);
    None
}

fn has_live_listener(candidate: &SocketCandidate) -> bool {
    use std::os::unix::net::UnixStream;

    if UnixStream::connect(&candidate.path).is_err() {
        return false;
    }
    log::info!(
        "Stale socket {} has a live listener, skipping",
        candidate.socket
    );
    true
}

fn remove_socket(candidate: &SocketCandidate) {
    log::info!("Removing stale socket: {}", candidate.socket);
    let _ = std::fs::remove_file(&candidate.path);
}

fn is_managed_daemon_socket_path(path: &Path, policy: &SocketPathPolicy) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !file_name.starts_with("qol-") || !file_name.ends_with(".sock") {
        return false;
    }
    if starts_in_allowed_temp_root(path, policy) {
        return true;
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .is_some_and(|runtime_dir| path.starts_with(runtime_dir))
}

fn starts_in_allowed_temp_root(path: &Path, policy: &SocketPathPolicy) -> bool {
    if path.starts_with(std::env::temp_dir()) {
        return true;
    }
    if matches!(policy, SocketPathPolicy::StandardUnix) {
        return false;
    }
    starts_in_macos_temp_root(path)
}

fn starts_in_macos_temp_root(path: &Path) -> bool {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    starts_in_root(path, "/tmp")
        || starts_in_root(path, "/private/tmp")
        || starts_in_root(&canonical, "/tmp")
        || starts_in_root(&canonical, "/private/tmp")
}

fn starts_in_root(path: &Path, root: &str) -> bool {
    path.starts_with(Path::new(root))
}
