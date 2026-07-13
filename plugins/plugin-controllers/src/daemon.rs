use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_plugin_daemon::notification::send_notification;

use crate::fixes::{match_device, match_devices, DetectedDevice};
use crate::state::{compute, FixState, SystemPaths};
use crate::{apply, platform};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

const SNAPSHOT_COHERENCE_WINDOW: Duration = Duration::from_secs(1);
static NEXT_SNAPSHOT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct ControllerRow {
    pub name: String,
    pub transport: &'static str,
    pub driver: String,
    pub version: String,
    pub verdict: String,
    pub fixable: bool,
    pub virtual_device: bool,
    pub has_force_feedback: bool,
    pub fix_state: Option<FixState>,
}

#[derive(Clone)]
pub struct ControllerSnapshot {
    pub id: u64,
    pub rows: Vec<ControllerRow>,
}

#[derive(Default)]
struct SnapshotCache {
    cached: Option<(Instant, ControllerSnapshot)>,
}

impl SnapshotCache {
    fn current(&mut self) -> ControllerSnapshot {
        self.current_at(Instant::now(), snapshot)
    }

    fn current_at<F>(&mut self, now: Instant, capture: F) -> ControllerSnapshot
    where
        F: FnOnce() -> ControllerSnapshot,
    {
        if let Some((captured_at, cached)) = &self.cached {
            if now.saturating_duration_since(*captured_at) <= SNAPSHOT_COHERENCE_WINDOW {
                return cached.clone();
            }
        }
        let fresh = capture();
        self.cached = Some((now, fresh.clone()));
        fresh
    }

    fn invalidate(&mut self) {
        self.cached = None;
    }
}

#[derive(Default)]
struct DaemonRuntime {
    snapshots: SnapshotCache,
    input: platform::InputMonitor,
}

pub fn snapshot() -> ControllerSnapshot {
    let paths = SystemPaths::real();
    let devices = platform::read_devices();
    ControllerSnapshot {
        id: NEXT_SNAPSHOT_ID.fetch_add(1, Ordering::Relaxed),
        rows: build_rows(&paths, &devices),
    }
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
        return capability_row(device);
    };
    let state = compute(paths, &target);
    let (verdict, fixable) = match state {
        FixState::Pending => (target.entry.summary.to_string(), true),
        FixState::LiveOnly => (
            "xpadneo rumble workaround active until reboot".to_string(),
            true,
        ),
        FixState::Applied => ("xpadneo rumble workaround applied".to_string(), false),
    };
    ControllerRow {
        name: device.name.clone(),
        transport: device.transport(),
        driver: device.driver_label().to_string(),
        version: device.version_label(),
        verdict,
        fixable,
        virtual_device: device.is_virtual(),
        has_force_feedback: device.has_force_feedback,
        fix_state: Some(state),
    }
}

fn capability_row(device: &DetectedDevice) -> ControllerRow {
    let verdict = if device.has_force_feedback {
        "Input and rumble available"
    } else {
        "Input available; no kernel rumble interface"
    };
    ControllerRow {
        name: device.name.clone(),
        transport: device.transport(),
        driver: device.driver_label().to_string(),
        version: device.version_label(),
        verdict: verdict.to_string(),
        fixable: false,
        virtual_device: device.is_virtual(),
        has_force_feedback: device.has_force_feedback,
        fix_state: None,
    }
}

pub fn run_from_env() -> Result<()> {
    core_daemon::run_stateful_listener(&DAEMON_CONFIG, DaemonRuntime::default(), handle_action)
        .context("plugin-controllers daemon listener failed")
}

fn is_supported_action(action: &str) -> bool {
    matches!(
        action,
        "apply_fixes"
            | "settings"
            | "status"
            | "controllers_snapshot"
            | "controllers_status"
            | "list_controllers"
            | "controller_input"
    )
}

fn handle_action(runtime: &mut DaemonRuntime, action: &str) -> ReadResult<()> {
    match action {
        "apply_fixes" => match apply_pending() {
            Ok(message) => {
                runtime.snapshots.invalidate();
                send_notification("Controller fixes", &message);
                ReadResult::Handled
            }
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        "settings" => match platform::open_settings() {
            Ok(()) => ReadResult::Handled,
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        "status" | "controllers_snapshot" | "controllers_status" | "list_controllers" => {
            let snapshot = runtime.snapshots.current();
            ReadResult::HandledWithData(snapshot_payload(&snapshot))
        }
        "controller_input" => {
            ReadResult::HandledWithData(native_input_payload(runtime.input.snapshot()))
        }
        _ => ReadResult::Error(format!("unknown action: {action}")),
    }
}

fn aggregate_state(rows: &[ControllerRow]) -> &'static str {
    if rows.is_empty() {
        return "none";
    }
    if rows.iter().any(|row| row.fixable) {
        return "optional_fix";
    }
    "ok"
}

pub fn snapshot_payload(snapshot: &ControllerSnapshot) -> serde_json::Value {
    let message = snapshot
        .rows
        .iter()
        .filter(|row| row.fixable)
        .map(|row| format!("{}: {}", row.name, row.verdict))
        .collect::<Vec<_>>()
        .join("; ");
    let items = snapshot
        .rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "name": row.name,
                "transport": row.transport,
                "driver": row.driver,
                "version": row.version,
                "verdict": row.verdict,
                "fixable": row.fixable,
                "virtual": row.virtual_device,
                "force_feedback": row.has_force_feedback,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "snapshot_id": snapshot.id,
        "state": aggregate_state(&snapshot.rows),
        "message": message,
        "items": items,
    })
}

fn native_input_payload(snapshot: platform::NativeInputSnapshot) -> serde_json::Value {
    let items = snapshot
        .items
        .into_iter()
        .map(|item| {
            let buttons = item
                .buttons
                .into_iter()
                .map(|button| {
                    serde_json::json!({
                        "index": button.index,
                        "pressed": button.pressed,
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "name": item.name,
                "vendor": item.vendor,
                "product": item.product,
                "connection": {
                    "transport": item.connection.transport,
                    "signal_dbm": item.connection.signal_dbm,
                },
                "buttons": buttons,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "available": snapshot.available,
        "source": snapshot.source,
        "items": items,
    })
}

fn apply_pending() -> Result<String> {
    let paths = SystemPaths::real();
    let devices = platform::read_devices();
    let targets = match_devices(&devices);
    if targets.is_empty() {
        bail!("no active driver-specific fixes apply; xpadneo is optional");
    }
    let pending = targets
        .iter()
        .filter(|target| compute(&paths, target) != FixState::Applied)
        .count();
    if pending == 0 {
        return Ok("all driver-specific fixes already applied".to_string());
    }
    apply::apply(&targets)?;
    Ok(format!("applied {pending} driver-specific fix(es)"))
}

pub fn execute_action_once(action: &str) -> Result<()> {
    if !is_supported_action(action) {
        bail!("unknown action: {action}");
    }
    let mut runtime = DaemonRuntime::default();
    match handle_action(&mut runtime, action) {
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

    fn row(name: &str, fixable: bool, fix_state: Option<FixState>) -> ControllerRow {
        ControllerRow {
            name: name.to_string(),
            transport: "Bluetooth",
            driver: "hid-generic".to_string(),
            version: "0903".to_string(),
            verdict: "verdict".to_string(),
            fixable,
            virtual_device: false,
            has_force_feedback: false,
            fix_state,
        }
    }

    fn device(driver: Option<&str>, virtual_device: bool, force_feedback: bool) -> DetectedDevice {
        DetectedDevice {
            bus: 0x0005,
            vendor: 0x045e,
            product: 0x02e0,
            version: 0x0903,
            name: "GuliKit Controller XW".into(),
            uniq: Some("06:71:10:20:26:b4".into()),
            sysfs_path: virtual_device.then(|| "/devices/virtual/input/input40".into()),
            event_handler: Some("event21".into()),
            driver: driver.map(str::to_string),
            is_gamepad: true,
            has_force_feedback: force_feedback,
        }
    }

    #[test]
    fn action_dispatch_recognizes_current_and_legacy_queries() {
        let cases = [
            ("apply_fixes", true),
            ("settings", true),
            ("status", true),
            ("controllers_snapshot", true),
            ("controllers_status", true),
            ("list_controllers", true),
            ("controller_input", true),
            ("bogus", false),
        ];
        for (action, known) in cases {
            assert_eq!(is_supported_action(action), known, "action: {action}");
        }
    }

    #[test]
    fn aggregate_state_reports_only_active_optional_fixes() {
        let cases: [(&str, Vec<ControllerRow>, &str); 3] = [
            ("empty is none", vec![], "none"),
            (
                "detected controllers are healthy",
                vec![row("physical", false, None), row("virtual", false, None)],
                "ok",
            ),
            (
                "active optional fix needs attention",
                vec![row("pad", true, Some(FixState::Pending))],
                "optional_fix",
            ),
        ];
        for (label, rows, expected) in cases {
            assert_eq!(aggregate_state(&rows), expected, "case: {label}");
        }
    }

    #[test]
    fn snapshot_payload_pins_the_full_query_contract() {
        let snapshot = ControllerSnapshot {
            id: 42,
            rows: vec![row("foo pad", false, None)],
        };
        assert_eq!(
            snapshot_payload(&snapshot),
            serde_json::json!({
                "snapshot_id": 42,
                "state": "ok",
                "message": "",
                "items": [{
                    "name": "foo pad",
                    "transport": "Bluetooth",
                    "driver": "hid-generic",
                    "version": "0903",
                    "verdict": "verdict",
                    "fixable": false,
                    "virtual": false,
                    "force_feedback": false,
                }],
            })
        );
    }

    #[test]
    fn native_input_payload_pins_controller_input_contract() {
        let payload = native_input_payload(platform::NativeInputSnapshot {
            available: true,
            source: Some("linux-evdev"),
            items: vec![platform::NativeControllerInput {
                name: "foo pad".into(),
                vendor: 0x1234,
                product: 0xabcd,
                connection: platform::NativeConnection {
                    transport: "bluetooth",
                    signal_dbm: Some(-58),
                },
                buttons: vec![platform::NativeButtonInput {
                    index: 10,
                    pressed: true,
                }],
            }],
        });

        assert_eq!(
            payload,
            serde_json::json!({
                "available": true,
                "source": "linux-evdev",
                "items": [{
                    "name": "foo pad",
                    "vendor": 0x1234,
                    "product": 0xabcd,
                    "connection": {
                        "transport": "bluetooth",
                        "signal_dbm": -58,
                    },
                    "buttons": [{"index": 10, "pressed": true}],
                }],
            })
        );
    }

    #[test]
    fn build_rows_applies_fixes_only_to_the_bound_target_driver() {
        let root = tempfile::tempdir().expect("tempdir");
        let paths = SystemPaths {
            modprobe_dir: root.path().join("modprobe.d"),
            sys_module_dir: root.path().join("module"),
        };
        let native = device(Some("hid-generic"), false, false);
        let xpadneo = device(Some("xpadneo"), false, true);
        let virtual_pad = device(None, true, true);
        let rows = build_rows(&paths, &[native, xpadneo, virtual_pad]);

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].driver, "hid-generic");
        assert_eq!(
            rows[0].verdict,
            "Input available; no kernel rumble interface"
        );
        assert!(!rows[0].fixable);
        assert_eq!(rows[0].fix_state, None);
        assert_eq!(rows[1].driver, "xpadneo");
        assert_eq!(rows[1].fix_state, Some(FixState::Pending));
        assert!(rows[1].fixable);
        assert_eq!(rows[2].transport, "Virtual");
        assert_eq!(rows[2].driver, "userspace");
        assert_eq!(rows[2].verdict, "Input and rumble available");

        let non_gamepad = DetectedDevice {
            is_gamepad: false,
            ..device(Some("hid-generic"), false, false)
        };
        assert!(build_rows(&paths, &[non_gamepad]).is_empty());
    }

    #[test]
    fn cache_keeps_separate_ui_queries_on_one_snapshot() {
        let mut cache = SnapshotCache::default();
        let started = Instant::now();
        let first = cache.current_at(started, || ControllerSnapshot {
            id: 1,
            rows: Vec::new(),
        });
        let coherent = cache.current_at(started + Duration::from_millis(900), || {
            ControllerSnapshot {
                id: 2,
                rows: Vec::new(),
            }
        });
        let refreshed = cache.current_at(started + Duration::from_secs(2), || ControllerSnapshot {
            id: 3,
            rows: Vec::new(),
        });

        assert_eq!(first.id, 1);
        assert_eq!(coherent.id, 1);
        assert_eq!(refreshed.id, 3);
    }
}
