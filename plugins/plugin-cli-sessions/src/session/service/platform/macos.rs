use std::collections::{HashMap, HashSet};
use std::process::Command;

use super::super::ProcessSnapshot;

pub(in super::super) fn process_snapshot() -> ProcessSnapshot {
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
