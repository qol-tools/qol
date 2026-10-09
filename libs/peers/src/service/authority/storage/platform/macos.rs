use std::{
    ffi::OsStr,
    fs::File,
    io,
    os::{fd::AsRawFd, unix::ffi::OsStrExt},
    path::PathBuf,
};

use super::unix::io_error;
use crate::service::authority::AuthorityError;

pub(super) fn anchored_root(directory: &File) -> Result<PathBuf, AuthorityError> {
    let mut buffer = [0_u8; libc::PATH_MAX as usize];
    if unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) } == -1 {
        return Err(io_error(io::Error::last_os_error()));
    }
    let length = buffer
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(AuthorityError::Storage)?;
    Ok(PathBuf::from(OsStr::from_bytes(&buffer[..length])))
}
