use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use qol_watch::{Watch, WatchError, WatchRoot};

use crate::app::warm::WarmState;
use crate::ingest::{self, IngestRoots};

const SETTLE_WINDOW: Duration = Duration::from_millis(250);

pub struct WatchHandle {
    _watch: Watch,
}

pub fn spawn(roots: IngestRoots, state: Arc<Mutex<WarmState>>) -> Result<WatchHandle, WatchError> {
    let watch_roots: Vec<WatchRoot> = roots
        .roots
        .iter()
        .map(|root| WatchRoot::deep(root.path.clone()))
        .collect();
    let (watch, batches) = qol_watch::settled(&watch_roots, SETTLE_WINDOW)?;
    std::thread::Builder::new()
        .name("qol-memory-watch".to_owned())
        .spawn(move || drain(batches, roots, state))
        .map_err(|error| WatchError::NoWatchableRoot(error.to_string()))?;
    Ok(WatchHandle { _watch: watch })
}

fn drain(
    batches: std::sync::mpsc::Receiver<Vec<PathBuf>>,
    roots: IngestRoots,
    state: Arc<Mutex<WarmState>>,
) {
    for batch in batches {
        let paths: Vec<PathBuf> = batch
            .into_iter()
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
            .filter(|path| !ingest::is_ignored(&roots, path))
            .collect();
        if paths.is_empty() {
            continue;
        }
        let (store, compactions) = {
            let mut warm = match state.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let store = warm.store().clone();
            let compactions = match ingest::ingest_paths(&store, &roots, &paths, warm.keys()) {
                Ok(report) => {
                    if report.appended > 0 {
                        warm.invalidate_layers();
                    }
                    report.compactions
                }
                Err(error) => {
                    eprintln!("qol-memory: transcript ingest failed: {error:#}");
                    qol_runtime::probe!("QOL_MEMORY_WATCH", "event=ingest_failed error={error}");
                    0
                }
            };
            (store, compactions)
        };
        if compactions == 0 {
            continue;
        }
        match crate::distill::run(&store) {
            Ok(report) if !report.unchanged => {
                let mut warm = match state.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                warm.invalidate_layers();
            }
            Ok(_) => {}
            Err(error) if crate::distill::is_busy(&error) => {}
            Err(error) => {
                eprintln!("qol-memory: watch distill failed: {error:#}");
                qol_runtime::probe!("QOL_MEMORY_WATCH", "event=distill_failed error={error}");
            }
        }
    }
}
