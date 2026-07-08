use std::collections::HashSet;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_plugin_daemon::notification::send_notification;

use crate::fixes::match_devices;
use crate::state::{compute, FixState, SystemPaths};
use crate::{apply, detect, platform};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

const POLL_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct TargetStatus {
    pub fix_id: &'static str,
    pub mac: String,
    pub summary: &'static str,
    pub state: FixState,
}

pub fn snapshot() -> Vec<TargetStatus> {
    let paths = SystemPaths::real();
    let devices = detect::read_devices();
    match_devices(&devices)
        .iter()
        .map(|target| TargetStatus {
            fix_id: target.entry.id,
            mac: target.mac.to_string(),
            summary: target.entry.summary,
            state: compute(
                &paths,
                target,
                platform::driver_installed(target.entry.driver),
            ),
        })
        .collect()
}

pub fn run_from_env() -> Result<()> {
    thread::spawn(poll_loop);
    core_daemon::run_stateful_listener(&DAEMON_CONFIG, (), |(), action| handle_action(action))
        .context("plugin-controllers daemon listener failed")
}

fn poll_loop() {
    let mut notified: HashSet<String> = HashSet::new();
    loop {
        for status in snapshot() {
            let key = format!("{}/{}", status.fix_id, status.mac);
            if notified.contains(&key) {
                continue;
            }
            match status.state {
                FixState::Pending | FixState::LiveOnly => {
                    notified.insert(key);
                    send_notification(
                        "Controller fix available",
                        &format!("{} - run Apply Controller Fixes", status.summary),
                    );
                }
                FixState::DriverMissing => {
                    notified.insert(key);
                    send_notification(
                        "Controller driver missing",
                        &format!("{} - install xpadneo first", status.summary),
                    );
                }
                FixState::Applied => {}
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn is_supported_action(action: &str) -> bool {
    matches!(action, "apply_fixes" | "settings" | "status")
}

fn handle_action(action: &str) -> ReadResult<()> {
    match action {
        "apply_fixes" => match apply_pending() {
            Ok(message) => {
                send_notification("Controller fixes", &message);
                ReadResult::Handled
            }
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        "settings" => match platform::open_settings() {
            Ok(()) => ReadResult::Handled,
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        "status" => ReadResult::HandledWithData(status_json()),
        _ => ReadResult::Error(format!("unknown action: {action}")),
    }
}

fn status_json() -> serde_json::Value {
    let statuses: Vec<serde_json::Value> = snapshot()
        .iter()
        .map(|status| {
            serde_json::json!({
                "fix": status.fix_id,
                "mac": status.mac,
                "state": format!("{:?}", status.state),
            })
        })
        .collect();
    serde_json::json!({ "targets": statuses })
}

fn apply_pending() -> Result<String> {
    let paths = SystemPaths::real();
    let devices = detect::read_devices();
    let targets = match_devices(&devices);
    if targets.is_empty() {
        bail!("no known controllers connected");
    }
    let actionable: Vec<_> = targets
        .into_iter()
        .filter(|target| platform::driver_installed(target.entry.driver))
        .collect();
    if actionable.is_empty() {
        bail!("driver missing; install xpadneo: https://github.com/atar-axis/xpadneo");
    }
    let pending: Vec<_> = actionable
        .iter()
        .filter(|target| compute(&paths, target, true) != FixState::Applied)
        .cloned()
        .collect();
    if pending.is_empty() {
        return Ok("all fixes already applied".to_string());
    }
    apply::apply(&actionable)?;
    Ok(format!("applied {} fix(es)", pending.len()))
}

pub fn execute_action_once(action: &str) -> Result<()> {
    if !is_supported_action(action) {
        bail!("unknown action: {action}");
    }
    match handle_action(action) {
        ReadResult::Handled => Ok(()),
        ReadResult::HandledWithData(data) => {
            println!("{data}");
            Ok(())
        }
        ReadResult::Error(message) => bail!(message),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_dispatch_recognizes_known_actions() {
        let cases = [
            ("apply_fixes", true),
            ("settings", true),
            ("status", true),
            ("bogus", false),
        ];
        for (action, known) in cases {
            assert_eq!(is_supported_action(action), known, "action: {action}");
        }
    }
}
