use crate::RgbaImage;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback::Platform;
#[cfg(target_os = "linux")]
use linux::Platform;
#[cfg(target_os = "macos")]
use macos::Platform;
#[cfg(target_os = "windows")]
use windows::Platform;

pub(super) trait AppIconPlatform {
    fn icon_for_bundle_id(&self, bundle_id: &str, size: usize) -> Option<RgbaImage>;
    fn icon_for_pid(&self, pid: i32, size: usize) -> Option<RgbaImage>;
    fn app_display_name(&self, app_id: &str) -> Option<String>;
    fn parent_pid(&self, pid: i32) -> Option<i32>;
    fn process_start_time_us(&self, pid: i32) -> Option<u64>;
}

pub(super) fn icon_for_bundle_id(bundle_id: &str, size: usize) -> Option<RgbaImage> {
    Platform.icon_for_bundle_id(bundle_id, size)
}

pub(super) fn icon_for_pid(pid: i32, size: usize) -> Option<RgbaImage> {
    Platform.icon_for_pid(pid, size)
}

pub(super) fn app_display_name(app_id: &str) -> Option<String> {
    Platform.app_display_name(app_id)
}

pub(super) fn parent_pid(pid: i32) -> Option<i32> {
    Platform.parent_pid(pid)
}

pub(super) fn process_start_time_us(pid: i32) -> Option<u64> {
    Platform.process_start_time_us(pid)
}
