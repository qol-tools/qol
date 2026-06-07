use crate::platform::{Platform, PlatformOps};
use anyhow::Result;

pub(crate) fn exe_name(name: &str) -> String {
    Platform.exe_name(name)
}

pub(crate) fn os_name() -> &'static str {
    Platform.os_name()
}

pub(crate) fn stop_qol_tray() -> Result<()> {
    Platform.stop_qol_tray()
}

pub(crate) fn open_url(url: &str) {
    Platform.open_url(url)
}
