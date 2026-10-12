use std::collections::{HashMap, HashSet};

use super::super::ProcessSnapshot;

pub(in super::super) fn process_snapshot() -> Option<ProcessSnapshot> {
    let processes = qol_process::processes().ok()?;
    if processes.is_empty() {
        return None;
    }
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for process in processes {
        let (Ok(pid), Ok(parent)) = (i32::try_from(process.pid), i32::try_from(process.parent))
        else {
            continue;
        };
        children.entry(parent).or_default().push(pid);
    }
    Some(ProcessSnapshot {
        listeners: listening_pids()?,
        children,
    })
}

fn listening_pids() -> Option<HashSet<i32>> {
    Some(
        qol_process::tcp_listeners()
            .ok()?
            .into_iter()
            .filter_map(|entry| entry.pid)
            .filter_map(|pid| i32::try_from(pid).ok())
            .collect(),
    )
}
