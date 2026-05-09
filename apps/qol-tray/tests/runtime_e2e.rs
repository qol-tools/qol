#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use qol_runtime::protocol::RuntimeEvent;
use qol_runtime::MonitorBounds;
use qol_tray::runtime::testing::TestRuntime;
use serde_json::Value;
use tempfile::TempDir;

const READ_TIMEOUT: Duration = Duration::from_secs(1);

struct Listener {
    socket_path: PathBuf,
    runtime: TestRuntime,
    _tempdir: TempDir,
}

fn listener() -> &'static Listener {
    static LISTENER: OnceLock<Listener> = OnceLock::new();
    LISTENER.get_or_init(|| {
        let tempdir = TempDir::new().expect("tempdir");
        let socket_path = tempdir.path().join("runtime.sock");
        let runtime = TestRuntime::new(vec![mon(0.0), mon(2000.0), mon(4000.0)]);
        runtime.spawn_listener(&socket_path);
        wait_until_bound(&socket_path);
        Listener {
            socket_path,
            runtime,
            _tempdir: tempdir,
        }
    })
}

fn wait_until_bound(path: &PathBuf) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if UnixStream::connect(path).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("listener never bound at {}", path.display());
}

fn mon(x: f32) -> MonitorBounds {
    MonitorBounds {
        x,
        y: 0.0,
        width: 1000.0,
        height: 1000.0,
    }
}

fn connect() -> UnixStream {
    let stream = UnixStream::connect(&listener().socket_path).expect("connect");
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .expect("set_read_timeout");
    stream
        .set_write_timeout(Some(READ_TIMEOUT))
        .expect("set_write_timeout");
    stream
}

fn send_line(stream: &mut UnixStream, payload: &[u8]) {
    stream.write_all(payload).expect("write_all");
    if !payload.ends_with(b"\n") {
        stream.write_all(b"\n").expect("write newline");
    }
    stream.flush().expect("flush");
}

fn read_to_eof(stream: &mut UnixStream) -> String {
    let mut buf = String::new();
    stream.read_to_string(&mut buf).expect("read_to_string");
    buf
}

fn read_line_with_deadline(reader: &mut BufReader<UnixStream>) -> String {
    let mut line = String::new();
    let n = reader.read_line(&mut line).expect("read_line");
    assert!(n > 0, "expected a line before EOF");
    line
}

#[test]
fn text_get_state_returns_serialized_monitors_with_trailing_newline() {
    let mut stream = connect();
    send_line(&mut stream, b"GET_STATE");

    let response = read_to_eof(&mut stream);

    assert!(
        response.ends_with('\n'),
        "response must end with newline: {response:?}",
    );
    let parsed: Value = serde_json::from_str(response.trim()).expect("valid JSON");
    let monitors = parsed
        .get("monitors")
        .and_then(|v| v.as_array())
        .expect("monitors array present");
    assert_eq!(monitors.len(), 3, "three configured monitors: {parsed}");
}

#[test]
fn json_get_state_returns_same_shape_as_text_get_state() {
    let mut stream = connect();
    send_line(&mut stream, br#"{"cmd":"get_state"}"#);

    let response = read_to_eof(&mut stream);

    let parsed: Value = serde_json::from_str(response.trim()).expect("valid JSON");
    assert!(
        parsed.get("monitors").is_some(),
        "json get_state must include monitors: {parsed}",
    );
    assert!(
        parsed.get("active_monitor_idx").is_some(),
        "json get_state must include active_monitor_idx: {parsed}",
    );
}

#[test]
fn json_set_focus_is_fire_and_forget_with_no_response_body() {
    let mut stream = connect();
    send_line(&mut stream, br#"{"cmd":"set_focus","monitor_idx":1}"#);

    let response = read_to_eof(&mut stream);

    assert!(
        response.is_empty(),
        "set_focus must not write a response: {response:?}",
    );

    let mut probe = connect();
    send_line(&mut probe, b"GET_STATE");
    let probe_response = read_to_eof(&mut probe);
    let parsed: Value = serde_json::from_str(probe_response.trim()).expect("valid JSON");
    assert_eq!(
        parsed.get("focus_monitor_idx").and_then(|v| v.as_u64()),
        Some(1),
        "set_focus must stamp focus visible to the next get_state: {parsed}",
    );
}

#[test]
fn unknown_text_verb_returns_no_response_and_closes_clean() {
    let mut stream = connect();
    send_line(&mut stream, b"REBOOT");

    let response = read_to_eof(&mut stream);

    assert!(
        response.is_empty(),
        "unknown verb must not write a response: {response:?}",
    );
}

#[test]
fn eight_concurrent_get_state_clients_all_receive_monitors_payload() {
    const N: usize = 8;
    let handles: Vec<_> = (0..N)
        .map(|i| {
            thread::spawn(move || {
                let mut stream = connect();
                send_line(&mut stream, b"GET_STATE");
                let response = read_to_eof(&mut stream);
                (i, response)
            })
        })
        .collect();

    for handle in handles {
        let (i, response) = handle.join().expect("client thread join");
        let parsed: Value = serde_json::from_str(response.trim()).unwrap_or_else(|err| {
            panic!("client #{i} must receive valid JSON: err={err}, body={response:?}")
        });
        assert!(
            parsed.get("monitors").is_some(),
            "client #{i} must receive monitors: {parsed}",
        );
    }
}

#[test]
fn json_subscribe_round_trips_ack_then_delivers_published_event() {
    let stream = connect();
    let mut writer = stream.try_clone().expect("clone stream");
    let mut reader = BufReader::new(stream);

    send_line(
        &mut writer,
        br#"{"cmd":"subscribe","events":["cursor_moved"]}"#,
    );

    let ack_line = read_line_with_deadline(&mut reader);
    let ack: Value = serde_json::from_str(ack_line.trim()).expect("ack is JSON");
    assert_eq!(
        ack.get("status").and_then(|v| v.as_str()),
        Some("subscribed"),
        "first line must be the Subscribed ack: {ack}",
    );

    let runtime = listener().runtime.clone();
    runtime.publish(&[RuntimeEvent::CursorMoved { x: 42.0, y: 99.0 }]);

    let event_line = read_line_with_deadline(&mut reader);
    let parsed: RuntimeEvent =
        serde_json::from_str(event_line.trim()).expect("event line is RuntimeEvent JSON");
    let RuntimeEvent::CursorMoved { x, y } = parsed else {
        panic!("expected CursorMoved, got: {parsed:?}");
    };
    assert_eq!((x, y), (42.0, 99.0), "event payload must round-trip");

    drop(reader);
    drop(writer);
    runtime.publish(&[RuntimeEvent::CursorMoved { x: 0.0, y: 0.0 }]);
}
