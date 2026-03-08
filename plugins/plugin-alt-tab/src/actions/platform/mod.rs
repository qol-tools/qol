#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(target_os = "windows")]
mod imp {
    pub fn activate_window(_window_id: u32) {}
    pub fn close_window(_window_id: u32) {}
    pub fn quit_app(_window_id: u32) {}
    pub fn minimize_window_by_id(_window_id: u32) {}
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("plugin-alt-tab actions: unsupported target OS");

pub fn activate_window(window_id: u32) {
    imp::activate_window(window_id)
}
pub fn close_window(window_id: u32) {
    imp::close_window(window_id)
}
pub fn quit_app(window_id: u32) {
    imp::quit_app(window_id)
}
pub fn minimize_window_by_id(window_id: u32) {
    imp::minimize_window_by_id(window_id)
}
