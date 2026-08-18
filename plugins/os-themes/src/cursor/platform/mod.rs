use crate::cursor::CursorEffect;

pub trait CursorPlatform {
    fn create_effect(&self) -> Box<dyn CursorEffect>;
    fn install_signal_handlers(&self);
    fn reset_external_stop(&self);
    fn external_stop_requested(&self) -> bool;
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use fallback::Platform;
#[cfg(target_os = "linux")]
pub use linux::recover as recover_linux;
#[cfg(target_os = "linux")]
pub use linux::Platform;
#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(target_os = "windows")]
pub use windows::Platform;
