use anyhow::Result;

use crate::cursor::CursorEffect;

pub trait CursorPlatform {
    fn create_effect(&self) -> Box<dyn CursorEffect>;
    fn install_signal_handlers(&self);
    fn reset_external_stop(&self);
    fn external_stop_requested(&self) -> bool;
    fn open_settings(&self) -> Result<()>;
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::Platform;
#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(target_os = "windows")]
pub use windows::Platform;
