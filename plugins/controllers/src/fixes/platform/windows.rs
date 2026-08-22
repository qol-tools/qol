use super::super::state::SystemPaths;
use super::fallback;
use super::FixPlatform;
use anyhow::Result;

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
}
