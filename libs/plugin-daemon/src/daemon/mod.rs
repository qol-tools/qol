//! Transport shared by every resident plugin daemon: request framing, the
//! listener loops, readiness and the instance fence, over `local_ipc`.

use std::io::{self, BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use qol_runtime::local_ipc::{self, LocalListener, LocalStream};
use qol_runtime::protocol::{DaemonRequest, DaemonResponse, ReadinessPhase};

mod platform;

use platform::{fallback_socket_dir, inherited_listener, remove_socket_file};
pub use platform::{inherited_port_fd, inherited_primary_port_fd, restore_cloexec};

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
                .unwrap_or_else(|_| fallback_socket_dir(*use_tmpdir_env).join(default_socket_name)),
        ),
        SocketSource::Fixed {
            socket_name,
            use_tmpdir_env,
        } => Some(fallback_socket_dir(*use_tmpdir_env).join(socket_name)),
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
    let mut stream = LocalStream::connect(path)?;
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
    let Ok(mut stream) = LocalStream::connect(path) else {
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
    stream: &mut LocalStream,
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
                log::warn!("accept error: {error:#}");
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
    run_stateful_request_listener_with_boundary(
        config,
        None,
        DaemonBoundary::from_environment(),
        state,
        handler,
    )
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
    run_stateful_request_listener_with_boundary(
        config,
        Some(readiness),
        DaemonBoundary::from_environment(),
        state,
        handler,
    )
}

pub fn run_stateful_request_listener_with_boundary<S, F>(
    config: &DaemonConfig,
    readiness: Option<&ReadinessGate>,
    boundary: DaemonBoundary,
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
                let result = read_request_with_boundary(&mut s, &boundary, readiness, |request| {
                    if request.action == "kill" {
                        kill_requested = true;
                    }
                    handler(&mut state, request)
                });
                handle_read_result(&mut s, result, |_| DaemonResponse::Handled { data: None });
                if kill_requested {
                    break;
                }
            }
            Err(error) => {
                log::warn!("accept error: {error:#}");
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

fn bind_listener(config: &DaemonConfig) -> io::Result<(LocalListener, Option<PathBuf>)> {
    if let Some(listener) = inherited_listener() {
        #[cfg(debug_assertions)]
        log::debug!("adopting a pre-bound listener fd, skipping bind()");
        return Ok((listener, None));
    }

    let Some(socket_path) = socket_path(config) else {
        log::warn!(
            "{} unset and no fallback socket - not binding",
            qol_conventions::ENV_DAEMON_SOCKET
        );
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("{} is not set", qol_conventions::ENV_DAEMON_SOCKET),
        ));
    };
    #[cfg(debug_assertions)]
    log::debug!("binding to {:?}", socket_path);

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
                log::info!("existing instance alive, exiting");
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

fn read_and_parse<C, F>(stream: &mut LocalStream, mut parser: F) -> ReadResult<C>
where
    F: FnMut(&str) -> ReadResult<C>,
{
    read_request_and_parse(stream, |request| parser(&request.action))
}

fn read_request_and_parse<C, F>(stream: &mut LocalStream, parser: F) -> ReadResult<C>
where
    F: FnMut(&DaemonRequest) -> ReadResult<C>,
{
    read_request_with_boundary(stream, &DaemonBoundary::from_environment(), None, parser)
}

fn read_request_with_boundary<C, F>(
    stream: &mut LocalStream,
    boundary: &DaemonBoundary,
    readiness: Option<&ReadinessGate>,
    mut parser: F,
) -> ReadResult<C>
where
    F: FnMut(&DaemonRequest) -> ReadResult<C>,
{
    let timeout = std::time::Duration::from_millis(ACK_TIMEOUT_MS);
    let _ = stream.set_read_timeout(Some(timeout));

    let mut reader = BufReader::new(&*stream);
    let line = match local_ipc::read_line(&mut reader) {
        Ok(None) => {
            #[cfg(debug_assertions)]
            log::debug!("read_line EOF (0 bytes)");
            return ReadResult::Ignore;
        }
        Err(e) => {
            log::warn!("read_line error: {:?}", e);
            return ReadResult::Ignore;
        }
        Ok(Some(line)) => line,
    };

    let trimmed = line.trim();

    if trimmed.is_empty() {
        return ReadResult::Ignore;
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if value.get("fence_version").is_some()
            || value.get("instance").is_some()
            || value.get("request").is_some()
        {
            let Ok(envelope) =
                serde_json::from_str::<qol_runtime::protocol::FencedDaemonRequest>(trimmed)
            else {
                return ReadResult::Error("invalid daemon fence".into());
            };
            if envelope.fence_version != 1 || boundary.instance.as_ref() != Some(&envelope.instance)
            {
                return ReadResult::Error("daemon instance changed".into());
            }
            return boundary.parse(stream, readiness, &envelope.request, &mut parser);
        }
    }
    if let Ok(request) = serde_json::from_str::<DaemonRequest>(trimmed) {
        return boundary.parse(stream, readiness, &request, &mut parser);
    }

    if trimmed.starts_with('{') {
        return ReadResult::Error("invalid daemon request".into());
    }
    let cmd = match trimmed.strip_prefix("action:") {
        Some(a) => a,
        None => trimmed,
    };
    boundary.parse(
        stream,
        readiness,
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
    stream: &mut LocalStream,
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

fn write_response(stream: &mut LocalStream, response: &DaemonResponse) {
    if let Ok(payload) = local_ipc::encode_secret_json(response) {
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(10)));
        let _ = stream.write_all(&payload);
    }
}

#[derive(Clone)]
pub struct DaemonBoundary {
    instance: Option<String>,
}

impl DaemonBoundary {
    pub fn from_environment() -> Self {
        static INSTANCE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        Self {
            instance: INSTANCE
                .get_or_init(|| {
                    std::env::var(qol_conventions::ENV_DAEMON_INSTANCE)
                        .ok()
                        .filter(|value| {
                            value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                })
                .clone(),
        }
    }

    pub fn with_instance(instance: String) -> io::Result<Self> {
        if instance.len() != 32 || !instance.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "invalid daemon instance",
            ));
        }
        Ok(Self {
            instance: Some(instance),
        })
    }

    fn parse<C, F>(
        &self,
        stream: &mut LocalStream,
        readiness: Option<&ReadinessGate>,
        request: &DaemonRequest,
        parser: &mut F,
    ) -> ReadResult<C>
    where
        F: FnMut(&DaemonRequest) -> ReadResult<C>,
    {
        if request.action != "kill" {
            if let Some((phase, detail)) = readiness.and_then(ReadinessGate::snapshot) {
                write_response(stream, &DaemonResponse::NotReady { phase, detail });
                return ReadResult::Ignore;
            }
        }
        if request.action == "ping" {
            if let Some(instance) = &self.instance {
                return ReadResult::HandledWithData(
                    serde_json::json!({ "fence_version": 1, "instance": instance }),
                );
            }
        }
        parse_with_theme_override(request, parser)
    }
}
