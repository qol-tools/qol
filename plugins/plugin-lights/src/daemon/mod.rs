#[cfg(not(unix))]
compile_error!("plugin-lights daemon requires unix domain sockets");

mod state;
pub mod ws;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::backend::zigbee::ZigbeeBackend;
use crate::config::model::{DeviceEntry, EndpointEntry};
use crate::config::store;
use crate::service::light_service::LightService;

pub use state::{DaemonOutcome, DaemonState};

#[derive(Debug, Deserialize)]
struct DaemonRequest {
    action: String,
}

#[derive(Debug, Serialize)]
struct DaemonResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

enum DaemonRuntime {
    Ready(Box<DaemonState>),
    Unavailable(String),
}

pub fn run_from_env() -> Result<()> {
    let socket_path =
        std::env::var("QOL_TRAY_DAEMON_SOCKET").context("QOL_TRAY_DAEMON_SOCKET is not set")?;
    run(&socket_path)
}

pub fn execute_action_once(action: &str) -> Result<()> {
    let mut state = DaemonState::new()?;
    let outcome = state.handle_action(action);
    map_outcome(action, outcome)
}

pub fn run(socket_path: &str) -> Result<()> {
    let listener = bind_listener(socket_path)?;
    let mut runtime = runtime_state();

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_stream(&mut runtime, stream) {
                    eprintln!("{error:#}");
                }
            }
            Err(error) => eprintln!("accept error: {error:#}"),
        }
    }

    Ok(())
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

fn bind_listener(socket_path: &str) -> Result<UnixListener> {
    let path = Path::new(socket_path);
    let _ = std::fs::remove_file(path);
    UnixListener::bind(path)
        .with_context(|| format!("failed to bind daemon socket {}", path.display()))
}

fn handle_stream(runtime: &mut DaemonRuntime, stream: UnixStream) -> Result<()> {
    let request = read_request(&stream)?;
    let Some(request) = request else {
        return Ok(());
    };
    let outcome = dispatch_action(runtime, &request.action);
    let response = response_line(outcome.clone())?;
    write_response(stream, &response)?;
    if let DaemonOutcome::Error(message) = outcome {
        eprintln!("action '{}' failed: {}", request.action, message);
    }
    Ok(())
}

const CONNECTION_STATUS_QUERY: &str = "connection_status";
const LIST_DEVICES_QUERY: &str = "list_devices";

fn dispatch_action(runtime: &mut DaemonRuntime, action: &str) -> DaemonOutcome {
    // Reads from the coordinator's live device registry, not the config file.
    // Devices appear here immediately when they join (via DeviceJoined event),
    // without needing a config reload.
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
        let state = match runtime {
            DaemonRuntime::Ready(_) => "ok",
            DaemonRuntime::Unavailable(_) => "offline",
        };
        return DaemonOutcome::HandledWithData(serde_json::json!({ "state": state }));
    }

    if let DaemonRuntime::Ready(state) = runtime {
        return state.handle_action(action);
    }

    if action != "reload" {
        if let DaemonRuntime::Unavailable(message) = runtime {
            return DaemonOutcome::Error(message.clone());
        }
        unreachable!()
    }

    let state = match DaemonState::new() {
        Ok(state) => state,
        Err(error) => {
            let message = error.to_string();
            *runtime = DaemonRuntime::Unavailable(message.clone());
            return DaemonOutcome::Error(message);
        }
    };

    eprintln!("coordinator ready");
    start_background_services(&state);
    *runtime = DaemonRuntime::Ready(Box::new(state));
    DaemonOutcome::Handled
}

fn read_request(stream: &UnixStream) -> Result<Option<DaemonRequest>> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(trimmed)
        .map(Some)
        .context("failed to parse daemon request")
}

fn response_line(outcome: DaemonOutcome) -> Result<String> {
    let response = match outcome {
        DaemonOutcome::Handled => DaemonResponse {
            status: "handled",
            message: None,
            data: None,
        },
        DaemonOutcome::HandledWithData(data) => DaemonResponse {
            status: "handled",
            message: None,
            data: Some(data),
        },
        DaemonOutcome::Fallback => DaemonResponse {
            status: "fallback",
            message: None,
            data: None,
        },
        DaemonOutcome::Error(message) => DaemonResponse {
            status: "error",
            message: Some(message),
            data: None,
        },
    };
    let mut line = serde_json::to_string(&response)?;
    line.push('\n');
    Ok(line)
}

fn write_response(mut stream: UnixStream, response: &str) -> Result<()> {
    stream.write_all(response.as_bytes())?;
    Ok(())
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
