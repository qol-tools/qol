use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub(super) fn c_path(path: &Path) -> std::io::Result<std::ffi::CString> {
    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL byte")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_path_rejects_nul_bytes() {
        let error = c_path(Path::new("foo\0bar")).expect_err("NUL byte should be rejected");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "path contains NUL byte");
    }
}
