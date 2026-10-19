use std::collections::{HashMap, HashSet};
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use qol_watch::{Watch, WatchNotice, WatchRoot};

use super::REFRESH_QUIET;
use crate::listing::{self, FolderCount, ListingEvent, ListingRead};
use crate::places;

pub(super) const FOLLOWED_FOLDERS: usize = 1024;

type Stale = Arc<Mutex<HashSet<PathBuf>>>;

#[derive(Default)]
pub(super) struct LiveCounts {
    folders: HashSet<PathBuf>,
    watch: Arc<Mutex<Option<Watch>>>,
    stale: Stale,
    stale_at: Option<Instant>,
    reads: Vec<ListingRead>,
}

impl LiveCounts {
    pub(super) fn follow(&mut self, folders: Vec<PathBuf>, wake: impl Fn() + Send + 'static) {
        let folders: HashSet<PathBuf> = folders.into_iter().collect();
        if folders == self.folders {
            return;
        }
        self.watch = Arc::default();
        let watched: Vec<PathBuf> = folders.iter().cloned().collect();
        self.folders = folders;
        if watched.is_empty() {
            return;
        }
        let slot = Arc::downgrade(&self.watch);
        let stale = Arc::clone(&self.stale);
        let spawned = thread::Builder::new()
            .name("qol-explorer-counts".into())
            .spawn(move || watch_counts(&watched, &slot, stale, wake));
        if let Err(error) = spawned {
            log::warn!("folder counts will not follow changes: {error}");
        }
    }

    #[cfg(test)]
    pub(super) fn following(&self) -> bool {
        lock(&self.watch).is_some()
    }

    pub(super) fn drain(&mut self) -> Vec<FolderCount> {
        let mut counts = Vec::new();
        self.reads.retain(|read| loop {
            match read.events().try_recv() {
                Ok(ListingEvent::Counts(batch)) => counts.extend(batch),
                Ok(_) => {}
                Err(TryRecvError::Empty) => break true,
                Err(TryRecvError::Disconnected) => break false,
            }
        });
        counts
    }

    pub(super) fn recount_when_due(
        &mut self,
        now: Instant,
        folder: &Path,
        wake: impl Fn() + Send + Clone + 'static,
    ) -> Option<Duration> {
        if self.stale_at.is_none() && !lock(&self.stale).is_empty() {
            self.stale_at = Some(now);
        }
        let due = self.stale_at? + REFRESH_QUIET;
        if now < due {
            return Some(due - now);
        }
        self.stale_at = None;
        let stale = mem::take(&mut *lock(&self.stale));
        let folders: Vec<PathBuf> = stale
            .into_iter()
            .filter(|path| self.folders.contains(path))
            .collect();
        if !folders.is_empty() {
            self.reads
                .push(listing::recount(folder.to_path_buf(), folders, wake));
        }
        None
    }
}

fn watch_counts(
    folders: &[PathBuf],
    slot: &Weak<Mutex<Option<Watch>>>,
    stale: Stale,
    wake: impl Fn() + Send + 'static,
) {
    let same_folders = same_folders(folders);
    let roots: Vec<WatchRoot> = folders.iter().map(WatchRoot::shallow).collect();
    let watched = qol_watch::watch(&roots, move |notice| {
        let WatchNotice::Changed(paths) = notice else {
            return;
        };
        lock(&stale).extend(
            paths
                .iter()
                .filter_map(|path| path.parent())
                .flat_map(|folder| {
                    same_folders
                        .get(folder)
                        .cloned()
                        .unwrap_or_else(|| vec![folder.to_path_buf()])
                }),
        );
        wake();
    });
    match watched {
        Ok(watch) => {
            if let Some(slot) = slot.upgrade() {
                *lock(&slot) = Some(watch);
            }
        }
        Err(error) => log::debug!("folder counts will not follow changes: {error}"),
    }
}

fn same_folders(folders: &[PathBuf]) -> HashMap<PathBuf, Vec<PathBuf>> {
    let mut by_id: HashMap<(u64, u64), Vec<PathBuf>> = HashMap::new();
    for folder in folders {
        let id = fs::metadata(folder)
            .ok()
            .and_then(|metadata| places::file_id(&metadata));
        if let Some(id) = id {
            by_id.entry(id).or_default().push(folder.clone());
        }
    }
    by_id
        .into_values()
        .filter(|same| same.len() > 1)
        .flat_map(|same| {
            same.clone()
                .into_iter()
                .map(move |folder| (folder, same.clone()))
        })
        .collect()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
