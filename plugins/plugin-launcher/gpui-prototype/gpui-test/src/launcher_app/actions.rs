use crate::desktop_entry::DesktopEntry;
use std::process::Command;

use super::search;
use super::state::SearchMode;

pub fn launch_selected(
    app_entries: &[DesktopEntry],
    file_entries: &[search::FileEntry],
    query: &str,
    mode: SearchMode,
    selected: usize,
) {
    let filtered = search::filtered(app_entries, file_entries, query, mode);
    let Some(item) = filtered.get(selected) else {
        return;
    };

    match &item.item {
        search::ResultItem::App(entry) => spawn_detached(&entry.exec),
        search::ResultItem::File(entry) => open_path_detached(&entry.path),
    }
}

fn spawn_detached(exec: &str) {
    let mut parts = exec.split_whitespace();
    let Some(cmd) = parts.next() else { return };
    let _ = Command::new(cmd).args(parts).spawn();
}

fn open_path_detached(path: &std::path::Path) {
    #[cfg(target_os = "linux")]
    let _ = Command::new("xdg-open").arg(path).spawn();

    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(path).spawn();

    #[cfg(target_os = "windows")]
    let _ = Command::new("cmd")
        .args(["/C", "start", "", &path.to_string_lossy()])
        .spawn();
}
