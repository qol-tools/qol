use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use qol_watch::{Watch, WatchError, WatchNotice, WatchRoot};

const FOLLOWED_LINK_FOLDERS: usize = 64;
const FOLLOWED_LINKS: usize = 1024;

#[derive(Default)]
pub(super) struct FolderWatch {
    watch: Option<Watch>,
    chain: Option<Watch>,
    links: Arc<Mutex<Option<FollowedLinks>>>,
    watched: Option<PathBuf>,
    changed: Arc<AtomicBool>,
    gone: Arc<AtomicBool>,
}

struct FollowedLinks {
    targets: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    _watch: Watch,
}

impl FolderWatch {
    pub(super) fn arm(
        &mut self,
        folder: &Path,
        wake: impl Fn() + Send + Clone + 'static,
    ) -> Result<(), WatchError> {
        self.changed.store(false, Ordering::Relaxed);
        self.links = Arc::default();
        self.watch_nearest(folder, wake)
    }

    pub(super) fn rearm(
        &mut self,
        folder: &Path,
        failed: bool,
        wake: impl Fn() + Send + Clone + 'static,
    ) -> Result<(), WatchError> {
        let elsewhere = !self.watches(folder);
        if failed || elsewhere || self.gone.swap(false, Ordering::Relaxed) {
            return self.watch_nearest(folder, wake);
        }
        Ok(())
    }

    pub(super) fn watches(&self, folder: &Path) -> bool {
        self.watched.as_deref() == Some(folder)
    }

    pub(super) fn changed(&self) -> bool {
        self.changed.swap(false, Ordering::Relaxed)
    }

    pub(super) fn follow_links(
        &mut self,
        folder: &Path,
        links: impl IntoIterator<Item = PathBuf>,
        wake: impl Fn() + Send + 'static,
    ) {
        let links: Vec<PathBuf> = links.into_iter().take(FOLLOWED_LINKS).collect();
        let kept = lock(&self.links).take().filter(|_| !links.is_empty());
        self.links = Arc::new(Mutex::new(kept));
        if links.is_empty() {
            return;
        }
        let slot = Arc::downgrade(&self.links);
        let mark = self.mark(wake);
        let folder = fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
        let spawned = thread::Builder::new()
            .name("qol-explorer-links".into())
            .spawn(move || {
                let targets = link_targets(&folder, &links);
                let Some(slot) = slot.upgrade() else {
                    return;
                };
                let kept = lock(&slot)
                    .as_ref()
                    .is_some_and(|followed| followed.targets == targets);
                if !kept {
                    let followed = watch_links(targets, mark);
                    *lock(&slot) = followed;
                }
            });
        if let Err(error) = spawned {
            *lock(&self.links) = None;
            log::warn!("linked files will not be followed: {error}");
        }
    }

    #[cfg(all(test, unix))]
    pub(super) fn follows_links(&self) -> bool {
        lock(&self.links).is_some()
    }

    fn watch_nearest(
        &mut self,
        folder: &Path,
        wake: impl Fn() + Send + Clone + 'static,
    ) -> Result<(), WatchError> {
        self.watch = None;
        self.chain = None;
        self.watched = None;
        self.gone.store(false, Ordering::Relaxed);
        let mut missed = Ok(());
        for root in folder.ancestors() {
            match self.watch_root(root, wake.clone()) {
                Ok(watch) => {
                    self.watch = Some(watch);
                    self.watched = Some(root.to_path_buf());
                    if root == folder {
                        self.chain = self.watch_chain(folder, wake.clone());
                    }
                    return missed;
                }
                Err(error) if root == folder => missed = Err(error),
                Err(_) => {}
            }
        }
        missed
    }

    fn watch_root(
        &self,
        root: &Path,
        wake: impl Fn() + Send + 'static,
    ) -> Result<Watch, WatchError> {
        let mark = self.mark(wake);
        let watched = root.to_path_buf();
        qol_watch::watch(&[WatchRoot::shallow(root)], move |notice| {
            mark(matches!(&notice, WatchNotice::Changed(paths) if paths.contains(&watched)));
        })
    }

    fn watch_chain(&self, folder: &Path, wake: impl Fn() + Send + 'static) -> Option<Watch> {
        let mut parents = Vec::new();
        let mut steps = HashSet::new();
        for step in folder.ancestors() {
            let (Some(parent), Some(name)) = (step.parent(), step.file_name()) else {
                continue;
            };
            let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
            steps.insert(parent.join(name));
            if !parents.contains(&parent) {
                parents.push(parent);
            }
        }
        let roots: Vec<WatchRoot> = parents.into_iter().map(WatchRoot::shallow).collect();
        let mark = self.mark(wake);
        qol_watch::watch(&roots, move |notice| {
            if let WatchNotice::Changed(paths) = notice {
                if paths.iter().any(|path| steps.contains(path)) {
                    mark(true);
                }
            }
        })
        .ok()
    }

    fn mark(&self, wake: impl Fn() + Send + 'static) -> impl Fn(bool) + Send + 'static {
        let changed = self.changed.clone();
        let gone = self.gone.clone();
        move |vanished| {
            if vanished {
                gone.store(true, Ordering::Relaxed);
            }
            changed.store(true, Ordering::Relaxed);
            wake();
        }
    }
}

fn watch_links(
    targets: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    mark: impl Fn(bool) + Send + 'static,
) -> Option<FollowedLinks> {
    if targets.is_empty() {
        return None;
    }
    let roots: Vec<WatchRoot> = targets.keys().map(WatchRoot::shallow).collect();
    let files: HashSet<PathBuf> = targets.values().flatten().cloned().collect();
    let watched = qol_watch::watch(&roots, move |notice| {
        if let WatchNotice::Changed(paths) = notice {
            if paths.iter().any(|path| files.contains(path)) {
                mark(false);
            }
        }
    });
    match watched {
        Ok(watch) => Some(FollowedLinks {
            targets,
            _watch: watch,
        }),
        Err(error) => {
            log::debug!("linked files will not be followed: {error}");
            None
        }
    }
}

fn link_targets(folder: &Path, links: &[PathBuf]) -> BTreeMap<PathBuf, BTreeSet<PathBuf>> {
    let mut targets: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    for link in links {
        let Some(target) = link_target(link) else {
            continue;
        };
        let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
            continue;
        };
        if parent == folder
            || (!targets.contains_key(parent) && targets.len() >= FOLLOWED_LINK_FOLDERS)
        {
            continue;
        }
        targets
            .entry(parent.to_path_buf())
            .or_default()
            .insert(parent.join(name));
    }
    targets
}

fn link_target(link: &Path) -> Option<PathBuf> {
    if let Ok(target) = fs::canonicalize(link) {
        return Some(target);
    }
    let target = link.parent()?.join(fs::read_link(link).ok()?);
    let parent = fs::canonicalize(target.parent()?).ok()?;
    Some(parent.join(target.file_name()?))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
