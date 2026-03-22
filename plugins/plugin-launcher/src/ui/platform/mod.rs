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
compile_error!("platform implementation is required for this target OS");

pub fn activate_app(cx: &mut gpui::App) {
    imp::activate_app(cx)
}

pub fn set_activation_policy() {
    imp::set_activation_policy()
}

pub use qol_platform::current_capabilities;
