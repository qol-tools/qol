use crate::plugins::Plugin;
use std::path::{Path, PathBuf};

use super::{DaemonTrackerPlatform, ManagedProcess};

pub(super) struct Platform;

impl DaemonTrackerPlatform for Platform {
    fn pid_exe_path(pid: i32) -> Option<PathBuf> {
        pid_exe_path(pid)
    }

    fn kill_orphan_daemons() {
        kill_orphan_daemons();
    }

    fn managed_processes() -> Vec<ManagedProcess> {
        managed_processes()
    }

    fn clean_stale_sockets(plugins: &[Plugin]) {
        super::unix::clean_stale_sockets(plugins, false);
    }

    fn kill_managed_process(process: &ManagedProcess, roots: &super::ManagedRoots) -> bool {
        super::unix::kill_managed_process(process, roots, pid_exe_path)
    }
}

pub(super) fn pid_exe_path(pid: i32) -> Option<PathBuf> {
    std::fs::read_link(proc_exe_path(pid)).ok()
}

fn proc_exe_path(pid: i32) -> PathBuf {
    Path::new("/proc").join(pid.to_string()).join("exe")
}

pub(super) fn kill_orphan_daemons() {
    kill_orphan_plugin_binaries();
    super::unix::kill_from_pid_files(pid_exe_path);
}

fn kill_orphan_plugin_binaries() {
    for process in super::super::managed_processes() {
        terminate_process(process.pid, &process.executable);
    }
}

pub(super) fn managed_processes() -> Vec<ManagedProcess> {
    let roots = super::unix::ManagedRoots::load();
    let Some(entries) = proc_entries() else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| entry_pid(&entry))
        .filter_map(|pid| managed_process(pid, &roots))
        .collect()
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

fn managed_process(pid: i32, roots: &super::unix::ManagedRoots) -> Option<ManagedProcess> {
    let target = pid_exe_path(pid)?;
    if !roots.contains(&target) {
        return None;
    }
    Some(ManagedProcess {
        pid,
        executable: target,
    })
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
    crate::process_utils::terminate_group(pid, std::time::Duration::from_millis(50));
}
