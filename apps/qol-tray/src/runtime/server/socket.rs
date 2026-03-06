use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind, RuntimeRequest, SubscribeAck};
use serde::Serialize;

use super::shared::SharedState;
use crate::paths::STATE_SOCKET_PATH;

const IO_TIMEOUT_MS: u64 = 50;
const SUBSCRIBER_WRITE_TIMEOUT_SECS: u64 = 5;

pub(super) fn run(shared: Arc<SharedState>) {
    let _ = std::fs::remove_file(STATE_SOCKET_PATH);

    let Some(listener) = bind_listener() else {
        return;
    };

    log::info!("Runtime socket listening on {}", STATE_SOCKET_PATH);

    for stream in listener.incoming() {
        spawn_connection(stream, shared.clone());
    }
}

fn apply_focus(shared: &SharedState, monitor_idx: usize, label: &str) {
    let Some(monitor) = shared.monitor_at(monitor_idx) else {
        return;
    };

    log::debug!(
        "{} idx={} mon=({}, {})",
        label,
        monitor_idx,
        monitor.x,
        monitor.y
    );

    shared.with_input(|input| input.update_focus(monitor, Instant::now()));
}

fn bind_listener() -> Option<UnixListener> {
    match UnixListener::bind(STATE_SOCKET_PATH) {
        Ok(listener) => Some(listener),
        Err(error) => {
            log::error!(
                "Failed to bind runtime socket at {}: {}",
                STATE_SOCKET_PATH,
                error
            );
            None
        }
    }
}

fn forward_events(writer: &mut UnixStream, rx: std_mpsc::Receiver<RuntimeEvent>) {
    for event in rx {
        if !write_flushed_json_line(writer, &event) {
            return;
        }
    }
}

fn handle_connection(stream: UnixStream, shared: &SharedState) {
    let Some((mut reader, mut writer)) = prepare_stream(stream) else {
        return;
    };

    let Some(request) = read_request(&mut reader) else {
        return;
    };

    if handle_json_request(&request, &mut writer, shared) {
        return;
    }

    handle_text_request(&request, &mut writer, shared);
}

fn handle_json_request(request: &str, writer: &mut UnixStream, shared: &SharedState) -> bool {
    let Ok(request) = serde_json::from_str::<RuntimeRequest>(request) else {
        return false;
    };

    match request {
        RuntimeRequest::GetState => write_state(writer, shared),
        RuntimeRequest::SetFocus { monitor_idx } => {
            apply_focus(shared, monitor_idx, "[runtime/socket] SET_FOCUS")
        }
        RuntimeRequest::Subscribe { events } => handle_subscription(writer, shared, events),
    }

    true
}

fn handle_subscription(
    writer: &mut UnixStream,
    shared: &SharedState,
    events: Vec<RuntimeEventKind>,
) {
    if !write_flushed_json_line(writer, &SubscribeAck::Subscribed) {
        return;
    }

    let _ = writer.set_write_timeout(Some(Duration::from_secs(SUBSCRIBER_WRITE_TIMEOUT_SECS)));

    let interests: HashSet<_> = events.into_iter().collect();
    let (tx, rx) = std_mpsc::channel::<RuntimeEvent>();

    log::info!("[runtime/socket] new subscriber: {:?}", interests);

    shared.add_subscriber(interests, tx);
    forward_events(writer, rx);

    log::info!("[runtime/socket] subscriber disconnected");
}

fn handle_text_request(request: &str, writer: &mut UnixStream, shared: &SharedState) {
    if let Some(rest) = request.strip_prefix("SET_FOCUS ") {
        handle_text_set_focus(rest, shared);
        return;
    }

    if !request.eq_ignore_ascii_case("GET_STATE") {
        return;
    }

    write_state(writer, shared);
}

fn handle_text_set_focus(request: &str, shared: &SharedState) {
    let Ok(idx) = request.parse::<usize>() else {
        return;
    };
    apply_focus(shared, idx, "[runtime/socket] SET_FOCUS (text)");
}

fn prepare_stream(stream: UnixStream) -> Option<(BufReader<UnixStream>, UnixStream)> {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(IO_TIMEOUT_MS)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(IO_TIMEOUT_MS)));
    let writer = stream.try_clone().ok()?;
    Some((BufReader::new(stream), writer))
}

fn read_request(reader: &mut BufReader<UnixStream>) -> Option<String> {
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return None;
    }

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

fn spawn_connection(stream: std::io::Result<UnixStream>, shared: Arc<SharedState>) {
    let Ok(stream) = stream else {
        return;
    };

    let _ = std::thread::Builder::new()
        .name("runtime-conn".into())
        .spawn(move || handle_connection(stream, &shared));
}

fn write_flushed_json_line<T: Serialize>(writer: &mut UnixStream, value: &T) -> bool {
    if !write_json_line(writer, value) {
        return false;
    }
    writer.flush().is_ok()
}

fn write_json_line<T: Serialize>(writer: &mut UnixStream, value: &T) -> bool {
    let Ok(json) = serde_json::to_string(value) else {
        return false;
    };

    if writer.write_all(json.as_bytes()).is_err() {
        return false;
    }

    writer.write_all(b"\n").is_ok()
}

fn write_state(writer: &mut UnixStream, shared: &SharedState) {
    let _ = write_json_line(writer, &shared.build_state());
}
