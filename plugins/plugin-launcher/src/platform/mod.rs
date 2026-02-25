#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported {
    use std::path::Path;

    pub fn launch_app(_path: &Path, _exec: &[String]) -> bool {
        false
    }

    pub fn open_path(_path: &Path) -> bool {
        false
    }

    pub fn activate_app(cx: &mut gpui::App) {
        cx.activate(true);
    }

    pub fn set_activation_policy() {}
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use unsupported as imp;

use std::path::Path;

pub fn launch_app(path: &Path, exec: &[String]) -> bool {
    imp::launch_app(path, exec)
}

pub fn open_path(path: &Path) -> bool {
    imp::open_path(path)
}

pub fn activate_app(cx: &mut gpui::App) {
    imp::activate_app(cx)
}

pub fn set_activation_policy() {
    imp::set_activation_policy()
}

pub use qol_plugin_api::platform::{
    current_capabilities, launch_working_dir, linux_display_backend, LinuxDisplayBackend,
    PlatformCapabilities,
};
