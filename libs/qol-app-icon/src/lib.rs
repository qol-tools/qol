#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;

#[derive(Debug, Clone)]
pub struct RgbaImage {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn icon_for_bundle_id(bundle_id: &str, size: usize) -> Option<RgbaImage> {
    imp::icon_for_bundle_id(bundle_id, size)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn icon_for_pid(pid: i32, size: usize) -> Option<RgbaImage> {
    imp::icon_for_pid(pid, size)
}

#[cfg(target_os = "macos")]
pub fn parent_pid(pid: i32) -> Option<i32> {
    imp::parent_pid(pid)
}

#[cfg(target_os = "macos")]
pub fn process_start_time_us(pid: i32) -> Option<u64> {
    imp::process_start_time_us(pid)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn icon_for_bundle_id(_bundle_id: &str, _size: usize) -> Option<RgbaImage> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn icon_for_pid(_pid: i32, _size: usize) -> Option<RgbaImage> {
    None
}
