use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

const LOCK_FILE_NAME: &str = "qol-shot-capture.lock";

pub(crate) struct CaptureGuard {
    action: &'static str,
    file: File,
}

impl CaptureGuard {
    fn new(action: &'static str, file: File) -> Self {
        Self { action, file }
    }
}

pub(crate) fn try_acquire(action: &'static str) -> Option<CaptureGuard> {
    try_acquire_unix(action)
}

fn try_acquire_unix(action: &'static str) -> Option<CaptureGuard> {
    use std::os::fd::AsRawFd;

    let path = lock_path();
    let mut file = match OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) => {
            qol_runtime::probe!(
                "SHOT_CAPTURE_LOCK",
                "action={action} result=open-error err={}",
                error.kind()
            );
            return None;
        }
    };

    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        qol_runtime::probe!(
            "SHOT_CAPTURE_LOCK",
            "action={action} result=busy err={}",
            error.raw_os_error().unwrap_or_default()
        );
        return None;
    }

    let _ = file.set_len(0);
    let _ = writeln!(file, "pid={} action={action}", std::process::id());
    qol_runtime::probe!("SHOT_CAPTURE_LOCK", "action={action} result=acquired");
    Some(CaptureGuard::new(action, file))
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;

        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        qol_runtime::probe!(
            "SHOT_CAPTURE_LOCK",
            "action={} result=released",
            self.action
        );
    }
}

fn lock_path() -> PathBuf {
    std::env::temp_dir().join(LOCK_FILE_NAME)
}
