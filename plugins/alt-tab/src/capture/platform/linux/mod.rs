use crate::discovery::WindowInfo;
use qol_app_icon::RgbaImage;
use std::collections::HashMap;

mod x11_snapshot;

pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    x11_snapshot::capture_previews_cg(targets, max_w, max_h)
}

pub fn get_app_icons(windows: &[WindowInfo]) -> HashMap<String, RgbaImage> {
    x11_snapshot::get_app_icons(windows)
}
