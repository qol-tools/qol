use crate::discovery::WindowInfo;
use qol_app_icon::RgbaImage;

pub(crate) use super::fallback::{
    live_frame_element, live_shots_available, warm_shots_session, LiveFrame, SendCVBuf, ShotReply,
    PIXEL_FORMAT_420F,
};

pub fn capture_previews_cg(
    _targets: &[(usize, u32)],
    _max_w: usize,
    _max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    Vec::new()
}

pub fn get_app_icons(_windows: &[WindowInfo]) -> std::collections::HashMap<String, RgbaImage> {
    std::collections::HashMap::new()
}
