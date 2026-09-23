use super::Platform;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use fallback as active;
#[cfg(target_os = "linux")]
use linux as active;
#[cfg(target_os = "macos")]
use macos as active;

#[cfg(target_os = "linux")]
#[cfg(feature = "linux_evdev")]
pub(crate) use linux::is_wayland;

pub(crate) fn create() -> impl Platform {
    active::create()
}
