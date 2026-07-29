use anyhow::Result;

use crate::theme::ColorScheme;

pub trait ThemePlatform {
    fn current_scheme(&self) -> Result<ColorScheme>;
    fn apply_scheme(&self, target: ColorScheme) -> Result<()>;
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(test, not(target_os = "linux")))]
#[path = "linux/desktop.rs"]
mod linux_desktop;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::Platform;
#[cfg(target_os = "linux")]
pub(crate) use linux::{classify_desktop, DesktopEnvironment};
#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(target_os = "windows")]
pub use windows::Platform;
