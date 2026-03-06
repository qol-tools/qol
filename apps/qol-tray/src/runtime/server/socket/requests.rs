use std::collections::HashSet;
use std::os::unix::net::UnixStream;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind, RuntimeRequest, SubscribeAck};

use super::super::shared::SharedState;
use super::io::{write_flushed_json_line, write_state};

const SUBSCRIBER_WRITE_TIMEOUT_SECS: u64 = 5;

pub(super) fn handle_request(request: &str, writer: &mut UnixStream, shared: &SharedState) {
    if handle_json_request(request, writer, shared) {
        return;
    }

    handle_text_request(request, writer, shared);
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

fn forward_events(writer: &mut UnixStream, rx: std_mpsc::Receiver<RuntimeEvent>) {
    for event in rx {
        if !write_flushed_json_line(writer, &event) {
            return;
        }
    }
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
