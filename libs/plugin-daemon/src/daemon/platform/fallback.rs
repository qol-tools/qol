//! Fail-closed transport adapter for hosts without Unix-domain sockets.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

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

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "resident plugin daemon transport is unavailable on this platform",
    )
}

pub fn socket_path(_config: &DaemonConfig) -> Option<PathBuf> {
    None
}

pub fn send_action(_config: &DaemonConfig, _action: &str, _expect_reply: bool) -> bool {
    false
}

pub fn send_request(
    _config: &DaemonConfig,
    _action: &str,
    _input: serde_json::Value,
    _timeout: Duration,
) -> io::Result<DaemonResponse> {
    Err(unsupported())
}

pub fn send_kill(_config: &DaemonConfig) -> bool {
    false
}

pub fn send_ping(_config: &DaemonConfig) -> bool {
    false
}

pub fn cleanup(_config: &DaemonConfig) {}

pub fn start_listener<C: Send + 'static>(
    _config: &DaemonConfig,
    _tx: std::sync::mpsc::Sender<C>,
    _parser: fn(&str) -> ReadResult<C>,
) -> bool {
    false
}

pub fn start_request_listener<C: Send + 'static>(
    _config: &DaemonConfig,
    _tx: std::sync::mpsc::Sender<C>,
    _parser: fn(&DaemonRequest) -> ReadResult<C>,
) -> bool {
    false
}

pub fn run_stateful_listener<S, F>(_config: &DaemonConfig, _state: S, _handler: F) -> io::Result<()>
where
    F: FnMut(&mut S, &str) -> ReadResult<()>,
{
    Err(unsupported())
}

pub fn run_stateful_request_listener<S, F>(
    _config: &DaemonConfig,
    _state: S,
    _handler: F,
) -> io::Result<()>
where
    F: FnMut(&mut S, &DaemonRequest) -> ReadResult<()>,
{
    Err(unsupported())
}

pub fn restore_cloexec(_fd: i32) -> io::Result<()> {
    Err(unsupported())
}

pub fn inherited_port_fd(_name: &str) -> Option<i32> {
    None
}

pub fn inherited_primary_port_fd() -> Option<i32> {
    None
}

#[derive(Clone)]
pub struct DaemonBoundary;

impl DaemonBoundary {
    pub fn from_environment() -> Self {
        Self
    }
    pub fn with_instance(_: String) -> io::Result<Self> {
        Err(unsupported())
    }
}

#[derive(Clone)]
pub struct ReadinessGate;

impl ReadinessGate {
    pub fn starting() -> Self {
        Self
    }
    pub fn set_phase(&self, _: qol_runtime::protocol::ReadinessPhase, _: Option<String>) {}
    pub fn mark_ready(&self) {}
}

pub fn run_stateful_request_listener_with_boundary<S, F>(
    _: &DaemonConfig,
    _: Option<&ReadinessGate>,
    _: DaemonBoundary,
    _: S,
    _: F,
) -> io::Result<()>
where
    F: FnMut(&mut S, &DaemonRequest) -> ReadResult<()>,
{
    Err(unsupported())
}
