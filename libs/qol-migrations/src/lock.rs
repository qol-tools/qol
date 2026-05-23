use anyhow::{Context, Result};
use fs4::fs_std::FileExt;
use std::fs::File;
use std::path::{Path, PathBuf};

const LOCK_FILE: &str = ".migration-lock";

pub(crate) struct MigrationLock {
    file: File,
    path: PathBuf,
}

impl MigrationLock {
    pub(crate) fn acquire(config_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(config_dir)
            .with_context(|| format!("ensuring config dir exists: {}", config_dir.display()))?;
        let path = config_dir.join(LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("opening lock file {}", path.display()))?;
        file.lock_exclusive()
            .with_context(|| format!("acquiring exclusive lock on {}", path.display()))?;
        Ok(Self { file, path })
    }
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        if let Err(err) = FileExt::unlock(&self.file) {
            log::warn!(
                "[qol-migrations] failed to release migration lock {}: {err}",
                self.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn second_acquire_blocks_until_first_drops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        let first = MigrationLock::acquire(&path).unwrap();

        let (tx_acquired, rx_acquired) = mpsc::channel::<()>();
        let path_clone = path.clone();
        let handle = thread::spawn(move || {
            let _second = MigrationLock::acquire(&path_clone).unwrap();
            tx_acquired.send(()).unwrap();
        });

        // give the child thread a moment to attempt acquisition
        thread::sleep(Duration::from_millis(150));
        match rx_acquired.try_recv() {
            Err(mpsc::TryRecvError::Empty) => {}
            other => panic!("second acquire should still be blocked, got {other:?}"),
        }

        drop(first);

        rx_acquired
            .recv_timeout(Duration::from_secs(5))
            .expect("second acquire should succeed after first is released");
        handle.join().unwrap();
    }

    #[test]
    fn lock_file_is_created_under_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = MigrationLock::acquire(dir.path()).unwrap();
        assert!(dir.path().join(LOCK_FILE).is_file());
    }

    #[test]
    fn acquire_creates_missing_config_dir() {
        let parent = tempfile::tempdir().unwrap();
        let nested = parent.path().join("nested").join("config");
        let _guard = MigrationLock::acquire(&nested).unwrap();
        assert!(nested.join(LOCK_FILE).is_file());
    }
}
