mod pairing;
mod payloads;
mod state;
pub mod ws;

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

use crate::backend::zigbee::ZigbeeBackend;
use crate::config::model::{DeviceEntry, EndpointEntry};
use crate::config::store;
use crate::service::light_service::LightService;

use pairing::SharedPairing;
use payloads::{DongleStatus, LightRow};
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
        .context(format!("{} daemon listener failed", store::PLUGIN_ID))
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
    pairing: SharedPairing,
) {
    while let Ok(event) = events.recv() {
        match event {
            crate::znp::ZigbeeEvent::DeviceJoined(device) => {
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
                    network_address: Some(device.network_address),
                };
                let mut config = match store::load() {
                    Ok(config) => config,
                    Err(error) => {
                        log::warn!("device joined: {ieee}, but its config did not load: {error:#}");
                        continue;
                    }
                };
                let known = config.devices.contains_key(&ieee);
                config.devices.insert(ieee.clone(), entry);
                let _ = store::save(&config);
                log::info!("device joined: {} (0x{:04X})", ieee, device.network_address);
                let mut counted = false;
                pairing::update(&pairing, |session| {
                    counted = session.light_joined(ieee.clone(), known, Instant::now())
                });
                if !counted {
                    continue;
                }
                if let Ok(guard) = service.lock() {
                    if let Some(svc) = guard.as_ref() {
                        let _ = svc.backend().permit_join(0);
                        log::info!("pairing auto-stopped after device joined");
                    }
                }
            }
        }
    }
}

fn runtime_state() -> DaemonRuntime {
    let state = match DaemonState::new() {
        Ok(state) => state,
        Err(error) => {
            log::warn!("backend unavailable: {error:#}");
            return DaemonRuntime::Unavailable(error.to_string());
        }
    };

    log::info!("coordinator ready");
    start_background_services(&state);
    DaemonRuntime::Ready(Box::new(state))
}

fn start_background_services(state: &DaemonState) {
    let events = state.events();
    let monitor_service = state.shared_service();
    let monitor_pairing = state.shared_pairing();
    let _ = thread::Builder::new()
        .name("device-monitor".into())
        .spawn(move || device_monitor_loop(events, monitor_service, monitor_pairing));

    let command_buffer = ws::CommandBuffer::default();
    ws::start(
        command_buffer,
        state.shared_service(),
        state.shared_main_target(),
    );
}

fn format_ieee(addr: &[u8; 8]) -> String {
    addr.iter()
        .map(|byte| format!("{:02X}", byte))
        .collect::<Vec<_>>()
        .join(":")
}

const LIGHTS_QUERY: &str = "lights";
const FIND_LIGHTS_QUERY: &str = "find_lights";
const DONGLE_QUERY: &str = "dongle";
const LIGHT_OPTIONS_QUERY: &str = "light_options";
const PING_ACTION: &str = "ping";
const AUTOMATIC_LIGHT_LABEL: &str = "Automatic (one light only)";
const NO_COORDINATOR_MESSAGE: &str = "No light dongle found. Plug one in, then find it again.";

fn handle_action(runtime: &mut DaemonRuntime, action: &str) -> ReadResult<()> {
    let outcome = dispatch_action(runtime, action);
    match outcome {
        DaemonOutcome::Handled => ReadResult::Handled,
        DaemonOutcome::HandledWithData(data) => ReadResult::HandledWithData(data),
        DaemonOutcome::Fallback => ReadResult::Fallback,
        DaemonOutcome::Error(message) => {
            log::warn!("action '{}' failed: {}", action, message);
            ReadResult::Error(message)
        }
    }
}

fn dispatch_action(runtime: &mut DaemonRuntime, action: &str) -> DaemonOutcome {
    if action == PING_ACTION {
        return DaemonOutcome::Handled;
    }

    if action == "kill" {
        return DaemonOutcome::Handled;
    }

    if action == LIGHTS_QUERY {
        let main_target_id = store::load()
            .map(|config| config.main_target_id)
            .unwrap_or_default();
        return DaemonOutcome::HandledWithData(payloads::lights_payload(
            &light_rows(runtime),
            &main_target_id,
        ));
    }

    if action == FIND_LIGHTS_QUERY {
        return find_lights(runtime);
    }

    if action == DONGLE_QUERY {
        return DaemonOutcome::HandledWithData(payloads::dongle_payload(&dongle_status(runtime)));
    }

    if action == LIGHT_OPTIONS_QUERY {
        return light_options(runtime);
    }

    if let DaemonRuntime::Ready(state) = runtime {
        return state.handle_action(action);
    }

    if action != crate::runtime::actions::RELOAD {
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

    log::info!("coordinator ready");
    start_background_services(&state);
    *runtime = DaemonRuntime::Ready(Box::new(state));
    DaemonOutcome::Handled
}

fn light_rows(runtime: &DaemonRuntime) -> Vec<LightRow> {
    let DaemonRuntime::Ready(state) = runtime else {
        return Vec::new();
    };
    let guard = state.shared_service();
    let Ok(lock) = guard.lock() else {
        return Vec::new();
    };
    let Some(svc) = lock.as_ref() else {
        return Vec::new();
    };
    let names = store::load()
        .map(|config| config.devices)
        .unwrap_or_default();
    svc.backend()
        .devices()
        .iter()
        .map(|device| {
            let ieee = format_ieee(&device.ieee_address);
            let name = names
                .get(&ieee)
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| ieee.clone());
            LightRow {
                ieee,
                name,
                network_address: device.network_address,
            }
        })
        .collect()
}

fn find_lights(runtime: &DaemonRuntime) -> DaemonOutcome {
    let session = match runtime {
        DaemonRuntime::Ready(state) => pairing::snapshot(&state.shared_pairing()),
        DaemonRuntime::Unavailable(_) => pairing::PairingSession::default(),
    };
    let ready = matches!(runtime, DaemonRuntime::Ready(_));
    let lights = light_rows(runtime);
    let joined = session
        .joined()
        .iter()
        .filter_map(|ieee| lights.iter().find(|light| &light.ieee == ieee).cloned())
        .collect::<Vec<_>>();
    DaemonOutcome::HandledWithData(payloads::find_lights_payload(
        session.phase(ready, Instant::now()),
        &joined,
    ))
}

fn dongle_status(runtime: &DaemonRuntime) -> DongleStatus {
    match runtime {
        DaemonRuntime::Ready(state) => {
            let guard = state.shared_service();
            let Ok(lock) = guard.lock() else {
                return DongleStatus::Offline("service lock poisoned".into());
            };
            match lock.as_ref() {
                Some(svc) => DongleStatus::Ready {
                    port: svc.backend().serial_port().to_string(),
                    channel: svc.backend().channel(),
                },
                None => DongleStatus::Offline("service not available".into()),
            }
        }
        DaemonRuntime::Unavailable(message) => {
            let user_message = user_facing_unavailable_message(message);
            if user_message == NO_COORDINATOR_MESSAGE {
                DongleStatus::Missing(user_message)
            } else {
                DongleStatus::Offline(user_message)
            }
        }
    }
}

fn light_options(runtime: &DaemonRuntime) -> DaemonOutcome {
    let mut options = vec![serde_json::json!({
        "value": "",
        "label": AUTOMATIC_LIGHT_LABEL,
    })];
    options.extend(
        light_rows(runtime)
            .into_iter()
            .map(|light| serde_json::json!({ "value": light.ieee, "label": light.name })),
    );
    DaemonOutcome::HandledWithData(serde_json::json!(options))
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
            anyhow::bail!("{} fell back for action '{}'", store::PLUGIN_ID, action)
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
    fn daemon_answers_the_replace_kill_with_handled() {
        let mut runtime = DaemonRuntime::Unavailable("coordinator missing".to_string());
        let outcome = dispatch_action(&mut runtime, "kill");

        assert!(
            matches!(outcome, DaemonOutcome::Handled),
            "a support_replace_existing daemon must answer kill with Handled or the \
             replace handshake fails and the successor exits with \
             'existing daemon instance is alive'"
        );
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
    fn unavailable_runtime_reports_the_dongle_and_offers_to_find_it() {
        let cases = [
            (
                "no Zigbee coordinator responded on auto-detected serial ports: /dev/tty.fake",
                "No dongle found",
                NO_COORDINATOR_MESSAGE,
            ),
            ("permission denied", "Dongle offline", "permission denied"),
        ];
        for (failure, label, detail) in cases {
            let mut runtime = DaemonRuntime::Unavailable(failure.to_string());
            let DaemonOutcome::HandledWithData(data) = dispatch_action(&mut runtime, DONGLE_QUERY)
            else {
                panic!("expected dongle data for {failure}");
            };
            assert_eq!(data["lead"]["label"], label);
            assert_eq!(data["lead"]["detail"], detail);
            assert_eq!(data["lead"]["can_find"], true);
        }
    }

    #[test]
    fn unavailable_runtime_answers_every_settings_list_query() {
        let cases = [
            (LIGHTS_QUERY, serde_json::json!([])),
            (FIND_LIGHTS_QUERY, serde_json::json!([])),
        ];
        for (query, items) in cases {
            let mut runtime = DaemonRuntime::Unavailable("permission denied".to_string());
            let DaemonOutcome::HandledWithData(data) = dispatch_action(&mut runtime, query) else {
                panic!("expected data for {query}");
            };
            assert_eq!(data["items"], items, "{query}");
        }
        let mut runtime = DaemonRuntime::Unavailable("permission denied".to_string());
        let DaemonOutcome::HandledWithData(find) = dispatch_action(&mut runtime, FIND_LIGHTS_QUERY)
        else {
            panic!("expected find data");
        };
        assert_eq!(find["lead"]["can_find"], true);
        assert_eq!(find["lead"]["can_start"], false);
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
