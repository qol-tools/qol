use std::path::PathBuf;

use crate::AppRoot;

use super::DesktopPlatform;

pub(super) struct Platform;

impl DesktopPlatform for Platform {
    fn cache_dir(&self) -> Option<PathBuf> {
        None
    }

    fn app_roots(&self) -> Vec<AppRoot> {
        Vec::new()
    }
}
