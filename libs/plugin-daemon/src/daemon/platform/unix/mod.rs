//! Unix-domain transport adapter for resident plugin daemons: fd handoff from
//! qol-tray and the socket-file conventions of Unix hosts.

use std::fs;
use std::io::{self, ErrorKind};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};

use qol_runtime::local_ipc::LocalListener;

mod platform;

use platform::is_listening_socket;

pub(in crate::daemon) fn fallback_socket_dir(use_tmpdir_env: bool) -> PathBuf {
    if use_tmpdir_env {
        std::env::var("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
    } else {
        PathBuf::from("/tmp")
    }
}

pub(in crate::daemon) fn remove_socket_file(path: impl AsRef<Path>) {
    let path = path.as_ref();
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_socket() {
        let _ = fs::remove_file(path);
    }
}

/// A daemon only ever receives this variable from the qol-tray that pre-bound
/// the fd for it. Anything else (an unrelated ancestor's leftover, a stale
/// number) names an fd that is not a listening socket here, so it is ignored
/// and the daemon binds its own socket path instead of dying on it.
pub(in crate::daemon) fn inherited_listener() -> Option<LocalListener> {
    let raw = std::env::var(qol_conventions::ENV_DAEMON_LISTENER_FD).ok()?;
    match listener_from_fd_str(&raw) {
        Ok(listener) => Some(listener),
        Err(error) => {
            log::warn!(
                "ignoring {}={raw}: {error}; binding the socket path instead",
                qol_conventions::ENV_DAEMON_LISTENER_FD
            );
            std::env::remove_var(qol_conventions::ENV_DAEMON_LISTENER_FD);
            None
        }
    }
}

fn listener_from_fd_str(raw: &str) -> io::Result<LocalListener> {
    let fd: RawFd = raw.parse().map_err(|_| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "malformed {}: {raw:?}",
                qol_conventions::ENV_DAEMON_LISTENER_FD
            ),
        )
    })?;
    if !is_listening_socket(fd) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("fd {fd} is not a listening socket in this process"),
        ));
    }
    restore_cloexec(fd)?;
    Ok(unsafe { LocalListener::from_raw_fd(fd) })
}

/// A pre-bound port fd is adoptable when it is a socket of a kind that can
/// already be serving: a stream socket must be listening, but a datagram
/// socket never is - `SO_ACCEPTCONN` is 0 for every UDP socket, so judging
/// one by that alone rejects a perfectly good handoff and forces the daemon
/// to rebind a port qol-tray still holds.
fn is_adoptable_socket(fd: RawFd) -> bool {
    match socket_opt(fd, libc::SO_TYPE) {
        Some(libc::SOCK_DGRAM) => true,
        Some(libc::SOCK_STREAM) => is_listening_socket(fd),
        _ => false,
    }
}

fn socket_opt(fd: RawFd, option: libc::c_int) -> Option<libc::c_int> {
    let mut value: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            option,
            (&mut value as *mut libc::c_int).cast::<libc::c_void>(),
            &mut len,
        )
    };
    (rc == 0).then_some(value)
}

/// qol-tray clears CLOEXEC on a pre-bound fd so it survives the exec into
/// this daemon's binary. That cleared flag would otherwise keep propagating
/// into every further child this daemon spawns (e.g. a launched app, a
/// terminal, ffmpeg), leaking the listener past this process. Callers must
/// invoke this immediately after adopting any fd handed off via the
/// `QOL_TRAY_DAEMON_*_FD` env vars, before wrapping it in a socket type.
pub fn restore_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let set = unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) };
    if set < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Looks up the fd qol-tray pre-bound for a named extra port (declared via
/// `[[daemon.extra_ports]]` in plugin.toml), e.g. `inherited_port_fd("discovery")`
/// for a port named `discovery`. Returns `None` if qol-tray didn't pre-bind
/// this port - the caller should fall back to binding it directly.
pub fn inherited_port_fd(name: &str) -> Option<RawFd> {
    let env_name = format!(
        "{}_{}",
        qol_conventions::ENV_DAEMON_PORT_FD,
        name.to_uppercase()
    );
    fd_from_env(&env_name)
}

/// Looks up the fd qol-tray pre-bound for the daemon's single top-level
/// `port` (declared as `port = ...` directly under `[daemon]` in
/// plugin.toml, as opposed to a named `[[daemon.extra_ports]]` entry).
/// Returns `None` if qol-tray didn't pre-bind it - the caller should fall
/// back to binding it directly.
pub fn inherited_primary_port_fd() -> Option<RawFd> {
    fd_from_env(qol_conventions::ENV_DAEMON_PORT_FD)
}

fn fd_from_env(env_name: &str) -> Option<RawFd> {
    let raw = std::env::var(env_name).ok()?;
    let fd: RawFd = raw.parse().ok()?;
    if !is_adoptable_socket(fd) {
        log::warn!("ignoring {env_name}={raw}: fd {fd} is not a pre-bound socket in this process");
        std::env::remove_var(env_name);
        return None;
    }
    Some(fd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::*;
    use std::io::BufRead;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_SOCKET_ID: AtomicUsize = AtomicUsize::new(0);

    fn stream_with_line(line: &str) -> UnixStream {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer.write_all(line.as_bytes()).unwrap();
        writer.write_all(b"\n").unwrap();
        reader
    }

    fn response_for_result<C>(
        result: ReadResult<C>,
        command_response: impl FnOnce(C) -> DaemonResponse,
    ) -> (bool, DaemonResponse) {
        let (mut server, client) = UnixStream::pair().unwrap();
        let should_continue = handle_read_result(&mut server, result, command_response);
        drop(server);

        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let response = serde_json::from_str(line.trim()).unwrap();
        (should_continue, response)
    }

    fn temp_socket_name(tag: &str) -> &'static str {
        let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
        Box::leak(
            format!("qol-plugin-daemon-{tag}-{}-{id}.sock", std::process::id()).into_boxed_str(),
        )
    }

    fn fallback_config(socket_name: &'static str) -> DaemonConfig {
        DaemonConfig {
            socket: SocketSource::Fallback {
                default_socket_name: socket_name,
                use_tmpdir_env: false,
            },
            support_replace_existing: false,
        }
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
        assert_eq!(path, Some(PathBuf::from("/tmp/panel.sock")));
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
        let mut stream =
            stream_with_line(r#"{"action":"pair_device","input":{"address":"AA:BB"}}"#);
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
                ReadResult::Command(action) => assert_eq!(action, expected),
                _ => panic!("expected command"),
            }
        }
    }

    #[test]
    fn ignores_empty_requests() {
        let mut stream = stream_with_line("");
        let result = read_and_parse(&mut stream, |_| ReadResult::<()>::Handled);

        match result {
            ReadResult::Ignore => {}
            _ => panic!("expected ignore"),
        }
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
        let path = PathBuf::from("/tmp").join(socket_name);
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
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
            &fallback_config(socket_name),
            "devices",
            serde_json::json!({ "paired": true }),
            std::time::Duration::from_secs(1),
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
        match response {
            DaemonResponse::Fallback => {}
            _ => panic!("expected fallback response"),
        }
    }

    #[test]
    fn command_send_fallback_stops_threaded_listener() {
        let (should_continue, response) =
            response_for_result(ReadResult::Command(()), |_| DaemonResponse::Fallback);

        assert!(!should_continue);
        match response {
            DaemonResponse::Fallback => {}
            _ => panic!("expected fallback response"),
        }
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

            match result {
                ReadResult::Handled => {}
                _ => panic!("expected handled"),
            }
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
        let path = PathBuf::from("/tmp").join(socket_name);
        let _ = fs::remove_file(&path);

        {
            let _stale_listener = UnixListener::bind(&path).unwrap();
        }

        assert!(fs::symlink_metadata(&path).is_ok());

        let config = fallback_config(socket_name);
        let (_listener, bound_path) = bind_listener(&config).unwrap();

        assert_eq!(bound_path, Some(path.clone()));
        remove_socket_file(path);
    }

    #[test]
    fn bind_listener_replaces_a_running_daemon_when_requested() {
        let _lock = daemon_listener_fd_env_lock();
        let socket_name = temp_socket_name("replace-running");
        let path = std::env::temp_dir().join(socket_name);
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
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
        let path = PathBuf::from("/tmp").join(socket_name);
        let _ = fs::remove_file(&path);
        let _listener = UnixListener::bind(&path).unwrap();
        std::env::set_var(qol_conventions::ENV_DAEMON_LISTENER_FD, "7");

        cleanup(&fallback_config(socket_name));
        let survived_inherited = path.exists();

        std::env::remove_var(qol_conventions::ENV_DAEMON_LISTENER_FD);
        cleanup(&fallback_config(socket_name));
        let survived_owned = path.exists();

        assert!(
            survived_inherited,
            "an inherited listener does not own its socket path; cleanup must not \
             unlink it out from under qol-tray's retained fd"
        );
        assert!(
            !survived_owned,
            "a self-bound daemon still cleans up its own socket path"
        );
        let _ = fs::remove_file(&path);
    }

    // `bind_listener` reads QOL_TRAY_DAEMON_LISTENER_FD from the process
    // environment, which is shared across every test thread in this binary.
    // Any test that sets it must hold this lock for the duration, so it can't
    // leak into a concurrently-running test that also calls bind_listener.
    fn daemon_listener_fd_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn assert_kill_contract(
        socket_name: &'static str,
        run_listener: impl FnOnce(DaemonConfig, Sender<io::Result<()>>) + Send + 'static,
    ) {
        let _lock = daemon_listener_fd_env_lock();
        let config = fallback_config(socket_name);
        let path = socket_path(&config).unwrap();
        let _ = fs::remove_file(&path);

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let listener = std::thread::spawn(move || run_listener(config, done_tx));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(path.exists(), "stateful listener must bind its socket");

        let mut client = UnixStream::connect(&path).expect("connect to stateful listener");
        write_request(&mut client, "kill", serde_json::Value::Null).expect("send kill request");

        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read kill response");
        let response: DaemonResponse =
            serde_json::from_str(line.trim()).expect("parse kill response");
        assert!(
            matches!(response, DaemonResponse::Handled { .. }),
            "a stateful daemon must answer kill with Handled so the replace handshake succeeds"
        );

        match done_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(result) => result.expect("stateful listener must exit cleanly after a kill request"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("stateful listener did not terminate after a kill request")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("stateful listener thread panicked before reporting its result")
            }
        }
        listener
            .join()
            .expect("stateful listener thread must not panic");
        assert!(
            path.exists(),
            "the replaced holder must leave the socket path for the replacer to reclaim"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn stateful_listener_answers_kill_with_handled_and_terminates() {
        assert_kill_contract(
            temp_socket_name("stateful-kill-action"),
            |config, done_tx| {
                let result =
                    run_stateful_listener(
                        &config,
                        (),
                        |_state: &mut (), action: &str| match action {
                            "ping" | "kill" => ReadResult::Handled,
                            _ => ReadResult::Fallback,
                        },
                    );
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
        let _ = fs::remove_file(&path);

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

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(path.exists(), "gated listener must bind its socket");

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

        let mut client = UnixStream::connect(&path).unwrap();
        write_request(&mut client, "kill", serde_json::Value::Null).unwrap();
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let response: DaemonResponse = serde_json::from_str(line.trim()).unwrap();
        assert!(
            matches!(response, DaemonResponse::Handled { .. }),
            "kill must still reach the handler through an open gate"
        );

        match done_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(result) => result.expect("gated listener must exit cleanly after a kill request"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("gated listener did not terminate after a kill request")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("gated listener thread panicked before reporting its result")
            }
        }
        listener_thread.join().unwrap();
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn bind_listener_uses_inherited_fd_when_env_var_present() {
        use std::os::fd::IntoRawFd;

        let _lock = daemon_listener_fd_env_lock();
        let socket_name = temp_socket_name("inherited");
        let path = PathBuf::from("/tmp").join(socket_name);
        let _ = fs::remove_file(&path);
        let pre_bound = UnixListener::bind(&path).unwrap();
        let fd = pre_bound.into_raw_fd();
        std::env::set_var(qol_conventions::ENV_DAEMON_LISTENER_FD, fd.to_string());

        let config = fallback_config(temp_socket_name("unused-when-inherited"));
        let result = bind_listener(&config);

        std::env::remove_var(qol_conventions::ENV_DAEMON_LISTENER_FD);

        let (_listener, bound_path) = result.unwrap();
        assert_eq!(
            bound_path, None,
            "an inherited listener does not own its socket path and must not unlink it"
        );
        remove_socket_file(path);
    }

    /// The env var is only meaningful for the daemon qol-tray bound the fd
    /// for. Leaked into any other process (a terminal opened from a plugin,
    /// then `qol dev`, then every daemon) it named an fd that was closed or
    /// unrelated, and daemons died on it instead of binding their own socket.
    #[test]
    fn bind_listener_ignores_an_inherited_fd_that_is_not_a_listening_socket() {
        use std::os::fd::AsRawFd;

        let _lock = daemon_listener_fd_env_lock();
        let not_a_socket = fs::File::open("/dev/null").unwrap();
        let socket_name = temp_socket_name("fallback-after-bogus-fd");
        let config = fallback_config(socket_name);
        let expected = PathBuf::from("/tmp").join(socket_name);
        let _ = fs::remove_file(&expected);

        for bogus in [not_a_socket.as_raw_fd().to_string(), "999999".to_owned()] {
            std::env::set_var(qol_conventions::ENV_DAEMON_LISTENER_FD, &bogus);
            let (_listener, bound_path) = bind_listener(&config).unwrap();
            assert_eq!(bound_path.as_deref(), Some(expected.as_path()));
            assert!(
                std::env::var_os(qol_conventions::ENV_DAEMON_LISTENER_FD).is_none(),
                "a rejected handoff must not linger and suppress socket cleanup"
            );
            remove_socket_file(expected.clone());
        }
    }

    #[test]
    fn listener_from_fd_str_rejects_malformed_value() {
        let error = listener_from_fd_str("not-a-number").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn listening_socket_validation_preserves_pending_connections_and_rejects_other_fds() {
        use std::os::fd::{AsRawFd, OwnedFd};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let _pending = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (connected, _peer) = UnixStream::pair().unwrap();
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let file = fs::File::open("/dev/null").unwrap();
        let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        assert!(raw >= 0);
        let unbound = unsafe { OwnedFd::from_raw_fd(raw) };

        for (label, fd, expected) in [
            ("listener", listener.as_raw_fd(), true),
            ("connected stream", connected.as_raw_fd(), false),
            ("unbound stream", unbound.as_raw_fd(), false),
            ("datagram", udp.as_raw_fd(), false),
            ("file", file.as_raw_fd(), false),
            ("invalid", -1, false),
        ] {
            assert_eq!(is_listening_socket(fd), expected, "{label}");
            if fd >= 0 {
                assert!(unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0, "{label}");
            }
        }

        listener.set_nonblocking(true).unwrap();
        assert!(
            listener.accept().is_ok(),
            "validation must not accept clients"
        );
        assert!(is_adoptable_socket(udp.as_raw_fd()));
        assert!(!is_adoptable_socket(unbound.as_raw_fd()));
        assert!(!is_adoptable_socket(connected.as_raw_fd()));
    }

    fn fd_has_cloexec(fd: RawFd) -> bool {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert!(flags >= 0, "fd {fd} must be open");
        flags & libc::FD_CLOEXEC != 0
    }

    #[test]
    fn restore_cloexec_sets_the_close_on_exec_flag() {
        use std::os::fd::IntoRawFd;

        let path = PathBuf::from(format!(
            "/tmp/qol-plugin-daemon-test-restore-cloexec-{}.sock",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let fd = listener.into_raw_fd();
        unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };
        assert!(
            !fd_has_cloexec(fd),
            "test setup must start with cloexec cleared"
        );

        restore_cloexec(fd).unwrap();

        assert!(fd_has_cloexec(fd), "restore_cloexec must set FD_CLOEXEC");
        unsafe { libc::close(fd) };
        let _ = fs::remove_file(&path);
    }

    // Regression test for the leak this whole redesign was meant to close:
    // qol-tray clears CLOEXEC so the fd survives its own exec into the
    // daemon. Once adopted here, that cleared flag must not keep propagating
    // into every further child the daemon spawns.
    #[test]
    fn listener_from_fd_str_restores_cloexec_on_the_adopted_fd() {
        use std::os::fd::IntoRawFd;

        let path = PathBuf::from(format!(
            "/tmp/qol-plugin-daemon-test-adopt-cloexec-{}.sock",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let pre_bound = UnixListener::bind(&path).unwrap();
        let fd = pre_bound.into_raw_fd();
        unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };

        let listener = listener_from_fd_str(&fd.to_string()).unwrap();

        assert!(
            fd_has_cloexec(fd),
            "adopting an inherited fd must re-arm cloexec so it can't leak \
             into a further child this daemon spawns"
        );
        drop(listener);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn inherited_port_fd_reads_the_named_env_var() {
        use std::os::fd::AsRawFd;

        let _lock = daemon_listener_fd_env_lock();
        let env_name = format!("{}_TESTPORT", qol_conventions::ENV_DAEMON_PORT_FD);
        let pre_bound = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std::env::set_var(&env_name, pre_bound.as_raw_fd().to_string());

        let fd = inherited_port_fd("testport");

        std::env::remove_var(&env_name);
        assert_eq!(fd, Some(pre_bound.as_raw_fd()));
    }

    // Regression test: `SO_ACCEPTCONN` is 0 for every UDP socket, so judging
    // a pre-bound port fd by "is it listening" rejected every datagram
    // handoff. qol-pointz then rebound its discovery port, hit
    // EADDRINUSE against the qol-tray-held socket, and crash-looped.
    #[test]
    fn inherited_port_fd_adopts_a_pre_bound_udp_socket() {
        use std::os::fd::AsRawFd;

        let _lock = daemon_listener_fd_env_lock();
        let env_name = format!("{}_TESTUDPPORT", qol_conventions::ENV_DAEMON_PORT_FD);
        let pre_bound = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        std::env::set_var(&env_name, pre_bound.as_raw_fd().to_string());

        let fd = inherited_port_fd("testudpport");

        std::env::remove_var(&env_name);
        assert_eq!(
            fd,
            Some(pre_bound.as_raw_fd()),
            "a datagram socket is never listening, but it is still a valid handoff"
        );
    }

    #[test]
    fn inherited_port_fd_ignores_a_number_that_is_not_a_listening_socket() {
        let _lock = daemon_listener_fd_env_lock();
        let env_name = format!("{}_BOGUSPORT", qol_conventions::ENV_DAEMON_PORT_FD);
        std::env::set_var(&env_name, "999999");

        let fd = inherited_port_fd("bogusport");

        assert_eq!(fd, None, "a leaked or stale fd number must not be adopted");
        assert!(
            std::env::var_os(&env_name).is_none(),
            "a rejected handoff variable is dropped so nothing downstream trusts it"
        );
    }

    #[test]
    fn inherited_port_fd_returns_none_when_env_var_absent() {
        let _lock = daemon_listener_fd_env_lock();
        let env_name = format!("{}_ABSENTPORT", qol_conventions::ENV_DAEMON_PORT_FD);
        std::env::remove_var(&env_name);

        assert_eq!(inherited_port_fd("absentport"), None);
    }

    #[test]
    fn inherited_port_fd_returns_none_for_malformed_value() {
        let _lock = daemon_listener_fd_env_lock();
        let env_name = format!("{}_MALFORMEDPORT", qol_conventions::ENV_DAEMON_PORT_FD);
        std::env::set_var(&env_name, "not-a-number");

        let fd = inherited_port_fd("malformedport");

        std::env::remove_var(&env_name);
        assert_eq!(
            fd, None,
            "a malformed port fd falls back to direct binding rather than propagating an error"
        );
    }

    #[test]
    fn inherited_primary_port_fd_reads_the_unsuffixed_env_var() {
        use std::os::fd::AsRawFd;

        let _lock = daemon_listener_fd_env_lock();
        let pre_bound = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std::env::set_var(
            qol_conventions::ENV_DAEMON_PORT_FD,
            pre_bound.as_raw_fd().to_string(),
        );

        let fd = inherited_primary_port_fd();

        std::env::remove_var(qol_conventions::ENV_DAEMON_PORT_FD);
        assert_eq!(fd, Some(pre_bound.as_raw_fd()));
    }

    #[test]
    fn inherited_primary_port_fd_returns_none_when_env_var_absent() {
        let _lock = daemon_listener_fd_env_lock();
        std::env::remove_var(qol_conventions::ENV_DAEMON_PORT_FD);

        assert_eq!(inherited_primary_port_fd(), None);
    }

    #[test]
    fn theme_override_args_parses_theme_lines_only() {
        assert_eq!(theme_override_args("ping"), None);
        assert_eq!(theme_override_args("themeX"), None);
        assert_eq!(theme_override_args("theme"), Some((None, None)));
        assert_eq!(
            theme_override_args("theme bone amber"),
            Some((Some("bone"), Some("amber")))
        );
        assert_eq!(
            theme_override_args("theme - amber"),
            Some((None, Some("amber")))
        );
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
}

#[cfg(test)]
mod operation_fence_tests {
    use crate::daemon::*;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
            let mut client = UnixStream::connect(&socket).unwrap();
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
            let (mut client, mut server) = UnixStream::pair().unwrap();
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
                ready
            );
            assert_eq!(matches!(response, DaemonResponse::NotReady { .. }), !ready);
        }
    }
}
