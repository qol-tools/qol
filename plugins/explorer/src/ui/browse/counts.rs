use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use qol_watch::{Watch, WatchNotice, WatchRoot};

use super::REFRESH_QUIET;
use crate::listing::{self, FolderCount, ListingEvent, ListingRead};
use crate::places;

pub(super) const FOLLOWED_FOLDERS: usize = 1024;
const CLOCK_SLACK: Duration = Duration::from_secs(2);

type Stale = Arc<Mutex<HashSet<PathBuf>>>;

struct Recount {
    read: ListingRead,
    pending: HashSet<PathBuf>,
}

#[derive(Default)]
pub(super) struct LiveCounts {
    folders: HashSet<PathBuf>,
    watch: Arc<Mutex<Option<Watch>>>,
    stale: Stale,
    stale_at: Option<Instant>,
    recounts: Vec<Recount>,
}

impl LiveCounts {
    pub(super) fn follow(
        &mut self,
        folders: Vec<PathBuf>,
        counted_from: SystemTime,
        wake: impl Fn() + Send + Clone + 'static,
    ) {
        self.watch = Arc::default();
        self.folders = folders.iter().cloned().collect();
        let watched = folders;
        if watched.is_empty() {
            return;
        }
        let slot = Arc::downgrade(&self.watch);
        let stale = Arc::clone(&self.stale);
        let spawned = thread::Builder::new()
            .name("qol-explorer-counts".into())
            .spawn(move || watch_counts(&watched, counted_from, &slot, stale, wake));
        if let Err(error) = spawned {
            log::warn!("folder counts will not follow changes: {error}");
        }
    }

    #[cfg(test)]
    pub(super) fn following(&self) -> bool {
        lock(&self.watch).is_some()
    }

    #[cfg(test)]
    pub(super) fn recounting(&self) -> usize {
        self.recounts.len()
    }

    #[cfg(test)]
    pub(super) fn is_stale(&self, folder: &Path) -> bool {
        lock(&self.stale).contains(folder)
    }

    pub(super) fn drain(&mut self) -> Vec<FolderCount> {
        let mut counts = Vec::new();
        self.recounts.retain_mut(|recount| loop {
            match recount.read.events().try_recv() {
                Ok(ListingEvent::Counts(batch)) => {
                    for count in &batch {
                        recount.pending.remove(&count.path);
                    }
                    counts.extend(batch);
                }
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
        if self.stale_at.is_none() && !self.ready().is_empty() {
            self.stale_at = Some(now);
        }
        let due = self.stale_at? + REFRESH_QUIET;
        if now < due {
            return Some(due - now);
        }
        self.stale_at = None;
        let ready = self.ready();
        let mut stale = lock(&self.stale);
        for path in &ready {
            stale.remove(path);
        }
        drop(stale);
        let folders: Vec<PathBuf> = ready
            .into_iter()
            .filter(|path| self.folders.contains(path))
            .collect();
        if !folders.is_empty() {
            let pending = folders.iter().cloned().collect();
            let read = listing::recount(folder.to_path_buf(), folders, wake);
            self.recounts.push(Recount { read, pending });
        }
        None
    }

    fn ready(&self) -> Vec<PathBuf> {
        let counting: HashSet<&PathBuf> = self
            .recounts
            .iter()
            .flat_map(|recount| &recount.pending)
            .collect();
        lock(&self.stale)
            .iter()
            .filter(|path| !counting.contains(path))
            .cloned()
            .collect()
    }
}

fn watch_counts(
    folders: &[PathBuf],
    counted_from: SystemTime,
    slot: &Weak<Mutex<Option<Watch>>>,
    stale: Stale,
    wake: impl Fn() + Send + Clone + 'static,
) {
    let same_folders = same_folders(folders);
    let roots: Vec<WatchRoot> = folders.iter().map(WatchRoot::shallow).collect();
    let noticed = Arc::clone(&stale);
    let notice_wake = wake.clone();
    let watched =
        qol_watch::watch(&roots, move |notice| {
            let WatchNotice::Changed(paths) = notice else {
                return;
            };
            lock(&noticed).extend(paths.iter().filter_map(|path| path.parent()).flat_map(
                |folder| {
                    same_folders
                        .get(folder)
                        .cloned()
                        .unwrap_or_else(|| vec![folder.to_path_buf()])
                },
            ));
            notice_wake();
        });
    let watch = match watched {
        Ok(watch) => watch,
        Err(error) => {
            log::debug!("folder counts will not follow changes: {error}");
            return;
        }
    };
    let Some(slot) = slot.upgrade() else {
        return;
    };
    *lock(&slot) = Some(watch);
    let unseen: Vec<PathBuf> = folders
        .iter()
        .filter(|folder| changed_since(folder, counted_from))
        .cloned()
        .collect();
    if !unseen.is_empty() {
        lock(&stale).extend(unseen);
        wake();
    }
}

fn changed_since(folder: &Path, since: SystemTime) -> bool {
    let since = since.checked_sub(CLOCK_SLACK).unwrap_or(since);
    fs::metadata(folder)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified >= since)
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
