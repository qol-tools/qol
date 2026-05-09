use std::collections::HashSet;
use std::sync::{OnceLock, RwLock};

static IGNORE_PIDS: OnceLock<RwLock<HashSet<u32>>> = OnceLock::new();

fn ignore_pids() -> &'static RwLock<HashSet<u32>> {
    IGNORE_PIDS.get_or_init(|| RwLock::new(HashSet::new()))
}

pub(crate) fn add_ignore_pid(pid: u32) {
    if let Ok(mut set) = ignore_pids().write() {
        set.insert(pid);
        log::debug!("[runtime/ignore_pids] ADD {} → set={:?}", pid, *set);
    }
}

pub(crate) fn remove_ignore_pid(pid: u32) {
    if let Ok(mut set) = ignore_pids().write() {
        set.remove(&pid);
        log::debug!("[runtime/ignore_pids] REMOVE {} → set={:?}", pid, *set);
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn is_ignored_pid(pid: u32) -> bool {
    ignore_pids()
        .read()
        .map(|set| set.contains(&pid))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn snapshot() -> HashSet<u32> {
        ignore_pids().read().unwrap().clone()
    }

    #[test]
    fn add_then_remove_round_trip() {
        let pid = 700_001;
        add_ignore_pid(pid);
        assert!(snapshot().contains(&pid));
        remove_ignore_pid(pid);
        assert!(!snapshot().contains(&pid));
    }

    #[test]
    fn add_is_idempotent_for_same_pid() {
        let pid = 700_002;
        add_ignore_pid(pid);
        add_ignore_pid(pid);
        let count = snapshot().iter().filter(|&&p| p == pid).count();
        assert_eq!(count, 1);
        remove_ignore_pid(pid);
    }

    #[test]
    fn remove_unknown_pid_does_not_panic_or_clear_others() {
        let kept = 700_003;
        add_ignore_pid(kept);
        remove_ignore_pid(900_001);
        assert!(snapshot().contains(&kept));
        remove_ignore_pid(kept);
    }
}
