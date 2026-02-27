#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("app_icon: unsupported platform; add src/app_icon/<os>.rs");

#[derive(Debug, Clone)]
pub struct RgbaImage {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

pub fn icon_for_bundle_id(bundle_id: &str, size: usize) -> Option<RgbaImage> {
    imp::icon_for_bundle_id(bundle_id, size)
}

pub fn icon_for_pid(pid: i32, size: usize) -> Option<RgbaImage> {
    imp::icon_for_pid(pid, size)
}
