use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::{Entry, ListingError};

const BATCH_SIZE: usize = 128;
const BATCH_GROWTH_LIMIT: usize = 8192;
const HELD_COUNT_RETRY: Duration = Duration::from_millis(100);

static COUNTING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemCount {
    Items { all: u64, shown: u64 },
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderCount {
    pub path: PathBuf,
    pub count: ItemCount,
    pub read_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    WithCounts,
    NamesOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingEvent {
    Entries(Vec<Entry>),
    Done,
    Failed(ListingError),
    Counts(Vec<FolderCount>),
}

#[derive(Debug)]
pub struct ListingRead {
    folder: PathBuf,
    events: Receiver<ListingEvent>,
    cancel: CancelToken,
}

impl ListingRead {
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn events(&self) -> &Receiver<ListingEvent> {
        &self.events
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for ListingRead {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub fn start(folder: PathBuf, pass: Pass, wake: impl Fn() + Send + Clone + 'static) -> ListingRead {
    let worker_folder = folder.clone();
    spawn(folder, wake, move |cancel, emit| {
        run(&worker_folder, pass, BATCH_SIZE, cancel, emit);
    })
}

pub fn recount(
    folder: PathBuf,
    folders: Vec<PathBuf>,
    wake: impl Fn() + Send + Clone + 'static,
) -> ListingRead {
    spawn(folder, wake, move |cancel, mut emit| {
        count_until_free(folders.iter().collect(), BATCH_SIZE, cancel, &mut emit);
    })
}

fn spawn<Work>(folder: PathBuf, wake: impl Fn() + Send + Clone + 'static, work: Work) -> ListingRead
where
    Work: FnOnce(&CancelToken, &mut dyn FnMut(ListingEvent) -> bool) + Send + 'static,
{
    let cancel = CancelToken::new();
    let (sender, events) = mpsc::channel();
    let failure = sender.clone();
    let worker_cancel = cancel.clone();
    let worker_wake = wake.clone();
    let spawned = thread::Builder::new()
        .name("qol-explorer-listing".into())
        .spawn(move || {
            work(&worker_cancel, &mut |event| {
                let sent = sender.send(event).is_ok();
                worker_wake();
                sent
            });
        });
    if let Err(error) = spawned {
        log::error!("could not start reading {}: {error}", folder.display());
        let _ = failure.send(ListingEvent::Failed(ListingError::from_io(
            folder.clone(),
            &error,
        )));
        wake();
    }
    ListingRead {
        folder,
        events,
        cancel,
    }
}

pub(super) fn run(
    folder: &Path,
    pass: Pass,
    batch_size: usize,
    cancel: &CancelToken,
    mut emit: impl FnMut(ListingEvent) -> bool,
) {
    let Some(folders) = read_entries(folder, batch_size, cancel, &mut emit) else {
        return;
    };
    if !emit(ListingEvent::Done) || pass == Pass::NamesOnly {
        return;
    }
    count_until_free(folders.iter().collect(), batch_size, cancel, &mut emit);
}

fn count_until_free(
    mut pending: Vec<&PathBuf>,
    batch_size: usize,
    cancel: &CancelToken,
    emit: &mut impl FnMut(ListingEvent) -> bool,
) {
    while let Some(held) = count_folders(&pending, batch_size, cancel, emit) {
        if held.is_empty() {
            return;
        }
        thread::sleep(HELD_COUNT_RETRY);
        pending = held;
    }
}

fn open(folder: &Path) -> Result<fs::ReadDir, ListingError> {
    let error = |error: &std::io::Error| ListingError::from_io(folder.to_path_buf(), error);
    let metadata = fs::metadata(folder).map_err(|e| error(&e))?;
    if !metadata.is_dir() {
        return Err(error(&std::io::Error::from(
            std::io::ErrorKind::NotADirectory,
        )));
    }
    fs::read_dir(folder).map_err(|e| error(&e))
}

fn read_entries(
    folder: &Path,
    batch_size: usize,
    cancel: &CancelToken,
    emit: &mut impl FnMut(ListingEvent) -> bool,
) -> Option<Vec<PathBuf>> {
    let items = match open(folder) {
        Ok(items) => items,
        Err(error) => {
            emit(ListingEvent::Failed(error));
            return None;
        }
    };
    let real_sizes = !crate::places::is_pseudo_fs(folder);
    let mut limit = batch_size;
    let mut batch = Vec::with_capacity(limit);
    let mut folders = Vec::new();
    for item in items {
        if cancel.is_cancelled() {
            return None;
        }
        let entry = match item.and_then(|item| Entry::read(&item, real_sizes)) {
            Ok(entry) => entry,
            Err(error) => {
                log::warn!("skipped an entry in {}: {error}", folder.display());
                continue;
            }
        };
        if entry.opens_as_folder {
            folders.push(entry.path.clone());
        }
        batch.push(entry);
        if batch.len() < limit {
            continue;
        }
        if !emit(ListingEvent::Entries(mem::take(&mut batch))) {
            return None;
        }
        limit = (limit * 2).min(BATCH_GROWTH_LIMIT);
    }
    if cancel.is_cancelled() {
        return None;
    }
    if !batch.is_empty() && !emit(ListingEvent::Entries(batch)) {
        return None;
    }
    Some(folders)
}

fn count_folders<'a>(
    folders: &[&'a PathBuf],
    batch_size: usize,
    cancel: &CancelToken,
    emit: &mut impl FnMut(ListingEvent) -> bool,
) -> Option<Vec<&'a PathBuf>> {
    let mut batch = Vec::with_capacity(batch_size.min(folders.len()));
    let mut held = Vec::new();
    for &folder in folders {
        if cancel.is_cancelled() {
            return None;
        }
        let Some(_claim) = CountClaim::take(folder) else {
            held.push(folder);
            continue;
        };
        let read_at = Instant::now();
        let count = count_items(folder, cancel)?;
        batch.push(FolderCount {
            path: folder.clone(),
            count,
            read_at,
        });
        if batch.len() >= batch_size && !emit(ListingEvent::Counts(mem::take(&mut batch))) {
            return None;
        }
    }
    if !batch.is_empty() && !emit(ListingEvent::Counts(batch)) {
        return None;
    }
    Some(held)
}

pub(super) struct CountClaim(PathBuf);

impl CountClaim {
    pub(super) fn take(folder: &Path) -> Option<Self> {
        let mut counting = COUNTING.lock().unwrap_or_else(PoisonError::into_inner);
        if counting.iter().any(|busy| busy == folder) {
            return None;
        }
        counting.push(folder.to_path_buf());
        Some(Self(folder.to_path_buf()))
    }
}

impl Drop for CountClaim {
    fn drop(&mut self) {
        let mut counting = COUNTING.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(index) = counting.iter().position(|busy| *busy == self.0) {
            counting.swap_remove(index);
        }
    }
}

fn count_items(folder: &Path, cancel: &CancelToken) -> Option<ItemCount> {
    let Ok(items) = fs::read_dir(folder) else {
        return Some(ItemCount::Unreadable);
    };
    let mut all = 0;
    let mut shown = 0;
    for item in items {
        if cancel.is_cancelled() {
            return None;
        }
        let Ok(item) = item else {
            continue;
        };
        all += 1;
        if !crate::places::is_hidden_item(&item) {
            shown += 1;
        }
    }
    Some(ItemCount::Items { all, shown })
}
