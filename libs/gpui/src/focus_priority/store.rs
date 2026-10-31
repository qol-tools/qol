use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::FocusPriority;

pub(crate) const CLAIM_MAX_AGE: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusClaim {
    pub priority: FocusPriority,
    pub pid: u32,
    pub title: String,
    pub claimed_at_ms: u64,
}

impl FocusClaim {
    pub(crate) fn new(priority: FocusPriority, pid: u32, title: &str, now: SystemTime) -> Self {
        Self {
            priority,
            pid,
            title: title.to_owned(),
            claimed_at_ms: unix_ms(now),
        }
    }

    fn age(&self, now: SystemTime) -> Duration {
        Duration::from_millis(unix_ms(now).saturating_sub(self.claimed_at_ms))
    }
}

pub(crate) fn outranking_claim(
    mine: FocusPriority,
    own_pid: u32,
    claims: impl IntoIterator<Item = FocusClaim>,
    now: SystemTime,
    alive: impl Fn(u32) -> bool,
) -> Option<FocusClaim> {
    claims
        .into_iter()
        .filter(|claim| claim.priority.outranks(mine))
        .filter(|claim| claim.pid != own_pid)
        .filter(|claim| claim.age(now) <= CLAIM_MAX_AGE)
        .filter(|claim| alive(claim.pid))
        .max_by_key(|claim| claim.priority)
}

pub(crate) struct ClaimStore {
    dir: PathBuf,
}

impl ClaimStore {
    pub(crate) fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub(crate) fn runtime() -> Option<Self> {
        qol_config::runtime_dir().map(|dir| Self::at(dir.join("focus-priority")))
    }

    fn path(&self, priority: FocusPriority) -> PathBuf {
        self.dir.join(format!("{}.json", priority.key()))
    }

    pub(crate) fn write(&self, claim: &FocusClaim) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let target = self.path(claim.priority);
        let staging = self
            .dir
            .join(format!(".{}.{}.tmp", claim.priority.key(), claim.pid));
        let body = serde_json::to_vec(claim).map_err(io::Error::other)?;
        fs::write(&staging, body)?;
        fs::rename(&staging, &target)
    }

    pub(crate) fn read(&self, priority: FocusPriority) -> Option<FocusClaim> {
        read_claim(&self.path(priority)).filter(|claim| claim.priority == priority)
    }

    pub(crate) fn read_above(&self, mine: FocusPriority) -> Vec<FocusClaim> {
        FocusPriority::ALL
            .into_iter()
            .filter(|priority| priority.outranks(mine))
            .filter_map(|priority| self.read(priority))
            .collect()
    }

    pub(crate) fn remove_own(&self, priority: FocusPriority, own_pid: u32) -> bool {
        let path = self.path(priority);
        match read_claim(&path) {
            Some(claim) if claim.pid == own_pid => fs::remove_file(&path).is_ok(),
            _ => false,
        }
    }
}

fn read_claim(path: &Path) -> Option<FocusClaim> {
    let body = fs::read(path).ok()?;
    serde_json::from_slice(&body).ok()
}

fn unix_ms(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{outranking_claim, ClaimStore, FocusClaim, CLAIM_MAX_AGE};
    use crate::focus_priority::FocusPriority;

    const OWN_PID: u32 = 100;
    const OTHER_PID: u32 = 200;

    fn at(ms: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(ms)
    }

    fn claim(priority: FocusPriority, pid: u32, claimed_at_ms: u64) -> FocusClaim {
        FocusClaim::new(priority, pid, "popup", at(claimed_at_ms))
    }

    #[test]
    fn alt_tab_outranks_the_launcher_and_never_the_reverse() {
        assert!(FocusPriority::AltTab.outranks(FocusPriority::Launcher));
        assert!(!FocusPriority::Launcher.outranks(FocusPriority::AltTab));
        for priority in FocusPriority::ALL {
            assert!(!priority.outranks(priority), "{priority:?} outranks itself");
        }
    }

    #[test]
    fn only_a_live_higher_claim_from_another_process_makes_a_popup_yield() {
        let now = at(10_000);
        let cases = [
            (
                FocusPriority::Launcher,
                FocusPriority::AltTab,
                OTHER_PID,
                9_000,
                true,
                true,
            ),
            (
                FocusPriority::Launcher,
                FocusPriority::AltTab,
                OTHER_PID,
                9_000,
                false,
                false,
            ),
            (
                FocusPriority::Launcher,
                FocusPriority::AltTab,
                OWN_PID,
                9_000,
                true,
                false,
            ),
            (
                FocusPriority::Launcher,
                FocusPriority::Launcher,
                OTHER_PID,
                9_000,
                true,
                false,
            ),
            (
                FocusPriority::AltTab,
                FocusPriority::Launcher,
                OTHER_PID,
                9_000,
                true,
                false,
            ),
            (
                FocusPriority::AltTab,
                FocusPriority::AltTab,
                OTHER_PID,
                9_000,
                true,
                false,
            ),
            (
                FocusPriority::Launcher,
                FocusPriority::AltTab,
                OTHER_PID,
                12_000,
                true,
                true,
            ),
        ];
        for (mine, theirs, pid, claimed_at_ms, alive, yields) in cases {
            let found = outranking_claim(
                mine,
                OWN_PID,
                [claim(theirs, pid, claimed_at_ms)],
                now,
                |_| alive,
            );
            assert_eq!(
                found.is_some(),
                yields,
                "mine={mine:?} theirs={theirs:?} pid={pid} alive={alive}"
            );
        }
    }

    #[test]
    fn a_claim_older_than_the_max_age_is_stale() {
        let claimed = 1_000;
        let max = u64::try_from(CLAIM_MAX_AGE.as_millis()).unwrap();
        let fresh = outranking_claim(
            FocusPriority::Launcher,
            OWN_PID,
            [claim(FocusPriority::AltTab, OTHER_PID, claimed)],
            at(claimed + max),
            |_| true,
        );
        let stale = outranking_claim(
            FocusPriority::Launcher,
            OWN_PID,
            [claim(FocusPriority::AltTab, OTHER_PID, claimed)],
            at(claimed + max + 1),
            |_| true,
        );
        assert!(fresh.is_some());
        assert!(stale.is_none());
    }

    #[test]
    fn the_store_round_trips_claims_and_removes_only_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let store = ClaimStore::at(dir.path().join("focus-priority"));
        let alt_tab = claim(FocusPriority::AltTab, OTHER_PID, 5);
        store.write(&alt_tab).unwrap();

        assert_eq!(store.read(FocusPriority::AltTab), Some(alt_tab.clone()));
        assert_eq!(store.read_above(FocusPriority::Launcher), vec![alt_tab]);
        assert!(store.read_above(FocusPriority::AltTab).is_empty());

        assert!(!store.remove_own(FocusPriority::AltTab, OWN_PID));
        assert!(store.read(FocusPriority::AltTab).is_some());
        assert!(store.remove_own(FocusPriority::AltTab, OTHER_PID));
        assert!(store.read(FocusPriority::AltTab).is_none());
    }

    #[test]
    fn a_corrupt_claim_file_reads_as_no_claim() {
        let dir = tempfile::tempdir().unwrap();
        let store = ClaimStore::at(dir.path());
        std::fs::write(dir.path().join("alt_tab.json"), b"{not json").unwrap();
        assert!(store.read(FocusPriority::AltTab).is_none());
    }
}
