use crate::discovery::WindowInfo;
use qol_plugin_api::app_icon::RgbaImage;

pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    _max_w: usize,
    _max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    targets.iter().map(|&(idx, _)| (idx, None)).collect()
}

pub fn get_app_icons(windows: &[WindowInfo]) -> std::collections::HashMap<String, RgbaImage> {
    let mut icons = std::collections::HashMap::new();
    for win in windows {
        if icons.contains_key(&win.app_name) {
            continue;
        }
        if let Some(ref icon) = win.icon {
            icons.insert(win.app_name.clone(), icon.clone());
        }
    }
    icons
}
