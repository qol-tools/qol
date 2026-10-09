use std::{fs::File, os::fd::AsRawFd, path::PathBuf};

use crate::service::authority::AuthorityError;

pub(super) fn anchored_root(directory: &File) -> Result<PathBuf, AuthorityError> {
    Ok(PathBuf::from(format!(
        "/proc/self/fd/{}",
        directory.as_raw_fd()
    )))
}
