use std::process::Command;

use super::search;

pub fn launch_item(item: &search::ResultItem<'_>) {
    match item {
        search::ResultItem::App(entry) => spawn_detached(&entry.exec),
        search::ResultItem::File(entry) => open_path_detached(&entry.path),
    };
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
