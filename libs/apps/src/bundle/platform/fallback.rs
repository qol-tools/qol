use std::path::{Path, PathBuf};

use crate::AppRoot;

use super::BundlePlatform;

pub(super) struct Platform;

impl BundlePlatform for Platform {
    fn cache_dir(&self) -> Option<PathBuf> {
        None
    }

    fn launcher_roots(&self) -> Vec<AppRoot> {
        Vec::new()
    }

    fn bundle_info(&self, _path: &Path) -> (Option<String>, Option<String>) {
        (None, None)
    }

    fn spotlight_app_paths(&self, _roots: &[PathBuf]) -> Vec<PathBuf> {
        Vec::new()
    }
}
