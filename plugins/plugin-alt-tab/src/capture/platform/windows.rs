use crate::discovery::WindowInfo;
use qol_app_icon::RgbaImage;

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
