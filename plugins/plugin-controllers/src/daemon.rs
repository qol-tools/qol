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
    pub name: &'static str,
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
            name: target.entry.name,
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
    matches!(
        action,
        "apply_fixes" | "settings" | "status" | "controllers_status" | "list_controllers"
    )
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
        "controllers_status" => {
            ReadResult::HandledWithData(controllers_status_payload(&snapshot()))
        }
        "list_controllers" => ReadResult::HandledWithData(list_controllers_payload(&snapshot())),
        _ => ReadResult::Error(format!("unknown action: {action}")),
    }
}

fn aggregate_state(statuses: &[TargetStatus]) -> &'static str {
    if statuses.is_empty() {
        return "none";
    }
    if statuses
        .iter()
        .any(|status| status.state == FixState::DriverMissing)
    {
        return "driver_missing";
    }
    if statuses
        .iter()
        .any(|status| matches!(status.state, FixState::Pending | FixState::LiveOnly))
    {
        return "pending";
    }
    "ok"
}

fn state_label(state: FixState) -> &'static str {
    match state {
        FixState::DriverMissing => "Driver missing",
        FixState::Pending => "Fix available",
        FixState::LiveOnly => "Live only",
        FixState::Applied => "Applied",
    }
}

fn controllers_status_payload(statuses: &[TargetStatus]) -> serde_json::Value {
    let message = match aggregate_state(statuses) {
        "none" => String::new(),
        _ => statuses
            .iter()
            .map(|status| format!("{}: {}", status.name, state_label(status.state)))
            .collect::<Vec<_>>()
            .join("; "),
    };
    serde_json::json!({
        "state": aggregate_state(statuses),
        "message": message,
    })
}

fn list_controllers_payload(statuses: &[TargetStatus]) -> serde_json::Value {
    let rows: Vec<serde_json::Value> = statuses
        .iter()
        .map(|status| {
            serde_json::json!({
                "name": status.name,
                "mac": status.mac,
                "state": state_label(status.state),
            })
        })
        .collect();
    serde_json::json!(rows)
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
            ("controllers_status", true),
            ("list_controllers", true),
            ("bogus", false),
        ];
        for (action, known) in cases {
            assert_eq!(is_supported_action(action), known, "action: {action}");
        }
    }

    fn target_status(state: FixState) -> TargetStatus {
        TargetStatus {
            fix_id: "gulikit-xw-bt-rumble",
            name: "GuliKit Controller XW",
            mac: "06:71:10:20:26:b4".to_string(),
            summary: "summary",
            state,
        }
    }

    #[test]
    fn aggregate_state_picks_worst_first() {
        let cases: [(&str, Vec<FixState>, &str); 5] = [
            ("empty is none", vec![], "none"),
            (
                "all applied is ok",
                vec![FixState::Applied, FixState::Applied],
                "ok",
            ),
            (
                "pending beats applied",
                vec![FixState::Applied, FixState::Pending],
                "pending",
            ),
            (
                "live only counts as pending",
                vec![FixState::LiveOnly],
                "pending",
            ),
            (
                "driver missing beats pending",
                vec![FixState::Pending, FixState::DriverMissing],
                "driver_missing",
            ),
        ];
        for (label, states, expected) in cases {
            let statuses: Vec<TargetStatus> = states.into_iter().map(target_status).collect();
            assert_eq!(aggregate_state(&statuses), expected, "case: {label}");
        }
    }

    #[test]
    fn controllers_status_payload_has_state_and_message() {
        let payload = controllers_status_payload(&[target_status(FixState::Pending)]);
        assert_eq!(payload["state"], "pending");
        assert!(
            payload["message"]
                .as_str()
                .expect("message")
                .contains("GuliKit"),
            "message names the pad: {payload}"
        );
    }

    #[test]
    fn list_controllers_payload_is_row_array() {
        let payload = list_controllers_payload(&[target_status(FixState::Applied)]);
        assert_eq!(
            payload,
            serde_json::json!([{
                "name": "GuliKit Controller XW",
                "mac": "06:71:10:20:26:b4",
                "state": "Applied",
            }])
        );
    }
}
