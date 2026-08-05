use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::host::Pane;

mod platform;

const SNAPSHOT_TTL: Duration = Duration::from_secs(30);

pub trait ServiceProbe {
    fn is_service(&self, pane: &Pane) -> bool;
}

pub struct NoServiceProbe;

impl ServiceProbe for NoServiceProbe {
    fn is_service(&self, _pane: &Pane) -> bool {
        false
    }
}

pub struct SystemServiceProbe {
    declared: Vec<String>,
    load: fn() -> Option<ProcessSnapshot>,
    shared: SharedSnapshotCache,
}

#[derive(Default, Clone)]
struct ProcessSnapshot {
    listeners: HashSet<i32>,
    children: HashMap<i32, Vec<i32>>,
}

#[derive(Default)]
pub(crate) struct CachedProcessSnapshot {
    value: Option<Arc<ProcessSnapshot>>,
    loaded_at: Option<Instant>,
}

pub(crate) type SharedSnapshotCache = Arc<Mutex<CachedProcessSnapshot>>;

impl SystemServiceProbe {
    pub(crate) fn with_shared_cache(declared: Vec<String>, shared: SharedSnapshotCache) -> Self {
        Self {
            declared,
            load: platform::process_snapshot,
            shared,
        }
    }

    #[cfg(test)]
    fn with_loader(declared: Vec<String>, load: fn() -> Option<ProcessSnapshot>) -> Self {
        Self::with_loader_and_cache(declared, load, SharedSnapshotCache::default())
    }

    #[cfg(test)]
    fn with_loader_and_cache(
        declared: Vec<String>,
        load: fn() -> Option<ProcessSnapshot>,
        shared: SharedSnapshotCache,
    ) -> Self {
        Self {
            declared,
            load,
            shared,
        }
    }

    fn declares(&self, pane: &Pane) -> bool {
        if self.declared.is_empty() {
            return false;
        }
        let cmd_match = pane
            .reported_cmd
            .as_deref()
            .map(str::trim)
            .is_some_and(|c| self.declared.iter().any(|d| d == c));
        cmd_match
            || pane
                .foreground_basenames
                .iter()
                .any(|b| self.declared.iter().any(|d| d == b))
    }

    fn subtree_listens(&self, pane: &Pane) -> bool {
        let Some(snapshot) = self.process_snapshot() else {
            return false;
        };
        let mut stack: Vec<i32> = pane.foreground_pids.clone();
        stack.push(pane.root_pid);
        let mut seen = HashSet::new();
        while let Some(pid) = stack.pop() {
            if !seen.insert(pid) {
                continue;
            }
            if snapshot.listeners.contains(&pid) {
                return true;
            }
            if let Some(kids) = snapshot.children.get(&pid) {
                stack.extend(kids);
            }
        }
        false
    }

    fn process_snapshot(&self) -> Option<Arc<ProcessSnapshot>> {
        let mut cache = self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if snapshot_fresh(cache.loaded_at, Instant::now()) {
            qol_runtime::probe!(
                "CLI_SESSIONS_RECON",
                "phase=service_probe cache=hit outcome={} age_ms={}",
                if cache.value.is_some() {
                    "available"
                } else {
                    "unavailable"
                },
                cache
                    .loaded_at
                    .map_or(0_u128, |loaded_at| loaded_at.elapsed().as_millis()),
            );
            return cache.value.clone();
        }
        let fresh = (self.load)();
        let outcome = if fresh.is_some() {
            cache.value = fresh.map(Arc::new);
            cache.loaded_at = Some(Instant::now());
            "available"
        } else {
            "transient_failure"
        };
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=service_probe cache=load outcome={outcome} age_ms=0"
        );
        cache.value.clone()
    }
}

fn snapshot_fresh(loaded_at: Option<Instant>, now: Instant) -> bool {
    loaded_at.is_some_and(|at| now.saturating_duration_since(at) < SNAPSHOT_TTL)
}

impl ServiceProbe for SystemServiceProbe {
    fn is_service(&self, pane: &Pane) -> bool {
        self.declares(pane) || self.subtree_listens(pane)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::host::kitty_session_id;

    static LOADS: AtomicUsize = AtomicUsize::new(0);
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn counted_snapshot() -> Option<ProcessSnapshot> {
        LOADS.fetch_add(1, Ordering::SeqCst);
        Some(ProcessSnapshot {
            listeners: HashSet::from([44]),
            children: HashMap::from([(10, vec![44])]),
        })
    }

    fn listening_snapshot() -> Option<ProcessSnapshot> {
        Some(ProcessSnapshot {
            listeners: HashSet::from([44]),
            children: HashMap::from([(10, vec![44])]),
        })
    }

    fn stopped_snapshot() -> Option<ProcessSnapshot> {
        Some(ProcessSnapshot::default())
    }

    fn transient_failure() -> Option<ProcessSnapshot> {
        None
    }

    fn reset_loads() {
        LOADS.store(0, Ordering::SeqCst);
    }

    fn load_count() -> usize {
        LOADS.load(Ordering::SeqCst)
    }

    fn pane(root_pid: i32, command: &str, foreground_pids: Vec<i32>) -> Pane {
        Pane {
            id: kitty_session_id(1),
            root_pid,
            cwd: "/tmp".into(),
            title: "test".into(),
            at_prompt: false,
            reported_cmd: Some(command.into()),
            foreground_basenames: vec![command.into()],
            foreground_pids,
            capabilities: qol_terminal_sessions::SessionCapabilities::ALL,
        }
    }

    #[test]
    fn construction_does_not_load_process_snapshot() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_loads();
        let _probe = SystemServiceProbe::with_loader(Vec::new(), counted_snapshot);
        assert_eq!(load_count(), 0);
    }

    #[test]
    fn declared_command_does_not_load_process_snapshot() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_loads();
        let probe = SystemServiceProbe::with_loader(vec!["qol dev".into()], counted_snapshot);
        assert!(probe.is_service(&pane(10, "qol dev", Vec::new())));
        assert_eq!(load_count(), 0);
    }

    #[test]
    fn shared_cache_reuses_fresh_snapshot_across_probes() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_loads();
        let shared = SharedSnapshotCache::default();
        let first =
            SystemServiceProbe::with_loader_and_cache(Vec::new(), counted_snapshot, shared.clone());
        let second =
            SystemServiceProbe::with_loader_and_cache(Vec::new(), counted_snapshot, shared.clone());
        let pane = pane(10, "server", Vec::new());
        assert!(first.is_service(&pane));
        assert!(second.is_service(&pane));
        assert_eq!(
            load_count(),
            1,
            "second probe must reuse the fresh snapshot"
        );
    }

    #[test]
    fn shared_cache_retries_failed_load_on_next_probe() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_loads();
        let shared = SharedSnapshotCache::default();
        let pane = pane(10, "server", Vec::new());
        let failing = SystemServiceProbe::with_loader_and_cache(
            Vec::new(),
            transient_failure,
            shared.clone(),
        );
        assert!(
            !failing.is_service(&pane),
            "failed load must not report a service"
        );
        let recovered =
            SystemServiceProbe::with_loader_and_cache(Vec::new(), counted_snapshot, shared.clone());
        assert!(
            recovered.is_service(&pane),
            "next probe must retry the load"
        );
    }

    #[test]
    fn snapshot_freshness_respects_ttl() {
        let now = Instant::now();
        assert!(!snapshot_fresh(None, now));
        assert!(snapshot_fresh(Some(now - Duration::from_secs(29)), now));
        assert!(!snapshot_fresh(Some(now - Duration::from_secs(31)), now));
    }

    #[test]
    fn process_snapshot_loads_once_for_subtree_checks() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_loads();
        let probe = SystemServiceProbe::with_loader(Vec::new(), counted_snapshot);
        assert!(probe.is_service(&pane(10, "server", Vec::new())));
        assert!(probe.is_service(&pane(10, "server", Vec::new())));
        assert_eq!(load_count(), 1);
    }

    #[test]
    fn service_start_and_stop_transitions_are_seen_across_passes() {
        let _guard = TEST_LOCK.lock().unwrap();
        let stopped = SystemServiceProbe::with_loader(Vec::new(), stopped_snapshot);
        let started = SystemServiceProbe::with_loader(Vec::new(), listening_snapshot);
        let stopped_again = SystemServiceProbe::with_loader(Vec::new(), stopped_snapshot);
        let pane = pane(10, "server", Vec::new());

        assert!(!stopped.is_service(&pane));
        assert!(started.is_service(&pane));
        assert!(!stopped_again.is_service(&pane));
    }

    #[test]
    fn transient_process_probe_failure_is_retried_on_next_pass() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_loads();
        let failed = SystemServiceProbe::with_loader(Vec::new(), transient_failure);
        let recovered = SystemServiceProbe::with_loader(Vec::new(), counted_snapshot);
        let pane = pane(10, "server", Vec::new());

        assert!(!failed.is_service(&pane));
        assert!(recovered.is_service(&pane));
        assert_eq!(load_count(), 1);
    }

    #[test]
    fn foreground_process_can_match_listener_directly() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_loads();
        let probe = SystemServiceProbe::with_loader(Vec::new(), counted_snapshot);
        assert!(probe.is_service(&pane(10, "server", vec![44])));
        assert_eq!(load_count(), 1);
    }
}
