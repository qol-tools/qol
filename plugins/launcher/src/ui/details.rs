use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui::{AppContext as _, AsyncApp, Context, WeakEntity};

use super::LauncherView;
use crate::discovery::details::{self, AppDetails, FileDetails};
use crate::discovery::AppEntry;

pub(super) enum Want {
    App(AppEntry),
    File(PathBuf),
}

enum Found {
    App(PathBuf, Box<AppDetails>),
    File(PathBuf, FileDetails),
}

#[derive(Default)]
pub(super) struct DetailCache {
    apps: HashMap<PathBuf, AppDetails>,
    files: HashMap<PathBuf, FileDetails>,
    loading_apps: HashSet<PathBuf>,
    loading_files: HashSet<PathBuf>,
}

impl DetailCache {
    pub(super) fn app(&self, path: &Path) -> Option<&AppDetails> {
        self.apps.get(path)
    }

    pub(super) fn file(&self, path: &Path) -> Option<&FileDetails> {
        self.files.get(path)
    }

    pub(super) fn forget_files(&mut self) {
        self.files.clear();
    }

    fn claim(&mut self, wants: Vec<Want>) -> Vec<Want> {
        wants
            .into_iter()
            .filter(|want| match want {
                Want::App(entry) => {
                    !self.apps.contains_key(&entry.path)
                        && self.loading_apps.insert(entry.path.clone())
                }
                Want::File(path) => {
                    !self.files.contains_key(path) && self.loading_files.insert(path.clone())
                }
            })
            .collect()
    }

    fn store(&mut self, found: Vec<Found>) {
        for found in found {
            match found {
                Found::App(path, details) => {
                    self.loading_apps.remove(&path);
                    self.apps.insert(path, *details);
                }
                Found::File(path, details) => {
                    self.loading_files.remove(&path);
                    self.files.insert(path, details);
                }
            }
        }
    }
}

fn resolve(wants: Vec<Want>) -> Vec<Found> {
    let now = SystemTime::now();
    wants
        .into_iter()
        .map(|want| match want {
            Want::App(entry) => {
                let details = details::app_details(&entry);
                Found::App(entry.path, Box::new(details))
            }
            Want::File(path) => {
                let details = details::file_details(&path, now);
                Found::File(path, details)
            }
        })
        .collect()
}

impl LauncherView {
    pub(super) fn load_details(&mut self, wants: Vec<Want>, cx: &mut Context<Self>) {
        let wants = self.details.claim(wants);
        if wants.is_empty() {
            return;
        }
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let found = async_cx
                    .background_spawn(async move { resolve(wants) })
                    .await;
                this.update(&mut async_cx, |view, cx| {
                    view.details.store(found);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn load_recent_files(&mut self, cx: &mut Context<Self>) {
        cx.spawn(|this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let recent = async_cx
                    .background_spawn(async { crate::discovery::recent_files() })
                    .await;
                this.update(&mut async_cx, |view, cx| {
                    view.store.set_recent_files(recent);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_claimed_once_until_its_details_arrive() {
        let mut cache = DetailCache::default();
        let path = PathBuf::from("/tmp/notes.md");
        assert_eq!(cache.claim(vec![Want::File(path.clone())]).len(), 1);
        assert!(cache.claim(vec![Want::File(path.clone())]).is_empty());
        cache.store(vec![Found::File(path.clone(), FileDetails::default())]);
        assert!(cache.file(&path).is_some());
        assert!(cache.claim(vec![Want::File(path.clone())]).is_empty());
        cache.forget_files();
        assert_eq!(cache.claim(vec![Want::File(path)]).len(), 1);
    }
}
