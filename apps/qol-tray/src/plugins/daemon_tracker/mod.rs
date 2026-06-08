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

pub fn kill_orphan_daemons() {
    platform::kill_orphan_daemons();
}

pub fn clean_stale_sockets(plugins: &[Plugin]) {
    platform::clean_stale_sockets(plugins);
}

pub fn managed_processes() -> Vec<ManagedProcess> {
    without_host_binaries(platform::managed_processes())
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
    let tracked = tracked_pid_set(&crate::paths::runtime_pids_dir());
    untracked_managed_processes(managed_processes(), &tracked)
}

pub fn kill_managed_processes(processes: &[ManagedProcess]) -> usize {
    let roots = ManagedRoots::load();
    processes
        .iter()
        .filter(|process| kill_managed_process(process, &roots))
        .count()
}

pub fn save_plugin_pid(pids_dir: &Path, plugin_id: &str, pid: u32) {
    let path = pids_dir.join(format!("{}.pid", plugin_id));
    let _ = std::fs::write(&path, pid.to_string());
}

pub fn remove_plugin_pid(pids_dir: &Path, plugin_id: &str) {
    let path = pids_dir.join(format!("{}.pid", plugin_id));
    let _ = std::fs::remove_file(&path);
}

pub fn list_tracked_pids(pids_dir: &Path) -> impl Iterator<Item = (String, u32)> {
    std::fs::read_dir(pids_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()?.to_str()? != "pid" {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            let pid: u32 = std::fs::read_to_string(&path).ok()?.trim().parse().ok()?;
            Some((id, pid))
        })
}

pub fn clear_all_pids(pids_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(pids_dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "pid") {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn tracked_pid_set(pids_dir: &Path) -> HashSet<i32> {
    list_tracked_pids(pids_dir)
        .map(|(_, pid)| pid as i32)
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
    crate::process_utils::terminate_pid(process.pid, std::time::Duration::from_millis(100));
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
    use tempfile::TempDir;

    #[test]
    fn save_and_load_pid_roundtrip() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "foo", 12345);

        let pid_file = tmp.path().join("foo.pid");
        assert!(pid_file.exists());

        let content = std::fs::read_to_string(&pid_file).unwrap();
        assert_eq!(content.trim(), "12345");
    }

    #[test]
    fn remove_plugin_pid_deletes_file() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "foo", 12345);
        remove_plugin_pid(tmp.path(), "foo");
        assert!(!tmp.path().join("foo.pid").exists());
    }

    #[test]
    fn remove_plugin_pid_noop_when_missing() {
        let tmp = TempDir::new().unwrap();
        remove_plugin_pid(tmp.path(), "nonexistent");
    }

    #[test]
    fn list_tracked_pids_returns_all_entries() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "a", 111);
        save_plugin_pid(tmp.path(), "b", 222);

        let mut pids: Vec<_> = list_tracked_pids(tmp.path()).collect();
        pids.sort_by_key(|(id, _)| id.clone());

        assert_eq!(pids.len(), 2);
        assert_eq!(pids[0], ("a".to_string(), 111));
        assert_eq!(pids[1], ("b".to_string(), 222));
    }

    #[test]
    fn list_tracked_pids_skips_corrupt_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("bad.pid"), "not-a-number").unwrap();
        save_plugin_pid(tmp.path(), "good", 42);

        let pids: Vec<_> = list_tracked_pids(tmp.path()).collect();
        assert_eq!(pids.len(), 1);
        assert_eq!(pids[0], ("good".to_string(), 42));
    }

    #[test]
    fn clear_all_pids_removes_all_pid_files() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "a", 1);
        save_plugin_pid(tmp.path(), "b", 2);
        clear_all_pids(tmp.path());
        assert!(list_tracked_pids(tmp.path()).next().is_none());
    }

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
