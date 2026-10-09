use std::path::Path;

use super::ConnectResult;
use crate::local_ipc::LocalStream;

pub(super) fn connect(path: &Path) -> ConnectResult {
    let stream = LocalStream::connect(path)?;
    crate::local_ipc::authorize_peer(&stream)?;
    Ok(Box::new(stream))
}
