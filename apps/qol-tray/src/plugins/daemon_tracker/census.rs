use super::ManagedProcess;
use std::collections::HashSet;

pub fn managed_processes() -> Vec<ManagedProcess> {
    without_host_binaries(super::platform::managed_processes())
}

pub fn leaked_processes() -> Vec<ManagedProcess> {
    let tracked = super::registry::tracked_pid_set(&crate::paths::runtime_pids_dir());
    untracked_managed_processes(managed_processes(), &tracked)
}

fn without_host_binaries(processes: Vec<ManagedProcess>) -> Vec<ManagedProcess> {
    processes
        .into_iter()
        .filter(|process| !super::platform::is_host_binary(&process.executable))
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
