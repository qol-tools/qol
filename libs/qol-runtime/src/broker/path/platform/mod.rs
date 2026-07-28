use std::path::{Path, PathBuf};

use super::BrokerPathError;

#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
use fallback as active;
#[cfg(unix)]
use unix as active;

pub use active::BrokerListener;

pub(super) fn broker_socket_path_for_uid(
    uid: u32,
    xdg_runtime_dir: Option<&str>,
) -> Result<PathBuf, BrokerPathError> {
    active::broker_socket_path_for_uid(uid, xdg_runtime_dir)
}

pub(super) fn broker_socket_path() -> Result<PathBuf, BrokerPathError> {
    active::broker_socket_path()
}

pub(super) fn bind_broker_listener(socket: &Path) -> Result<BrokerListener, BrokerPathError> {
    active::bind_broker_listener(socket)
}
