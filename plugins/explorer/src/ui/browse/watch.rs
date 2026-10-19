use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use qol_watch::{Watch, WatchError, WatchNotice, WatchRoot};

#[derive(Default)]
pub(super) struct FolderWatch {
    watch: Option<Watch>,
    watched: Option<PathBuf>,
    changed: Arc<AtomicBool>,
    gone: Arc<AtomicBool>,
}

impl FolderWatch {
    pub(super) fn arm(
        &mut self,
        folder: &Path,
        wake: impl Fn() + Send + Clone + 'static,
    ) -> Result<(), WatchError> {
        self.changed.store(false, Ordering::Relaxed);
        self.watch_nearest(folder, wake)
    }

    pub(super) fn rearm(
        &mut self,
        folder: &Path,
        failed: bool,
        wake: impl Fn() + Send + Clone + 'static,
    ) -> Result<(), WatchError> {
        let elsewhere = self.watched.as_deref() != Some(folder);
        if failed || elsewhere || self.gone.swap(false, Ordering::Relaxed) {
            return self.watch_nearest(folder, wake);
        }
        Ok(())
    }

    pub(super) fn changed(&self) -> bool {
        self.changed.swap(false, Ordering::Relaxed)
    }

    fn watch_nearest(
        &mut self,
        folder: &Path,
        wake: impl Fn() + Send + Clone + 'static,
    ) -> Result<(), WatchError> {
        self.watch = None;
        self.watched = None;
        self.gone.store(false, Ordering::Relaxed);
        let mut missed = Ok(());
        for root in folder.ancestors() {
            match self.watch_root(root, wake.clone()) {
                Ok(watch) => {
                    self.watch = Some(watch);
                    self.watched = Some(root.to_path_buf());
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
        let changed = self.changed.clone();
        let gone = self.gone.clone();
        let watched = root.to_path_buf();
        qol_watch::watch(&[WatchRoot::shallow(root)], move |notice| {
            if matches!(&notice, WatchNotice::Changed(paths) if paths.contains(&watched)) {
                gone.store(true, Ordering::Relaxed);
            }
            changed.store(true, Ordering::Relaxed);
            wake();
        })
    }
}
