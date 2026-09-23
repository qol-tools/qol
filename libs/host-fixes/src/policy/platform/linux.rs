use anyhow::{Context, Result};

pub(crate) fn fail_next(point: &str) -> Result<()> {
    #[cfg(any(test, feature = "sandbox"))]
    if std::env::var("QOL_RESIDENT_FAIL_NEXT").as_deref() == Ok(point) {
        std::env::remove_var("QOL_RESIDENT_FAIL_NEXT");
        return Err(std::io::Error::other(format!("injected {point} failure")).into());
    }
    #[cfg(not(any(test, feature = "sandbox")))]
    let _ = point;
    Ok(())
}

pub(crate) fn expected_policy_file_owner() -> (u32, u32) {
    #[cfg(any(test, feature = "sandbox"))]
    {
        (unsafe { libc::geteuid() }, unsafe { libc::getegid() })
    }
    #[cfg(not(any(test, feature = "sandbox")))]
    {
        (0, 0)
    }
}

pub(crate) fn sync_directory_fd_strict(dir_fd: &std::fs::File) -> Result<()> {
    use std::os::unix::io::AsRawFd;
    let result = unsafe { libc::fsync(dir_fd.as_raw_fd()) };
    if result == 0 {
        return Ok(());
    }
    Err(std::io::Error::last_os_error()).context("strict directory fsync failed")
}
