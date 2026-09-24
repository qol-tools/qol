use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_plugin_daemon::notification::send_notification;

use crate::detection::clash::{
    classify, holders_label, LinkEvidence, LinkState, TIMEOUT_THRESHOLD,
};
use crate::fixes::state::{compute, FixState, SystemPaths};
use crate::fixes::{apply, hidraw_guard};
use crate::fixes::{match_device, match_devices, DetectedDevice};
use crate::platform;

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
    pub link: Option<LinkEvidence>,
    pub link_state: LinkState,
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
    reclaim: ReclaimJob,
}

// The fix waits on a pkexec prompt, so it runs off the listener thread and the
// panel keeps polling the snapshot, which reports "fixing" until it finishes.
#[derive(Default)]
struct ReclaimJob {
    running: Arc<AtomicBool>,
}

impl ReclaimJob {
    fn running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    fn start(&self) {
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        let running = Arc::clone(&self.running);
        std::thread::spawn(move || {
            if let Err(error) = reclaim_once() {
                send_notification("Controller fix failed", &format!("{error:#}"));
            }
            running.store(false, Ordering::Release);
        });
    }
}

pub fn snapshot() -> ControllerSnapshot {
    let paths = SystemPaths::real();
    let devices = platform::read_devices();
    let links = platform::link_evidence(&devices);
    ControllerSnapshot {
        id: NEXT_SNAPSHOT_ID.fetch_add(1, Ordering::Relaxed),
        rows: build_rows(&paths, &devices, &links),
    }
}

fn build_rows(
    paths: &SystemPaths,
    devices: &[DetectedDevice],
    links: &[Option<LinkEvidence>],
) -> Vec<ControllerRow> {
    devices
        .iter()
        .enumerate()
        .filter(|(_, device)| device.is_gamepad)
        .map(|(index, device)| build_row(paths, device, links.get(index).cloned().flatten()))
        .collect()
}

fn build_row(
    paths: &SystemPaths,
    device: &DetectedDevice,
    link: Option<LinkEvidence>,
) -> ControllerRow {
    let link_state = link.as_ref().map(classify).unwrap_or(LinkState::Ok);
    let Some(target) = match_device(device) else {
        return capability_row(device, link, link_state);
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
        verdict: apply_link_verdict(verdict, link.as_ref(), link_state),
        fixable,
        virtual_device: device.is_virtual(),
        has_force_feedback: device.has_force_feedback,
        fix_state: Some(state),
        link,
        link_state,
    }
}

fn apply_link_verdict(verdict: String, link: Option<&LinkEvidence>, state: LinkState) -> String {
    let Some(link) = link else {
        return verdict;
    };
    match state {
        LinkState::Contended => format!(
            "Not responding: {} is grabbing it from the driver. Fix stops that and reconnects it.",
            holders_label(&link.holders)
        ),
        LinkState::Stalled if link.driver_timeouts >= TIMEOUT_THRESHOLD => format!(
            "Not responding: the driver timed out {} times in the last minute. Fix reconnects it.",
            link.driver_timeouts
        ),
        LinkState::Stalled => {
            "Not responding: both sticks read stuck at their limits. Fix reconnects it.".to_string()
        }
        LinkState::Shared => format!("{verdict}; shared with {}", holders_label(&link.holders)),
        LinkState::Ok => verdict,
    }
}

fn capability_row(
    device: &DetectedDevice,
    link: Option<LinkEvidence>,
    link_state: LinkState,
) -> ControllerRow {
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
        verdict: apply_link_verdict(verdict.to_string(), link.as_ref(), link_state),
        fixable: false,
        virtual_device: device.is_virtual(),
        has_force_feedback: device.has_force_feedback,
        fix_state: None,
        link,
        link_state,
    }
}

pub fn run_from_env() -> Result<()> {
    core_daemon::run_stateful_listener(&DAEMON_CONFIG, DaemonRuntime::default(), handle_action)
        .context(format!("{} daemon listener failed", crate::PLUGIN_ID))
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
            | "reclaim_controller"
    )
}

fn handle_action(runtime: &mut DaemonRuntime, action: &str) -> ReadResult<()> {
    match action {
        "ping" => ReadResult::Handled,
        "kill" => ReadResult::Handled,
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
        "reclaim_controller" => {
            runtime.reclaim.start();
            ReadResult::Handled
        }
        "status" | "controllers_snapshot" | "controllers_status" | "list_controllers" => {
            let snapshot = runtime.snapshots.current();
            let mut payload = snapshot_payload(&snapshot);
            if runtime.reclaim.running() {
                payload["state"] = "fixing".into();
            }
            ReadResult::HandledWithData(payload)
        }
        "controller_input" => {
            ReadResult::HandledWithData(native_input_payload(runtime.input.snapshot()))
        }
        _ => ReadResult::Error(format!("unknown action: {action}")),
    }
}

fn reclaim_once() -> Result<()> {
    let message = reclaim()?;
    send_notification("Controllers", &message);
    Ok(())
}

fn reclaim() -> Result<String> {
    let devices = platform::read_devices();
    let links = platform::link_evidence(&devices);
    let stuck = devices
        .iter()
        .zip(links)
        .filter(|(device, link)| {
            device.transport() == "Bluetooth"
                && link
                    .as_ref()
                    .is_some_and(|evidence| state_needs_reclaim(classify(evidence)))
        })
        .map(|(device, _)| device)
        .collect::<Vec<_>>();
    if stuck.is_empty() {
        bail!("every controller is responding");
    }
    hidraw_guard::ensure()?;
    let mut messages = Vec::new();
    for device in stuck {
        let adapter = platform::disconnect_bluetooth(device)?;
        messages.push(format!(
            "Disconnected {} on {}. Press Home on the controller to reconnect.",
            device.name, adapter
        ));
    }
    Ok(messages.join(" "))
}

fn state_needs_reclaim(state: LinkState) -> bool {
    matches!(state, LinkState::Contended | LinkState::Stalled)
}

fn is_not_responding(row: &ControllerRow) -> bool {
    state_needs_reclaim(row.link_state)
}

fn is_bluetooth(row: &ControllerRow) -> bool {
    row.transport == "Bluetooth"
}

fn reclaimable(row: &ControllerRow) -> bool {
    is_not_responding(row) && is_bluetooth(row)
}

fn aggregate_state(rows: &[ControllerRow]) -> &'static str {
    if rows.is_empty() {
        return "none";
    }
    if rows.iter().any(is_not_responding) {
        return "not_responding";
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
        .filter(|row| row.fixable || is_not_responding(row))
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
                "link": row.link_state.name(),
                "holders": row
                    .link
                    .as_ref()
                    .map(|link| holders_label(&link.holders))
                    .unwrap_or_default(),
                "reclaimable": reclaimable(row),
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
            let state_buttons = item
                .state
                .buttons
                .into_iter()
                .map(|button| {
                    serde_json::json!({
                        "index": button.index,
                        "name": button.name,
                        "pressed": button.pressed,
                        "value": button.value,
                    })
                })
                .collect::<Vec<_>>();
            let state_axes = item
                .state
                .axes
                .into_iter()
                .map(|axis| {
                    serde_json::json!({
                        "index": axis.index,
                        "name": axis.name,
                        "value": axis.value,
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "name": item.name,
                "vendor": item.vendor,
                "product": item.product,
                "connection": native_connection_payload(item.connection),
                "buttons": buttons,
                "state": {
                    "mapping": item.state.mapping,
                    "buttons": state_buttons,
                    "axes": state_axes,
                },
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "available": snapshot.available,
        "source": snapshot.source,
        "items": items,
    })
}

fn native_connection_payload(connection: platform::NativeConnection) -> serde_json::Value {
    let signal = connection.signal.map(|signal| match signal {
        platform::NativeSignal::AdvertisedDbm(value) => serde_json::json!({
            "kind": "absolute_dbm",
            "source": "bluez_device",
            "value": value,
        }),
        platform::NativeSignal::BredrLinkMarginDb(value) => serde_json::json!({
            "kind": "bredr_link_margin_db",
            "source": "hci_link",
            "value": value,
        }),
    });
    let adapter = connection.adapter.map(|adapter| {
        serde_json::json!({
            "name": adapter.name,
            "address": adapter.address,
            "vendor": adapter.vendor,
            "model": adapter.model,
            "hardware_id": adapter.hardware_id,
            "path": adapter.path,
        })
    });
    serde_json::json!({
        "transport": connection.transport,
        "signal": signal,
        "adapter": adapter,
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
    if action == "reclaim_controller" {
        return reclaim_once();
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
    use crate::detection::clash;

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
            link: None,
            link_state: LinkState::Ok,
        }
    }

    fn linked_row(
        name: &str,
        transport: &'static str,
        link: LinkEvidence,
        link_state: LinkState,
    ) -> ControllerRow {
        ControllerRow {
            transport,
            link: Some(link),
            link_state,
            ..row(name, false, None)
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
            ("reclaim_controller", true),
            ("bogus", false),
        ];
        for (action, known) in cases {
            assert_eq!(is_supported_action(action), known, "action: {action}");
        }
    }

    #[test]
    fn snapshot_reports_fixing_while_a_reclaim_runs() {
        let cases = [("idle", false, "none"), ("reclaim running", true, "fixing")];
        for (label, running, expected) in cases {
            let mut runtime = DaemonRuntime::default();
            runtime.snapshots.cached = Some((
                Instant::now(),
                ControllerSnapshot {
                    id: 1,
                    rows: Vec::new(),
                },
            ));
            runtime.reclaim.running.store(running, Ordering::Release);
            let ReadResult::HandledWithData(payload) =
                handle_action(&mut runtime, "controllers_snapshot")
            else {
                panic!("case: {label}: snapshot query must answer with data");
            };
            assert_eq!(payload["state"], expected, "case: {label}");
        }
    }

    #[test]
    fn daemon_answers_the_readiness_ping() {
        let mut runtime = DaemonRuntime::default();
        assert!(
            matches!(handle_action(&mut runtime, "ping"), ReadResult::Handled),
            "the tray gates every action on a ping response"
        );
    }

    #[test]
    fn daemon_answers_the_replace_kill_with_handled() {
        let mut runtime = DaemonRuntime::default();
        assert!(
            matches!(handle_action(&mut runtime, "kill"), ReadResult::Handled),
            "a support_replace_existing daemon must answer kill with Handled or the \
             replace handshake fails and the successor exits with \
             'existing daemon instance is alive'"
        );
    }

    #[test]
    fn aggregate_state_reports_only_active_optional_fixes() {
        let cases: [(&str, Vec<ControllerRow>, &str); 4] = [
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
            (
                "a contended controller needs attention first",
                vec![linked_row(
                    "pad",
                    "Bluetooth",
                    LinkEvidence {
                        driver_timeouts: clash::TIMEOUT_THRESHOLD,
                        holders: vec![clash::Holder {
                            pid: 4242,
                            name: "steam".into(),
                        }],
                        ..Default::default()
                    },
                    LinkState::Contended,
                )],
                "not_responding",
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
                    "link": "ok",
                    "holders": "",
                    "reclaimable": false,
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
                    signal: Some(platform::NativeSignal::BredrLinkMarginDb(-11)),
                    adapter: Some(platform::NativeAdapter {
                        name: "hci7".into(),
                        address: Some("00:11:22:33:44:55".into()),
                        vendor: Some("Foo Corp.".into()),
                        model: Some("Bar Radio".into()),
                        hardware_id: Some("1234:abcd".into()),
                        path: Some("pci-0000:00:01.0-usb-0:2:1.0".into()),
                    }),
                },
                buttons: vec![platform::NativeButtonInput {
                    index: 10,
                    pressed: true,
                }],
                state: platform::NativeGamepadState {
                    mapping: "standard",
                    buttons: vec![platform::NativeGamepadButton {
                        index: 0,
                        name: "South",
                        pressed: true,
                        value: 1.0,
                    }],
                    axes: vec![platform::NativeGamepadAxis {
                        index: 0,
                        name: "Left X",
                        value: -0.5,
                    }],
                },
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
                        "signal": {
                            "kind": "bredr_link_margin_db",
                            "source": "hci_link",
                            "value": -11,
                        },
                        "adapter": {
                            "name": "hci7",
                            "address": "00:11:22:33:44:55",
                            "vendor": "Foo Corp.",
                            "model": "Bar Radio",
                            "hardware_id": "1234:abcd",
                            "path": "pci-0000:00:01.0-usb-0:2:1.0",
                        },
                    },
                    "buttons": [{"index": 10, "pressed": true}],
                    "state": {
                        "mapping": "standard",
                        "buttons": [{
                            "index": 0,
                            "name": "South",
                            "pressed": true,
                            "value": 1.0,
                        }],
                        "axes": [{
                            "index": 0,
                            "name": "Left X",
                            "value": -0.5,
                        }],
                    },
                }],
            })
        );
    }

    #[test]
    fn build_rows_applies_fixes_only_to_the_bound_target_driver() {
        let root = tempfile::tempdir().expect("tempdir");
        let paths = SystemPaths {
            modprobe_dir: Some(root.path().join("modprobe.d")),
            sys_module_dir: Some(root.path().join("module")),
        };
        let native = device(Some("hid-generic"), false, false);
        let xpadneo = device(Some("xpadneo"), false, true);
        let virtual_pad = device(None, true, true);
        let rows = build_rows(&paths, &[native, xpadneo, virtual_pad], &[]);

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
        assert!(build_rows(&paths, &[non_gamepad], &[]).is_empty());
    }

    #[test]
    fn link_state_overrides_verdict_and_gates_row_actions() {
        let root = tempfile::tempdir().expect("tempdir");
        let paths = SystemPaths {
            modprobe_dir: Some(root.path().join("modprobe.d")),
            sys_module_dir: Some(root.path().join("module")),
        };
        let steam = clash::Holder {
            pid: 4242,
            name: "steam".into(),
        };
        let cases = [
            (
                "ok",
                0x0005,
                LinkEvidence::default(),
                LinkState::Ok,
                "Input available; no kernel rumble interface",
                "",
                false,
            ),
            (
                "shared",
                0x0005,
                LinkEvidence {
                    holders: vec![steam.clone()],
                    ..Default::default()
                },
                LinkState::Shared,
                "Input available; no kernel rumble interface; shared with steam (pid 4242)",
                "steam (pid 4242)",
                false,
            ),
            (
                "contended",
                0x0005,
                LinkEvidence {
                    driver_timeouts: clash::TIMEOUT_THRESHOLD,
                    sticks_pinned: Some(true),
                    holders: vec![steam.clone()],
                },
                LinkState::Contended,
                "Not responding: steam (pid 4242) is grabbing it from the driver. Fix stops that and reconnects it.",
                "steam (pid 4242)",
                true,
            ),
            (
                "stalled",
                0x0005,
                LinkEvidence {
                    driver_timeouts: clash::TIMEOUT_THRESHOLD,
                    sticks_pinned: Some(false),
                    holders: vec![],
                },
                LinkState::Stalled,
                "Not responding: the driver timed out 5 times in the last minute. Fix reconnects it.",
                "",
                true,
            ),
            (
                "stalled with pinned sticks only",
                0x0005,
                LinkEvidence {
                    sticks_pinned: Some(true),
                    ..Default::default()
                },
                LinkState::Stalled,
                "Not responding: both sticks read stuck at their limits. Fix reconnects it.",
                "",
                true,
            ),
            (
                "steam with pinned sticks and no timeouts",
                0x0005,
                LinkEvidence {
                    sticks_pinned: Some(true),
                    holders: vec![steam.clone()],
                    ..Default::default()
                },
                LinkState::Contended,
                "Not responding: steam (pid 4242) is grabbing it from the driver. Fix stops that and reconnects it.",
                "steam (pid 4242)",
                true,
            ),
            (
                "usb contention is not reclaimable",
                0x0003,
                LinkEvidence {
                    driver_timeouts: clash::TIMEOUT_THRESHOLD,
                    holders: vec![steam.clone()],
                    ..Default::default()
                },
                LinkState::Contended,
                "Not responding: steam (pid 4242) is grabbing it from the driver. Fix stops that and reconnects it.",
                "steam (pid 4242)",
                false,
            ),
        ];
        for (label, bus, link, state, verdict, holders, reclaimable) in cases {
            let pad = DetectedDevice {
                bus,
                ..device(Some("hid-generic"), false, false)
            };
            let rows = build_rows(&paths, &[pad], &[Some(link)]);
            assert_eq!(rows[0].link_state, state, "case: {label}");
            assert_eq!(rows[0].verdict, verdict, "case: {label}");
            let not_responding = matches!(state, LinkState::Contended | LinkState::Stalled);
            let expected_message = if not_responding {
                format!("GuliKit Controller XW: {verdict}")
            } else {
                String::new()
            };
            let payload = snapshot_payload(&ControllerSnapshot { id: 1, rows });
            let expected_state = if not_responding {
                "not_responding"
            } else {
                "ok"
            };
            assert_eq!(payload["state"], expected_state, "case: {label}");
            assert_eq!(payload["message"], expected_message, "case: {label}");
            let item = &payload["items"][0];
            assert_eq!(item["link"], state.name(), "case: {label}");
            assert_eq!(item["holders"], holders, "case: {label}");
            assert_eq!(item["reclaimable"], reclaimable, "case: {label}");
        }
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
