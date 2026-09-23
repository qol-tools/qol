use std::path::{Path, PathBuf};

use super::super::BrokerPathError;

#[derive(Debug)]
pub struct BrokerListener;

pub(super) fn broker_socket_path_for_uid(
    _uid: u32,
    _xdg_runtime_dir: Option<&str>,
) -> Result<PathBuf, BrokerPathError> {
    Err(BrokerPathError::Unsupported)
}

pub(super) fn broker_socket_path() -> Result<PathBuf, BrokerPathError> {
    Err(BrokerPathError::Unsupported)
}

pub(super) fn bind_broker_listener(_socket: &Path) -> Result<BrokerListener, BrokerPathError> {
    Err(BrokerPathError::Unsupported)
}
