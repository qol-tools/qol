mod io;
mod requests;

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;

use super::shared::SharedState;
use crate::paths::STATE_SOCKET_PATH;
use io::{prepare_stream, read_request};
use requests::handle_request;

pub(super) fn run(shared: Arc<SharedState>) {
    run_at(shared, Path::new(STATE_SOCKET_PATH));
}

pub(crate) fn run_at(shared: Arc<SharedState>, path: &Path) {
    let _ = std::fs::remove_file(path);

    let Some(listener) = bind_listener(path) else {
        return;
    };

    log::info!("Runtime socket listening on {}", path.display());

    for stream in listener.incoming() {
        spawn_connection(stream, Arc::clone(&shared));
    }
}

fn bind_listener(path: &Path) -> Option<UnixListener> {
    match UnixListener::bind(path) {
        Ok(listener) => Some(listener),
        Err(error) => {
            log::error!(
                "Failed to bind runtime socket at {}: {}",
                path.display(),
                error
            );
            None
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

    handle_request(&request, &mut writer, shared);
}

fn spawn_connection(stream: std::io::Result<UnixStream>, shared: Arc<SharedState>) {
    let Ok(stream) = stream else {
        return;
    };

    let _ = std::thread::Builder::new()
        .name("runtime-conn".into())
        .spawn(move || handle_connection(stream, &shared));
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use qol_runtime::MonitorBounds;
    use std::io::{Read, Write};
    use std::time::Duration;

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

    fn read_response(stream: &mut UnixStream, timeout_ms: u64) -> String {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(timeout_ms)));
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    #[test]
    fn handle_connection_writes_state_for_text_get_state_request() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let (server_side, mut client_side) = pair();
        client_side.write_all(b"GET_STATE\n").unwrap();

        handle_connection(server_side, &shared);

        let response = read_response(&mut client_side, 200);
        assert!(
            response.contains("monitors"),
            "response must include serialized monitors: {response:?}",
        );
    }

    #[test]
    fn handle_connection_dispatches_json_set_focus_then_returns_silently() {
        let monitors = vec![mon(0.0), mon(2000.0)];
        let shared = SharedState::new(monitors.clone());
        let (server_side, mut client_side) = pair();
        client_side
            .write_all(br#"{"cmd":"set_focus","monitor_idx":1}"#)
            .unwrap();
        client_side.write_all(b"\n").unwrap();

        handle_connection(server_side, &shared);

        let focus = shared.input().focus.expect("focus must be stamped");
        assert_eq!(focus.monitor, monitors[1]);
        let response = read_response(&mut client_side, 100);
        assert!(
            response.is_empty(),
            "set_focus is a fire-and-forget command (no response): {response:?}",
        );
    }

    #[test]
    fn handle_connection_returns_silently_when_request_is_empty_line() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let (server_side, mut client_side) = pair();
        client_side.write_all(b"\n").unwrap();

        handle_connection(server_side, &shared);

        let response = read_response(&mut client_side, 100);
        assert!(
            response.is_empty(),
            "blank line ⇒ read_request returns None ⇒ no response: {response:?}",
        );
    }

    #[test]
    fn handle_connection_returns_silently_when_request_is_unrecognised_text() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let (server_side, mut client_side) = pair();
        client_side.write_all(b"NOT_A_KNOWN_VERB\n").unwrap();

        handle_connection(server_side, &shared);

        let response = read_response(&mut client_side, 100);
        assert!(
            response.is_empty(),
            "unknown verbs are dropped, no response: {response:?}",
        );
    }

    #[test]
    fn handle_connection_returns_silently_when_peer_closes_before_writing() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let (server_side, client_side) = pair();
        drop(client_side);

        handle_connection(server_side, &shared);
    }

    #[test]
    fn spawn_connection_drops_silently_on_error_stream() {
        let shared = Arc::new(SharedState::new(vec![mon(0.0)]));
        let err = std::io::Error::other("synthetic");
        spawn_connection(Err(err), shared);
    }

    #[test]
    fn spawn_connection_handles_request_in_background_thread() {
        let shared = Arc::new(SharedState::new(vec![mon(0.0)]));
        let (server_side, mut client_side) = pair();
        client_side.write_all(b"GET_STATE\n").unwrap();

        spawn_connection(Ok(server_side), Arc::clone(&shared));

        let response = read_response(&mut client_side, 1_000);
        assert!(
            response.contains("monitors"),
            "spawned thread must service the request: {response:?}",
        );
    }

    #[test]
    fn spawn_connection_serves_concurrent_clients_independently() {
        let shared = Arc::new(SharedState::new(vec![mon(0.0), mon(2000.0)]));
        let mut clients = Vec::with_capacity(4);
        for _ in 0..4 {
            let (server_side, mut client_side) = pair();
            client_side.write_all(b"GET_STATE\n").unwrap();
            spawn_connection(Ok(server_side), Arc::clone(&shared));
            clients.push(client_side);
        }
        for (i, mut client) in clients.into_iter().enumerate() {
            let response = read_response(&mut client, 1_000);
            assert!(
                response.contains("monitors"),
                "client #{i} must be served: {response:?}",
            );
        }
    }

    type TextRequestCase = (&'static str, &'static str, bool);

    #[test]
    fn handle_connection_text_request_table() {
        let cases: &[TextRequestCase] = &[
            ("get_state lower-case", "get_state\n", true),
            ("GET_STATE upper-case", "GET_STATE\n", true),
            ("Get_State mixed-case", "Get_State\n", true),
            ("padded GET_STATE trims", "  GET_STATE  \n", true),
            ("unknown verb is silent", "REBOOT\n", false),
            ("blank request is silent", "\n", false),
            ("whitespace-only request is silent", "    \n", false),
            ("text SET_FOCUS does not respond", "SET_FOCUS 0\n", false),
            (
                "text SET_FOCUS oob does not respond",
                "SET_FOCUS 99\n",
                false,
            ),
            (
                "text SET_FOCUS non-numeric does not respond",
                "SET_FOCUS abc\n",
                false,
            ),
        ];

        for (label, payload, expect_response) in cases {
            let shared = SharedState::new(vec![mon(0.0), mon(2000.0)]);
            let (server_side, mut client_side) = pair();
            client_side.write_all(payload.as_bytes()).unwrap();

            handle_connection(server_side, &shared);

            let response = read_response(&mut client_side, 100);
            let got = !response.is_empty();
            assert_eq!(
                got, *expect_response,
                "case: {label} (response={response:?})"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_handle_connection_get_state_always_emits_monitors_field(
            n_monitors in 1usize..6,
        ) {
            let monitors: Vec<_> = (0..n_monitors).map(|i| mon(i as f32 * 1000.0)).collect();
            let shared = SharedState::new(monitors);
            let (server_side, mut client_side) = pair();
            client_side.write_all(b"GET_STATE\n").unwrap();

            handle_connection(server_side, &shared);

            let response = read_response(&mut client_side, 200);
            prop_assert!(response.contains("monitors"), "response: {response:?}");
            prop_assert!(response.ends_with('\n'), "response must end with newline: {response:?}");
        }

        #[test]
        fn prop_handle_connection_unknown_verbs_return_no_response(
            verb in "[A-Z][A-Z0-9_]{0,20}",
        ) {
            prop_assume!(!verb.eq_ignore_ascii_case("GET_STATE"));
            prop_assume!(!verb.starts_with("SET_FOCUS"));
            let shared = SharedState::new(vec![mon(0.0)]);
            let (server_side, mut client_side) = pair();
            let line = format!("{verb}\n");
            client_side.write_all(line.as_bytes()).unwrap();

            handle_connection(server_side, &shared);

            let response = read_response(&mut client_side, 100);
            prop_assert!(response.is_empty(), "verb={verb:?} response={response:?}");
        }

        #[test]
        fn prop_handle_connection_blank_or_whitespace_lines_are_dropped(
            ws in "[ \\t]{0,16}",
        ) {
            let shared = SharedState::new(vec![mon(0.0)]);
            let (server_side, mut client_side) = pair();
            let line = format!("{ws}\n");
            client_side.write_all(line.as_bytes()).unwrap();

            handle_connection(server_side, &shared);

            let response = read_response(&mut client_side, 100);
            prop_assert!(response.is_empty(), "ws={ws:?} response={response:?}");
        }

        #[test]
        fn prop_handle_connection_text_set_focus_in_range_stamps_focus(
            n_monitors in 2usize..6,
            target_idx in 0usize..6,
        ) {
            let monitors: Vec<_> = (0..n_monitors).map(|i| mon(i as f32 * 1000.0)).collect();
            let shared = SharedState::new(monitors.clone());
            let (server_side, mut client_side) = pair();
            let line = format!("SET_FOCUS {target_idx}\n");
            client_side.write_all(line.as_bytes()).unwrap();

            handle_connection(server_side, &shared);

            let response = read_response(&mut client_side, 100);
            prop_assert!(response.is_empty(), "SET_FOCUS never responds");
            let focus = shared.input().focus;
            if target_idx < n_monitors {
                let focus = focus.expect("in-range idx must stamp");
                prop_assert_eq!(focus.monitor, monitors[target_idx]);
            } else {
                prop_assert!(focus.is_none(), "out-of-range idx must not stamp");
            }
        }
    }
}
