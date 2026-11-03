use std::collections::BTreeSet;
use std::io;
use std::process::ExitStatus;
use std::time::Duration;

use qol_process::ProcessEntry;

const REAP_POLL: Duration = Duration::from_secs(1);

pub fn reap_in_background() {
    let children = match qol_process::processes() {
        Ok(processes) => inherited(&processes, std::process::id()),
        Err(error) => {
            log::warn!("[inherited-children] cannot list processes: {error}");
            return;
        }
    };
    if children.is_empty() {
        return;
    }
    log::info!(
        "[inherited-children] reaping {} children of the previous tray image as they exit",
        children.len()
    );
    std::thread::spawn(move || reap_until_gone(children, qol_process::try_wait_pid, REAP_POLL));
}

fn inherited(processes: &[ProcessEntry], own_pid: u32) -> BTreeSet<u32> {
    processes
        .iter()
        .filter(|process| process.parent == own_pid && process.pid != own_pid)
        .map(|process| process.pid)
        .collect()
}

fn reap_until_gone(
    mut children: BTreeSet<u32>,
    try_wait: impl Fn(u32) -> io::Result<Option<ExitStatus>>,
    poll: Duration,
) {
    loop {
        children.retain(|&pid| matches!(try_wait(pid), Ok(None)));
        if children.is_empty() {
            return;
        }
        std::thread::sleep(poll);
    }
}

#[cfg(test)]
mod tests;
