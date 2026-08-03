use anyhow::{bail, Result};
use std::time::{Duration, Instant};

mod platform;

use platform::{DevShutdownPlatform, Platform};

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

pub(crate) fn snapshot_runtime_daemon_pids() -> Vec<TrackedDaemonPid> {
    Platform.snapshot_runtime_daemon_pids()
}

pub(crate) fn configure_tray_child(command: &mut std::process::Command) -> Result<()> {
    Platform
        .configure_tray_child(command)
        .map_err(anyhow::Error::from)
}

fn daemon_group_is_owned(daemon: &TrackedDaemonPid) -> bool {
    Platform.group_is_owned(daemon)
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
    daemons.extend(snapshot_runtime_daemon_pids());
    daemons.sort_by(|left, right| {
        left.plugin_id
            .cmp(&right.plugin_id)
            .then(left.pid.cmp(&right.pid))
    });
    daemons.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
