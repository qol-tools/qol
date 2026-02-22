use super::Plugin;
use crate::paths;

fn daemon_pids_path() -> Option<std::path::PathBuf> {
    paths::config_dir().ok().map(|p| p.join(".daemon-pids"))
}

#[cfg(target_os = "linux")]
pub fn kill_orphan_daemons() {
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
pub fn kill_orphan_daemons() {}

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
pub fn clean_stale_sockets(_plugins: &[Plugin]) {}

pub fn save_daemon_pids(pids: &[u32]) {
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
