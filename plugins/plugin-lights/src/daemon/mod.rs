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

    let mut state = DaemonState::new()?;
    eprintln!("coordinator ready");

    let events_rx = state.events();
    thread::Builder::new()
        .name("device-monitor".into())
        .spawn(move || device_monitor_loop(events_rx))
        .context("failed to spawn device monitor")?;

    let ws_buffer = ws::CommandBuffer::default();
    ws::start(
        ws_buffer,
        state.shared_service(),
        state.main_target().clone(),
    );

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_stream(&mut state, stream) {
                    eprintln!("{error:#}");
                }
            }
            Err(e) => eprintln!("accept error: {e:#}"),
        }
    }

    Ok(())
}

fn device_monitor_loop(events: crossbeam_channel::Receiver<zigbee_znp::ZigbeeEvent>) {
    loop {
        match events.recv() {
            Ok(zigbee_znp::ZigbeeEvent::DeviceJoined(device)) => {
                let ieee = format_ieee(&device.ieee_address);
                let entry = DeviceEntry {
                    ieee_address: ieee.clone(),
                    name: format!("Device {:04X}", device.network_address),
                    endpoints: device
                        .endpoints
                        .iter()
                        .map(|ep| EndpointEntry {
                            id: ep.id,
                            clusters: ep.input_clusters.clone(),
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
            Ok(zigbee_znp::ZigbeeEvent::DeviceLeft(_)) => {}
            Err(_) => break,
        }
    }
}

fn format_ieee(addr: &[u8; 8]) -> String {
    addr.iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(":")
}

fn bind_listener(socket_path: &str) -> Result<UnixListener> {
    let path = Path::new(socket_path);
    let _ = std::fs::remove_file(path);
    UnixListener::bind(path)
        .with_context(|| format!("failed to bind daemon socket {}", path.display()))
}

fn handle_stream(state: &mut DaemonState, stream: UnixStream) -> Result<()> {
    let request = read_request(&stream)?;
    let ack = response_line(DaemonOutcome::Handled)?;
    write_response(stream, &ack)?;
    let outcome = state.handle_action(&request.action);
    if let DaemonOutcome::Error(msg) = outcome {
        eprintln!("action '{}' failed: {}", request.action, msg);
    }
    Ok(())
}

fn read_request(stream: &UnixStream) -> Result<DaemonRequest> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    serde_json::from_str(line.trim()).context("failed to parse daemon request")
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
