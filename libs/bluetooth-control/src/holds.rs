use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const HOLDS_FILE: &str = "holds.json";
const LOCK_FILE: &str = "holds.lock";
const MAX_HOLDS: usize = 64;
const MAX_FILE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldOwner {
    Handoff,
    ControllerReclaim,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Hold {
    pub address: String,
    pub owner: HoldOwner,
    pub until_ms: u64,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    holds: Vec<Hold>,
}

pub struct Holds {
    directory: PathBuf,
}

impl Holds {
    pub fn shared() -> Option<Self> {
        qol_config::runtime_dir().map(|runtime| Self::at(runtime.join("bluetooth-control")))
    }

    pub fn at(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn active(&self, address: &str, now_ms: u64) -> io::Result<Option<Hold>> {
        let _lock = self.lock(false)?;
        Ok(self
            .read()
            .holds
            .into_iter()
            .find(|hold| hold.address == address && hold.until_ms > now_ms))
    }

    pub fn place(
        &self,
        address: &str,
        owner: HoldOwner,
        until_ms: u64,
        now_ms: u64,
    ) -> io::Result<()> {
        self.edit(now_ms, |holds| {
            holds.retain(|hold| !(hold.address == address && hold.owner == owner));
            if holds.len() == MAX_HOLDS {
                return Err(io::Error::other("too many Bluetooth reconnect holds"));
            }
            holds.push(Hold {
                address: address.to_string(),
                owner,
                until_ms,
            });
            Ok(())
        })
    }

    pub fn release(&self, address: &str, owner: HoldOwner, now_ms: u64) -> io::Result<bool> {
        let mut released = false;
        self.edit(now_ms, |holds| {
            let before = holds.len();
            holds.retain(|hold| !(hold.address == address && hold.owner == owner));
            released = holds.len() != before;
            Ok(())
        })?;
        Ok(released)
    }

    pub fn release_address(&self, address: &str, now_ms: u64) -> io::Result<()> {
        self.edit(now_ms, |holds| {
            holds.retain(|hold| hold.address != address);
            Ok(())
        })
    }

    fn edit(
        &self,
        now_ms: u64,
        change: impl FnOnce(&mut Vec<Hold>) -> io::Result<()>,
    ) -> io::Result<()> {
        qol_fs::create_private_dir(&self.directory)?;
        let _lock = self.lock(true)?;
        let mut stored = self.read();
        stored.holds.retain(|hold| hold.until_ms > now_ms);
        change(&mut stored.holds)?;
        let bytes = serde_json::to_vec(&stored).map_err(io::Error::other)?;
        qol_fs::atomic_write_private(&self.directory.join(HOLDS_FILE), &bytes)
    }

    fn read(&self) -> Stored {
        read_bounded(&self.directory.join(HOLDS_FILE))
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn lock(&self, exclusive: bool) -> io::Result<Option<File>> {
        let path = self.directory.join(LOCK_FILE);
        let file = match OpenOptions::new()
            .read(true)
            .write(exclusive)
            .create(exclusive)
            .truncate(false)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if !exclusive && error.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if exclusive {
            file.lock()?;
        } else {
            file.lock_shared()?;
        }
        Ok(Some(file))
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or_default()
}

fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EARBUDS: &str = "AA:BB:CC:DD:EE:FF";

    #[test]
    fn a_hold_blocks_until_it_expires_or_is_released() {
        let temporary = tempfile::tempdir().unwrap();
        let holds = Holds::at(temporary.path().join("control"));

        assert_eq!(holds.active(EARBUDS, 0).unwrap(), None);
        holds.place(EARBUDS, HoldOwner::Handoff, 100, 0).unwrap();

        let other_process = Holds::at(temporary.path().join("control"));
        assert_eq!(
            other_process
                .active(EARBUDS, 50)
                .unwrap()
                .map(|hold| hold.owner),
            Some(HoldOwner::Handoff)
        );
        assert_eq!(other_process.active(EARBUDS, 100).unwrap(), None);
        assert_eq!(other_process.active("11:22:33:44:55:66", 50).unwrap(), None);
        assert!(holds.release(EARBUDS, HoldOwner::Handoff, 50).unwrap());
        assert_eq!(holds.active(EARBUDS, 50).unwrap(), None);
        assert!(!holds.release(EARBUDS, HoldOwner::Handoff, 50).unwrap());
    }

    #[test]
    fn owners_release_only_their_own_hold_and_a_user_action_releases_all() {
        let temporary = tempfile::tempdir().unwrap();
        let holds = Holds::at(temporary.path());
        holds.place(EARBUDS, HoldOwner::Handoff, 100, 0).unwrap();
        holds
            .place(EARBUDS, HoldOwner::ControllerReclaim, 200, 0)
            .unwrap();

        holds.release(EARBUDS, HoldOwner::Handoff, 10).unwrap();
        assert_eq!(
            holds.active(EARBUDS, 150).unwrap().map(|hold| hold.owner),
            Some(HoldOwner::ControllerReclaim)
        );
        holds.release_address(EARBUDS, 10).unwrap();
        assert_eq!(holds.active(EARBUDS, 10).unwrap(), None);
    }

    #[test]
    fn a_corrupt_store_holds_nothing_and_is_replaced_on_the_next_write() {
        let temporary = tempfile::tempdir().unwrap();
        std::fs::write(temporary.path().join(HOLDS_FILE), "not json").unwrap();
        let holds = Holds::at(temporary.path());

        assert_eq!(holds.active(EARBUDS, 0).unwrap(), None);
        holds.place(EARBUDS, HoldOwner::Handoff, 100, 0).unwrap();
        assert!(holds.active(EARBUDS, 0).unwrap().is_some());
    }

    #[test]
    fn expired_holds_are_pruned_and_the_store_stays_bounded() {
        let temporary = tempfile::tempdir().unwrap();
        let holds = Holds::at(temporary.path());
        for index in 0..MAX_HOLDS {
            holds
                .place(&format!("{index:02}"), HoldOwner::Handoff, 10, 0)
                .unwrap();
        }
        assert!(holds.place(EARBUDS, HoldOwner::Handoff, 100, 0).is_err());
        holds.place(EARBUDS, HoldOwner::Handoff, 100, 20).unwrap();
        assert!(holds.active(EARBUDS, 20).unwrap().is_some());
    }
}
