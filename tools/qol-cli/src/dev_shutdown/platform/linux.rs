use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::pid_files;
use super::DevShutdownPlatform;
use crate::dev_shutdown::TrackedDaemonPid;

pub(crate) struct Platform;

impl DevShutdownPlatform for Platform {
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

    fn configure_tray_child(&self, command: &mut Command) -> std::io::Result<()> {
        let expected_parent = i32::try_from(std::process::id())
            .map_err(|_| std::io::Error::other("qol dev process ID is too large"))?;
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != expected_parent {
                    return Err(std::io::Error::from_raw_os_error(libc::ECHILD));
                }
                Ok(())
            });
        }
        Ok(())
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
    use std::time::{Duration, Instant};

    const HELPER_PATH_ENV: &str = "QOL_DEV_PARENT_WATCH_HELPER_PATH";

    #[test]
    fn managed_executable_accepts_workspace_artifacts_and_rejects_unrelated_processes() {
        let workspace_artifact = crate::workspace::repo_root()
            .unwrap()
            .join("target/debug/plugin-test");

        assert!(is_managed_executable(&workspace_artifact));
        assert!(!is_managed_executable(Path::new("/usr/bin/sleep")));
        assert!(!is_managed_executable(Path::new("/tmp/qol-tray")));
    }

    #[test]
    fn configured_tray_exits_when_qol_dev_parent_exits() {
        let temp = tempfile::tempdir().unwrap();
        let child_path = temp.path().join("child-pid");
        let mut parent = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "dev_shutdown::platform::linux::tests::parent_watch_helper",
                "--nocapture",
            ])
            .env(HELPER_PATH_ENV, &child_path)
            .spawn()
            .unwrap();
        assert!(parent.wait().unwrap().success());
        let child_pid = std::fs::read_to_string(&child_path)
            .unwrap()
            .parse::<u32>()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while qol_process::is_pid_alive(child_pid)
            && !qol_process::is_pid_zombie(child_pid)
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        let alive = qol_process::is_pid_alive(child_pid) && !qol_process::is_pid_zombie(child_pid);
        if alive {
            qol_process::terminate_pid(child_pid, Duration::from_millis(100));
        }
        assert!(!alive, "tray child {child_pid} survived its qol dev parent");
    }

    #[test]
    fn parent_watch_helper() {
        let Some(path) = std::env::var_os(HELPER_PATH_ENV) else {
            return;
        };
        let mut command = Command::new("sleep");
        command.arg("30");
        Platform.configure_tray_child(&mut command).unwrap();
        let child = command.spawn().unwrap();
        let child_pid = child.id();
        std::mem::forget(child);
        std::fs::write(path, child_pid.to_string()).unwrap();
    }
}
