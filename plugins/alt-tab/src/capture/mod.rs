use crate::discovery::WindowInfo;
use crate::rendering::preview_image::shot_request_dims;
use qol_app_icon::RgbaImage;
use std::time::Duration;

mod platform;

pub(crate) use platform::{
    live_frame_element, live_shots_available, warm_shots_session, LiveFrame, SendCVBuf, ShotReply,
    PIXEL_FORMAT_420F,
};

pub(crate) fn fresh_frame_target(window: &WindowInfo) -> Option<(u32, (usize, usize))> {
    if window.is_minimized || !live_shots_available() {
        return None;
    }
    Some((window.id, shot_request_dims(window.width, window.height)))
}

pub(crate) fn capture_live_frame(wid: u32, (width, height): (usize, usize)) -> Option<SendCVBuf> {
    let session = warm_shots_session(&[wid])?;
    let (tx, rx) = std::sync::mpsc::channel();
    if !session.request_capture(wid, width, height, &tx) {
        return None;
    }
    match rx.recv_timeout(Duration::from_millis(120)) {
        Ok((reply_wid, Some(buf)))
            if reply_wid == wid && buf.pixel_format() == PIXEL_FORMAT_420F =>
        {
            Some(buf)
        }
        _ => None,
    }
}

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
