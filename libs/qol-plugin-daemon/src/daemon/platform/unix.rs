//! Unix-domain transport adapter for resident plugin daemons.

use std::fs;
use std::io::{self, BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use qol_runtime::local_ipc;
use qol_runtime::protocol::{DaemonRequest, DaemonResponse, ReadinessPhase};

const ACK_TIMEOUT_MS: u64 = 80;
const HOST_DEATH_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
const REPLACE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
const REPLACE_POLL: std::time::Duration = std::time::Duration::from_millis(20);

pub struct DaemonConfig {
    pub socket: SocketSource,
    pub support_replace_existing: bool,
}

pub enum SocketSource {
    EnvRequired,
    Path(PathBuf),
    Fallback {
        default_socket_name: &'static str,
        use_tmpdir_env: bool,
    },
    Fixed {
        socket_name: &'static str,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PingState {
    Ready,
    NotReady {
        phase: ReadinessPhase,
        detail: Option<String>,
    },
}

type NotReadyState = Option<(ReadinessPhase, Option<String>)>;

#[derive(Clone)]
pub struct ReadinessGate {
    not_ready: Arc<Mutex<NotReadyState>>,
}

impl ReadinessGate {
    pub fn starting() -> Self {
        Self {
            not_ready: Arc::new(Mutex::new(Some((ReadinessPhase::Starting, None)))),
        }
    }

    pub fn set_phase(&self, phase: ReadinessPhase, detail: Option<String>) {
        let mut not_ready = self
            .not_ready
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if not_ready.is_some() {
            *not_ready = Some((phase, detail));
        }
    }

    pub fn mark_ready(&self) {
        let mut not_ready = self
            .not_ready
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *not_ready = None;
    }

    fn snapshot(&self) -> NotReadyState {
        self.not_ready
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

pub fn socket_path(config: &DaemonConfig) -> Option<PathBuf> {
    match &config.socket {
        SocketSource::EnvRequired => std::env::var(qol_conventions::ENV_DAEMON_SOCKET)
            .ok()
            .map(PathBuf::from),
        SocketSource::Path(path) => Some(path.clone()),
        SocketSource::Fallback {
            default_socket_name,
            use_tmpdir_env,
        } => Some(
            std::env::var(qol_conventions::ENV_DAEMON_SOCKET)
                .map(PathBuf::from)
                .unwrap_or_else(|_| fallback_socket_path(default_socket_name, *use_tmpdir_env)),
        ),
        SocketSource::Fixed {
            socket_name,
            use_tmpdir_env,
        } => Some(fallback_socket_path(socket_name, *use_tmpdir_env)),
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
    if !expect_reply {
        return send_request_without_reply(config, action, serde_json::Value::Null);
    }
    matches!(
        send_request(
            config,
            action,
            serde_json::Value::Null,
            std::time::Duration::from_millis(ACK_TIMEOUT_MS),
        ),
        Ok(DaemonResponse::Handled { .. })
    )
}

pub fn send_request(
    config: &DaemonConfig,
    action: &str,
    input: serde_json::Value,
    timeout: std::time::Duration,
) -> io::Result<DaemonResponse> {
    let Some(path) = socket_path(config) else {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("{} is unset", qol_conventions::ENV_DAEMON_SOCKET),
        ));
    };
    let mut stream = UnixStream::connect(path)?;
    local_ipc::authorize_peer(&stream)?;
    stream.set_write_timeout(Some(timeout))?;
    stream.set_read_timeout(Some(timeout))?;
    write_request(&mut stream, action, input)?;
    stream.shutdown(Shutdown::Write)?;

    let mut reader = BufReader::new(stream);
    let Some(line) = local_ipc::read_line(&mut reader)? else {
        return Err(io::Error::new(
            ErrorKind::UnexpectedEof,
            "daemon closed without a response",
        ));
    };
    serde_json::from_str::<DaemonResponse>(line.trim())
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
}

fn send_request_without_reply(
    config: &DaemonConfig,
    action: &str,
    input: serde_json::Value,
) -> bool {
    let Some(path) = socket_path(config) else {
        return false;
    };
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    if local_ipc::authorize_peer(&stream).is_err() {
        return false;
    }
    let timeout = std::time::Duration::from_millis(ACK_TIMEOUT_MS);
    let _ = stream.set_write_timeout(Some(timeout));
    write_request(&mut stream, action, input).is_ok()
}

fn write_request(
    stream: &mut UnixStream,
    action: &str,
    input: serde_json::Value,
) -> io::Result<()> {
    let request = DaemonRequest {
        action: action.to_string(),
        input,
    };
    let mut payload = serde_json::to_string(&request)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    payload.push('\n');
    stream.write_all(payload.as_bytes())
}

pub fn send_kill(config: &DaemonConfig) -> bool {
    send_action(config, "kill", true)
}

pub fn send_ping(config: &DaemonConfig) -> bool {
    matches!(
        send_ping_state(config),
        Ok(PingState::Ready) | Ok(PingState::NotReady { .. })
    )
}

pub fn send_ping_state(config: &DaemonConfig) -> io::Result<PingState> {
    match send_request(
        config,
        "ping",
        serde_json::Value::Null,
        std::time::Duration::from_millis(ACK_TIMEOUT_MS),
    ) {
        Ok(DaemonResponse::Handled { .. }) => Ok(PingState::Ready),
        Ok(DaemonResponse::NotReady { phase, detail }) => Ok(PingState::NotReady { phase, detail }),
        Ok(_) => Err(io::Error::new(
            ErrorKind::InvalidData,
            "unexpected response to ping",
        )),
        Err(error) => Err(error),
    }
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
    start_listener_with(config, tx, move |request| parser(&request.action))
}

pub fn start_request_listener<C: Send + 'static>(
    config: &DaemonConfig,
    tx: Sender<C>,
    parser: fn(&DaemonRequest) -> ReadResult<C>,
) -> bool {
    start_listener_with(config, tx, parser)
}

fn start_listener_with<C, F>(config: &DaemonConfig, tx: Sender<C>, parser: F) -> bool
where
    C: Send + 'static,
    F: Fn(&DaemonRequest) -> ReadResult<C> + Copy + Send + 'static,
{
    let Ok((listener, socket_path)) = bind_listener(config) else {
        return false;
    };

    let host_death_tx = tx.clone();
    qol_runtime::spawn_host_death_watchdog_with(move || {
        if dispatch_shutdown_command(&host_death_tx, parser) {
            std::thread::sleep(HOST_DEATH_GRACE);
        }
        std::process::exit(0);
    });

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut s) => {
                    if local_ipc::authorize_peer(&s).is_err() {
                        continue;
                    }
                    let result = read_request_and_parse(&mut s, parser);
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
        if !dispatch_shutdown_command(&tx, parser) {
            std::process::exit(0);
        }
    });

    true
}

fn dispatch_shutdown_command<C, F>(tx: &Sender<C>, parser: F) -> bool
where
    F: Fn(&DaemonRequest) -> ReadResult<C>,
{
    let request = DaemonRequest {
        action: "kill".into(),
        input: serde_json::Value::Null,
    };
    let ReadResult::Command(command) = parser(&request) else {
        return false;
    };
    tx.send(command).is_ok()
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

    let mut kill_requested = false;
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                if local_ipc::authorize_peer(&stream).is_err() {
                    continue;
                }
                let result = read_and_parse(&mut stream, |action| {
                    if action == "kill" {
                        kill_requested = true;
                    }
                    handler(&mut state, action)
                });
                handle_read_result(&mut stream, result, |_| DaemonResponse::Handled {
                    data: None,
                });
                if kill_requested {
                    break;
                }
            }
            Err(error) => {
                eprintln!("accept error: {error:#}");
            }
        }
    }

    if let Some(socket_path) = socket_path {
        if !kill_requested {
            remove_socket_file(socket_path);
        }
    }
    Ok(())
}

pub fn run_stateful_request_listener<S, F>(
    config: &DaemonConfig,
    state: S,
    handler: F,
) -> io::Result<()>
where
    F: FnMut(&mut S, &DaemonRequest) -> ReadResult<()>,
{
    run_stateful_request_listener_inner(config, None, state, handler)
}

pub fn run_stateful_request_listener_with_readiness<S, F>(
    config: &DaemonConfig,
    readiness: &ReadinessGate,
    state: S,
    handler: F,
) -> io::Result<()>
where
    F: FnMut(&mut S, &DaemonRequest) -> ReadResult<()>,
{
    run_stateful_request_listener_inner(config, Some(readiness), state, handler)
}

fn run_stateful_request_listener_inner<S, F>(
    config: &DaemonConfig,
    readiness: Option<&ReadinessGate>,
    mut state: S,
    mut handler: F,
) -> io::Result<()>
where
    F: FnMut(&mut S, &DaemonRequest) -> ReadResult<()>,
{
    let (listener, socket_path) = bind_listener(config)?;

    qol_runtime::spawn_host_death_watchdog();

    let mut kill_requested = false;
    for stream in listener.incoming() {
        match stream {
            Ok(mut s) => {
                if local_ipc::authorize_peer(&s).is_err() {
                    continue;
                }
                let mut not_ready: NotReadyState = None;
                let result = read_request_and_parse(&mut s, |request| {
                    if request.action == "kill" {
                        kill_requested = true;
                        return handler(&mut state, request);
                    }
                    if let Some(state) = readiness.and_then(ReadinessGate::snapshot) {
                        not_ready = Some(state);
                        return ReadResult::Ignore;
                    }
                    handler(&mut state, request)
                });
                if let Some((phase, detail)) = not_ready {
                    write_response(&mut s, &DaemonResponse::NotReady { phase, detail });
                } else {
                    handle_read_result(&mut s, result, |_| DaemonResponse::Handled { data: None });
                }
                if kill_requested {
                    break;
                }
            }
            Err(error) => {
                eprintln!("accept error: {error:#}");
            }
        }
    }

    if let Some(socket_path) = socket_path {
        if !kill_requested {
            remove_socket_file(socket_path);
        }
    }
    Ok(())
}

fn bind_listener(config: &DaemonConfig) -> io::Result<(UnixListener, Option<PathBuf>)> {
    if let Some(listener) = inherited_listener() {
        #[cfg(debug_assertions)]
        eprintln!("[daemon] adopting a pre-bound listener fd, skipping bind()");
        return Ok((listener, None));
    }

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
    #[cfg(debug_assertions)]
    eprintln!("[daemon] binding to {:?}", socket_path);

    let listener = match local_ipc::bind_listener(&socket_path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == ErrorKind::AddrInUse => {
            if replace_existing(config)? {
                remove_socket_file(&socket_path);
                return local_ipc::bind_listener(&socket_path)
                    .map(|listener| (listener, Some(socket_path)));
            }
            if send_ping(config) {
                #[cfg(debug_assertions)]
                eprintln!("[daemon] existing instance alive, exiting");
                return Err(io::Error::new(
                    ErrorKind::AddrInUse,
                    "existing daemon instance is alive",
                ));
            }
            remove_socket_file(&socket_path);
            local_ipc::bind_listener(&socket_path)?
        }
        Err(error) => return Err(error),
    };

    Ok((listener, Some(socket_path)))
}

fn replace_existing(config: &DaemonConfig) -> io::Result<bool> {
    if !config.support_replace_existing
        || std::env::var_os(qol_conventions::ENV_DAEMON_REPLACE_EXISTING).is_none()
    {
        return Ok(false);
    }
    if !send_kill(config) {
        return Ok(false);
    }

    let deadline = std::time::Instant::now() + REPLACE_WAIT;
    while std::time::Instant::now() < deadline {
        if !send_ping(config) {
            return Ok(true);
        }
        std::thread::sleep(REPLACE_POLL);
    }

    Err(io::Error::new(
        ErrorKind::AddrInUse,
        "existing daemon did not exit after replacement request",
    ))
}

/// A daemon only ever receives this variable from the qol-tray that pre-bound
/// the fd for it. Anything else (an unrelated ancestor's leftover, a stale
/// number) names an fd that is not a listening socket here, so it is ignored
/// and the daemon binds its own socket path instead of dying on it.
fn inherited_listener() -> Option<UnixListener> {
    let raw = std::env::var(qol_conventions::ENV_DAEMON_LISTENER_FD).ok()?;
    match listener_from_fd_str(&raw) {
        Ok(listener) => Some(listener),
        Err(error) => {
            eprintln!(
                "[daemon] ignoring {}={raw}: {error}; binding the socket path instead",
                qol_conventions::ENV_DAEMON_LISTENER_FD
            );
            std::env::remove_var(qol_conventions::ENV_DAEMON_LISTENER_FD);
            None
        }
    }
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
    if !is_listening_socket(fd) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("fd {fd} is not a listening socket in this process"),
        ));
    }
    restore_cloexec(fd)?;
    Ok(unsafe { UnixListener::from_raw_fd(fd) })
}

fn is_listening_socket(fd: RawFd) -> bool {
    socket_opt(fd, libc::SO_ACCEPTCONN).is_some_and(|accepting| accepting != 0)
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
        eprintln!(
            "[daemon] ignoring {env_name}={raw}: fd {fd} is not a pre-bound socket in this process"
        );
        std::env::remove_var(env_name);
        return None;
    }
    Some(fd)
}

fn read_and_parse<C, F>(stream: &mut UnixStream, mut parser: F) -> ReadResult<C>
where
    F: FnMut(&str) -> ReadResult<C>,
{
    read_request_and_parse(stream, |request| parser(&request.action))
}

fn read_request_and_parse<C, F>(stream: &mut UnixStream, mut parser: F) -> ReadResult<C>
where
    F: FnMut(&DaemonRequest) -> ReadResult<C>,
{
    let timeout = std::time::Duration::from_millis(ACK_TIMEOUT_MS);
    let _ = stream.set_read_timeout(Some(timeout));

    let mut reader = BufReader::new(&*stream);
    let line = match local_ipc::read_line(&mut reader) {
        Ok(None) => {
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
        Ok(Some(line)) => line,
    };

    let trimmed = line.trim();

    if trimmed.is_empty() {
        return ReadResult::Ignore;
    }

    if let Ok(request) = serde_json::from_str::<DaemonRequest>(trimmed) {
        return parse_with_theme_override(&request, &mut parser);
    }

    let cmd = match trimmed.strip_prefix("action:") {
        Some(a) => a,
        None => trimmed,
    };
    parse_with_theme_override(
        &DaemonRequest {
            action: cmd.to_string(),
            input: serde_json::Value::Null,
        },
        &mut parser,
    )
}

fn theme_override_args(action: &str) -> Option<(Option<&str>, Option<&str>)> {
    let rest = if action == "theme" {
        ""
    } else {
        action.strip_prefix("theme ")?
    };
    let mut parts = rest.split_whitespace();
    let native = parts.next().filter(|value| *value != "-");
    let accent = parts.next().filter(|value| *value != "-");
    Some((native, accent))
}

fn parse_with_theme_override<C, F>(request: &DaemonRequest, parser: &mut F) -> ReadResult<C>
where
    F: FnMut(&DaemonRequest) -> ReadResult<C>,
{
    let Some((native, accent)) = theme_override_args(&request.action) else {
        return parser(request);
    };
    qol_theme::set_runtime_theme_override(native, accent);
    match parser(request) {
        ReadResult::Fallback | ReadResult::Error(_) | ReadResult::Ignore => ReadResult::Handled,
        parsed => parsed,
    }
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
    use std::io::BufRead;
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
    // handoff. plugin-pointz then rebound its discovery port, hit
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
