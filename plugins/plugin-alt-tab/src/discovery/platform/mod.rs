use super::WindowInfo;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(target_os = "windows")]
mod imp {
    use super::WindowInfo;
    pub fn get_open_windows() -> Vec<WindowInfo> {
        Vec::new()
    }
    pub fn get_on_screen_windows() -> Vec<WindowInfo> {
        Vec::new()
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("plugin-alt-tab discovery: unsupported target OS");

pub fn get_open_windows() -> Vec<WindowInfo> {
    imp::get_open_windows()
}
pub fn get_on_screen_windows() -> Vec<WindowInfo> {
    imp::get_on_screen_windows()
}
