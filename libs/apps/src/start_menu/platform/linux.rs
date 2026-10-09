use std::path::PathBuf;

use crate::AppRoot;

use super::StartMenuPlatform;

pub(super) struct Platform;

impl StartMenuPlatform for Platform {
    fn program_roots(&self) -> Vec<AppRoot> {
        Vec::new()
    }

    fn shortcut_paths(&self, _root: &AppRoot) -> Vec<PathBuf> {
        Vec::new()
    }
}
