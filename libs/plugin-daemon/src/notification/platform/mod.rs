#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "windows", test))]
mod toast_identity;
#[cfg(any(target_os = "windows", test))]
mod toast_xml;
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

use qol_runtime::Notification;

pub(super) trait NotificationPlatform {
    fn send(&self, notification: &Notification) -> bool;
    fn os_do_not_disturb(&self) -> Option<bool>;
    fn os_banners_showing(&self) -> Option<bool>;
    fn set_os_banners(&self, showing: bool);
    fn acquire_inhibit(&self) -> Option<NotificationInhibit>;
    fn release_toast_identity(&self) -> std::io::Result<()>;
    fn install_toast_identity(&self) -> std::io::Result<()>;
    fn uninstall_toast_identity(&self) -> std::io::Result<()>;
}

pub fn send(notification: &Notification) -> bool {
    imp::Platform.send(notification)
}

pub fn send_notification(title: &str, message: &str) -> bool {
    send(&Notification::new(title, message))
}

pub fn os_do_not_disturb() -> Option<bool> {
    imp::Platform.os_do_not_disturb()
}

pub fn os_banners_showing() -> Option<bool> {
    imp::Platform.os_banners_showing()
}

pub fn set_os_banners(showing: bool) {
    imp::Platform.set_os_banners(showing)
}

pub fn acquire_inhibit() -> Option<NotificationInhibit> {
    imp::Platform.acquire_inhibit()
}

pub fn release_toast_identity() -> std::io::Result<()> {
    imp::Platform.release_toast_identity()
}

pub fn install_toast_identity() -> std::io::Result<()> {
    imp::Platform.install_toast_identity()
}

pub fn uninstall_toast_identity() -> std::io::Result<()> {
    imp::Platform.uninstall_toast_identity()
}
