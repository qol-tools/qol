use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub(super) fn path_cstring(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

pub(super) fn already_exists(error: io::Error) -> io::Error {
    match error.raw_os_error() {
        Some(libc::EEXIST | libc::ENOTEMPTY) => io::Error::new(io::ErrorKind::AlreadyExists, error),
        _ => error,
    }
}
