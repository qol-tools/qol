use crate::plugins::Plugin;
use std::path::PathBuf;

/// Get the executable path for a given PID by reading `/proc/<pid>/exe`.
pub fn pid_exe_path(pid: i32) -> Option<PathBuf> {
    let exe_path = std::path::Path::new("/proc")
        .join(pid.to_string())
        .join("exe");
    std::fs::read_link(exe_path).ok()
}

pub fn kill_orphan_daemons() {
    // Pass 1: scan all /proc entries for managed plugin binaries
    kill_orphan_plugin_binaries();
    // Pass 2: kill from PID files (shared logic)
    super::super::kill_from_pid_files();
}

fn kill_orphan_plugin_binaries() {
    let roots = super::super::ManagedRoots::load();

    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let pid = match entry.file_name().to_string_lossy().parse::<i32>() {
            Ok(pid) if pid > 0 => pid,
            _ => continue,
        };

        let Some(target) = pid_exe_path(pid) else {
            continue;
        };
        if !roots.contains(&target) {
            continue;
        }

        if crate::process_utils::is_pid_alive(pid) {
            log::info!(
                "Killing orphan plugin process: {} ({})",
                pid,
                target.display()
            );
            crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(50));
        }
    }
}

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
