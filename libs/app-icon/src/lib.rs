mod platform;

#[derive(Debug, Clone)]
pub struct RgbaImage {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

pub fn icon_for_bundle_id(bundle_id: &str, size: usize) -> Option<RgbaImage> {
    platform::icon_for_bundle_id(bundle_id, size)
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
