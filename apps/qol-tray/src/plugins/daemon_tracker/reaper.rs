use super::identity::ManagedRoots;
use super::ManagedProcess;

pub fn kill_orphan_daemons() {
    super::platform::kill_orphan_daemons();
}

pub fn kill_managed_processes(processes: &[ManagedProcess]) -> usize {
    let roots = ManagedRoots::load();
    processes
        .iter()
        .filter(|process| kill_managed_process(process, &roots))
        .count()
}

#[cfg(unix)]
fn kill_managed_process(process: &ManagedProcess, roots: &ManagedRoots) -> bool {
    let Some(current_executable) = super::platform::pid_exe_path(process.pid) else {
        return false;
    };
    if current_executable != process.executable {
        return false;
    }
    if !roots.contains(&current_executable) {
        return false;
    }
    crate::process_utils::terminate_group(process.pid, std::time::Duration::from_millis(100));
    true
}

#[cfg(not(unix))]
fn kill_managed_process(_process: &ManagedProcess, _roots: &ManagedRoots) -> bool {
    false
}

#[cfg(unix)]
use crate::paths;
#[cfg(unix)]
use std::path::{Path, PathBuf};

#[cfg(unix)]
pub(crate) fn kill_from_pid_files() {
    let roots = ManagedRoots::load();
    let pids_dir = crate::paths::runtime_pids_dir();
    for (_, pid) in super::registry::tracked_pids(&pids_dir) {
        kill_pid_if_managed(&(pid as i32).to_string(), &roots);
    }
    super::registry::clear_all(&pids_dir);

    for path in legacy_pid_files() {
        if path.exists() {
            process_pid_file(&path, &roots);
        }
    }
}

#[cfg(unix)]
fn shared_daemon_pids_path() -> Option<PathBuf> {
    paths::shared_config_dir()
        .ok()
        .map(|p| p.join(".daemon-pids"))
}

#[cfg(unix)]
fn legacy_pid_files() -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Some(current) = shared_daemon_pids_path() {
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
fn process_pid_file(path: &Path, roots: &ManagedRoots) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for line in content.lines() {
        kill_pid_if_managed(line, roots);
    }
    let _ = std::fs::remove_file(path);
}

#[cfg(unix)]
fn kill_pid_if_managed(line: &str, roots: &ManagedRoots) {
    let Ok(pid) = line.trim().parse::<i32>() else {
        return;
    };
    if !crate::process_utils::is_pid_alive(pid) {
        crate::process_utils::reap_children_nonblocking();
        return;
    }
    let exe = super::platform::pid_exe_path(pid);
    let is_managed = exe.as_ref().is_some_and(|e| roots.contains(e));
    if !is_managed {
        if exe.is_some() {
            return;
        }
        log::info!(
            "Killing saved daemon pid {} (exe path unavailable - zombie or crashed)",
            pid
        );
    } else {
        log::info!(
            "Killing orphan daemon process: {} ({})",
            pid,
            exe.unwrap().display()
        );
    }
    crate::process_utils::terminate_group(pid, std::time::Duration::from_millis(100));
    crate::process_utils::reap_children_nonblocking();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn kill_managed_process_rejects_changed_executable() {
        let process = ManagedProcess {
            pid: std::process::id() as i32,
            executable: PathBuf::from("/different/plugin-binary"),
        };

        assert!(!kill_managed_process(&process, &ManagedRoots::load()));
    }
}
