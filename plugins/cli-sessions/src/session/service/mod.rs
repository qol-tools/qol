use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::Instant;

use crate::host::Pane;

mod platform;

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
    snapshot: OnceLock<CachedProcessSnapshot>,
    load: fn() -> Option<ProcessSnapshot>,
}

#[derive(Default)]
struct ProcessSnapshot {
    listeners: HashSet<i32>,
    children: HashMap<i32, Vec<i32>>,
}

struct CachedProcessSnapshot {
    value: Option<ProcessSnapshot>,
    loaded_at: Option<Instant>,
}

impl SystemServiceProbe {
    pub fn snapshot(declared: Vec<String>) -> Self {
        Self {
            declared,
            snapshot: OnceLock::new(),
            load: platform::process_snapshot,
        }
    }

    #[cfg(test)]
    fn with_loader(declared: Vec<String>, load: fn() -> Option<ProcessSnapshot>) -> Self {
        Self {
            declared,
            snapshot: OnceLock::new(),
            load,
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
        let snapshot = self.process_snapshot();
        let Some(snapshot) = snapshot.value.as_ref() else {
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

    fn process_snapshot(&self) -> &CachedProcessSnapshot {
        #[cfg(debug_assertions)]
        let was_cached = self.snapshot.get().is_some();
        let snapshot = self.snapshot.get_or_init(|| {
            let value = (self.load)();
            #[cfg(debug_assertions)]
            let outcome = if value.is_some() {
                "available"
            } else {
                "transient_failure"
            };
            #[cfg(not(debug_assertions))]
            let outcome = "unavailable";
            qol_runtime::probe!(
                "CLI_SESSIONS_RECON",
                "phase=service_probe cache=load outcome={outcome} age_ms=0"
            );
            CachedProcessSnapshot {
                value,
                loaded_at: {
                    #[cfg(debug_assertions)]
                    {
                        Some(Instant::now())
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None
                    }
                },
            }
        });
        #[cfg(debug_assertions)]
        if was_cached {
            let outcome = if snapshot.value.is_some() {
                "available"
            } else {
                "transient_failure"
            };
            let age_ms = snapshot
                .loaded_at
                .map_or(0, |loaded_at| loaded_at.elapsed().as_millis());
            qol_runtime::probe!(
                "CLI_SESSIONS_RECON",
                "phase=service_probe cache=hit outcome={outcome} age_ms={age_ms}"
            );
        }
        #[cfg(not(debug_assertions))]
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=service_probe cache=hit outcome={} age_ms={}",
            "unavailable",
            snapshot.loaded_at.map_or(0_u128, |_| 0_u128)
        );
        snapshot
    }
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
