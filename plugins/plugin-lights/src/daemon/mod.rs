#[cfg(not(unix))]
compile_error!("plugin-lights daemon requires unix domain sockets");

mod state;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

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

    loop {
        let (stream, _) = listener.accept()?;
        if let Err(error) = handle_stream(&mut state, stream) {
            eprintln!("{error:#}");
        }
    }
}

fn bind_listener(socket_path: &str) -> Result<UnixListener> {
    let path = Path::new(socket_path);
    let _ = std::fs::remove_file(path);
    UnixListener::bind(path)
        .with_context(|| format!("failed to bind daemon socket {}", path.display()))
}

fn handle_stream(state: &mut DaemonState, stream: UnixStream) -> Result<()> {
    let request = read_request(&stream)?;
    let response = response_line(state.handle_action(&request.action))?;
    write_response(stream, &response)
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
