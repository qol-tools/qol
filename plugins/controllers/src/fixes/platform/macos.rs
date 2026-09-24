use super::super::state::SystemPaths;
use super::fallback;
use super::FixPlatform;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub(super) struct Platform;

impl FixPlatform for Platform {
    fn system_paths() -> SystemPaths {
        fallback::system_paths()
    }

    fn live_quirk_path(driver: &str) -> Option<String> {
        fallback::live_quirk_path(driver)
    }

    fn authorization_available() -> bool {
        fallback::authorization_available()
    }

    fn apply(conf: &str, writes: &[(String, String)]) -> Result<()> {
        fallback::apply(conf, writes)
    }

    fn hidraw_guard_path() -> Option<PathBuf> {
        fallback::hidraw_guard_path()
    }

    fn install_hidraw_guard(path: &Path, content: &str) -> Result<()> {
        fallback::install_hidraw_guard(path, content)
    }
}
