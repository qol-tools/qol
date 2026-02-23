use crate::plugins::Plugin;
use std::path::PathBuf;

const PROC_PIDPATHINFO_MAXSIZE: u32 = 4096;

#[link(name = "proc", kind = "dylib")]
extern "C" {
    fn proc_pidpath(pid: i32, buffer: *mut u8, buffersize: u32) -> i32;
    fn proc_listallpids(buffer: *mut i32, buffersize: i32) -> i32;
}

/// Get the executable path for a given PID using macOS `proc_pidpath(2)`.
pub fn pid_exe_path(pid: i32) -> Option<PathBuf> {
    let mut buf = vec![0u8; PROC_PIDPATHINFO_MAXSIZE as usize];
    let ret = unsafe { proc_pidpath(pid, buf.as_mut_ptr(), PROC_PIDPATHINFO_MAXSIZE) };
    if ret <= 0 {
        return None;
    }
    let path = std::ffi::CStr::from_bytes_until_nul(&buf)
        .ok()?
        .to_str()
        .ok()?;
    Some(PathBuf::from(path))
}

pub fn kill_orphan_daemons() {
    // Pass 1: kill from PID files (shared logic)
    super::super::kill_from_pid_files();

    // Pass 2: scan all processes for managed plugin binaries
    kill_orphan_plugin_processes();
}

fn kill_orphan_plugin_processes() {
    let roots = super::super::ManagedRoots::load();

    // Get list of all PIDs
    let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return;
    }

    let mut pids = vec![0i32; (count as usize) * 2];
    let actual = unsafe {
        proc_listallpids(
            pids.as_mut_ptr(),
            (pids.len() * std::mem::size_of::<i32>()) as i32,
        )
    };
    if actual <= 0 {
        return;
    }

    let pid_count = actual as usize;
    for &pid in &pids[..pid_count] {
        if pid <= 0 {
            continue;
        }
        let Some(exe) = pid_exe_path(pid) else {
            continue;
        };
        if !roots.contains(&exe) {
            continue;
        }
        if crate::process_utils::is_pid_alive(pid) {
            log::info!(
                "Killing orphan plugin process: {} ({})",
                pid,
                exe.display()
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
    // On macOS, std::env::temp_dir() returns /var/folders/... but sockets live in /tmp
    // which is a symlink to /private/tmp. Check both.
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let tmp = std::path::Path::new("/tmp");
    let private_tmp = std::path::Path::new("/private/tmp");
    if path.starts_with(tmp) || path.starts_with(private_tmp)
        || canonical.starts_with(tmp) || canonical.starts_with(private_tmp)
        || path.starts_with(std::env::temp_dir())
    {
        return true;
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .is_some_and(|runtime_dir| path.starts_with(runtime_dir))
}
