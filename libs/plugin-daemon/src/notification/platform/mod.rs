#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use fallback::NotificationInhibit;
#[cfg(target_os = "linux")]
pub use linux::NotificationInhibit;
#[cfg(target_os = "macos")]
pub use macos::NotificationInhibit;
#[cfg(target_os = "windows")]
pub use windows::NotificationInhibit;

pub(super) trait NotificationPlatform {
    fn send_notification(&self, title: &str, message: &str) -> bool;
    fn os_do_not_disturb(&self) -> Option<bool>;
    fn acquire_inhibit(&self) -> Option<NotificationInhibit>;
}

pub(super) fn send_notification(title: &str, message: &str) -> bool {
    imp::Platform.send_notification(title, message)
}

pub fn os_do_not_disturb() -> Option<bool> {
    imp::Platform.os_do_not_disturb()
}

pub fn acquire_inhibit() -> Option<NotificationInhibit> {
    imp::Platform.acquire_inhibit()
}
