use std::collections::{HashMap, HashSet};
use std::process::Command;

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
    listeners: HashSet<i32>,
    children: HashMap<i32, Vec<i32>>,
    declared: Vec<String>,
}

impl SystemServiceProbe {
    pub fn snapshot(declared: Vec<String>) -> Self {
        Self {
            listeners: listening_pids(),
            children: child_map(),
            declared,
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
        let mut stack: Vec<i32> = pane.foreground_pids.clone();
        stack.push(pane.root_pid);
        let mut seen = HashSet::new();
        while let Some(pid) = stack.pop() {
            if !seen.insert(pid) {
                continue;
            }
            if self.listeners.contains(&pid) {
                return true;
            }
            if let Some(kids) = self.children.get(&pid) {
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

fn listening_pids() -> HashSet<i32> {
    let Ok(out) = Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fp"])
        .output()
    else {
        return HashSet::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix('p'))
        .filter_map(|p| p.trim().parse::<i32>().ok())
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
