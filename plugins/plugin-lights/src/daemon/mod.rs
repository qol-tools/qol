mod state;
pub mod ws;

use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

use crate::backend::zigbee::ZigbeeBackend;
use crate::config::model::{DeviceEntry, EndpointEntry};
use crate::config::store;
use crate::service::light_service::LightService;

pub use state::{DaemonOutcome, DaemonState};

enum DaemonRuntime {
    Ready(Box<DaemonState>),
    Unavailable(String),
}

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

pub fn run_from_env() -> Result<()> {
    let runtime = runtime_state();
    core_daemon::run_stateful_listener(&DAEMON_CONFIG, runtime, handle_action)
        .context("plugin-lights daemon listener failed")
}

pub fn execute_action_once(action: &str) -> Result<()> {
    let mut state = DaemonState::new()?;
    let outcome = state.handle_action(action);
    map_outcome(action, outcome)
}

// Runs on a background thread. When a device joins (ZDO_END_DEVICE_ANNCE_IND),
// persists it to config and auto-closes permit_join so the user doesn't have to
// manually stop pairing. The service Arc is shared with DaemonState and WS thread.
fn device_monitor_loop(
    events: crossbeam_channel::Receiver<crate::znp::ZigbeeEvent>,
    service: Arc<Mutex<Option<LightService<ZigbeeBackend>>>>,
) {
    loop {
        match events.recv() {
            Ok(crate::znp::ZigbeeEvent::DeviceJoined(device)) => {
                let ieee = format_ieee(&device.ieee_address);
                let entry = DeviceEntry {
                    ieee_address: ieee.clone(),
                    name: format!("Device {:04X}", device.network_address),
                    endpoints: device
                        .endpoints
                        .iter()
                        .map(|endpoint| EndpointEntry {
                            id: endpoint.id,
                            clusters: endpoint.input_clusters.clone(),
                        })
                        .collect(),
                    online: true,
                };
                if let Ok(mut config) = store::load() {
                    config.devices.insert(ieee.clone(), entry);
                    let _ = store::save(&config);
                    eprintln!("device joined: {} (0x{:04X})", ieee, device.network_address);
                }
                if let Ok(guard) = service.lock() {
                    if let Some(svc) = guard.as_ref() {
                        let _ = svc.backend().permit_join(0);
                        eprintln!("pairing auto-stopped after device joined");
                    }
                }
            }
            Ok(crate::znp::ZigbeeEvent::DeviceLeft(_)) => {}
            Err(_) => break,
        }
    }
}

fn runtime_state() -> DaemonRuntime {
    let state = match DaemonState::new() {
        Ok(state) => state,
        Err(error) => {
            eprintln!("backend unavailable: {error:#}");
            return DaemonRuntime::Unavailable(error.to_string());
        }
    };

    eprintln!("coordinator ready");
    start_background_services(&state);
    DaemonRuntime::Ready(Box::new(state))
}

fn start_background_services(state: &DaemonState) {
    let events = state.events();
    let monitor_service = state.shared_service();
    let _ = thread::Builder::new()
        .name("device-monitor".into())
        .spawn(move || device_monitor_loop(events, monitor_service));

    let command_buffer = ws::CommandBuffer::default();
    ws::start(
        command_buffer,
        state.shared_service(),
        state.main_target().clone(),
    );
}

fn format_ieee(addr: &[u8; 8]) -> String {
    addr.iter()
        .map(|byte| format!("{:02X}", byte))
        .collect::<Vec<_>>()
        .join(":")
}

const CONNECTION_STATUS_QUERY: &str = "connection_status";
const LIST_DEVICES_QUERY: &str = "list_devices";
const PING_ACTION: &str = "ping";
const NO_COORDINATOR_MESSAGE: &str = "No compatible Zigbee coordinator detected on this PC. Plug in a supported Zigbee dongle, then scan again.";

fn handle_action(runtime: &mut DaemonRuntime, action: &str) -> ReadResult<()> {
    let outcome = dispatch_action(runtime, action);
    match outcome {
        DaemonOutcome::Handled => ReadResult::Handled,
        DaemonOutcome::HandledWithData(data) => ReadResult::HandledWithData(data),
        DaemonOutcome::Fallback => ReadResult::Fallback,
        DaemonOutcome::Error(message) => {
            eprintln!("action '{}' failed: {}", action, message);
            ReadResult::Error(message)
        }
    }
}

fn dispatch_action(runtime: &mut DaemonRuntime, action: &str) -> DaemonOutcome {
    if action == PING_ACTION {
        return DaemonOutcome::Handled;
    }

    if action == LIST_DEVICES_QUERY {
        let devices: Vec<serde_json::Value> = match runtime {
            DaemonRuntime::Ready(s) => {
                let guard = s.shared_service();
                let Ok(lock) = guard.lock() else {
                    return DaemonOutcome::Error("service lock poisoned".into());
                };
                match lock.as_ref() {
                    Some(svc) => svc
                        .backend()
                        .devices()
                        .iter()
                        .map(|d| {
                            serde_json::json!({
                                "address": format!("0x{:04X}", d.network_address),
                                "name": format_ieee(&d.ieee_address),
                                "ieee": format_ieee(&d.ieee_address),
                                "online": true,
                            })
                        })
                        .collect(),
                    None => vec![],
                }
            }
            DaemonRuntime::Unavailable(_) => vec![],
        };
        return DaemonOutcome::HandledWithData(serde_json::json!(devices));
    }

    if action == CONNECTION_STATUS_QUERY {
        let data = match runtime {
            DaemonRuntime::Ready(_) => serde_json::json!({
                "state": "ok",
                "can_pair": true,
            }),
            DaemonRuntime::Unavailable(message) => serde_json::json!({
                "state": "offline",
                "can_pair": false,
                "message": user_facing_unavailable_message(message),
            }),
        };
        return DaemonOutcome::HandledWithData(data);
    }

    if let DaemonRuntime::Ready(state) = runtime {
        return state.handle_action(action);
    }

    if action != "reload" {
        if let DaemonRuntime::Unavailable(message) = runtime {
            return DaemonOutcome::Error(user_facing_unavailable_message(message));
        }
        unreachable!()
    }

    let state = match DaemonState::new() {
        Ok(state) => state,
        Err(error) => {
            let detailed_message = error.to_string();
            let user_message = user_facing_unavailable_message(&detailed_message);
            *runtime = DaemonRuntime::Unavailable(detailed_message);
            return DaemonOutcome::Error(user_message);
        }
    };

    eprintln!("coordinator ready");
    start_background_services(&state);
    *runtime = DaemonRuntime::Ready(Box::new(state));
    DaemonOutcome::Handled
}

fn user_facing_unavailable_message(message: &str) -> String {
    if message.contains("no supported Zigbee coordinator")
        || message.contains("no Zigbee coordinator responded")
    {
        return NO_COORDINATOR_MESSAGE.to_string();
    }
    message.to_string()
}

fn map_outcome(action: &str, outcome: DaemonOutcome) -> Result<()> {
    match outcome {
        DaemonOutcome::Handled | DaemonOutcome::HandledWithData(_) => Ok(()),
        DaemonOutcome::Fallback => {
            anyhow::bail!("plugin-lights fell back for action '{}'", action)
        }
        DaemonOutcome::Error(message) => anyhow::bail!(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_runtime_still_handles_ping() {
        let mut runtime = DaemonRuntime::Unavailable("coordinator missing".to_string());
        let outcome = dispatch_action(&mut runtime, PING_ACTION);

        assert!(matches!(outcome, DaemonOutcome::Handled));
    }

    #[test]
    fn unavailable_runtime_rejects_pair_with_user_facing_message() {
        let mut runtime = DaemonRuntime::Unavailable(
            "no supported Zigbee coordinator detected automatically; available serial devices: /dev/tty.fake"
                .to_string(),
        );
        let outcome = dispatch_action(&mut runtime, crate::runtime::actions::PAIR);

        match outcome {
            DaemonOutcome::Error(message) => assert_eq!(message, NO_COORDINATOR_MESSAGE),
            _ => panic!("expected backend error"),
        }
    }

    #[test]
    fn unavailable_runtime_status_reports_pairing_unavailable() {
        let mut runtime = DaemonRuntime::Unavailable(
            "no Zigbee coordinator responded on auto-detected serial ports: /dev/tty.fake"
                .to_string(),
        );
        let outcome = dispatch_action(&mut runtime, CONNECTION_STATUS_QUERY);

        match outcome {
            DaemonOutcome::HandledWithData(data) => assert_eq!(
                data,
                serde_json::json!({
                    "state": "offline",
                    "can_pair": false,
                    "message": NO_COORDINATOR_MESSAGE,
                })
            ),
            _ => panic!("expected status data"),
        }
    }

    #[test]
    fn unavailable_runtime_preserves_unknown_errors() {
        let mut runtime = DaemonRuntime::Unavailable("permission denied".to_string());
        let outcome = dispatch_action(&mut runtime, crate::runtime::actions::PAIR);

        match outcome {
            DaemonOutcome::Error(message) => assert_eq!(message, "permission denied"),
            _ => panic!("expected backend error"),
        }
    }
}
