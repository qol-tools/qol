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

pub fn restore_cloexec(_fd: i32) -> io::Result<()> {
    Err(unsupported())
}

pub fn inherited_port_fd(_name: &str) -> Option<i32> {
    None
}

pub fn inherited_primary_port_fd() -> Option<i32> {
    None
}
