use std::fs;
use std::path::{Path, PathBuf};

use super::pid_files;
use super::DaemonSupervision;
use crate::dev_shutdown::TrackedDaemonPid;

pub(crate) struct Platform;

impl DaemonSupervision for Platform {
    fn snapshot_runtime_daemon_pids(&self) -> Vec<TrackedDaemonPid> {
        pid_files::tracked_pids_from_dir(&pid_files::runtime_pids_dir())
            .into_iter()
            .map(|mut daemon| {
                daemon.executable = running_executable(daemon.pid);
                daemon
            })
            .filter(|daemon| self.group_is_owned(daemon))
            .collect()
    }

    fn group_is_owned(&self, daemon: &TrackedDaemonPid) -> bool {
        if !qol_process::is_group_alive(daemon.pid) {
            return false;
        }
        let Some(recorded) = &daemon.executable else {
            return false;
        };
        if !is_managed_executable(recorded) {
            return false;
        }
        match running_executable(daemon.pid) {
            Some(current) => current == *recorded,
            None => {
                !qol_process::is_pid_alive(daemon.pid) || qol_process::is_pid_zombie(daemon.pid)
            }
        }
    }
}

fn running_executable(pid: u32) -> Option<PathBuf> {
    fs::read_link(Path::new("/proc").join(pid.to_string()).join("exe")).ok()
}

fn is_managed_executable(executable: &Path) -> bool {
    if is_host_executable(executable) {
        return false;
    }
    managed_roots()
        .iter()
        .any(|root| executable.starts_with(root))
}

fn is_host_executable(executable: &Path) -> bool {
    let Some(name) = executable.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name = name.strip_suffix(" (deleted)").unwrap_or(name);
    matches!(name, "qol" | "qol-tray" | "qol-tray-doctor")
}

fn managed_roots() -> &'static [PathBuf] {
    static ROOTS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let mut roots = Vec::new();
            if let Ok(root) = crate::workspace::repo_root() {
                roots.push(root.join("plugins"));
                roots.push(root.join("target"));
            }
            if let Some(root) = qol_config::data_dir() {
                roots.push(root);
            }
            if let Some(config_dir) = qol_config::config_dir() {
                roots.extend(qol_dev_build::registry::dev_linked_paths(&config_dir).into_values());
            }
            for root in &mut roots {
                if let Ok(canonical) = root.canonicalize() {
                    *root = canonical;
                }
            }
            roots.sort();
            roots.dedup();
            roots
        })
        .as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_executable_accepts_workspace_artifacts_and_rejects_unrelated_processes() {
        let workspace_artifact = crate::workspace::repo_root()
            .unwrap()
            .join("target/debug/plugin-test");

        assert!(is_managed_executable(&workspace_artifact));
        assert!(!is_managed_executable(Path::new("/usr/bin/sleep")));
        assert!(!is_managed_executable(Path::new("/tmp/qol-tray")));
    }
}
