pub(crate) use crate::platform::OpenPathOutcome;
use crate::platform::{Platform, PlatformOps};
use anyhow::Result;

pub(crate) fn exe_name(name: &str) -> String {
    Platform.exe_name(name)
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

pub(crate) fn open_path(dir: &std::path::Path) -> Result<OpenPathOutcome> {
    Platform.open_path(dir)
}

pub(crate) fn copy_to_clipboard(text: &str) -> Result<()> {
    Platform.copy_to_clipboard(text)
}

pub(crate) fn available_memory_mb() -> Option<u64> {
    Platform.available_memory_mb()
}

pub(crate) fn home_dir() -> Option<std::path::PathBuf> {
    Platform.home_dir()
}

pub(crate) fn supports_immutable_payload_build() -> bool {
    Platform.supports_immutable_payload_build()
}

pub(crate) fn open_text_file(path: &std::path::Path) -> bool {
    Platform.open_text_file(path)
}

pub(crate) fn available_cpus() -> Option<u64> {
    std::thread::available_parallelism()
        .ok()
        .and_then(|cpus| u64::try_from(cpus.get()).ok())
}
