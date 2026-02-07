use crate::desktop_entry::DesktopEntry;
use std::process::Command;

use super::search;

pub fn launch_selected(entries: &[DesktopEntry], query: &str, selected: usize) {
    let exec = search::filtered(entries, query)
        .get(selected)
        .map(|s| s.entry.exec.clone());

    if let Some(exec) = exec {
        spawn_detached(&exec);
    }
}

fn spawn_detached(exec: &str) {
    let mut parts = exec.split_whitespace();
    let Some(cmd) = parts.next() else { return };
    let _ = Command::new(cmd).args(parts).spawn();
}
