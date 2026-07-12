use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

const ACK_TIMEOUT_MS: u64 = 80;
const HOST_DEATH_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
const REPLACE_EXISTING_ENV: &str = qol_conventions::ENV_DAEMON_REPLACE_EXISTING;

pub struct DaemonConfig {
    pub socket: SocketSource,
    pub support_replace_existing: bool,
}

pub enum SocketSource {
    EnvRequired,
    Fallback {
        default_socket_name: &'static str,
        use_tmpdir_env: bool,
    },
}

pub enum ReadResult<C> {
    Command(C),
    Handled,
    HandledWithData(serde_json::Value),
    Fallback,
    Error(String),
    Ignore,
}

pub fn socket_path(config: &DaemonConfig) -> Option<PathBuf> {
    if let Ok(path) = std::env::var(qol_conventions::ENV_DAEMON_SOCKET) {
        return Some(PathBuf::from(path));
    }
    match &config.socket {
        SocketSource::EnvRequired => None,
        SocketSource::Fallback {
            default_socket_name,
            use_tmpdir_env,
        } => Some(fallback_socket_path(default_socket_name, *use_tmpdir_env)),
    }
}

fn fallback_socket_path(name: &str, use_tmpdir_env: bool) -> PathBuf {
    if use_tmpdir_env {
        let dir = std::env::var("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));
        dir.join(name)
    } else {
        PathBuf::from("/tmp").join(name)
    }
}

pub fn send_action(config: &DaemonConfig, action: &str, expect_reply: bool) -> bool {
    let Some(path) = socket_path(config) else {
        #[cfg(debug_assertions)]
        eprintln!(
            "[daemon] {} unset and no fallback socket - cannot send action",
            qol_conventions::ENV_DAEMON_SOCKET
        );
        return false;
    };
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    let timeout = std::time::Duration::from_millis(ACK_TIMEOUT_MS);
    let _ = stream.set_write_timeout(Some(timeout));

    let request = DaemonRequest {
        action: action.to_string(),
    };
    let Ok(mut payload) = serde_json::to_string(&request) else {
        return false;
    };
    payload.push('\n');

    if stream.write_all(payload.as_bytes()).is_err() {
        return false;
    }
    if !expect_reply {
        return true;
    }

    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.set_read_timeout(Some(timeout));

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => false,
        Ok(_) => matches!(
            serde_json::from_str::<DaemonResponse>(line.trim()),
            Ok(DaemonResponse::Handled { .. })
        ),
    }
}

pub fn send_kill(config: &DaemonConfig) -> bool {
    send_action(config, "kill", true)
}

pub fn send_ping(config: &DaemonConfig) -> bool {
    send_action(config, "ping", true)
}

pub fn cleanup(config: &DaemonConfig) {
    // An inherited listener's socket path is bound and owned by qol-tray's
    // retained fd; unlinking it here would leave every respawned daemon
    // serving a socket no path resolves to anymore.
    if std::env::var_os(qol_conventions::ENV_DAEMON_LISTENER_FD).is_some() {
        return;
    }
    if let Some(path) = socket_path(config) {
        remove_socket_file(path);
    }
}

pub fn start_listener<C: Send + 'static>(
    config: &DaemonConfig,
    tx: Sender<C>,
    parser: fn(&str) -> ReadResult<C>,
) -> bool {
    let Ok((listener, socket_path)) = bind_listener(config) else {
        return false;
    };

    let host_death_tx = tx.clone();
    qol_runtime::spawn_host_death_watchdog_with(move || {
        if let ReadResult::Command(cmd) = parser("kill") {
            if host_death_tx.send(cmd).is_ok() {
                std::thread::sleep(HOST_DEATH_GRACE);
            }
        }
        std::process::exit(0);
    });

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut s) => {
                    let result = read_and_parse(&mut s, parser);
                    let should_continue = handle_read_result(&mut s, result, |cmd| {
                        if tx.send(cmd).is_ok() {
                            DaemonResponse::Handled { data: None }
                        } else {
                            DaemonResponse::Fallback
                        }
                    });
                    if !should_continue {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        if let Some(socket_path) = socket_path {
            remove_socket_file(&socket_path);
        }
    });

    true
}

pub fn run_stateful_listener<S, F>(
    config: &DaemonConfig,
    mut state: S,
    mut handler: F,
) -> io::Result<()>
where
    F: FnMut(&mut S, &str) -> ReadResult<()>,
{
    let (listener, socket_path) = bind_listener(config)?;

    qol_runtime::spawn_host_death_watchdog();

    for stream in listener.incoming() {
        match stream {
            Ok(mut s) => {
                let result = read_and_parse(&mut s, |action| handler(&mut state, action));
                handle_read_result(&mut s, result, |_| DaemonResponse::Handled { data: None });
            }
            Err(error) => {
                eprintln!("accept error: {error:#}");
            }
        }
    }

    if let Some(socket_path) = socket_path {
        remove_socket_file(socket_path);
    }
    Ok(())
}

fn bind_listener(config: &DaemonConfig) -> io::Result<(UnixListener, Option<PathBuf>)> {
    if let Some(listener) = inherited_listener()? {
        #[cfg(debug_assertions)]
        eprintln!("[daemon] adopting a pre-bound listener fd, skipping bind()");
        return Ok((listener, None));
    }

    // Every daemon-bearing plugin in this repo is spawned with a pre-bound fd
    // today, so this self-bind path (and support_replace_existing below) is
    // not the common case anymore. It stays as the fallback for a plugin
    // binary launched by hand outside qol-tray, and for any pre-bind attempt
    // that failed and gracefully degraded (see daemon_lifecycle::listener in
    // qol-tray).
    let Some(socket_path) = socket_path(config) else {
        #[cfg(debug_assertions)]
        eprintln!(
            "[daemon] {} unset and no fallback socket - not binding",
            qol_conventions::ENV_DAEMON_SOCKET
        );
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("{} is not set", qol_conventions::ENV_DAEMON_SOCKET),
        ));
    };
    let support_replace = config.support_replace_existing;

    #[cfg(debug_assertions)]
    eprintln!("[daemon] binding to {:?}", socket_path);

    let listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == ErrorKind::AddrInUse => {
            if send_ping(config) {
                if !support_replace || !replace_existing_enabled() {
                    #[cfg(debug_assertions)]
                    eprintln!("[daemon] existing instance alive, exiting");
                    return Err(io::Error::new(
                        ErrorKind::AddrInUse,
                        "existing daemon instance is alive",
                    ));
                }
                #[cfg(debug_assertions)]
                eprintln!("[daemon] replacing existing socket owner");
            }
            remove_socket_file(&socket_path);
            UnixListener::bind(&socket_path)?
        }
        Err(error) => return Err(error),
    };

    Ok((listener, Some(socket_path)))
}

fn inherited_listener() -> io::Result<Option<UnixListener>> {
    let Ok(raw) = std::env::var(qol_conventions::ENV_DAEMON_LISTENER_FD) else {
        return Ok(None);
    };
    listener_from_fd_str(&raw).map(Some)
}

fn listener_from_fd_str(raw: &str) -> io::Result<UnixListener> {
    let fd: RawFd = raw.parse().map_err(|_| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "malformed {}: {raw:?}",
                qol_conventions::ENV_DAEMON_LISTENER_FD
            ),
        )
    })?;
    restore_cloexec(fd)?;
    Ok(unsafe { UnixListener::from_raw_fd(fd) })
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
    std::env::var(env_name).ok()?.parse().ok()
}

fn read_and_parse<C, F>(stream: &mut UnixStream, mut parser: F) -> ReadResult<C>
where
    F: FnMut(&str) -> ReadResult<C>,
{
    let timeout = std::time::Duration::from_millis(ACK_TIMEOUT_MS);
    let _ = stream.set_read_timeout(Some(timeout));

    let mut reader = BufReader::new(&*stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => {
            #[cfg(debug_assertions)]
            eprintln!("[daemon] read_line EOF (0 bytes)");
            return ReadResult::Ignore;
        }
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("[daemon] read_line error: {:?}", e);
            #[cfg(not(debug_assertions))]
            let _ = e;
            return ReadResult::Ignore;
        }
        Ok(_) => {}
    }

    let trimmed = line.trim();
    #[cfg(debug_assertions)]
    eprintln!("[daemon] read line: {:?}", trimmed);

    if trimmed.is_empty() {
        return ReadResult::Ignore;
    }

    if let Ok(request) = serde_json::from_str::<DaemonRequest>(trimmed) {
        #[cfg(debug_assertions)]
        eprintln!("[daemon] parsed DaemonRequest action: {:?}", request.action);
        return parser(&request.action);
    }

    let cmd = match trimmed.strip_prefix("action:") {
        Some(a) => a,
        None => trimmed,
    };
    parser(cmd)
}

fn handle_read_result<C, F>(
    stream: &mut UnixStream,
    result: ReadResult<C>,
    command_response: F,
) -> bool
where
    F: FnOnce(C) -> DaemonResponse,
{
    let response = match result {
        ReadResult::Command(cmd) => {
            let response = command_response(cmd);
            let should_continue = !matches!(response, DaemonResponse::Fallback);
            write_response(stream, &response);
            return should_continue;
        }
        ReadResult::Handled => DaemonResponse::Handled { data: None },
        ReadResult::HandledWithData(data) => DaemonResponse::Handled { data: Some(data) },
        ReadResult::Fallback => DaemonResponse::Fallback,
        ReadResult::Error(message) => DaemonResponse::Error { message },
        ReadResult::Ignore => return true,
    };
    write_response(stream, &response);
    true
}

fn write_response(stream: &mut UnixStream, response: &DaemonResponse) {
    if let Ok(json) = serde_json::to_string(response) {
        let _ = stream.write_all(json.as_bytes());
        let _ = stream.write_all(b"\n");
    }
}

fn replace_existing_enabled() -> bool {
    std::env::var(REPLACE_EXISTING_ENV).ok().is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn remove_socket_file(path: impl AsRef<std::path::Path>) {
    let path = path.as_ref();
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_socket() {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn listener_from_fd_str_rejects_malformed_value() {
        let error = listener_from_fd_str("not-a-number").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
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
        let _lock = daemon_listener_fd_env_lock();
        let env_name = format!("{}_TESTPORT", qol_conventions::ENV_DAEMON_PORT_FD);
        std::env::set_var(&env_name, "42");

        let fd = inherited_port_fd("testport");

        std::env::remove_var(&env_name);
        assert_eq!(fd, Some(42));
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
        let _lock = daemon_listener_fd_env_lock();
        std::env::set_var(qol_conventions::ENV_DAEMON_PORT_FD, "7");

        let fd = inherited_primary_port_fd();

        std::env::remove_var(qol_conventions::ENV_DAEMON_PORT_FD);
        assert_eq!(fd, Some(7));
    }

    #[test]
    fn inherited_primary_port_fd_returns_none_when_env_var_absent() {
        let _lock = daemon_listener_fd_env_lock();
        std::env::remove_var(qol_conventions::ENV_DAEMON_PORT_FD);

        assert_eq!(inherited_primary_port_fd(), None);
    }
}
