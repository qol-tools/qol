pub mod entry_store;
mod file_cache;
mod file_scan;
pub(crate) mod platform;
pub mod search;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::event::EventKind;
use notify::{RecursiveMode, Watcher};
use qol_gpui::protocol::{RuntimeEvent, RuntimeEventKind};
use qol_gpui::PlatformStateClient;

use platform::AppRoot;

enum WatchSignal {
    FsEvent(notify::Result<notify::Event>),
    HostHint(PathBuf),
}

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

pub fn load_file_entries() -> Vec<FileEntry> {
    let roots = platform::file_watch_roots();
    if let Some(entries) = file_cache::load(&roots) {
        return entries;
    }
    let entries = file_scan::scan_files(roots.clone());
    file_cache::store(&roots, &entries);
    entries
}

const DEBOUNCE: Duration = Duration::from_secs(2);
const RECV_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
struct AppCache {
    by_root: HashMap<PathBuf, Vec<AppEntry>>,
}

impl AppCache {
    fn rescan(&mut self, root: &AppRoot) {
        self.by_root
            .insert(root.path.clone(), platform::scan_root(root));
    }

    fn rescan_all(&mut self, roots: &[AppRoot]) {
        for root in roots {
            self.rescan(root);
        }
    }

    fn snapshot(&self) -> Vec<AppEntry> {
        let mut entries: Vec<AppEntry> = self
            .by_root
            .values()
            .flat_map(|v| v.iter().cloned())
            .collect();
        entries.sort_by_key(|e| e.name.to_lowercase());
        entries.dedup_by(|a, b| a.name.to_lowercase() == b.name.to_lowercase());
        entries
    }
}

fn publish(entries: &SharedEntries, cache: &AppCache, file_entries: &Arc<Vec<FileEntry>>) {
    let fresh = Arc::new(PreloadedEntries {
        app_entries: Arc::new(cache.snapshot()),
        file_entries: file_entries.clone(),
    });
    if let Ok(mut guard) = entries.lock() {
        guard.entries = fresh;
        guard.loaded_once = true;
    }
}

fn is_mutating_kind(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

fn is_app_relevant(path: &Path) -> bool {
    match path.extension() {
        Some(ext) => ext == "desktop",
        None => true,
    }
}

fn find_containing_root<'a>(path: &Path, roots: &'a [AppRoot]) -> Option<&'a AppRoot> {
    roots
        .iter()
        .filter(|r| path.starts_with(&r.path))
        .max_by_key(|r| r.path.as_os_str().len())
}

fn spawn_host_subscriber(tx: std::sync::mpsc::Sender<WatchSignal>) {
    std::thread::spawn(move || {
        let client = PlatformStateClient::from_env();
        let Some(mut sub) = client.subscribe(vec![RuntimeEventKind::LauncherAppsSynced]) else {
            eprintln!("[launcher] index: host subscribe failed; running on fs watcher only");
            return;
        };
        eprintln!("[launcher] index: subscribed to LauncherAppsSynced");
        while let Some(event) = sub.next_event() {
            if let RuntimeEvent::LauncherAppsSynced { dir } = event {
                if tx.send(WatchSignal::HostHint(dir)).is_err() {
                    break;
                }
            }
        }
    });
}

pub(crate) fn start(entries: SharedEntries) {
    std::thread::spawn(move || {
        let roots = platform::app_roots();
        let mut cache = AppCache::default();
        cache.rescan_all(&roots);
        let file_entries = Arc::new(load_file_entries());
        publish(&entries, &cache, &file_entries);
        eprintln!(
            "[launcher] index: initial load complete ({} roots)",
            roots.len()
        );

        if roots.is_empty() {
            eprintln!("[launcher] index: no watch roots, exiting watcher thread");
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel::<WatchSignal>();
        let fs_tx = tx.clone();
        let Ok(mut watcher) = notify::recommended_watcher(move |e| {
            let _ = fs_tx.send(WatchSignal::FsEvent(e));
        }) else {
            eprintln!("[launcher] index: failed to create watcher");
            return;
        };

        for root in &roots {
            let mode = if root.watch_recursive() {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            if let Err(e) = watcher.watch(&root.path, mode) {
                eprintln!(
                    "[launcher] index: watch failed for {}: {e}",
                    root.path.display()
                );
            }
        }

        spawn_host_subscriber(tx);

        let mut dirty: HashSet<PathBuf> = HashSet::new();
        loop {
            let timeout = if !dirty.is_empty() {
                DEBOUNCE
            } else {
                RECV_TIMEOUT
            };
            match rx.recv_timeout(timeout) {
                Ok(WatchSignal::FsEvent(Ok(event))) => {
                    if !is_mutating_kind(&event.kind) {
                        continue;
                    }
                    for path in &event.paths {
                        if !is_app_relevant(path) {
                            continue;
                        }
                        if let Some(root) = find_containing_root(path, &roots) {
                            dirty.insert(root.path.clone());
                        }
                    }
                    continue;
                }
                Ok(WatchSignal::FsEvent(Err(e))) => {
                    eprintln!("[launcher] index: watcher error: {e}");
                    continue;
                }
                Ok(WatchSignal::HostHint(dir)) => {
                    if let Some(root) = find_containing_root(&dir, &roots) {
                        dirty.insert(root.path.clone());
                        eprintln!(
                            "[launcher] index: host hint for {} -> rescan {}",
                            dir.display(),
                            root.path.display()
                        );
                    } else {
                        eprintln!(
                            "[launcher] index: host hint {} matches no root, ignoring",
                            dir.display()
                        );
                    }
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }

            if dirty.is_empty() {
                continue;
            }

            let dirty_now: Vec<PathBuf> = dirty.drain().collect();
            for dirty_path in &dirty_now {
                if let Some(root) = roots.iter().find(|r| r.path == *dirty_path) {
                    cache.rescan(root);
                }
            }
            publish(&entries, &cache, &file_entries);
            eprintln!(
                "[launcher] index: rescanned {} root(s): {}",
                dirty_now.len(),
                dirty_now
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    });
}
