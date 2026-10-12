use std::io::PipeReader;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use kqueue::{EventData, EventFilter, FilterFlag, Ident, Vnode, Watcher};

pub(super) enum Wake {
    Changed,
    Stop,
}

pub(super) struct Signal {
    watcher: Watcher,
    dir: PathBuf,
    wal: PathBuf,
    stop: RawFd,
}

impl Signal {
    pub(super) fn open(db: &Path, stop: &PipeReader) -> Result<Self> {
        let dir = db
            .parent()
            .with_context(|| format!("{} has no parent", db.display()))?
            .to_path_buf();
        let mut wal = db.as_os_str().to_owned();
        wal.push("-wal");
        let mut signal = Self {
            watcher: Watcher::new().context("kqueue unavailable")?,
            dir,
            wal: PathBuf::from(wal),
            stop: stop.as_raw_fd(),
        };
        signal
            .watcher
            .add_fd(signal.stop, EventFilter::EVFILT_READ, FilterFlag::empty())
            .context("failed to watch the stop pipe")?;
        signal
            .watcher
            .add_filename(
                &signal.dir,
                EventFilter::EVFILT_VNODE,
                FilterFlag::NOTE_WRITE,
            )
            .with_context(|| format!("failed to watch {}", signal.dir.display()))?;
        signal.arm_wal();
        signal
            .watcher
            .watch()
            .context("kqueue registration failed")?;
        Ok(signal)
    }

    pub(super) fn wait(&mut self) -> Wake {
        loop {
            let Some(event) = self.watcher.poll_forever(None) else {
                log::warn!("notification mirror: kqueue wait failed");
                return Wake::Stop;
            };
            match (event.ident, event.data) {
                (Ident::Fd(fd), _) if fd == self.stop => return Wake::Stop,
                (_, EventData::Error(error)) => {
                    log::warn!("notification mirror: kqueue event error: {error}");
                }
                (Ident::Filename(_, path), _) if Path::new(&path) == self.dir => {
                    self.rearm();
                    return Wake::Changed;
                }
                (_, EventData::Vnode(Vnode::Delete | Vnode::Rename | Vnode::Revoke)) => {
                    self.rearm();
                    return Wake::Changed;
                }
                _ => return Wake::Changed,
            }
        }
    }

    fn rearm(&mut self) {
        self.arm_wal();
        if let Err(error) = self.watcher.watch() {
            log::warn!("notification mirror: kqueue re-registration failed: {error}");
        }
    }

    fn arm_wal(&mut self) {
        let flags = FilterFlag::NOTE_WRITE
            | FilterFlag::NOTE_EXTEND
            | FilterFlag::NOTE_DELETE
            | FilterFlag::NOTE_RENAME
            | FilterFlag::NOTE_REVOKE;
        if let Err(error) = self
            .watcher
            .add_filename(&self.wal, EventFilter::EVFILT_VNODE, flags)
        {
            log::debug!(
                "notification mirror: {} not watchable yet: {error}",
                self.wal.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::Duration;

    use super::*;

    #[test]
    fn wakes_on_wal_writes_and_stops_when_the_pipe_closes() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db");
        let wal = dir.path().join("db-wal");
        std::fs::write(&db, b"").unwrap();
        std::fs::write(&wal, b"").unwrap();
        let (reader, writer) = std::io::pipe().unwrap();
        let mut signal = Signal::open(&db, &reader).unwrap();

        let writes = wal.clone();
        let writer_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&writes)
                .unwrap();
            file.write_all(b"frame").unwrap();
            file.sync_all().unwrap();
        });
        assert!(matches!(signal.wait(), Wake::Changed));
        writer_thread.join().unwrap();

        drop(writer);
        let mut stopped = false;
        for _ in 0..4 {
            if matches!(signal.wait(), Wake::Stop) {
                stopped = true;
                break;
            }
        }
        assert!(stopped);
    }

    #[test]
    fn rearms_when_the_wal_is_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db");
        let wal = dir.path().join("db-wal");
        std::fs::write(&db, b"").unwrap();
        let (reader, _writer) = std::io::pipe().unwrap();
        let mut signal = Signal::open(&db, &reader).unwrap();

        std::fs::write(&wal, b"").unwrap();
        assert!(matches!(signal.wait(), Wake::Changed));

        let writes = wal.clone();
        let writer_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&writes)
                .unwrap();
            file.write_all(b"frame").unwrap();
        });
        assert!(matches!(signal.wait(), Wake::Changed));
        writer_thread.join().unwrap();
    }
}
