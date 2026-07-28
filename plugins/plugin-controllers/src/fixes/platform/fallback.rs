use super::super::state::SystemPaths;
use anyhow::{bail, Result};

pub(super) struct Platform;

pub(super) fn system_paths() -> SystemPaths {
    SystemPaths {
        modprobe_dir: None,
        sys_module_dir: None,
    }
}

pub(super) fn live_quirk_path(_driver: &str) -> Option<String> {
    None
}

pub(super) fn authorization_available() -> bool {
    false
}

pub(super) fn apply(_conf: &str, _writes: &[(String, String)]) -> Result<()> {
    bail!("controller driver fixes are only supported on Linux")
}

impl super::FixPlatform for Platform {
    fn system_paths() -> SystemPaths {
        self::system_paths()
    }

    fn live_quirk_path(driver: &str) -> Option<String> {
        self::live_quirk_path(driver)
    }

    fn authorization_available() -> bool {
        self::authorization_available()
    }

    fn apply(conf: &str, writes: &[(String, String)]) -> Result<()> {
        self::apply(conf, writes)
    }
}
