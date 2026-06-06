use std::collections::HashSet;
use std::os::unix::net::UnixStream;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use qol_runtime::protocol::{
    ArmedLifelinesResponse, RuntimeEvent, RuntimeEventKind, RuntimeRequest, SubscribeAck,
};

use super::super::shared::SharedState;
use super::io::{write_flushed_json_line, write_state};

const SUBSCRIBER_WRITE_TIMEOUT_SECS: u64 = 5;
#[cfg(not(test))]
const SUBSCRIBER_KEEPALIVE_PROBE: Duration = Duration::from_secs(5);
#[cfg(test)]
const SUBSCRIBER_KEEPALIVE_PROBE: Duration = Duration::from_millis(50);

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
        RuntimeRequest::Subscribe { plugin_id, events } => {
            handle_subscription(writer, shared, plugin_id, events)
        }
        RuntimeRequest::Lifeline { plugin_id } => handle_lifeline(writer, shared, plugin_id),
        RuntimeRequest::ArmedLifelines => {
            let response = ArmedLifelinesResponse {
                plugin_ids: shared.armed_lifelines(),
            };
            let _ = write_flushed_json_line(writer, &response);
        }
    }

    true
}

fn handle_lifeline(writer: &mut UnixStream, shared: &SharedState, plugin_id: String) {
    log::info!("[runtime/socket] host-death lifeline armed by {plugin_id}");
    shared.arm_lifeline(plugin_id.clone());

    if !write_flushed_json_line(writer, &SubscribeAck::Subscribed) {
        shared.disarm_lifeline(&plugin_id);
        return;
    }

    let _ = writer.set_write_timeout(Some(Duration::from_secs(SUBSCRIBER_WRITE_TIMEOUT_SECS)));

    // No events ever flow on a lifeline; the held-open connection exists purely
    // so the daemon's read sees EOF when this host process dies. Block until the
    // daemon disconnects (it exited) or the probe detects the peer is gone.
    let (_tx, rx) = std_mpsc::channel::<RuntimeEvent>();
    forward_events(writer, rx);

    shared.disarm_lifeline(&plugin_id);
    log::info!("[runtime/socket] host-death lifeline dropped by {plugin_id}");
}

fn handle_subscription(
    writer: &mut UnixStream,
    shared: &SharedState,
    plugin_id: String,
    events: Vec<RuntimeEventKind>,
) {
    let interests: HashSet<_> = events.iter().copied().collect();
    let (tx, rx) = std_mpsc::channel::<RuntimeEvent>();

    let clean_id = plugin_id.strip_prefix("plugin-").unwrap_or(&plugin_id);
    log::info!(
        "[runtime/socket] new subscriber ({}): {:?}",
        clean_id,
        interests
    );
    shared.add_subscriber(plugin_id.clone(), interests, tx);

    if !write_flushed_json_line(writer, &SubscribeAck::Subscribed) {
        return;
    }

    let replayed_idx = replay_active_monitor(writer, shared, &events);
    super::super::trace::subscribed(clean_id, &events, replayed_idx);

    let _ = writer.set_write_timeout(Some(Duration::from_secs(SUBSCRIBER_WRITE_TIMEOUT_SECS)));

    forward_events(writer, rx);

    log::info!("[runtime/socket] subscriber disconnected: {}", clean_id);
}

fn replay_active_monitor(
    writer: &mut UnixStream,
    shared: &SharedState,
    events: &[RuntimeEventKind],
) -> Option<usize> {
    if !events.contains(&RuntimeEventKind::ActiveMonitorChanged) {
        return None;
    }
    let monitors = shared.monitors();
    let input = shared.input();
    let active =
        crate::runtime::state::pick_active_monitor(&input).or_else(|| monitors.first().copied())?;
    let idx = monitors.iter().position(|m| *m == active)?;
    let _ = write_flushed_json_line(
        writer,
        &RuntimeEvent::ActiveMonitorChanged {
            monitor_idx: Some(idx),
            monitor: Some(active),
        },
    );
    Some(idx)
}

fn forward_events(writer: &mut UnixStream, rx: std_mpsc::Receiver<RuntimeEvent>) {
    loop {
        match rx.recv_timeout(SUBSCRIBER_KEEPALIVE_PROBE) {
            Ok(event) => {
                if !write_flushed_json_line(writer, &event) {
                    return;
                }
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                if !peer_is_alive(writer) {
                    return;
                }
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn peer_is_alive(writer: &UnixStream) -> bool {
    use std::os::unix::io::AsRawFd;
    let fd = writer.as_raw_fd();
    let mut buf = [0u8; 1];
    let n = unsafe {
        libc::recv(
            fd,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    if n == 0 {
        return false;
    }
    if n > 0 {
        return true;
    }
    std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock
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

#[cfg(test)]
mod tests {
    use super::*;
    use qol_runtime::MonitorBounds;
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    fn mon(x: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y: 0.0,
            width: 1000.0,
            height: 1000.0,
        }
    }

    fn pair() -> (UnixStream, UnixStream) {
        UnixStream::pair().expect("UnixStream::pair")
    }

    fn read_to_string(stream: &mut UnixStream) -> String {
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(100)));
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    #[test]
    fn handle_request_dispatches_text_get_state_case_insensitive() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let cases = ["GET_STATE", "get_state", "Get_State"];
        for input in cases {
            let (mut writer, mut reader) = pair();
            handle_request(input, &mut writer, &shared);
            drop(writer);
            let response = read_to_string(&mut reader);
            assert!(
                response.contains("monitors"),
                "input {input:?} response: {response:?}",
            );
        }
    }

    #[test]
    fn handle_request_text_set_focus_updates_focus_when_index_in_range() {
        let monitors = vec![mon(0.0), mon(2000.0)];
        let shared = SharedState::new(monitors.clone());
        let (mut writer, _reader) = pair();

        handle_request("SET_FOCUS 1", &mut writer, &shared);

        let focus = shared.input().focus.expect("focus stamped");
        assert_eq!(focus.monitor, monitors[1], "idx=1 picks second monitor");
    }

    #[test]
    fn handle_request_text_set_focus_ignores_out_of_range_index() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let (mut writer, _reader) = pair();

        handle_request("SET_FOCUS 99", &mut writer, &shared);

        assert!(
            shared.input().focus.is_none(),
            "out-of-range idx must not stamp focus",
        );
    }

    #[test]
    fn handle_request_text_set_focus_ignores_non_numeric_index() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let (mut writer, _reader) = pair();

        handle_request("SET_FOCUS not_a_number", &mut writer, &shared);

        assert!(
            shared.input().focus.is_none(),
            "non-numeric idx parsed as Err, no stamp",
        );
    }

    #[test]
    fn handle_request_ignores_unknown_text_payloads() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let (mut writer, mut reader) = pair();
        handle_request("PLEASE_REBOOT", &mut writer, &shared);
        drop(writer);
        let response = read_to_string(&mut reader);
        assert!(
            response.is_empty(),
            "unknown text payload returns nothing: {response:?}",
        );
    }

    #[test]
    fn handle_request_dispatches_json_get_state() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let (mut writer, mut reader) = pair();
        handle_request(r#"{"cmd":"get_state"}"#, &mut writer, &shared);
        drop(writer);
        let response = read_to_string(&mut reader);
        assert!(
            response.contains("monitors"),
            "json get_state should respond: {response:?}",
        );
    }

    #[test]
    fn handle_request_dispatches_json_set_focus() {
        let monitors = vec![mon(0.0), mon(2000.0)];
        let shared = SharedState::new(monitors.clone());
        let (mut writer, _reader) = pair();
        handle_request(
            r#"{"cmd":"set_focus","monitor_idx":1}"#,
            &mut writer,
            &shared,
        );
        let focus = shared.input().focus.expect("focus stamped via json");
        assert_eq!(focus.monitor, monitors[1]);
    }

    #[test]
    fn apply_focus_no_op_when_index_out_of_range() {
        let shared = SharedState::new(vec![mon(0.0)]);
        apply_focus(&shared, 5, "test");
        assert!(shared.input().focus.is_none());
    }

    #[test]
    fn apply_focus_stamps_focus_when_index_resolves_to_monitor() {
        let monitors = vec![mon(0.0), mon(2000.0)];
        let shared = SharedState::new(monitors.clone());
        apply_focus(&shared, 1, "test");
        let focus = shared.input().focus.expect("focus must be stamped");
        assert_eq!(focus.monitor, monitors[1]);
    }

    #[test]
    fn peer_is_alive_returns_false_when_peer_closed() {
        let (writer, reader) = pair();
        drop(reader);
        assert!(!peer_is_alive(&writer));
    }

    #[test]
    fn peer_is_alive_returns_true_while_peer_holds_handle() {
        let (writer, _reader) = pair();
        assert!(peer_is_alive(&writer));
    }

    #[test]
    fn forward_events_exits_after_peer_disconnects_without_any_publish() {
        let (writer, reader) = pair();
        let (_tx, rx) = std_mpsc::channel::<RuntimeEvent>();
        drop(reader);

        let handle = std::thread::spawn(move || {
            let mut writer = writer;
            forward_events(&mut writer, rx);
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        while !handle.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            handle.is_finished(),
            "forward_events must exit on peer-close within keepalive window",
        );
        handle.join().expect("forward_events thread");
    }

    #[test]
    fn forward_events_exits_immediately_when_channel_disconnects() {
        let (writer, _reader) = pair();
        let (tx, rx) = std_mpsc::channel::<RuntimeEvent>();
        drop(tx);

        let handle = std::thread::spawn(move || {
            let mut writer = writer;
            forward_events(&mut writer, rx);
        });

        handle
            .join()
            .expect("forward_events must return on Disconnected");
    }

    #[test]
    fn forward_events_writes_event_when_peer_alive() {
        let (writer, mut reader) = pair();
        let (tx, rx) = std_mpsc::channel::<RuntimeEvent>();
        tx.send(RuntimeEvent::CursorMoved { x: 7.0, y: 11.0 })
            .unwrap();
        drop(tx);

        let handle = std::thread::spawn(move || {
            let mut writer = writer;
            forward_events(&mut writer, rx);
        });

        handle.join().expect("forward_events thread");
        let mut buf = [0u8; 256];
        let _ = reader.set_read_timeout(Some(Duration::from_millis(200)));
        let n = reader.read(&mut buf).unwrap_or(0);
        let body = String::from_utf8_lossy(&buf[..n]);
        assert!(body.contains("cursor_moved"), "got: {body:?}");
    }

    #[test]
    fn armed_lifelines_request_returns_sorted_armed_set() {
        let shared = SharedState::new(vec![mon(0.0)]);
        shared.arm_lifeline("plugin-launcher".to_string());
        shared.arm_lifeline("plugin-alt-tab".to_string());
        let (mut writer, mut reader) = pair();

        handle_request(r#"{"cmd":"armed_lifelines"}"#, &mut writer, &shared);
        drop(writer);

        let response = read_to_string(&mut reader);
        assert!(response.contains("plugin-alt-tab"), "got: {response:?}");
        assert!(response.contains("plugin-launcher"), "got: {response:?}");
    }

    #[test]
    fn lifeline_arms_on_connect_and_disarms_on_disconnect() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let (server, mut client) = pair();

        std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                let mut server = server;
                handle_request(
                    r#"{"cmd":"lifeline","plugin_id":"plugin-x"}"#,
                    &mut server,
                    &shared,
                );
            });

            let ack = read_to_string(&mut client);
            assert!(ack.contains("subscribed"), "ack: {ack:?}");
            assert!(
                shared.armed_lifelines().iter().any(|id| id == "plugin-x"),
                "lifeline must be armed once the daemon connects",
            );

            drop(client);
            handle.join().expect("lifeline handler thread");
        });

        assert!(
            shared.armed_lifelines().is_empty(),
            "lifeline must disarm when the daemon disconnects",
        );
    }
}
