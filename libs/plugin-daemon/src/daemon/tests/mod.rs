use super::*;
use std::io::BufRead;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_SOCKET_ID: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn daemon_listener_fd_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

pub(crate) fn temp_socket_name(tag: &str) -> &'static str {
    let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    Box::leak(format!("qol-plugin-daemon-{tag}-{}-{id}.sock", std::process::id()).into_boxed_str())
}

pub(crate) fn fallback_config(socket_name: &'static str) -> DaemonConfig {
    DaemonConfig {
        socket: SocketSource::Fallback {
            default_socket_name: socket_name,
            use_tmpdir_env: false,
        },
        support_replace_existing: false,
    }
}

fn socket_exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn wait_for_socket(path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !socket_exists(path) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(socket_exists(path), "listener must bind {}", path.display());
}

fn stream_with_line(line: &str) -> LocalStream {
    let (mut writer, reader) = LocalStream::pair().unwrap();
    writer.write_all(line.as_bytes()).unwrap();
    writer.write_all(b"\n").unwrap();
    reader
}

fn read_response(stream: LocalStream) -> DaemonResponse {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(line.trim()).unwrap()
}

fn response_for_result<C>(
    result: ReadResult<C>,
    command_response: impl FnOnce(C) -> DaemonResponse,
) -> (bool, DaemonResponse) {
    let (mut server, client) = LocalStream::pair().unwrap();
    let should_continue = handle_read_result(&mut server, result, command_response);
    drop(server);
    (should_continue, read_response(client))
}

#[test]
fn fixed_socket_ignores_the_host_daemon_socket() {
    let _lock = daemon_listener_fd_env_lock();
    std::env::set_var(qol_conventions::ENV_DAEMON_SOCKET, "/tmp/host.sock");
    let config = DaemonConfig {
        socket: SocketSource::Fixed {
            socket_name: "panel.sock",
            use_tmpdir_env: false,
        },
        support_replace_existing: false,
    };

    let path = socket_path(&config);

    std::env::remove_var(qol_conventions::ENV_DAEMON_SOCKET);
    assert_eq!(path, Some(fallback_socket_dir(false).join("panel.sock")));
}

#[test]
fn parses_json_request_action() {
    let mut stream = stream_with_line(r#"{"action":"open"}"#);
    let result = read_and_parse(&mut stream, |action| {
        ReadResult::<String>::Command(action.to_string())
    });

    match result {
        ReadResult::Command(action) => assert_eq!(action, "open"),
        _ => panic!("expected command"),
    }
}

#[test]
fn parses_json_request_input() {
    let mut stream = stream_with_line(r#"{"action":"pair_device","input":{"address":"AA:BB"}}"#);
    let result = read_request_and_parse(&mut stream, |request| {
        ReadResult::<serde_json::Value>::Command(request.input.clone())
    });

    match result {
        ReadResult::Command(input) => {
            assert_eq!(input, serde_json::json!({"address": "AA:BB"}));
        }
        _ => panic!("expected command"),
    }
}

#[test]
fn parses_plain_and_prefixed_actions() {
    for (line, expected) in [("open", "open"), ("action:reload", "reload")] {
        let mut stream = stream_with_line(line);
        let result = read_and_parse(&mut stream, |action| {
            ReadResult::<String>::Command(action.to_string())
        });

        match result {
            ReadResult::Command(action) => assert_eq!(action, expected, "line: {line}"),
            _ => panic!("expected command for line: {line}"),
        }
    }
}

#[test]
fn ignores_empty_requests() {
    let mut stream = stream_with_line("");
    let result = read_and_parse(&mut stream, |_| ReadResult::<()>::Handled);

    assert!(matches!(result, ReadResult::Ignore));
}

#[test]
fn writes_handled_data_response() {
    let payload = serde_json::json!({ "state": "offline" });
    let (should_continue, response) = response_for_result::<()>(
        ReadResult::HandledWithData(payload.clone()),
        |_| unreachable!(),
    );

    assert!(should_continue);
    match response {
        DaemonResponse::Handled { data } => assert_eq!(data, Some(payload)),
        _ => panic!("expected handled response"),
    }
}

#[test]
fn send_request_returns_daemon_payload() {
    let _lock = daemon_listener_fd_env_lock();
    let socket_name = temp_socket_name("request-payload");
    let config = fallback_config(socket_name);
    let path = socket_path(&config).unwrap();
    remove_socket_file(&path);
    let listener = local_ipc::bind_listener(&path).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let result = read_request_and_parse(&mut stream, |request| {
            assert_eq!(request.action, "devices");
            assert_eq!(request.input, serde_json::json!({ "paired": true }));
            ReadResult::<()>::HandledWithData(serde_json::json!({ "count": 3 }))
        });
        handle_read_result(&mut stream, result, |_| unreachable!());
    });

    let response = send_request(
        &config,
        "devices",
        serde_json::json!({ "paired": true }),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    server.join().unwrap();
    remove_socket_file(path);

    match response {
        DaemonResponse::Handled { data } => {
            assert_eq!(data, Some(serde_json::json!({ "count": 3 })));
        }
        other => panic!("expected handled response, got {other:?}"),
    }
}

#[test]
fn fallback_response_keeps_listener_running() {
    let (should_continue, response) =
        response_for_result::<()>(ReadResult::Fallback, |_| unreachable!());

    assert!(should_continue);
    assert!(matches!(response, DaemonResponse::Fallback));
}

#[test]
fn command_send_fallback_stops_threaded_listener() {
    let (should_continue, response) =
        response_for_result(ReadResult::Command(()), |_| DaemonResponse::Fallback);

    assert!(!should_continue);
    assert!(matches!(response, DaemonResponse::Fallback));
}

#[test]
fn listener_shutdown_dispatches_the_parser_kill_command() {
    let (tx, rx) = std::sync::mpsc::channel();
    let dispatched = dispatch_shutdown_command(&tx, |request| {
        if request.action == "kill" {
            return ReadResult::Command(request.action.clone());
        }
        ReadResult::Fallback
    });

    assert!(dispatched);
    assert_eq!(rx.recv().unwrap(), "kill");
}

#[test]
fn listener_shutdown_rejects_parsers_without_a_kill_command() {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let dispatched = dispatch_shutdown_command(&tx, |_| ReadResult::<String>::Handled);

    assert!(!dispatched);
    assert!(rx.try_recv().is_err());
}

#[test]
fn writes_owned_error_message() {
    let (should_continue, response) = response_for_result::<()>(
        ReadResult::Error("hardware offline".to_string()),
        |_| unreachable!(),
    );

    assert!(should_continue);
    match response {
        DaemonResponse::Error { message } => assert_eq!(message, "hardware offline"),
        _ => panic!("expected error response"),
    }
}

#[test]
fn parser_can_keep_state_across_requests() {
    let mut count = 0;

    for line in ["first", "second"] {
        let mut stream = stream_with_line(line);
        let result = read_and_parse(&mut stream, |action| {
            count += 1;
            assert_eq!(action, line);
            ReadResult::<()>::Handled
        });

        assert!(matches!(result, ReadResult::Handled), "line: {line}");
    }

    assert_eq!(count, 2);
}

#[test]
fn stateful_request_handler_receives_input_and_returns_data() {
    let mut count = 0;
    let mut stream = stream_with_line(r#"{"action":"add","input":{"value":3}}"#);

    let result = read_request_and_parse(&mut stream, |request| {
        count += request.input["value"].as_u64().unwrap();
        ReadResult::<()>::HandledWithData(serde_json::json!({ "count": count }))
    });
    let (should_continue, response) =
        response_for_result(result, |_| DaemonResponse::Handled { data: None });

    assert!(should_continue);
    match response {
        DaemonResponse::Handled { data } => {
            assert_eq!(data, Some(serde_json::json!({ "count": 3 })));
        }
        other => panic!("expected handled response, got {other:?}"),
    }
}

#[test]
fn bind_listener_replaces_stale_socket() {
    let _lock = daemon_listener_fd_env_lock();
    let socket_name = temp_socket_name("stale");
    let config = fallback_config(socket_name);
    let path = socket_path(&config).unwrap();
    remove_socket_file(&path);
    drop(local_ipc::bind_listener(&path).unwrap());
    assert_eq!(
        local_ipc::bind_listener(&path).unwrap_err().kind(),
        ErrorKind::AddrInUse,
        "a dead listener's socket file must block a plain bind"
    );

    let (_listener, bound_path) = bind_listener(&config).unwrap();

    assert_eq!(bound_path, Some(path.clone()));
    assert!(
        LocalStream::connect(&path).is_ok(),
        "the replacement listener must accept connections on the reclaimed path"
    );
    remove_socket_file(path);
}

#[test]
fn bind_listener_replaces_a_running_daemon_when_requested() {
    let _lock = daemon_listener_fd_env_lock();
    let socket_name = temp_socket_name("replace-running");
    let path = std::env::temp_dir().join(socket_name);
    remove_socket_file(&path);
    let listener = local_ipc::bind_listener(&path).unwrap();
    let server_path = path.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let result = read_request_and_parse(&mut stream, |request| {
            assert_eq!(request.action, "kill");
            ReadResult::<()>::Handled
        });
        handle_read_result(&mut stream, result, |_| unreachable!());
        drop(stream);
        drop(listener);
        remove_socket_file(server_path);
    });
    let config = DaemonConfig {
        socket: SocketSource::Path(path.clone()),
        support_replace_existing: true,
    };
    std::env::set_var(qol_conventions::ENV_DAEMON_REPLACE_EXISTING, "1");

    let result = bind_listener(&config);

    std::env::remove_var(qol_conventions::ENV_DAEMON_REPLACE_EXISTING);
    server.join().unwrap();
    let (_listener, bound_path) = result.unwrap();
    assert_eq!(bound_path, Some(path.clone()));
    remove_socket_file(path);
}

#[test]
fn cleanup_leaves_an_inherited_listeners_socket_path_alone() {
    let _lock = daemon_listener_fd_env_lock();
    let socket_name = temp_socket_name("cleanup-inherited");
    let config = fallback_config(socket_name);
    let path = socket_path(&config).unwrap();
    remove_socket_file(&path);
    let _listener = local_ipc::bind_listener(&path).unwrap();
    std::env::set_var(qol_conventions::ENV_DAEMON_LISTENER_FD, "7");

    cleanup(&config);
    let survived_inherited = socket_exists(&path);

    std::env::remove_var(qol_conventions::ENV_DAEMON_LISTENER_FD);
    cleanup(&config);
    let survived_owned = socket_exists(&path);

    assert!(
        survived_inherited,
        "an inherited listener does not own its socket path; cleanup must not \
         unlink it out from under qol-tray's retained fd"
    );
    assert!(
        !survived_owned,
        "a self-bound daemon still cleans up its own socket path"
    );
    remove_socket_file(path);
}

fn send_kill_and_read(path: &Path) -> DaemonResponse {
    let mut client = LocalStream::connect(path).expect("connect to listener");
    write_request(&mut client, "kill", serde_json::Value::Null).expect("send kill request");
    read_response(client)
}

fn await_listener_exit(done_rx: std::sync::mpsc::Receiver<io::Result<()>>) {
    match done_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(result) => result.expect("listener must exit cleanly after a kill request"),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("listener did not terminate after a kill request")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("listener thread panicked before reporting its result")
        }
    }
}

fn assert_kill_contract(
    socket_name: &'static str,
    run_listener: impl FnOnce(DaemonConfig, Sender<io::Result<()>>) + Send + 'static,
) {
    let _lock = daemon_listener_fd_env_lock();
    let config = fallback_config(socket_name);
    let path = socket_path(&config).unwrap();
    remove_socket_file(&path);

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let listener = std::thread::spawn(move || run_listener(config, done_tx));
    wait_for_socket(&path);

    assert!(
        matches!(send_kill_and_read(&path), DaemonResponse::Handled { .. }),
        "a stateful daemon must answer kill with Handled so the replace handshake succeeds"
    );
    await_listener_exit(done_rx);
    listener
        .join()
        .expect("stateful listener thread must not panic");
    assert!(
        socket_exists(&path),
        "the replaced holder must leave the socket path for the replacer to reclaim"
    );
    remove_socket_file(path);
}

#[test]
fn stateful_listener_answers_kill_with_handled_and_terminates() {
    assert_kill_contract(
        temp_socket_name("stateful-kill-action"),
        |config, done_tx| {
            let result =
                run_stateful_listener(&config, (), |_state: &mut (), action: &str| match action {
                    "ping" | "kill" => ReadResult::Handled,
                    _ => ReadResult::Fallback,
                });
            let _ = done_tx.send(result);
        },
    );
}

#[test]
fn stateful_request_listener_answers_kill_with_handled_and_terminates() {
    assert_kill_contract(
        temp_socket_name("stateful-kill-request"),
        |config, done_tx| {
            let result = run_stateful_request_listener(
                &config,
                (),
                |_state: &mut (), request: &DaemonRequest| match request.action.as_str() {
                    "ping" | "kill" => ReadResult::Handled,
                    _ => ReadResult::Fallback,
                },
            );
            let _ = done_tx.send(result);
        },
    );
}

#[test]
fn readiness_gated_listener_answers_not_ready_until_ready() {
    let _lock = daemon_listener_fd_env_lock();
    let socket_name = temp_socket_name("readiness-gate");
    let config = fallback_config(socket_name);
    let listener_config = fallback_config(socket_name);
    let path = socket_path(&config).unwrap();
    remove_socket_file(&path);

    let readiness = ReadinessGate::starting();
    let gate = readiness.clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let listener_thread = std::thread::spawn(move || {
        let result = run_stateful_request_listener_with_readiness(
            &listener_config,
            &gate,
            (),
            |_state: &mut (), request: &DaemonRequest| match request.action.as_str() {
                "ping" | "kill" => ReadResult::Handled,
                _ => ReadResult::Fallback,
            },
        );
        let _ = done_tx.send(result);
    });
    wait_for_socket(&path);

    match send_ping_state(&config).unwrap() {
        PingState::NotReady { phase, detail } => {
            assert_eq!(phase, ReadinessPhase::Starting);
            assert_eq!(detail, None);
        }
        other => panic!("expected not_ready while starting, got {other:?}"),
    }
    assert!(send_ping(&config), "a starting daemon is alive, not dead");

    readiness.set_phase(
        ReadinessPhase::Warming,
        Some("ingesting transcripts 320/900".to_string()),
    );
    match send_ping_state(&config).unwrap() {
        PingState::NotReady { phase, detail } => {
            assert_eq!(phase, ReadinessPhase::Warming);
            assert_eq!(detail.as_deref(), Some("ingesting transcripts 320/900"));
        }
        other => panic!("expected not_ready while warming, got {other:?}"),
    }

    readiness.mark_ready();
    assert!(matches!(
        send_ping_state(&config).unwrap(),
        PingState::Ready
    ));

    assert!(
        matches!(send_kill_and_read(&path), DaemonResponse::Handled { .. }),
        "kill must still reach the handler through an open gate"
    );
    await_listener_exit(done_rx);
    listener_thread.join().unwrap();
    remove_socket_file(path);
}

#[test]
fn theme_override_args_parses_theme_lines_only() {
    let cases = [
        ("ping", None),
        ("themeX", None),
        ("theme", Some((None, None))),
        ("theme bone amber", Some((Some("bone"), Some("amber")))),
        ("theme - amber", Some((None, Some("amber")))),
        ("theme bone -", Some((Some("bone"), None))),
    ];
    for (action, expected) in cases {
        assert_eq!(theme_override_args(action), expected, "action: {action}");
    }
}

#[test]
fn theme_requests_are_handled_even_when_the_parser_declines() {
    let request = DaemonRequest {
        action: "theme bone amber".to_string(),
        input: serde_json::Value::Null,
    };
    let result = parse_with_theme_override::<(), _>(&request, &mut |_| ReadResult::Fallback);
    assert!(matches!(result, ReadResult::Handled));
    let result =
        parse_with_theme_override::<(), _>(&request, &mut |_| ReadResult::Error("no".into()));
    assert!(matches!(result, ReadResult::Handled));
    let result = parse_with_theme_override(&request, &mut |_| ReadResult::Command(7));
    assert!(matches!(result, ReadResult::Command(7)));
}

#[test]
fn inherited_listener_replacement_rejects_old_instance_before_any_parser() {
    let temporary = tempfile::tempdir().unwrap();
    let socket = temporary.path().join("shared.sock");
    let inherited = local_ipc::bind_listener(&socket).unwrap();
    let replacement = inherited.try_clone().unwrap();
    let calls = AtomicUsize::new(0);
    for (listener, actual, expected, action, accepted) in [
        (&inherited, "a".repeat(32), "a".repeat(32), "count", true),
        (&replacement, "b".repeat(32), "a".repeat(32), "count", false),
        (
            &replacement,
            "b".repeat(32),
            "a".repeat(32),
            "theme dark red",
            false,
        ),
        (&replacement, "b".repeat(32), "b".repeat(32), "count", true),
    ] {
        let mut client = LocalStream::connect(&socket).unwrap();
        let request = qol_runtime::protocol::FencedDaemonRequest {
            fence_version: 1,
            instance: expected,
            request: DaemonRequest {
                action: action.into(),
                input: serde_json::Value::Null,
            },
        };
        writeln!(client, "{}", serde_json::to_string(&request).unwrap()).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        local_ipc::authorize_peer(&stream).unwrap();
        let before = calls.load(Ordering::SeqCst);
        let result = read_request_with_boundary(
            &mut stream,
            &DaemonBoundary::with_instance(actual).unwrap(),
            None,
            |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                ReadResult::<()>::Handled
            },
        );
        assert_eq!(matches!(result, ReadResult::Handled), accepted, "{action}");
        assert_eq!(
            calls.load(Ordering::SeqCst) - before,
            usize::from(accepted),
            "{action}"
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn operation_capability_is_advertised_only_by_the_active_ready_boundary() {
    let readiness = ReadinessGate::starting();
    for ready in [false, true] {
        if ready {
            readiness.mark_ready();
        }
        let (mut client, mut server) = LocalStream::pair().unwrap();
        writeln!(client, "{{\"action\":\"ping\"}}").unwrap();
        let result = read_request_with_boundary(
            &mut server,
            &DaemonBoundary::with_instance("a".repeat(32)).unwrap(),
            Some(&readiness),
            |_| -> ReadResult<()> { panic!("readiness must not invoke domain parser") },
        );
        handle_read_result(&mut server, result, |_| unreachable!());
        let line = local_ipc::read_line(&mut BufReader::new(client))
            .unwrap()
            .unwrap();
        let response: DaemonResponse = serde_json::from_str(&line).unwrap();
        assert_eq!(
            matches!(response, DaemonResponse::Handled { data: Some(_) }),
            ready,
            "ready: {ready}"
        );
        assert_eq!(
            matches!(response, DaemonResponse::NotReady { .. }),
            !ready,
            "ready: {ready}"
        );
    }
}
