#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("launch platform implementation is required for this target OS");

use std::path::Path;

use crate::discovery::search;

pub fn launch_app(path: &Path, exec: &[String]) -> bool {
    imp::launch_app(path, exec)
}

pub fn open_path(path: &Path) -> bool {
    imp::open_path(path)
}

pub fn launch_item(item: &search::ResultItem<'_>) -> bool {
    match item {
        search::ResultItem::App(entry) => {
            eprintln!("[launch] app: {:?} exec: {:?}", entry.name, entry.exec);
            launch_app(&entry.path, &entry.exec)
        }
        search::ResultItem::File(entry) => open_path(&entry.path),
    }
}
