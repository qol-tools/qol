use std::path::{Path, PathBuf};

mod platform;

#[derive(Debug, Clone)]
pub struct RgbaImage {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: i32,
    pub parent_pid: i32,
    pub name: String,
}

pub fn icon_for_bundle_id(bundle_id: &str, size: usize) -> Option<RgbaImage> {
    platform::icon_for_bundle_id(bundle_id, size)
}

pub fn icon_png_for_path(path: &Path, size: usize) -> Option<Vec<u8>> {
    platform::icon_png_for_path(path, size)
}

pub fn icon_png_for_bundle_id(bundle_id: &str, size: usize) -> Option<Vec<u8>> {
    platform::icon_png_for_bundle_id(bundle_id, size)
}

pub fn icon_for_pid(pid: i32, size: usize) -> Option<RgbaImage> {
    platform::icon_for_pid(pid, size)
}

pub fn app_display_name(app_id: &str) -> Option<String> {
    platform::app_display_name(app_id)
}

pub fn parent_pid(pid: i32) -> Option<i32> {
    platform::parent_pid(pid)
}

pub fn process_start_time_us(pid: i32) -> Option<u64> {
    platform::process_start_time_us(pid)
}

pub fn process_executable(pid: i32) -> Option<PathBuf> {
    platform::process_executable(pid)
}

pub fn processes() -> Vec<ProcessEntry> {
    platform::processes()
}
