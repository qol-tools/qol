use crate::plugins::Plugin;
use std::path::{Path, PathBuf};

pub(super) fn pid_exe_path(pid: i32) -> Option<PathBuf> {
    std::fs::read_link(proc_exe_path(pid)).ok()
}

fn proc_exe_path(pid: i32) -> PathBuf {
    Path::new("/proc").join(pid.to_string()).join("exe")
}

pub(super) fn kill_orphan_daemons() {
    kill_orphan_plugin_binaries();
    super::super::kill_from_pid_files();
}

fn kill_orphan_plugin_binaries() {
    let roots = super::super::ManagedRoots::load();
    let Some(entries) = proc_entries() else {
        return;
    };

    for entry in entries.flatten() {
        let Some(pid) = entry_pid(&entry) else {
            continue;
        };
        kill_managed_process(pid, &roots);
    }
}

fn proc_entries() -> Option<std::fs::ReadDir> {
    std::fs::read_dir("/proc").ok()
}

fn entry_pid(entry: &std::fs::DirEntry) -> Option<i32> {
    match entry.file_name().to_string_lossy().parse::<i32>() {
        Ok(pid) if pid > 0 => Some(pid),
        _ => None,
    }
}

fn kill_managed_process(pid: i32, roots: &super::super::ManagedRoots) {
    let Some(target) = pid_exe_path(pid) else {
        return;
    };
    if !roots.contains(&target) {
        return;
    }
    terminate_process(pid, &target);
}

fn terminate_process(pid: i32, target: &Path) {
    if !crate::process_utils::is_pid_alive(pid) {
        return;
    }
    log::info!(
        "Killing orphan plugin process: {} ({})",
        pid,
        target.display()
    );
    crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(50));
}

pub(super) fn clean_stale_sockets(plugins: &[Plugin]) {
    super::socket_cleanup::clean_stale_sockets(
        plugins,
        super::socket_cleanup::SocketPathPolicy::StandardUnix,
    );
}
