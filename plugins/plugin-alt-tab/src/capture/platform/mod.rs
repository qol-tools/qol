use crate::discovery::WindowInfo;
use qol_plugin_api::app_icon::RgbaImage;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(target_os = "windows")]
mod imp {
    use super::{RgbaImage, WindowInfo};
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
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("plugin-alt-tab capture: unsupported target OS");

pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    imp::capture_previews_cg(targets, max_w, max_h)
}

pub fn get_app_icons(windows: &[WindowInfo]) -> std::collections::HashMap<String, RgbaImage> {
    imp::get_app_icons(windows)
}
