use std::path::{Path, PathBuf};

use super::ConnectResult;
use crate::local_ipc::LocalStream;

pub(super) fn connect(path: &Path) -> ConnectResult {
    let stream = LocalStream::connect(path)?;
    crate::local_ipc::authorize_peer(&stream)?;
    Ok(Box::new(stream))
}

pub(in crate::client) fn fallback_state_socket() -> Option<PathBuf> {
    Some(PathBuf::from(qol_conventions::STATE_SOCKET_PATH))
}
