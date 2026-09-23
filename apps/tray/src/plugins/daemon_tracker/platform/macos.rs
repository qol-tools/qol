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
        super::unix::clean_stale_sockets(plugins, true);
    }

    fn kill_managed_process(process: &ManagedProcess, roots: &super::ManagedRoots) -> bool {
        super::unix::kill_managed_process(process, roots, pid_exe_path)
    }
}

const PROC_PIDPATHINFO_MAXSIZE: u32 = 4096;

#[link(name = "proc", kind = "dylib")]
extern "C" {
    fn proc_pidpath(pid: i32, buffer: *mut u8, buffersize: u32) -> i32;
    fn proc_listallpids(buffer: *mut i32, buffersize: i32) -> i32;
}

pub(super) fn pid_exe_path(pid: i32) -> Option<PathBuf> {
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

pub(super) fn kill_orphan_daemons() {
    super::unix::kill_from_pid_files(pid_exe_path);
    kill_orphan_plugin_processes();
}

fn kill_orphan_plugin_processes() {
    for process in super::super::managed_processes() {
        terminate_process(process.pid, &process.executable);
    }
}

pub(super) fn managed_processes() -> Vec<ManagedProcess> {
    let roots = super::unix::ManagedRoots::load();
    let Some(pids) = all_pids() else {
        return Vec::new();
    };

    pids.into_iter()
        .filter_map(|pid| managed_process(pid, &roots))
        .collect()
}

fn all_pids() -> Option<Vec<i32>> {
    let count = listed_pid_count();
    if count <= 0 {
        return None;
    }
    let mut pids = pid_buffer(count);
    let actual = listed_pids(&mut pids);
    if actual <= 0 {
        return None;
    }
    truncate_pids(&mut pids, actual);
    Some(pids)
}

fn listed_pid_count() -> i32 {
    unsafe { proc_listallpids(std::ptr::null_mut(), 0) }
}

fn pid_buffer(count: i32) -> Vec<i32> {
    vec![0i32; (count as usize) * 2]
}

fn listed_pids(pids: &mut [i32]) -> i32 {
    let size = std::mem::size_of_val(pids) as i32;
    unsafe { proc_listallpids(pids.as_mut_ptr(), size) }
}

fn truncate_pids(pids: &mut Vec<i32>, actual: i32) {
    pids.truncate(actual as usize);
    pids.retain(|pid| *pid > 0);
}

fn managed_process(pid: i32, roots: &super::unix::ManagedRoots) -> Option<ManagedProcess> {
    let exe = pid_exe_path(pid)?;
    if !roots.contains(&exe) {
        return None;
    }
    Some(ManagedProcess {
        pid,
        executable: exe,
    })
}

fn terminate_process(pid: i32, exe: &Path) {
    if !crate::process_utils::is_pid_alive(pid) {
        return;
    }
    log::info!("Killing orphan plugin process: {} ({})", pid, exe.display());
    crate::process_utils::terminate_group(pid, std::time::Duration::from_millis(50));
}
