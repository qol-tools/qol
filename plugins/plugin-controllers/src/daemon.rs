use std::collections::HashSet;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_plugin_daemon::notification::send_notification;

use crate::fixes::{match_device, match_devices, DetectedDevice};
use crate::state::{compute, FixState, SystemPaths};
use crate::{apply, detect, platform};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

const POLL_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct ControllerRow {
    pub name: String,
    pub transport: &'static str,
    pub verdict: String,
    pub fixable: bool,
    pub fix_state: Option<FixState>,
}

pub fn snapshot() -> Vec<ControllerRow> {
    let paths = SystemPaths::real();
    let devices = detect::read_devices();
    build_rows(&paths, &devices)
}

fn build_rows(paths: &SystemPaths, devices: &[DetectedDevice]) -> Vec<ControllerRow> {
    devices
        .iter()
        .filter(|device| device.is_gamepad)
        .map(|device| build_row(paths, device))
        .collect()
}

fn build_row(paths: &SystemPaths, device: &DetectedDevice) -> ControllerRow {
    let Some(target) = match_device(device) else {
        return ControllerRow {
            name: device.name.clone(),
            transport: device.transport(),
            verdict: "No known issues".to_string(),
            fixable: false,
            fix_state: None,
        };
    };
    let installed = platform::driver_installed(target.entry.driver);
    let state = compute(paths, &target, installed);
    let problem = target.entry.summary;
    let (verdict, fixable) = match state {
        FixState::DriverMissing => (
            format!(
                "{problem} - needs the {} driver, which is not installed",
                target.entry.driver
            ),
            false,
        ),
        FixState::Pending => (format!("{problem} - fix available"), true),
        FixState::LiveOnly => (format!("{problem} - fixed until reboot only"), true),
        FixState::Applied => (format!("{problem} - fixed"), false),
    };
    ControllerRow {
        name: device.name.clone(),
        transport: device.transport(),
        verdict,
        fixable,
        fix_state: Some(state),
    }
}

pub fn run_from_env() -> Result<()> {
    thread::spawn(poll_loop);
    core_daemon::run_stateful_listener(&DAEMON_CONFIG, (), |(), action| handle_action(action))
        .context("plugin-controllers daemon listener failed")
}

fn poll_loop() {
    let mut notified: HashSet<String> = HashSet::new();
    loop {
        for row in snapshot() {
            let needs_attention = matches!(
                row.fix_state,
                Some(FixState::Pending | FixState::LiveOnly | FixState::DriverMissing)
            );
            if !needs_attention {
                continue;
            }
            let key = format!("{}/{}", row.name, row.verdict);
            if notified.insert(key) {
                send_notification(&row.name, &row.verdict);
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
        "status" => ReadResult::HandledWithData(list_controllers_payload(&snapshot())),
        "controllers_status" => {
            ReadResult::HandledWithData(controllers_status_payload(&snapshot()))
        }
        "list_controllers" => ReadResult::HandledWithData(list_controllers_payload(&snapshot())),
        _ => ReadResult::Error(format!("unknown action: {action}")),
    }
}

fn aggregate_state(rows: &[ControllerRow]) -> &'static str {
    if rows.is_empty() {
        return "none";
    }
    if rows
        .iter()
        .any(|row| row.fix_state == Some(FixState::DriverMissing))
    {
        return "driver_missing";
    }
    if rows.iter().any(|row| row.fixable) {
        return "pending";
    }
    "ok"
}

fn controllers_status_payload(rows: &[ControllerRow]) -> serde_json::Value {
    let message = rows
        .iter()
        .filter(|row| row.fix_state.is_some())
        .map(|row| format!("{}: {}", row.name, row.verdict))
        .collect::<Vec<_>>()
        .join("; ");
    serde_json::json!({
        "state": aggregate_state(rows),
        "message": message,
    })
}

fn list_controllers_payload(rows: &[ControllerRow]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "name": row.name,
                "transport": row.transport,
                "verdict": row.verdict,
                "fixable": row.fixable,
            })
        })
        .collect();
    serde_json::json!(items)
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

    fn row(name: &str, fixable: bool, fix_state: Option<FixState>) -> ControllerRow {
        ControllerRow {
            name: name.to_string(),
            transport: "Bluetooth",
            verdict: "verdict".to_string(),
            fixable,
            fix_state,
        }
    }

    #[test]
    fn aggregate_state_picks_worst_first() {
        let cases: [(&str, Vec<ControllerRow>, &str); 5] = [
            ("empty is none", vec![], "none"),
            (
                "healthy pads are ok",
                vec![
                    row("foo", false, None),
                    row("bar", false, Some(FixState::Applied)),
                ],
                "ok",
            ),
            (
                "fixable beats ok",
                vec![
                    row("foo", false, None),
                    row("bar", true, Some(FixState::Pending)),
                ],
                "pending",
            ),
            (
                "live only is fixable",
                vec![row("foo", true, Some(FixState::LiveOnly))],
                "pending",
            ),
            (
                "driver missing beats fixable",
                vec![
                    row("foo", true, Some(FixState::Pending)),
                    row("bar", false, Some(FixState::DriverMissing)),
                ],
                "driver_missing",
            ),
        ];
        for (label, rows, expected) in cases {
            assert_eq!(aggregate_state(&rows), expected, "case: {label}");
        }
    }

    #[test]
    fn controllers_status_payload_mentions_only_known_defect_pads() {
        let rows = vec![
            row("foo pad", false, None),
            row("bar pad", true, Some(FixState::Pending)),
        ];
        let payload = controllers_status_payload(&rows);
        assert_eq!(payload["state"], "pending");
        let message = payload["message"].as_str().expect("message");
        assert!(message.contains("bar pad"), "known pad named: {message}");
        assert!(
            !message.contains("foo pad"),
            "healthy pad omitted: {message}"
        );
    }

    #[test]
    fn list_controllers_payload_is_row_array() {
        let payload = list_controllers_payload(&[row("foo pad", true, Some(FixState::Pending))]);
        assert_eq!(
            payload,
            serde_json::json!([{
                "name": "foo pad",
                "transport": "Bluetooth",
                "verdict": "verdict",
                "fixable": true,
            }])
        );
    }

    #[test]
    fn build_row_maps_fix_state_to_verdict_and_fixable() {
        let device = DetectedDevice {
            bus: 0x0005,
            vendor: 0x045e,
            product: 0x028e,
            name: "GuliKit Controller XW".into(),
            uniq: Some("06:71:10:20:26:b4".into()),
            is_gamepad: true,
        };
        let unknown = DetectedDevice {
            bus: 0x0003,
            vendor: 0x054c,
            product: 0x0ce6,
            name: "foo pad".into(),
            uniq: None,
            is_gamepad: true,
        };
        let root = tempfile::tempdir().expect("tempdir");
        let paths = SystemPaths {
            modprobe_dir: root.path().join("modprobe.d"),
            sys_module_dir: root.path().join("module"),
        };
        let rows = build_rows(&paths, &[device, unknown.clone()]);
        assert_eq!(rows.len(), 2);
        assert!(
            rows[0].verdict.starts_with("Rumble never stops"),
            "verdict: {}",
            rows[0].verdict
        );
        assert!(rows[0].fix_state.is_some());
        assert_eq!(rows[1].verdict, "No known issues");
        assert!(!rows[1].fixable);
        assert_eq!(rows[1].transport, "USB");

        let non_gamepad = DetectedDevice {
            is_gamepad: false,
            ..unknown
        };
        assert_eq!(
            build_rows(&paths, &[non_gamepad]).len(),
            0,
            "non-gamepads excluded"
        );
    }
}
