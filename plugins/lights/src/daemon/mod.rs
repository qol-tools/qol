mod input;
mod looks;
mod pairing;
mod payloads;
mod persist;
mod presets;
mod state;
pub mod ws;

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::DaemonRequest;
use serde_json::Value;

use crate::backend::zigbee::ZigbeeBackend;
use crate::config::model::{DeviceEntry, EndpointEntry, PresetConfig};
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
    core_daemon::run_stateful_request_listener(&DAEMON_CONFIG, runtime, handle_request)
        .context(format!("{} daemon listener failed", store::PLUGIN_ID))
}

pub fn execute_action_once(action: &str) -> Result<()> {
    let mut state = DaemonState::new()?;
    let outcome = state.handle_action(action);
    persist::flush();
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
                    look: None,
                };
                let known = match store::load() {
                    Ok(config) => config.devices.contains_key(&ieee),
                    Err(error) => {
                        log::warn!("device joined: {ieee}, but its config did not load: {error:#}");
                        continue;
                    }
                };
                persist::submit(persist::Change::Join(entry));
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
        state.shared_looks(),
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
const PRESETS_QUERY: &str = "presets";
const DONGLE_QUERY: &str = "dongle";
const LIGHT_OPTIONS_QUERY: &str = "light_options";
const PING_ACTION: &str = "ping";
const AUTOMATIC_LIGHT_LABEL: &str = "Automatic (one light only)";
const NO_COORDINATOR_MESSAGE: &str = "No light dongle found. Plug one in, then find it again.";

fn handle_request(runtime: &mut DaemonRuntime, request: &DaemonRequest) -> ReadResult<()> {
    let action = request.action.as_str();
    let outcome = dispatch_action(runtime, action, &request.input);
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

fn dispatch_action(runtime: &mut DaemonRuntime, action: &str, input: &Value) -> DaemonOutcome {
    if action == PING_ACTION {
        return DaemonOutcome::Handled;
    }

    if action == "kill" {
        return DaemonOutcome::Handled;
    }

    if action == crate::runtime::actions::SETTINGS {
        return match crate::platform::open_settings() {
            Ok(()) => DaemonOutcome::Handled,
            Err(error) => DaemonOutcome::Error(format!("{error:#}")),
        };
    }

    if action == LIGHTS_QUERY {
        let main_target_id = store::load()
            .map(|config| config.main_target_id)
            .unwrap_or_default();
        return DaemonOutcome::HandledWithData(payloads::lights_payload(
            &light_rows(runtime),
            &main_target_id,
            runtime_has_presets(runtime),
        ));
    }

    if action == PRESETS_QUERY {
        return DaemonOutcome::HandledWithData(payloads::presets_payload(&runtime_presets(
            runtime,
        )));
    }

    if presets::is_preset_action(action) && matches!(runtime, DaemonRuntime::Unavailable(_)) {
        return edit_presets_offline(action, input);
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
        return state.handle_input_action(action, input);
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
        return stored_light_rows(&store::load().unwrap_or_default());
    };
    let guard = state.shared_service();
    let Ok(lock) = guard.lock() else {
        return Vec::new();
    };
    let Some(svc) = lock.as_ref() else {
        return Vec::new();
    };
    let config = store::load().unwrap_or_default();
    let devices = svc.backend().devices();
    drop(lock);
    let shared_looks = state.shared_looks();
    let looks_view = {
        let mut guard = looks::lock(&shared_looks);
        guard.set_fallback(&config);
        guard.clone()
    };
    devices
        .iter()
        .map(|device| {
            let ieee = format_ieee(&device.ieee_address);
            let name = config
                .devices
                .get(&ieee)
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| ieee.clone());
            LightRow {
                look: looks_view.look(&ieee),
                ieee,
                name,
                network_address: device.network_address,
            }
        })
        .collect()
}

fn stored_light_rows(config: &crate::config::model::PluginConfig) -> Vec<LightRow> {
    let stored_looks = looks::LightLooks::new(config);
    config
        .devices
        .values()
        .map(|entry| LightRow {
            look: stored_looks.look(&entry.ieee_address),
            ieee: entry.ieee_address.clone(),
            name: entry.name.clone(),
            network_address: entry.network_address.unwrap_or_default(),
        })
        .collect()
}

fn runtime_presets(runtime: &DaemonRuntime) -> PresetConfig {
    match runtime {
        DaemonRuntime::Ready(state) => state.presets().clone(),
        DaemonRuntime::Unavailable(_) => store::load()
            .map(|config| config.presets)
            .unwrap_or_default(),
    }
}

fn runtime_has_presets(runtime: &DaemonRuntime) -> bool {
    presets::any_enabled(&runtime_presets(runtime))
}

fn edit_presets_offline(action: &str, input: &Value) -> DaemonOutcome {
    let config = match store::load() {
        Ok(config) => config,
        Err(error) => return DaemonOutcome::Error(error.to_string()),
    };
    let known_looks = looks::LightLooks::new(&config);
    let addresses = looks::config_addresses(&config.devices);
    let target = config.main_target();
    let mut edited = config.presets.clone();
    let result = presets::apply(&mut edited, action, input, |ieee| {
        looks::look_for(ieee, &target, &addresses, &known_looks)
    });
    match result {
        Ok(()) => {
            persist::submit(persist::Change::Presets(Box::new(edited)));
            DaemonOutcome::Handled
        }
        Err(message) => DaemonOutcome::Error(message),
    }
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
        let outcome = dispatch_action(&mut runtime, PING_ACTION, &Value::Null);

        assert!(matches!(outcome, DaemonOutcome::Handled));
    }

    #[test]
    fn daemon_answers_the_replace_kill_with_handled() {
        let mut runtime = DaemonRuntime::Unavailable("coordinator missing".to_string());
        let outcome = dispatch_action(&mut runtime, "kill", &Value::Null);

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
        let outcome = dispatch_action(&mut runtime, crate::runtime::actions::PAIR, &Value::Null);

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
            let DaemonOutcome::HandledWithData(data) =
                dispatch_action(&mut runtime, DONGLE_QUERY, &Value::Null)
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
            (PRESETS_QUERY, serde_json::json!([])),
        ];
        for (query, items) in cases {
            let mut runtime = DaemonRuntime::Unavailable("permission denied".to_string());
            let DaemonOutcome::HandledWithData(data) =
                dispatch_action(&mut runtime, query, &Value::Null)
            else {
                panic!("expected data for {query}");
            };
            assert_eq!(data["items"], items, "{query}");
        }
        let mut runtime = DaemonRuntime::Unavailable("permission denied".to_string());
        let DaemonOutcome::HandledWithData(find) =
            dispatch_action(&mut runtime, FIND_LIGHTS_QUERY, &Value::Null)
        else {
            panic!("expected find data");
        };
        assert_eq!(find["lead"]["can_find"], true);
        assert_eq!(find["lead"]["can_start"], false);
    }

    #[test]
    fn saved_lights_are_listed_from_the_config_without_a_dongle() {
        let mut config = crate::config::model::PluginConfig::default();
        config.devices.insert(
            "00:11:22:33:44:55:66:77".into(),
            DeviceEntry {
                ieee_address: "00:11:22:33:44:55:66:77".into(),
                name: "Desk".into(),
                endpoints: Vec::new(),
                online: false,
                network_address: Some(0x1A2B),
                look: Some(crate::config::model::LightLook {
                    on: false,
                    color_hex: "#FFAA00".into(),
                    brightness: 35,
                    mirek: 410,
                }),
            },
        );

        let rows = stored_light_rows(&config);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Desk");
        assert_eq!(rows[0].ieee, "00:11:22:33:44:55:66:77");
        assert_eq!(rows[0].network_address, 0x1A2B);
        assert_eq!(rows[0].look.brightness, 35);
        assert!(!rows[0].look.on);
    }

    #[test]
    fn unavailable_runtime_reports_the_dongle_status_word() {
        let cases = [
            (
                "no Zigbee coordinator responded on auto-detected serial ports: /dev/tty.fake",
                "Not found",
            ),
            ("permission denied", "Offline"),
        ];
        for (failure, status) in cases {
            let mut runtime = DaemonRuntime::Unavailable(failure.to_string());
            let DaemonOutcome::HandledWithData(data) =
                dispatch_action(&mut runtime, DONGLE_QUERY, &Value::Null)
            else {
                panic!("expected dongle data for {failure}");
            };
            assert_eq!(data["status"], status, "{failure}");
        }
    }

    #[test]
    fn light_actions_need_the_dongle_but_say_so_plainly() {
        let input = serde_json::json!({ "ieee": "00:11:22:33:44:55:66:77", "value": 50 });
        for action in [
            crate::runtime::actions::LIGHT_ON,
            crate::runtime::actions::SET_LIGHT_BRIGHTNESS,
            crate::runtime::actions::REMOVE_LIGHT,
        ] {
            let mut runtime = DaemonRuntime::Unavailable("permission denied".to_string());
            match dispatch_action(&mut runtime, action, &input) {
                DaemonOutcome::Error(message) => assert_eq!(message, "permission denied"),
                _ => panic!("expected an error for {action}"),
            }
        }
    }

    #[test]
    fn preset_edits_without_the_dongle_still_check_their_input() {
        let cases = [
            (
                crate::runtime::actions::SET_PRESET_COLOR,
                serde_json::json!({ "id": "preset_1", "value": "red" }),
            ),
            (
                crate::runtime::actions::REMOVE_PRESET,
                serde_json::json!({ "id": "preset_12" }),
            ),
            (
                crate::runtime::actions::SET_PRESET_BRIGHTNESS,
                serde_json::json!({ "value": 20 }),
            ),
        ];
        for (action, input) in cases {
            let mut runtime = DaemonRuntime::Unavailable("permission denied".to_string());
            match dispatch_action(&mut runtime, action, &input) {
                DaemonOutcome::Error(message) => {
                    assert_ne!(message, "permission denied", "{action}");
                }
                _ => panic!("expected an input error for {action}"),
            }
        }
    }

    #[test]
    fn unavailable_runtime_preserves_unknown_errors() {
        let mut runtime = DaemonRuntime::Unavailable("permission denied".to_string());
        let outcome = dispatch_action(&mut runtime, crate::runtime::actions::PAIR, &Value::Null);

        match outcome {
            DaemonOutcome::Error(message) => assert_eq!(message, "permission denied"),
            _ => panic!("expected backend error"),
        }
    }
}
