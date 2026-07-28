use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::SerialAccess;

pub(super) fn inspect_serial_access(path: &str) -> SerialAccess {
    let bytes = Path::new(path).as_os_str().as_bytes();
    let Ok(path_c) = CString::new(bytes) else {
        return SerialAccess {
            path: path.to_string(),
            readable_writable: false,
            issue: Some("serial path contains an interior NUL byte".to_string()),
        };
    };
    let readable_writable = unsafe { libc::access(path_c.as_ptr(), libc::R_OK | libc::W_OK) } == 0;
    let issue = (!readable_writable).then(|| std::io::Error::last_os_error().to_string());
    SerialAccess {
        path: path.to_string(),
        readable_writable,
        issue,
    }
}
