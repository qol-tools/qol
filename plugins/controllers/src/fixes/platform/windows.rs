use super::super::state::SystemPaths;
use super::FixPlatform;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub(super) struct Platform;

impl FixPlatform for Platform {
    fn system_paths() -> SystemPaths {
        SystemPaths {
            modprobe_dir: None,
            sys_module_dir: None,
        }
    }

    fn live_quirk_path(_driver: &str) -> Option<String> {
        None
    }

    fn authorization_available() -> bool {
        false
    }

    fn apply(_conf: &str, _writes: &[(String, String)]) -> Result<()> {
        bail!(
            "controller driver fixes change Linux kernel module options; Windows drives controllers through its own HID and XInput drivers, so no fix applies"
        )
    }

    fn hidraw_guard_path() -> Option<PathBuf> {
        None
    }

    fn install_hidraw_guard(_path: &Path, _content: &str) -> Result<()> {
        bail!("the raw HID guard is a Linux udev rule; Windows has no equivalent rule to install")
    }
}
