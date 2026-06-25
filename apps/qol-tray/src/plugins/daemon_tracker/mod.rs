use super::Plugin;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedProcess {
    pub pid: i32,
    pub executable: PathBuf,
}

#[cfg(unix)]
mod orphan_kill;
pub mod platform;
pub(crate) mod registry;

pub fn kill_orphan_daemons() {
    platform::kill_orphan_daemons();
}

pub fn clean_stale_sockets(plugins: &[Plugin]) {
    platform::clean_stale_sockets(plugins);
}

pub fn managed_processes() -> Vec<ManagedProcess> {
    without_host_binaries(platform::managed_processes())
}

pub fn running_exe_path(pid: i32) -> Option<PathBuf> {
    #[cfg(unix)]
    return platform::pid_exe_path(pid);

    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

fn without_host_binaries(processes: Vec<ManagedProcess>) -> Vec<ManagedProcess> {
    processes
        .into_iter()
        .filter(|process| !is_host_binary(&process.executable))
        .collect()
}

fn is_host_binary(executable: &Path) -> bool {
    let Some(name) = executable.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name = name.strip_suffix(" (deleted)").unwrap_or(name);
    let name = name.strip_suffix(".exe").unwrap_or(name);
    let tray = crate::installer::binary_filename();
    let tray = tray.strip_suffix(".exe").unwrap_or(&tray);
    name == tray || name == "qol" || name == "qol-tray-doctor"
}

pub fn leaked_processes() -> Vec<ManagedProcess> {
    let tracked = registry::tracked_pid_set(&crate::paths::runtime_pids_dir());
    untracked_managed_processes(managed_processes(), &tracked)
}

pub fn kill_managed_processes(processes: &[ManagedProcess]) -> usize {
    let roots = ManagedRoots::load();
    processes
        .iter()
        .filter(|process| kill_managed_process(process, &roots))
        .count()
}

fn untracked_managed_processes(
    processes: Vec<ManagedProcess>,
    tracked: &HashSet<i32>,
) -> Vec<ManagedProcess> {
    processes
        .into_iter()
        .filter(|process| !tracked.contains(&process.pid))
        .collect()
}

#[cfg(unix)]
fn kill_managed_process(process: &ManagedProcess, roots: &ManagedRoots) -> bool {
    let Some(current_executable) = platform::pid_exe_path(process.pid) else {
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
pub(crate) use orphan_kill::{kill_from_pid_files, ManagedRoots};

#[cfg(not(unix))]
struct ManagedRoots;

#[cfg(not(unix))]
impl ManagedRoots {
    fn load() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untracked_managed_processes_filters_tracked_pids() {
        let processes = vec![
            ManagedProcess {
                pid: 10,
                executable: PathBuf::from("/plugins/a"),
            },
            ManagedProcess {
                pid: 20,
                executable: PathBuf::from("/plugins/b"),
            },
        ];
        let tracked = HashSet::from([20]);

        assert_eq!(
            untracked_managed_processes(processes, &tracked),
            vec![ManagedProcess {
                pid: 10,
                executable: PathBuf::from("/plugins/a"),
            }]
        );
    }

    #[cfg(unix)]
    #[test]
    fn kill_managed_process_rejects_changed_executable() {
        let process = ManagedProcess {
            pid: std::process::id() as i32,
            executable: PathBuf::from("/different/plugin-binary"),
        };

        assert!(!kill_managed_process(&process, &ManagedRoots::load()));
    }

    #[test]
    fn without_host_binaries_drops_tray_even_when_binary_replaced_on_disk() {
        let tray_deleted = format!(
            "/qol/target/debug/{} (deleted)",
            crate::installer::binary_filename()
        );
        let processes = vec![
            ManagedProcess {
                pid: 100,
                executable: PathBuf::from(&tray_deleted),
            },
            ManagedProcess {
                pid: 200,
                executable: PathBuf::from("/qol/target/debug/keyremap"),
            },
        ];

        assert_eq!(
            without_host_binaries(processes),
            vec![ManagedProcess {
                pid: 200,
                executable: PathBuf::from("/qol/target/debug/keyremap"),
            }],
            "tray must be recognized as host even when /proc/<pid>/exe shows '(deleted)'",
        );
    }

    #[test]
    fn without_host_binaries_drops_tray_cli_and_doctor_keeps_plugins() {
        let tray = format!("/qol/target/debug/{}", crate::installer::binary_filename());
        let processes = vec![
            ManagedProcess {
                pid: 100,
                executable: PathBuf::from(&tray),
            },
            ManagedProcess {
                pid: 150,
                executable: PathBuf::from("/qol/target/debug/qol"),
            },
            ManagedProcess {
                pid: 175,
                executable: PathBuf::from("/qol/target/debug/qol-tray-doctor"),
            },
            ManagedProcess {
                pid: 200,
                executable: PathBuf::from("/qol/target/debug/keyremap"),
            },
        ];

        assert_eq!(
            without_host_binaries(processes),
            vec![ManagedProcess {
                pid: 200,
                executable: PathBuf::from("/qol/target/debug/keyremap"),
            }],
            "tray, qol cli, and qol-tray-doctor are host binaries; only the plugin remains",
        );
    }
}
