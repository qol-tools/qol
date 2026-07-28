use crate::RgbaImage;

use super::AppIconPlatform;

pub(super) struct Platform;

impl AppIconPlatform for Platform {
    fn icon_for_bundle_id(&self, _bundle_id: &str, _size: usize) -> Option<RgbaImage> {
        None
    }

    fn icon_for_pid(&self, _pid: i32, _size: usize) -> Option<RgbaImage> {
        None
    }

    fn app_display_name(&self, app_id: &str) -> Option<String> {
        (!app_id.is_empty()).then(|| app_id.to_string())
    }

    fn parent_pid(&self, _pid: i32) -> Option<i32> {
        None
    }

    fn process_start_time_us(&self, _pid: i32) -> Option<u64> {
        None
    }
}
