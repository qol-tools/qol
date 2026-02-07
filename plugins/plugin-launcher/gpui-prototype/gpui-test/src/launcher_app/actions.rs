use crate::desktop_entry::DesktopEntry;
use std::process::Command;

use super::search;
use super::state::SearchMode;

pub fn launch_selected(entries: &[DesktopEntry], query: &str, mode: SearchMode, selected: usize) {
    let exec = search::filtered(entries, query, mode)
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
