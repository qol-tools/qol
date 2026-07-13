use crate::platform::{Platform, PlatformOps};
use anyhow::Result;

pub(crate) fn exe_name(name: &str) -> String {
    Platform.exe_name(name)
}

pub(crate) fn os_name() -> &'static str {
    Platform.os_name()
}

pub(crate) fn force_stop_qol_tray() -> Result<()> {
    Platform.stop_qol_tray()
}

pub(crate) fn qol_tray_running() -> bool {
    Platform.qol_tray_running()
}

pub(crate) fn open_url(url: &str) {
    let _ = qol_apps::desktop_integration::open_with_default_app(url);
}

pub(crate) fn open_path(dir: &std::path::Path) {
    let _ = qol_apps::desktop_integration::open_with_default_app(dir);
}

pub(crate) fn copy_to_clipboard(text: &str) -> Result<()> {
    Platform.copy_to_clipboard(text)
}
