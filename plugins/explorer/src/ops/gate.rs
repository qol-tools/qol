use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::PathBuf;

const LOCK_NAME: &str = "explorer-jobs.lock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Taken;

impl Gate {
    pub fn system() -> Self {
        Self {
            path: qol_config::runtime_dir().map(|dir| dir.join(LOCK_NAME)),
        }
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub fn share(&self) -> Option<File> {
        let file = self.open()?;
        match file.lock_shared() {
            Ok(()) => Some(file),
            Err(error) => {
                log::warn!("could not share the job lock: {error}");
                None
            }
        }
    }

    pub fn claim(&self) -> Result<Option<File>, Taken> {
        let Some(file) = self.open() else {
            return Ok(None);
        };
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Err(Taken),
            Err(TryLockError::Error(error)) => {
                log::warn!("could not claim the job lock: {error}");
                Ok(None)
            }
        }
    }

    fn open(&self) -> Option<File> {
        let path = self.path.as_ref()?;
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                log::warn!("could not make {}: {error}", parent.display());
                return None;
            }
        }
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
        {
            Ok(file) => Some(file),
            Err(error) => {
                log::warn!("could not open {}: {error}", path.display());
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::thread;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::{Gate, Taken};

    fn gate() -> (TempDir, Gate) {
        let dir = tempfile::tempdir().unwrap();
        let gate = Gate::at(dir.path().join("runtime").join("jobs.lock"));
        (dir, gate)
    }

    fn unmakeable() -> (TempDir, Gate) {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "a file, not a folder").unwrap();
        let gate = Gate::at(blocker.join("runtime").join("jobs.lock"));
        (dir, gate)
    }

    #[test]
    fn claim_while_another_handle_shares_is_taken() {
        let (_dir, gate) = gate();
        let shared = gate.share();
        assert!(shared.is_some());
        assert!(matches!(gate.claim(), Err(Taken)));
    }

    #[test]
    fn claim_after_the_share_is_dropped_succeeds() {
        let (_dir, gate) = gate();
        let shared = gate.share();
        assert!(shared.is_some());
        drop(shared);
        assert!(matches!(gate.claim(), Ok(Some(_))));
    }

    #[test]
    fn claim_while_claimed_is_taken_and_free_again_once_released() {
        let (_dir, gate) = gate();
        let claimed = gate.claim().unwrap();
        assert!(claimed.is_some());
        assert!(matches!(gate.claim(), Err(Taken)));
        drop(claimed);
        assert!(matches!(gate.claim(), Ok(Some(_))));
    }

    #[test]
    fn claim_makes_the_folder_of_the_lock() {
        let (dir, gate) = gate();
        let claimed = gate.claim().unwrap();
        assert!(claimed.is_some());
        assert!(dir.path().join("runtime").join("jobs.lock").is_file());
    }

    #[test]
    fn shares_do_not_block_each_other() {
        let (_dir, gate) = gate();
        let first = gate.share();
        let second = gate.share();
        assert!(first.is_some());
        assert!(second.is_some());
    }

    #[test]
    fn share_while_claimed_waits_until_the_claim_is_released() {
        let (_dir, gate) = gate();
        let claimed = gate.claim().unwrap();
        assert!(claimed.is_some());
        let waiting = gate.clone();
        let (sender, receiver) = channel();
        let handle = thread::spawn(move || {
            let shared = waiting.share();
            sender.send(shared.is_some()).unwrap();
            shared
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_millis(300)),
            Err(RecvTimeoutError::Timeout)
        );
        drop(claimed);
        assert_eq!(receiver.recv_timeout(Duration::from_secs(10)), Ok(true));
        assert!(handle.join().unwrap().is_some());
    }

    #[test]
    fn a_gate_whose_folder_cannot_be_made_claims_nothing_and_shares_nothing() {
        let (_dir, gate) = unmakeable();
        assert!(matches!(gate.claim(), Ok(None)));
        assert!(gate.share().is_none());
    }

    #[test]
    fn gates_at_different_places_do_not_hold_each_other() {
        let (_first_dir, first) = gate();
        let (_second_dir, second) = gate();
        let claimed = first.claim().unwrap();
        assert!(claimed.is_some());
        assert!(matches!(second.claim(), Ok(Some(_))));
    }
}
