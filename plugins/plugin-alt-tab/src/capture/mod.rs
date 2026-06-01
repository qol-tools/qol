use crate::discovery::WindowInfo;
use qol_app_icon::RgbaImage;

mod platform;

pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    platform::capture_previews_cg(targets, max_w, max_h)
}

pub fn get_app_icons(windows: &[WindowInfo]) -> std::collections::HashMap<String, RgbaImage> {
    platform::get_app_icons(windows)
}
