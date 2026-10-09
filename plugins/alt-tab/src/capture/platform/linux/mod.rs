use crate::discovery::WindowInfo;
use qol_app_icon::RgbaImage;
use std::collections::HashMap;

pub(crate) use super::fallback::{
    live_frame_element, live_shots_available, warm_shots_session, LiveFrame, SendCVBuf, ShotReply,
    PIXEL_FORMAT_420F,
};

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
