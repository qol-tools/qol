use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::OnceLock;

use crate::host::Pane;

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
    snapshot: OnceLock<ProcessSnapshot>,
    load: fn() -> ProcessSnapshot,
}

struct ProcessSnapshot {
    listeners: HashSet<i32>,
    children: HashMap<i32, Vec<i32>>,
}

impl SystemServiceProbe {
    pub fn snapshot(declared: Vec<String>) -> Self {
        Self {
            declared,
            snapshot: OnceLock::new(),
            load: process_snapshot,
        }
    }

    #[cfg(test)]
    fn with_loader(declared: Vec<String>, load: fn() -> ProcessSnapshot) -> Self {
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
        let snapshot = self.snapshot.get_or_init(|| (self.load)());
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
}

impl ServiceProbe for SystemServiceProbe {
    fn is_service(&self, pane: &Pane) -> bool {
        self.declares(pane) || self.subtree_listens(pane)
    }
}

fn process_snapshot() -> ProcessSnapshot {
    ProcessSnapshot {
        listeners: listening_pids(),
        children: child_map(),
    }
}

fn listening_pids() -> HashSet<i32> {
    let Ok(out) = Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fp"])
        .output()
    else {
        return HashSet::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix('p'))
        .filter_map(|pid| pid.trim().parse::<i32>().ok())
        .collect()
}

fn child_map() -> HashMap<i32, Vec<i32>> {
    let Ok(out) = Command::new("ps").args(["-axo", "pid=,ppid="]).output() else {
        return HashMap::new();
    };
    let mut map: HashMap<i32, Vec<i32>> = HashMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(ppid)) = (it.next(), it.next()) else {
            continue;
        };
        if let (Ok(pid), Ok(ppid)) = (pid.parse::<i32>(), ppid.parse::<i32>()) {
            map.entry(ppid).or_default().push(pid);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;

    static LOADS: AtomicUsize = AtomicUsize::new(0);
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn counted_snapshot() -> ProcessSnapshot {
        LOADS.fetch_add(1, Ordering::SeqCst);
        ProcessSnapshot {
            listeners: HashSet::from([44]),
            children: HashMap::from([(10, vec![44])]),
        }
    }

    fn reset_loads() {
        LOADS.store(0, Ordering::SeqCst);
    }

    fn load_count() -> usize {
        LOADS.load(Ordering::SeqCst)
    }

    fn pane(root_pid: i32, command: &str, foreground_pids: Vec<i32>) -> Pane {
        Pane {
            window_id: 1,
            root_pid,
            cwd: "/tmp".into(),
            title: "test".into(),
            at_prompt: false,
            reported_cmd: Some(command.into()),
            foreground_basenames: vec![command.into()],
            foreground_pids,
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
    fn foreground_process_can_match_listener_directly() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_loads();
        let probe = SystemServiceProbe::with_loader(Vec::new(), counted_snapshot);
        assert!(probe.is_service(&pane(10, "server", vec![44])));
        assert_eq!(load_count(), 1);
    }
}
