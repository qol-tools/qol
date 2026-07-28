use anyhow::{bail, Result};
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(8);
const FORCED_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_TERM_GRACE: Duration = Duration::from_secs(2);
const WAIT_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrackedDaemonPid {
    pub(crate) plugin_id: String,
    pub(crate) pid: u32,
    executable: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShutdownMethod {
    Graceful,
    Forced,
}

pub(crate) fn stop_existing_tray() -> Result<ShutdownMethod> {
    let daemons = snapshot_runtime_daemon_pids();
    if crate::dev_server::post_shutdown().is_ok()
        && wait_for_clean_shutdown(daemons.clone(), GRACEFUL_TIMEOUT).is_empty()
        && !crate::dev_server::api_port_open()
    {
        return Ok(ShutdownMethod::Graceful);
    }

    crate::host_facade::force_stop_qol_tray()?;
    let remaining = wait_for_clean_shutdown(daemons, FORCED_TIMEOUT);
    let remaining = terminate_daemon_groups(remaining);
    let remaining = wait_for_daemons_to_exit(remaining, FORCED_TIMEOUT);
    if crate::host_facade::qol_tray_running()
        || crate::dev_server::api_port_open()
        || !remaining.is_empty()
    {
        bail!(
            "previous qol-tray did not stop cleanly; tray alive: {}; remaining daemons: {}",
            crate::host_facade::qol_tray_running(),
            format_daemon_pids(&remaining)
        );
    }
    Ok(ShutdownMethod::Forced)
}

pub(crate) fn finish_daemon_shutdown(daemons: Vec<TrackedDaemonPid>) -> Vec<TrackedDaemonPid> {
    let remaining = wait_for_daemons_to_exit(daemons, GRACEFUL_TIMEOUT);
    let remaining = terminate_daemon_groups(remaining);
    wait_for_daemons_to_exit(remaining, FORCED_TIMEOUT)
}

#[cfg(unix)]
pub(crate) fn snapshot_runtime_daemon_pids() -> Vec<TrackedDaemonPid> {
    runtime_daemon_pids_from_dir(&runtime_pids_dir())
        .into_iter()
        .filter(daemon_group_is_owned)
        .collect()
}

#[cfg(not(unix))]
pub(crate) fn snapshot_runtime_daemon_pids() -> Vec<TrackedDaemonPid> {
    Vec::new()
}

pub(crate) fn wait_for_daemons_to_exit(
    mut daemons: Vec<TrackedDaemonPid>,
    timeout: Duration,
) -> Vec<TrackedDaemonPid> {
    let deadline = Instant::now() + timeout;
    loop {
        merge_current_daemons(&mut daemons);
        daemons.retain(daemon_group_is_owned);
        if daemons.is_empty() || Instant::now() >= deadline {
            return daemons;
        }
        std::thread::sleep(WAIT_INTERVAL);
    }
}

pub(crate) fn terminate_daemon_groups(daemons: Vec<TrackedDaemonPid>) -> Vec<TrackedDaemonPid> {
    for daemon in &daemons {
        if daemon_group_is_owned(daemon) {
            qol_process::terminate_group(daemon.pid, DAEMON_TERM_GRACE);
        }
    }
    daemons.into_iter().filter(daemon_group_is_owned).collect()
}

pub(crate) fn format_daemon_pids(daemons: &[TrackedDaemonPid]) -> String {
    if daemons.is_empty() {
        return "none".to_string();
    }
    daemons
        .iter()
        .map(|daemon| format!("{}={}", daemon.plugin_id, daemon.pid))
        .collect::<Vec<_>>()
        .join(", ")
}

fn wait_for_clean_shutdown(
    mut daemons: Vec<TrackedDaemonPid>,
    timeout: Duration,
) -> Vec<TrackedDaemonPid> {
    let deadline = Instant::now() + timeout;
    loop {
        merge_current_daemons(&mut daemons);
        daemons.retain(daemon_group_is_owned);
        if !crate::host_facade::qol_tray_running()
            && !crate::dev_server::api_port_open()
            && daemons.is_empty()
        {
            return daemons;
        }
        if Instant::now() >= deadline {
            return daemons;
        }
        std::thread::sleep(WAIT_INTERVAL);
    }
}

fn merge_current_daemons(daemons: &mut Vec<TrackedDaemonPid>) {
    #[cfg(unix)]
    daemons.extend(
        runtime_daemon_pids_from_dir(&runtime_pids_dir())
            .into_iter()
            .filter(daemon_group_is_owned),
    );
    daemons.sort_by(|left, right| {
        left.plugin_id
            .cmp(&right.plugin_id)
            .then(left.pid.cmp(&right.pid))
    });
    daemons.dedup();
}

fn runtime_pids_dir() -> PathBuf {
    qol_config::runtime_dir()
        .map(|path| path.join("pids"))
        .unwrap_or_else(|| PathBuf::from(qol_conventions::RUNTIME_PIDS_DIR_PATH))
}

#[cfg(unix)]
fn runtime_daemon_pids_from_dir(pids_dir: &Path) -> Vec<TrackedDaemonPid> {
    let mut daemons: Vec<_> = fs::read_dir(pids_dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()?.to_str()? != "pid" {
                return None;
            }
            let plugin_id = path.file_stem()?.to_str()?.to_string();
            let pid = fs::read_to_string(&path).ok()?.trim().parse().ok()?;
            Some(TrackedDaemonPid {
                plugin_id,
                pid,
                executable: running_executable(pid),
            })
        })
        .collect();
    daemons.sort_by(|left, right| {
        left.plugin_id
            .cmp(&right.plugin_id)
            .then(left.pid.cmp(&right.pid))
    });
    daemons.dedup();
    daemons
}

#[cfg(target_os = "linux")]
fn daemon_group_is_owned(daemon: &TrackedDaemonPid) -> bool {
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
        None => !qol_process::is_pid_alive(daemon.pid) || qol_process::is_pid_zombie(daemon.pid),
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn daemon_group_is_owned(daemon: &TrackedDaemonPid) -> bool {
    qol_process::is_group_alive(daemon.pid)
}

#[cfg(not(unix))]
fn daemon_group_is_owned(daemon: &TrackedDaemonPid) -> bool {
    qol_process::is_pid_alive(daemon.pid)
}

#[cfg(target_os = "linux")]
fn running_executable(pid: u32) -> Option<std::path::PathBuf> {
    fs::read_link(Path::new("/proc").join(pid.to_string()).join("exe")).ok()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn running_executable(_pid: u32) -> Option<std::path::PathBuf> {
    None
}

#[cfg(target_os = "linux")]
fn is_managed_executable(executable: &Path) -> bool {
    if is_host_executable(executable) {
        return false;
    }
    managed_roots()
        .iter()
        .any(|root| executable.starts_with(root))
}

#[cfg(target_os = "linux")]
fn is_host_executable(executable: &Path) -> bool {
    let Some(name) = executable.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name = name.strip_suffix(" (deleted)").unwrap_or(name);
    matches!(name, "qol" | "qol-tray" | "qol-tray-doctor")
}

#[cfg(target_os = "linux")]
fn managed_roots() -> &'static [std::path::PathBuf] {
    static ROOTS: std::sync::OnceLock<Vec<std::path::PathBuf>> = std::sync::OnceLock::new();
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

    #[cfg(unix)]
    #[test]
    fn runtime_daemon_pids_from_dir_reads_valid_pid_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join("plugin-z.pid"), "222").unwrap();
        fs::write(tmp.path().join("plugin-a.pid"), "111\n").unwrap();
        fs::write(tmp.path().join("plugin-b.pid"), "not a pid").unwrap();
        fs::write(tmp.path().join("plugin-c.txt"), "333").unwrap();

        let daemons = runtime_daemon_pids_from_dir(tmp.path());

        assert_eq!(
            daemons,
            vec![
                TrackedDaemonPid {
                    plugin_id: "plugin-a".to_string(),
                    pid: 111,
                    executable: None,
                },
                TrackedDaemonPid {
                    plugin_id: "plugin-z".to_string(),
                    pid: 222,
                    executable: None,
                },
            ]
        );
    }

    #[test]
    fn format_daemon_pids_matches_shutdown_logs() {
        let daemons = vec![
            TrackedDaemonPid {
                plugin_id: "plugin-a".to_string(),
                pid: 111,
                executable: None,
            },
            TrackedDaemonPid {
                plugin_id: "plugin-z".to_string(),
                pid: 222,
                executable: None,
            },
        ];

        assert_eq!(format_daemon_pids(&daemons), "plugin-a=111, plugin-z=222");
        assert_eq!(format_daemon_pids(&[]), "none");
    }

    #[cfg(target_os = "linux")]
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
