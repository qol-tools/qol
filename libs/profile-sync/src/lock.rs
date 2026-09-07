//! Cross-process lock serializing profile-sync mutations.
//!
//! The tray daemon and `qol sync` can run concurrently against the same
//! profile repo. [`SyncLock`] is an advisory lock over a lockfile in the
//! per-device sync state dir (see [`crate::state::SyncPaths::lock_path`]);
//! both entry points acquire it around their pull/merge/push windows.
//!
//! The lock uses `std::fs::File::lock`, which maps to `flock(2)` on Unix and
//! `LockFileEx` on Windows. The OS releases the lock automatically when the
//! holding process exits or crashes, so a stale lockfile never wedges sync.
//! [`SyncLock::acquire`] blocks until the lock is free; use
//! [`SyncLock::try_acquire`] for a non-blocking probe.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// An acquired cross-process lock. Dropping (or releasing via [`Drop`])
/// unlocks the file, so the guard must outlive the protected window.
pub struct SyncLock {
    file: File,
}

impl SyncLock {
    /// Opens (creating if needed) the lockfile and blocks until the lock is
    /// acquired.
    pub fn acquire(lock_path: &Path) -> io::Result<SyncLock> {
        let file = open_lock_file(lock_path)?;
        file.lock()?;
        Ok(SyncLock { file })
    }

    /// Tries to acquire the lock without blocking. Returns `Ok(None)` when
    /// another process currently holds it.
    pub fn try_acquire(lock_path: &Path) -> io::Result<Option<SyncLock>> {
        let file = open_lock_file(lock_path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(SyncLock { file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn open_lock_file(lock_path: &Path) -> io::Result<File> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn acquire_creates_the_lockfile_and_blocks_other_acquires() {
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("sync.lock");

        let held = SyncLock::acquire(&lock_path).unwrap();
        assert!(lock_path.is_file());
        assert!(SyncLock::try_acquire(&lock_path).unwrap().is_none());

        drop(held);
        assert!(SyncLock::try_acquire(&lock_path).unwrap().is_some());
    }

    #[test]
    fn lock_serializes_across_processes() {
        if std::env::var("QOL_SYNC_LOCK_CHILD_PROBE").is_ok() {
            child_probe_acquire();
            return;
        }
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("sync.lock");
        let held = SyncLock::acquire(&lock_path).unwrap();

        // While we hold the lock, a separate process must not acquire it.
        let blocked = spawn_child_probe(&lock_path, "blocked");
        assert!(blocked.success(), "child must be blocked while held");

        drop(held);

        // Once released, a separate process can acquire it.
        let acquired = spawn_child_probe(&lock_path, "free");
        assert!(acquired.success(), "child must acquire after release");
    }

    fn spawn_child_probe(lock_path: &Path, expectation: &str) -> std::process::ExitStatus {
        std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("lock::tests::child_probe_acquire")
            .env("QOL_SYNC_LOCK_CHILD_PROBE", "1")
            .env("QOL_SYNC_LOCK_PATH", lock_path)
            .env("QOL_SYNC_LOCK_EXPECT", expectation)
            .status()
            .unwrap()
    }

    #[test]
    fn child_probe_acquire() {
        let Ok(raw) = std::env::var("QOL_SYNC_LOCK_PATH") else {
            return;
        };
        let expected_acquired = std::env::var("QOL_SYNC_LOCK_EXPECT").as_deref() == Ok("free");
        let lock_path = PathBuf::from(raw);
        let acquired = SyncLock::try_acquire(&lock_path).unwrap().is_some();
        std::process::exit(if acquired == expected_acquired { 0 } else { 1 });
    }
}
