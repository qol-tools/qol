#[cfg(not(unix))]
compile_error!("plugin-lights daemon requires unix domain sockets");

mod state;
pub mod ws;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::model::{DeviceEntry, EndpointEntry};
use crate::config::store;

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

fn device_monitor_loop(events: crossbeam_channel::Receiver<crate::znp::ZigbeeEvent>) {
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
                    config
                        .devices
                        .insert(format!("0x{:04X}", device.network_address), entry);
                    let _ = store::save(&config);
                    eprintln!("device joined: {} (0x{:04X})", ieee, device.network_address);
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
    let _ = thread::Builder::new()
        .name("device-monitor".into())
        .spawn(move || device_monitor_loop(events));

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

fn dispatch_action(runtime: &mut DaemonRuntime, action: &str) -> DaemonOutcome {
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
        },
        DaemonOutcome::Fallback => DaemonResponse {
            status: "fallback",
            message: None,
        },
        DaemonOutcome::Error(message) => DaemonResponse {
            status: "error",
            message: Some(message),
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
    if let DaemonOutcome::Handled = outcome {
        return Ok(());
    }
    if let DaemonOutcome::Fallback = outcome {
        anyhow::bail!("plugin-lights fell back for action '{}'", action);
    }
    if let DaemonOutcome::Error(message) = outcome {
        anyhow::bail!(message);
    }
    unreachable!()
}
