use crate::discovery::WindowInfo;
use qol_app_icon::RgbaImage;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("plugin-alt-tab capture: unsupported target OS");

#[cfg(not(target_os = "macos"))]
mod shots_stub;

#[cfg(target_os = "macos")]
pub(crate) use macos::shots::{
    cached_shots_session, fetch_shots_session, live_frame_element, live_shots_available,
    warm_shots_session, LiveFrame, SendCVBuf, ShotReply, PIXEL_FORMAT_420F,
};
#[cfg(not(target_os = "macos"))]
pub(crate) use shots_stub::{
    cached_shots_session, fetch_shots_session, live_frame_element, live_shots_available,
    warm_shots_session, LiveFrame, SendCVBuf, ShotReply, PIXEL_FORMAT_420F,
};

pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    imp::capture_previews_cg(targets, max_w, max_h)
}

pub fn capture_frontmost_preview(wid: u32, max_w: usize, max_h: usize) -> Option<RgbaImage> {
    imp::capture_frontmost_preview(wid, max_w, max_h)
}

pub fn get_app_icons(windows: &[WindowInfo]) -> std::collections::HashMap<String, RgbaImage> {
    imp::get_app_icons(windows)
}
