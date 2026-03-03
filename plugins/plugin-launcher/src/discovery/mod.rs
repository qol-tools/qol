pub mod entry_store;
mod file_cache;
mod file_scan;
pub(crate) mod platform;
pub mod search;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

#[derive(Debug, Clone)]
pub struct AppEntry {
    pub name: String,
    pub exec: Vec<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
}

pub struct PreloadedEntries {
    pub app_entries: Arc<Vec<AppEntry>>,
    pub file_entries: Arc<Vec<FileEntry>>,
}

impl PreloadedEntries {
    pub fn empty() -> Self {
        Self {
            app_entries: Arc::new(Vec::new()),
            file_entries: Arc::new(Vec::new()),
        }
    }

    pub fn load() -> Self {
        Self {
            app_entries: Arc::new(load_app_entries()),
            file_entries: Arc::new(load_file_entries()),
        }
    }
}

pub struct SharedEntryState {
    pub entries: Arc<PreloadedEntries>,
    pub loaded_once: bool,
}

impl SharedEntryState {
    pub fn pending() -> Self {
        Self {
            entries: Arc::new(PreloadedEntries::empty()),
            loaded_once: false,
        }
    }
}

pub type SharedEntries = Arc<Mutex<SharedEntryState>>;

pub fn load_app_entries() -> Vec<AppEntry> {
    platform::load_app_entries()
}

pub fn load_file_entries() -> Vec<FileEntry> {
    let roots = platform::file_watch_roots();
    if let Some(entries) = file_cache::load(&roots) {
        return entries;
    }
    let entries = file_scan::scan_files(roots.clone());
    file_cache::store(&roots, &entries);
    entries
}

pub fn watch_roots() -> Vec<PathBuf> {
    let mut roots = platform::app_watch_roots();
    roots.extend(platform::file_watch_roots());
    roots.sort();
    roots.dedup();
    roots.retain(|r| r.is_dir());
    roots
}

const DEBOUNCE: Duration = Duration::from_secs(2);
const RECV_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) fn start(entries: SharedEntries) {
    std::thread::spawn(move || {
        let fresh = Arc::new(PreloadedEntries::load());
        if let Ok(mut guard) = entries.lock() {
            guard.entries = fresh;
            guard.loaded_once = true;
        }
        eprintln!("[launcher] index: initial load complete");

        let roots = watch_roots();
        if roots.is_empty() {
            eprintln!("[launcher] index: no watch roots, exiting watcher thread");
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(mut watcher) = notify::recommended_watcher(tx) else {
            eprintln!("[launcher] index: failed to create watcher");
            return;
        };

        for root in &roots {
            if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
                eprintln!("[launcher] index: watch failed for {}: {e}", root.display());
            }
        }
        eprintln!("[launcher] index: watching {} roots", roots.len());

        let mut dirty = false;
        loop {
            let timeout = if dirty { DEBOUNCE } else { RECV_TIMEOUT };
            match rx.recv_timeout(timeout) {
                Ok(Ok(_event)) => {
                    dirty = true;
                    continue;
                }
                Ok(Err(e)) => {
                    eprintln!("[launcher] index: watcher error: {e}");
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }

            if !dirty {
                continue;
            }
            dirty = false;

            let fresh = Arc::new(PreloadedEntries::load());
            if let Ok(mut guard) = entries.lock() {
                guard.entries = fresh;
            }
            eprintln!("[launcher] index: reloaded after fs change");
        }
    });
}
