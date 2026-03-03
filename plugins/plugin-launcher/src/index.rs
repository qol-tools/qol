use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use crate::launcher_app::{PreloadedEntries, SharedEntries};
use crate::providers::{apps, files};

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

        let roots = collect_roots();
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

fn collect_roots() -> Vec<PathBuf> {
    let mut roots = apps::watch_roots();
    roots.extend(files::watch_roots());
    roots.sort();
    roots.dedup();
    roots.retain(|r| r.is_dir());
    roots
}
