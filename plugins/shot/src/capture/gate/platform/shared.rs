use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::PathBuf;

const LOCK_FILE_NAME: &str = "qol-shot-capture.lock";

pub(crate) struct CaptureGuard {
    action: &'static str,
    file: File,
}

pub(crate) fn try_acquire(action: &'static str) -> Option<CaptureGuard> {
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

    if let Err(error) = file.try_lock() {
        let reason = match error {
            TryLockError::WouldBlock => "busy".to_string(),
            TryLockError::Error(error) => format!("lock-error err={}", error.kind()),
        };
        qol_runtime::probe!("SHOT_CAPTURE_LOCK", "action={action} result={reason}");
        return None;
    }

    let _ = file.set_len(0);
    let _ = writeln!(file, "pid={} action={action}", std::process::id());
    qol_runtime::probe!("SHOT_CAPTURE_LOCK", "action={action} result=acquired");
    Some(CaptureGuard { action, file })
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
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

#[cfg(test)]
mod tests {
    use super::try_acquire;

    #[test]
    fn a_held_capture_lock_refuses_a_second_capture_until_released() {
        let first = try_acquire("test-first").expect("first capture takes the lock");
        assert!(try_acquire("test-second").is_none());
        drop(first);
        let again = try_acquire("test-again");
        assert!(again.is_some());
    }
}
